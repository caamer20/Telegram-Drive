//! Native backend journeys through a separate process, persistent files and real HTTP.
//! This intentionally does not stand in for full Tauri-window or live Telegram E2E.
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant, SystemTime},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "telegram-drive-native-e2e-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir(&path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        std::fs::write(
            path.join(".native-e2e-fixture"),
            "telegram-drive-synthetic-e2e\n",
        )
        .unwrap();
        Self(path)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
    fn write(&self, name: &str, bytes: impl AsRef<[u8]>) {
        std::fs::write(self.path(name), bytes).unwrap();
    }
    fn read(&self, name: &str) -> Vec<u8> {
        std::fs::read(self.path(name)).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct Backend {
    child: Child,
    input: ChildStdin,
    replies: Receiver<Value>,
    heic_pending: bool,
}
// Child readiness is setup, before any application journey clock.
const NATIVE_FIXTURE_STARTUP: Duration = Duration::from_secs(120);
impl Backend {
    fn start(root: &Path) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_native-e2e-driver"));
        if root.join(".video-upload-fixture").exists() {
            let scratch = root.join("video-upload-scratch");
            command
                .env("TMPDIR", &scratch)
                .env("TMP", &scratch)
                .env("TEMP", &scratch);
        }
        if root.join(".video-upload-empty-path").exists() {
            command.env("PATH", root.join("video-upload-scratch"));
        }
        if root.join(".video-upload-helper-path").exists() {
            command.env("PATH", root.join("video-upload-codec"));
        }
        let mut child = command
            .arg(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let (send, replies) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let Ok(line) = line else { break };
                let value = serde_json::from_str(&line)
                    .unwrap_or_else(|_| json!({"unexpectedOutput":line}));
                if send.send(value).is_err() {
                    break;
                }
            }
        });
        let instance = Self {
            child,
            input,
            replies,
            heic_pending: false,
        };
        assert_eq!(
            instance
                .replies
                .recv_timeout(NATIVE_FIXTURE_STARTUP)
                .expect("native process did not start"),
            json!({"ready":true})
        );
        instance
    }
    fn request(&mut self, request: Value) -> Value {
        self.request_before(request, Duration::from_secs(60))
    }
    fn request_before(&mut self, request: Value, deadline: Duration) -> Value {
        let command = request["command"].as_str().unwrap_or("");
        let heic = matches!(request["mime"].as_str(), Some("image/heic" | "image/heif"))
            || command == "asset_display_local"
            || (command == "asset_finish" && self.heic_pending);
        if command == "asset_start" && heic && request["display"] == true {
            self.heic_pending = true;
        }
        if command == "asset_finish" {
            self.heic_pending = false;
        }
        // Covers controlled probe, conversion readiness and the explicit fixture clock.
        // Explicit caller timeouts and every non-HEIC journey keep their existing bounds.
        let deadline = if heic && deadline == Duration::from_secs(60) {
            Duration::from_secs(480)
        } else {
            deadline
        };
        writeln!(self.input, "{request}").unwrap();
        self.input.flush().unwrap();
        self.replies.recv_timeout(deadline).unwrap_or_else(|error| {
            panic!("native process did not answer within {deadline:?}: {error}")
        })
    }
    fn ok(&mut self, request: Value) -> Value {
        self.ok_before(request, Duration::from_secs(60))
    }
    fn ok_before(&mut self, request: Value, deadline: Duration) -> Value {
        let command = request["command"].as_str().unwrap_or("unknown").to_string();
        let reply = self.request_before(request, deadline);
        let diagnostics = if reply["error"]
            .as_str()
            .is_some_and(|error| error.starts_with("HEIC_"))
        {
            self.request(json!({"command":"heic_status"}))
        } else {
            Value::Null
        };
        assert_eq!(
            reply["ok"], true,
            "command={command}; reply={reply}; diagnostics={diagnostics}"
        );
        reply["value"].clone()
    }
    fn failure(&mut self, request: Value) -> String {
        let reply = self.request(request);
        assert_eq!(reply["ok"], false, "{reply}");
        reply["error"].as_str().unwrap().to_string()
    }
    fn stop(mut self) {
        writeln!(self.input, "{}", json!({"command":"shutdown"})).unwrap();
        self.input.flush().unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success(), "backend shutdown failed: {status}");
                break;
            }
            assert!(
                Instant::now() < deadline,
                "backend did not shut down gracefully"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for Backend {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn database_upgrade_preserves_legacy_rows_and_backup_across_process_restarts() {
    let fixture = Fixture::new();
    let database = sqlite::open(fixture.path("shares.db")).unwrap();
    database.execute(concat!(
            "CREATE TABLE shared_links(id TEXT PRIMARY KEY,folder_id INTEGER,message_id INTEGER NOT ",
            "NULL,file_name TEXT NOT NULL,file_size INTEGER NOT NULL DEFAULT 0,password_hash ",
            "TEXT,password_salt TEXT,expires_at INTEGER,revoked INTEGER NOT NULL DEFAULT 0,created_at ",
            "INTEGER NOT NULL); INSERT INTO shared_links ",
            "VALUES('legacy',NULL,42,'keep-me.pdf',123,NULL,NULL,NULL,0,1)",
        )).unwrap();
    drop(database);
    let mut app = Backend::start(&fixture.0);
    assert_eq!(app.ok(json!({"command":"database"}))["version"], 4);
    app.stop();
    let backup = fixture.path("shares.db.pre-migration-v4");
    assert!(backup.is_file());
    let backup_database = sqlite::open(backup).unwrap();
    let mut query = backup_database
        .prepare("SELECT file_name,file_size FROM shared_links WHERE id='legacy'")
        .unwrap();
    assert_eq!(query.next().unwrap(), sqlite::State::Row);
    assert_eq!(query.read::<String, _>(0).unwrap(), "keep-me.pdf");
    assert_eq!(query.read::<i64, _>(1).unwrap(), 123);
    drop(query);
    let mut integrity = backup_database.prepare("PRAGMA integrity_check").unwrap();
    assert_eq!(integrity.next().unwrap(), sqlite::State::Row);
    assert_eq!(integrity.read::<String, _>(0).unwrap(), "ok");
    drop(integrity);
    drop(backup_database);
    let mut app = Backend::start(&fixture.0);
    assert_eq!(app.ok(json!({"command":"database"}))["version"], 4);
    app.stop();
    let database = sqlite::open(fixture.path("shares.db")).unwrap();
    let mut query = database
        .prepare("SELECT file_name,file_size,owner_id FROM shared_links WHERE id='legacy'")
        .unwrap();
    assert_eq!(query.next().unwrap(), sqlite::State::Row);
    assert_eq!(query.read::<String, _>(0).unwrap(), "keep-me.pdf");
    assert_eq!(query.read::<i64, _>(1).unwrap(), 123);
    assert_eq!(query.read::<Option<i64>, _>(2).unwrap(), None);
    drop(query);
    database
        .execute("INSERT INTO app_schema_migrations VALUES(999,'future','future',1,'future')")
        .unwrap();
    drop(database);
    let before = fixture.read("shares.db");
    let mut app = Backend::start(&fixture.0);
    assert!(app
        .failure(json!({"command":"database"}))
        .contains("supports up to"));
    app.stop();
    assert_eq!(fixture.read("shares.db"), before);
}

#[test]
fn account_isolation_survives_restart_contention_and_account_switch() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!({"command":"capture_account","owner":"101"}));
    app.ok(json!({"command":"save_collection","owner":"101","collection":{"id":"private","name":"Account A private collection","color":"blue","icon":"folder"}}));
    app.stop();
    let mut app = Backend::start(&fixture.0);
    assert_eq!(
        app.ok(json!({"command":"workspace","owner":"101"}))["collections"][0]["name"],
        "Account A private collection"
    );
    app.ok(json!({"command":"capture_account","owner":"101"}));
    // Hold the writer until the reader reports its bounded failure; no sleeps decide ordering.
    let writer = sqlite::open(fixture.path("telegram.session")).unwrap();
    writer
        .execute("BEGIN EXCLUSIVE; UPDATE peer_info SET peer_id=202 WHERE subtype=1")
        .unwrap();
    assert!(app
        .failure(json!({"command":"validate_account"}))
        .starts_with("ACCOUNT_UNAVAILABLE:"));
    writer.execute("COMMIT").unwrap();
    drop(writer);
    assert!(app
        .failure(json!({"command":"validate_account"}))
        .starts_with("ACCOUNT_CHANGED:"));
    assert!(app
        .failure(json!({"command":"workspace","owner":"101"}))
        .starts_with("ACCOUNT_CHANGED:"));
    assert!(
        app.ok(json!({"command":"workspace","owner":"202"}))["collections"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    app.stop();
}

#[test]
fn vault_and_envelope_survive_restart_corruption_recovery_and_passphrase_change() {
    let fixture = Fixture::new();
    let plaintext: Vec<u8> = (0..2_100_019).map(|index| (index % 251) as u8).collect();
    fixture.write("original.bin", &plaintext);
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"vault_create","passphrase":"synthetic initial vault passphrase"}));
    app.ok(json!({"command":"fixture_encrypt","source":"original.bin","destination":"protected.tdenc"}));
    app.ok(json!({"command":"vault_export","passphrase":"synthetic recovery passphrase"}));
    app.stop();
    let ciphertext = fixture.read("protected.tdenc");
    assert!(!ciphertext
        .windows(23)
        .any(|window| window == b"private-e2e-document.bin"));
    let mut app = Backend::start(&fixture.0);
    app.failure(json!({"command":"vault_unlock","passphrase":"incorrect passphrase"}));
    app.ok(json!({"command":"vault_unlock","passphrase":"synthetic initial vault passphrase"}));
    app.ok(
        json!({"command":"read_envelope","source":"protected.tdenc","destination":"restored.bin"}),
    );
    assert_eq!(fixture.read("restored.bin"), plaintext);
    let mut damaged = ciphertext.clone();
    let index = damaged.len() / 2;
    damaged[index] ^= 1;
    fixture.write("damaged.tdenc", damaged);
    app.failure(
        json!({"command":"read_envelope","source":"damaged.tdenc","destination":"unverified-envelope.bin"}),
    );
    fixture.write("truncated.tdenc", &ciphertext[..ciphertext.len() - 1]);
    app.failure(
        json!({"command":"read_envelope","source":"truncated.tdenc","destination":"unverified-envelope.bin"}),
    );
    app.stop();
    let mut vault = fixture.read("crypto.vault");
    let last = vault.len() - 1;
    vault[last] ^= 1;
    fixture.write("crypto.vault", &vault);
    let mut app = Backend::start(&fixture.0);
    app.failure(
        json!({"command":"vault_unlock","passphrase":"synthetic initial vault passphrase"}),
    );
    assert_eq!(fixture.read("crypto.vault"), vault);
    app.failure(json!({"command":"vault_recover","passphrase":"wrong recovery passphrase","vaultPassphrase":"synthetic restored vault passphrase"}));
    assert_eq!(fixture.read("crypto.vault"), vault);
    // A locked or unreadable vault cannot keep its passphrase, and the bundle
    // passphrase must never become the vault passphrase implicitly.
    assert!(app
        .failure(json!({"command":"vault_recover","passphrase":"synthetic recovery passphrase"}))
        .contains("RECOVERY_VAULT_PASSPHRASE_REQUIRED"));
    assert_eq!(fixture.read("crypto.vault"), vault);
    app.ok(json!({"command":"vault_recover","passphrase":"synthetic recovery passphrase","vaultPassphrase":"synthetic restored vault passphrase"}));
    app.ok(
        json!({"command":"read_envelope","source":"protected.tdenc","destination":"recovered.bin"}),
    );
    assert_eq!(fixture.read("recovered.bin"), plaintext);
    app.stop();
    let mut app = Backend::start(&fixture.0);
    app.failure(json!({"command":"vault_unlock","passphrase":"synthetic recovery passphrase"}));
    app.ok(json!({"command":"vault_unlock","passphrase":"synthetic restored vault passphrase"}));
    app.ok(json!({"command":"vault_change_passphrase","currentPassphrase":"synthetic restored vault passphrase","passphrase":"synthetic replacement passphrase"}));
    app.stop();
    let mut app = Backend::start(&fixture.0);
    app.failure(
        json!({"command":"vault_unlock","passphrase":"synthetic initial vault passphrase"}),
    );
    app.failure(
        json!({"command":"vault_unlock","passphrase":"synthetic restored vault passphrase"}),
    );
    app.ok(json!({"command":"vault_unlock","passphrase":"synthetic replacement passphrase"}));
    app.ok(
        json!({"command":"read_envelope","source":"protected.tdenc","destination":"updated.bin"}),
    );
    assert_eq!(fixture.read("updated.bin"), plaintext);
    app.stop();
}

fn replaced_vault_archives(fixture: &Fixture) -> Vec<PathBuf> {
    let mut archives: Vec<PathBuf> = std::fs::read_dir(&fixture.0)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("crypto.vault.replaced-")
        })
        .collect();
    archives.sort();
    archives
}

#[test]
fn recovery_drill_changes_nothing_and_import_keeps_the_vault_passphrase() {
    let fixture = Fixture::new();
    fixture.write("original.bin", b"synthetic protected document");
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"vault_create","passphrase":"synthetic vault passphrase"}));
    app.ok(json!({"command":"fixture_encrypt","source":"original.bin","destination":"protected.tdenc"}));
    app.ok(json!({"command":"vault_export","passphrase":"synthetic bundle passphrase"}));
    let identity = app.ok(json!({"command":"vault_identity"}));
    assert_eq!(identity.as_str().unwrap().len(), 16);
    let vault = fixture.read("crypto.vault");

    // The drill proves the bundle restores this vault and writes nothing.
    app.failure(json!({"command":"vault_verify_recovery","passphrase":"wrong bundle passphrase"}));
    assert_eq!(
        app.ok(
            json!({"command":"vault_verify_recovery","passphrase":"synthetic bundle passphrase"})
        ),
        json!({"matches_vault_key":true,"missing_profiles":0})
    );
    assert_eq!(fixture.read("crypto.vault"), vault);
    assert!(replaced_vault_archives(&fixture).is_empty());
    app.stop();

    let mut app = Backend::start(&fixture.0);
    assert_eq!(app.ok(json!({"command":"vault_identity"})), json!(null));
    app.failure(
        json!({"command":"vault_verify_recovery","passphrase":"synthetic bundle passphrase"}),
    );
    app.failure(json!({"command":"vault_unlock","passphrase":"synthetic bundle passphrase"}));
    app.ok(json!({"command":"vault_unlock","passphrase":"synthetic vault passphrase"}));
    assert_eq!(app.ok(json!({"command":"vault_identity"})), identity);

    // Importing into the unlocked vault keeps the passphrase the user chose
    // and preserves the replaced file.
    app.ok(json!({"command":"vault_recover","passphrase":"synthetic bundle passphrase"}));
    let archives = replaced_vault_archives(&fixture);
    assert_eq!(archives.len(), 1);
    assert_eq!(std::fs::read(&archives[0]).unwrap(), vault);
    app.stop();
    let mut app = Backend::start(&fixture.0);
    app.failure(json!({"command":"vault_unlock","passphrase":"synthetic bundle passphrase"}));
    app.ok(json!({"command":"vault_unlock","passphrase":"synthetic vault passphrase"}));
    app.ok(json!({"command":"read_envelope","source":"protected.tdenc","destination":"after-import.bin"}));
    assert_eq!(
        fixture.read("after-import.bin"),
        b"synthetic protected document"
    );

    // A bundle exported before a key was added must not silently drop it.
    app.ok(json!({"command":"vault_save_profile","profile":"profile-added-after-export"}));
    assert_eq!(
        app.ok(
            json!({"command":"vault_verify_recovery","passphrase":"synthetic bundle passphrase"})
        ),
        json!({"matches_vault_key":true,"missing_profiles":1})
    );
    let current = fixture.read("crypto.vault");
    assert!(app
        .failure(json!({"command":"vault_recover","passphrase":"synthetic bundle passphrase"}))
        .contains("RECOVERY_BUNDLE_MISMATCH"));
    assert_eq!(fixture.read("crypto.vault"), current);
    assert_eq!(replaced_vault_archives(&fixture).len(), 1);
    assert_eq!(
        app.ok(
            json!({"command":"vault_verify_recovery","passphrase":"synthetic bundle passphrase"})
        ),
        json!({"matches_vault_key":true,"missing_profiles":1})
    );

    // An explicit decision replaces the keys, still under the same passphrase,
    // and the replaced vault remains restorable.
    app.ok(json!({"command":"vault_recover","passphrase":"synthetic bundle passphrase","allowKeyReplacement":true}));
    let archives = replaced_vault_archives(&fixture);
    assert_eq!(archives.len(), 2);
    assert_eq!(std::fs::read(archives.last().unwrap()).unwrap(), current);
    assert_eq!(
        app.ok(
            json!({"command":"vault_verify_recovery","passphrase":"synthetic bundle passphrase"})
        ),
        json!({"matches_vault_key":true,"missing_profiles":0})
    );
    app.stop();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"vault_unlock","passphrase":"synthetic vault passphrase"}));
    assert_eq!(app.ok(json!({"command":"vault_identity"})), identity);
    app.stop();
}

#[test]
fn disconnected_folder_changes_fail_without_touching_local_records() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"database"}));
    app.ok(json!({"command":"seed_folder_row","id":77,"name":"Existing folder"}));
    let rows = json!([{"id":77,"name":"Existing folder"}]);
    assert_eq!(app.ok(json!({"command":"folder_rows"})), rows);
    for request in [
        json!({"command":"folder_create","name":"Phantom folder"}),
        json!({"command":"folder_delete","id":77}),
        json!({"command":"folder_rename","id":77,"name":"Renamed offline"}),
    ] {
        assert_eq!(app.failure(request), "Client not connected");
        assert_eq!(app.ok(json!({"command":"folder_rows"})), rows);
    }
    app.stop();
    let mut app = Backend::start(&fixture.0);
    assert_eq!(app.ok(json!({"command":"folder_rows"})), rows);
    app.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn media_server_uses_the_preferred_port_and_falls_back_when_it_is_taken() {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    // Another process holds the preferred port: the server still starts and
    // the sponsor page policy names the port that is really listening.
    let occupied = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let taken = occupied.local_addr().unwrap().port();
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    let started = app.ok(json!({"command":"start_stream_server","preferredPort":taken}));
    let port = started["port"].as_u64().unwrap();
    assert_ne!(port, u64::from(taken));
    assert_eq!(started["publishedPort"].as_u64().unwrap(), port);
    let url = started["url"].as_str().unwrap().to_string();
    let banner = client.get(format!("{url}/ad-banner")).send().await.unwrap();
    assert_eq!(banner.status(), 200);
    let policy = banner
        .headers()
        .get("content-security-policy")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        policy.contains(&format!("http://localhost:{port}/ad-script")),
        "{policy}"
    );
    assert!(!policy.contains(&format!(":{taken}/")), "{policy}");
    assert_eq!(
        client
            .get(format!("{url}/stream/home/1"))
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    app.stop();
    drop(occupied);

    // A free preferred port is used as-is, keeping existing share links valid.
    let free = {
        let probe = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        probe.local_addr().unwrap().port()
    };
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    let started = app.ok(json!({"command":"start_stream_server","preferredPort":free}));
    assert_eq!(started["port"].as_u64().unwrap(), u64::from(free));
    assert_eq!(started["publishedPort"].as_u64().unwrap(), u64::from(free));
    app.stop();
}

#[test]
fn download_publication_preserves_collisions_and_rejects_a_stale_account() {
    let fixture = Fixture::new();
    fixture.write("download.txt", b"existing bytes");
    fixture.write("verified.part", b"downloaded bytes");
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!({"command":"capture_account","owner":"101"}));
    let saved=app.ok(json!({"command":"publish_download","source":"verified.part","destination":"download.txt","policy":"keep_both"}));
    assert_eq!(saved["outcome"], "saved");
    assert_eq!(fixture.read("download.txt"), b"existing bytes");
    assert_eq!(fixture.read("download (1).txt"), b"downloaded bytes");
    fixture.write("skipped.part", b"skip these");
    assert_eq!(app.ok(json!({"command":"publish_download","source":"skipped.part","destination":"download.txt","policy":"skip"}))["outcome"],"skipped");
    assert_eq!(fixture.read("download.txt"), b"existing bytes");
    fixture.write("replacement.part", b"replacement");
    app.ok(json!({"command":"publish_download","source":"replacement.part","destination":"download.txt","policy":"replace"}));
    assert_eq!(fixture.read("download.txt"), b"replacement");
    fixture.write("private.part", b"account A private bytes");
    app.ok(json!({"command":"seed_account","owner":202}));
    assert!(app.failure(json!({"command":"publish_download","source":"private.part","destination":"private.txt","policy":"keep_both"})).starts_with("ACCOUNT_CHANGED:"));
    assert!(!fixture.path("private.txt").exists());
    assert_eq!(fixture.read("private.part"), b"account A private bytes");
    app.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_http_server_streams_owned_cache_and_revokes_it_after_account_switch() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!({"command":"database"}));
    let db = sqlite::open(fixture.path("shares.db")).unwrap();
    db.execute(concat!(
            "INSERT INTO shared_links(id,owner_id,message_id,file_name,password_hash,created_at) ",
            "VALUES('private',101,42,'A-private.pdf','synthetic-hash',1),('legacy',NULL,42,'legacy-priv",
            "ate.pdf','synthetic-hash',1)",
        )).unwrap();
    let password_hash = bcrypt::hash("synthetic share password", 4).unwrap();
    let mut password = db
        .prepare("UPDATE shared_links SET password_hash=? WHERE id='private'")
        .unwrap();
    password.bind((1, password_hash.as_str())).unwrap();
    password.next().unwrap();
    drop(password);
    drop(db);
    let fmp4_root = fixture.path("transcode/fmp4/101_0_42");
    std::fs::create_dir_all(&fmp4_root).unwrap();
    let fmp4_bytes: Vec<u8> = (0..64).collect();
    std::fs::write(fmp4_root.join("output.mp4"), &fmp4_bytes).unwrap();
    let hls = fixture.path("transcode/hls/101_0_42/480p");
    std::fs::create_dir_all(&hls).unwrap();
    std::fs::write(
        hls.join("index.m3u8"),
        b"#EXTM3U\n#EXTINF:1,\nsegment000.ts\n#EXT-X-ENDLIST\n",
    )
    .unwrap();
    std::fs::write(hls.join("segment000.ts"), b"synthetic owned media bytes").unwrap();
    let url = app.ok(json!({"command":"start_http"}))["url"]
        .as_str()
        .unwrap()
        .to_string();
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    assert_eq!(
        client
            .get(format!("{url}/stream/home/1"))
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        client
            .get(format!("{url}/stream/home/1?token=wrong"))
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let denied = client
        .get(format!("{url}/stream/home/1"))
        .header("Origin", "https://untrusted.invalid")
        .send()
        .await
        .unwrap();
    assert!(!denied.headers().contains_key("access-control-allow-origin"));
    let owned = client.get(format!("{url}/d/private")).send().await.unwrap();
    assert_eq!(owned.status(), 200);
    assert_eq!(owned.headers()["x-content-type-options"], "nosniff");
    assert!(owned.headers().contains_key("content-security-policy"));
    assert!(owned.text().await.unwrap().contains("A-private.pdf"));
    let wrong = client
        .post(format!("{url}/d/private/verify"))
        .form(&[("password", "wrong password")])
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status(), 200);
    assert!(!wrong.headers().contains_key("set-cookie"));
    let verified = client
        .post(format!("{url}/d/private/verify"))
        .form(&[("password", "synthetic share password")])
        .send()
        .await
        .unwrap();
    assert_eq!(verified.status(), 302);
    assert_eq!(verified.headers()["location"], "/d/private");
    let cookie = verified.headers()["set-cookie"].to_str().unwrap();
    assert!(cookie.contains("HttpOnly"));
    assert!(cookie.contains("SameSite=Strict"));
    assert!(cookie.contains("Path=/d/private"));
    let missing = client.get(format!("{url}/d/missing")).send().await.unwrap();
    assert_eq!(missing.status(), 404);
    let token = "synthetic-e2e-stream-token";
    let playlist = format!("{url}/hls/101_0_42/480p/index.m3u8?token={token}");
    let segment = format!("{url}/hls/101_0_42/480p/segment000.ts?token={token}");
    let response = client.get(&playlist).send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert!(response
        .text()
        .await
        .unwrap()
        .contains(&format!("segment000.ts?token={token}")));
    let response = client.get(&segment).send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "private, no-store");
    assert_eq!(
        response.bytes().await.unwrap().as_ref(),
        b"synthetic owned media bytes"
    );
    let fmp4 = format!("{url}/fmp4/101_0_42/output.mp4?token={token}");
    assert_eq!(
        client
            .get(format!("{url}/fmp4/101_0_42/output.mp4"))
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let response = client
        .get(&fmp4)
        .header("Range", "bytes=4-12")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 206);
    assert_eq!(response.headers()["content-range"], "bytes 4-12/64");
    assert_eq!(response.bytes().await.unwrap().as_ref(), &fmp4_bytes[4..13]);
    app.ok(json!({"command":"seed_account","owner":202}));
    assert_eq!(client.get(&fmp4).send().await.unwrap().status(), 403);
    for token in ["private", "legacy"] {
        let response = client.get(format!("{url}/d/{token}")).send().await.unwrap();
        assert_eq!(response.status(), 404);
        assert!(!response.text().await.unwrap().contains("private.pdf"));
        let response = client
            .post(format!("{url}/d/{token}/verify"))
            .form(&[("password", "anything")])
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 404);
        assert!(!response.headers().contains_key("set-cookie"));
    }
    assert_eq!(client.get(&playlist).send().await.unwrap().status(), 403);
    assert_eq!(client.get(&segment).send().await.unwrap().status(), 403);
    app.stop();
    assert!(
        client
            .get(format!("{url}/stream/home/1"))
            .send()
            .await
            .is_err(),
        "stopped native server still accepts connections"
    );
}

#[test]
fn interrupted_download_persists_progress_and_requires_review_before_resume() {
    let fixture = Fixture::new();
    fixture.write("partial-download.part", b"already fetched bytes");
    let job = json!(
        {
            "id":"download-42",
            "ownerId":"101",
            "direction":"download",
            "kind":"download",
            "status":"downloading",
            "messageId":42,
            "filename":"download.txt",
            "savePath":fixture.path("download.txt"),
            "progress":40,
            "transferredBytes":21,
            "totalBytes":50,
            "speedBytesPerSec":1000,
            "queuePosition":0,
            "revision":7,
            "createdAt":1,
            "updatedAt":2
        }
    );
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"save_transfer","job":job}));
    app.stop();
    let mut app = Backend::start(&fixture.0);
    let recovered = app.ok(json!({"command":"transfers"}));
    assert_eq!(recovered[0]["status"], "paused");
    assert_eq!(recovered[0]["errorCategory"], "interrupted");
    assert_eq!(recovered[0]["transferredBytes"], 21);
    assert_eq!(recovered[0]["speedBytesPerSec"], 0);
    assert_eq!(recovered[0]["revision"], 8);
    assert_eq!(
        fixture.read("partial-download.part"),
        b"already fetched bytes"
    );
    assert!(!fixture.path("download.txt").exists());
    app.stop();
    let mut app = Backend::start(&fixture.0);
    assert_eq!(app.ok(json!({"command":"transfers"}))[0]["revision"], 8);
    app.ok(json!({"command":"remove_transfer","id":"download-42"}));
    assert!(app
        .failure(json!({"command":"save_transfer","job":job}))
        .contains("stale updates are rejected"));
    assert_eq!(app.ok(json!({"command":"transfers"})), json!([]));
    app.stop();
}

// ---------------------------------------------------------------------------
// Folder sync journeys. The local folder, SQLite state, scanning, baselining,
// planning and result bookkeeping are production code in a separate process.
// Telegram is the controlled boundary: each journey supplies the folder's file
// messages and the executor's outcome. Transfers themselves are not exercised.
// ---------------------------------------------------------------------------

fn sync_folder(fixture: &Fixture) -> PathBuf {
    let folder = fixture.path("library");
    std::fs::create_dir_all(&folder).unwrap();
    folder
}

/// Give a file a modification time far enough in the past to be trusted.
fn settle(path: &Path) -> SystemTime {
    let modified = SystemTime::now() - Duration::from_secs(3_600);
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(modified)
        .unwrap();
    std::fs::metadata(path).unwrap().modified().unwrap()
}

fn telegram_file(message_id: i32, document_name: Option<&str>, caption: &str, size: u64) -> Value {
    json!({
        "messageId": message_id,
        "caption": caption,
        "documentName": document_name,
        "mediaId": 9_000 + i64::from(message_id),
        "fileSize": size,
        "createdAt": 1_700_000_000 + i64::from(message_id),
    })
}

fn planned(plan: &Value) -> Vec<(String, String)> {
    let mut operations: Vec<(String, String)> = plan["operations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|operation| {
            (
                operation["relative_path"].as_str().unwrap().to_string(),
                operation["action"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    operations.sort();
    operations
}

fn planned_pairs(expected: &[(&str, &str)]) -> Vec<(String, String)> {
    expected
        .iter()
        .map(|(path, action)| (path.to_string(), action.to_string()))
        .collect()
}

fn sync_states(app: &mut Backend, pair: i64) -> Vec<(String, String)> {
    app.ok(json!({"command":"sync_state","pairId":pair}))
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                row["path"].as_str().unwrap().to_string(),
                row["status"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

#[test]
fn sync_plans_real_folders_names_photos_uniquely_and_adopts_matching_files_only_when_asked() {
    let fixture = Fixture::new();
    let folder = sync_folder(&fixture);
    std::fs::write(folder.join("report.txt"), b"local report").unwrap();
    std::fs::create_dir_all(folder.join("notes")).unwrap();
    std::fs::write(folder.join("notes/todo.txt"), b"buy milk").unwrap();
    std::fs::write(folder.join("Photo.jpg"), b"old").unwrap();
    std::fs::write(folder.join("ignored.td-sync-tmp"), b"staging").unwrap();

    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"database"}));
    let pair = app.ok(json!({"command":"sync_seed_pair","folder":"library"}))["pairId"]
        .as_i64()
        .unwrap();
    // An older release recorded this folder's single photo as "Photo.jpg".
    let db = sqlite::open(fixture.path("shares.db")).unwrap();
    db.execute(format!(
        concat!(
            "INSERT INTO sync_state(pair_id, relative_path, local_hash, remote_hash, ",
            "file_size, message_id, sync_status) VALUES ({}, 'Photo.jpg', ",
            "'recorded-before-upgrade', 'recorded-before-upgrade', 3, 21, 'conflict')",
        ),
        pair
    ))
    .unwrap();
    drop(db);

    let remote = json!([
        telegram_file(11, Some("report.txt"), "", 12),
        telegram_file(12, Some("remote-only.bin"), "", 5),
        telegram_file(13, Some("shared.txt"), "notes/shared.txt", 40),
        // Protected files uploaded elsewhere have no recorded path here. They
        // are left alone instead of colliding on their shared marker caption.
        telegram_file(14, Some("3f9c.tdenc"), "TDENC2", 400),
        telegram_file(15, Some("a01b.tdenc"), "TDENC2", 500),
        telegram_file(21, None, "", 3),
        telegram_file(22, None, "", 900),
        telegram_file(23, None, "", 901),
    ]);
    let plan = app.ok(json!({"command":"sync_cycle","pairId":pair,"remoteFiles":remote}));
    assert_eq!(
        plan["localFiles"], 3,
        "staging files are never part of the tree"
    );
    assert_eq!(
        plan["remoteFiles"], 6,
        "unmapped protected files are not part of the tree"
    );
    // Several unnamed photos no longer collide; the recorded one keeps its path.
    assert_eq!(
        planned(&plan),
        planned_pairs(&[
            ("Photo-22.jpg", "download"),
            ("Photo-23.jpg", "download"),
            ("Photo.jpg", "conflict"),
            ("notes/shared.txt", "download"),
            ("notes/todo.txt", "upload"),
            ("remote-only.bin", "download"),
            ("report.txt", "conflict"),
        ])
    );
    assert_eq!(plan["conflicts"], 2);
    assert!(plan["baselined"].as_array().unwrap().is_empty());
    assert!(sync_states(&mut app, pair).contains(&("report.txt".into(), "conflict".into())));
    app.stop();

    // The same folder with adoption turned on: the same-size report is
    // recorded as in sync; nothing is transferred or overwritten for it.
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"sync_set_preferences","pairId":pair,"preferences":{"adoptMatchingFiles":true}}));
    let plan = app.ok(json!({"command":"sync_cycle","pairId":pair,"remoteFiles":remote}));
    assert_eq!(plan["baselined"], json!(["report.txt"]));
    assert_eq!(plan["conflicts"], 1);
    assert!(!planned(&plan).iter().any(|(path, _)| path == "report.txt"));
    assert!(sync_states(&mut app, pair).contains(&("report.txt".into(), "synced".into())));
    assert_eq!(
        std::fs::read(folder.join("report.txt")).unwrap(),
        b"local report"
    );
    // A different-size copy is never adopted.
    assert!(planned(&plan).contains(&("Photo.jpg".into(), "conflict".into())));
    app.stop();

    // Two files that claim one path still stop the mapping.
    let mut app = Backend::start(&fixture.0);
    let duplicate = json!([
        telegram_file(41, Some("same.bin"), "", 1),
        telegram_file(42, Some("same.bin"), "", 2),
    ]);
    assert!(app
        .failure(json!({"command":"sync_cycle","pairId":pair,"remoteFiles":duplicate}))
        .contains("Multiple Telegram messages map to the same sync path 'same.bin'"));
    app.stop();
}

#[test]
fn sync_finishes_an_interrupted_upload_after_restart_without_downloading_it_again() {
    let fixture = Fixture::new();
    let folder = sync_folder(&fixture);
    std::fs::write(folder.join("a.txt"), b"first document").unwrap();
    std::fs::write(folder.join("b.txt"), b"second document").unwrap();

    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"database"}));
    let pair = app.ok(json!({"command":"sync_seed_pair","folder":"library"}))["pairId"]
        .as_i64()
        .unwrap();
    let plan = app.ok(json!({"command":"sync_cycle","pairId":pair,"remoteFiles":[]}));
    assert_eq!(
        planned(&plan),
        planned_pairs(&[("a.txt", "upload"), ("b.txt", "upload")])
    );

    // Both uploads reached Telegram, but the engine stopped before it could
    // read a.txt's message back. b.txt was verified.
    let recorded = app.ok(json!({"command":"sync_record","results":[
        {"relativePath":"a.txt","action":"upload","success":true,"messageId":31},
        {"relativePath":"b.txt","action":"upload","success":true,"messageId":32},
    ],"uploadedFiles":[telegram_file(32, Some("b.txt"), "b.txt", 15)]}));
    assert_eq!(recorded, json!({"journaledUploads":2,"verifiedUploads":1}));
    let states = app.ok(json!({"command":"sync_state","pairId":pair}));
    assert_eq!(states[0]["path"], "a.txt");
    assert_eq!(states[0]["status"], "syncing");
    assert_eq!(states[0]["messageId"], 31);
    assert_eq!(states[0]["remoteHashRecorded"], false);
    assert_eq!(states[1]["status"], "synced");
    assert_eq!(states[1]["remoteHashRecorded"], true);
    app.stop();

    // After a restart the uploaded message is recognised by its id, even
    // under a protected filename, and recorded instead of downloaded again.
    let mut app = Backend::start(&fixture.0);
    let remote = json!([
        telegram_file(31, Some("7c1e.tdenc"), "TDENC2", 800),
        telegram_file(32, Some("b.txt"), "b.txt", 15),
    ]);
    let plan = app.ok(json!({"command":"sync_cycle","pairId":pair,"remoteFiles":remote}));
    assert_eq!(plan["baselined"], json!(["a.txt"]));
    assert!(planned(&plan).is_empty(), "{plan}");
    assert_eq!(
        sync_states(&mut app, pair),
        vec![
            ("a.txt".to_string(), "synced".to_string()),
            ("b.txt".to_string(), "synced".to_string())
        ]
    );
    let plan = app.ok(json!({"command":"sync_cycle","pairId":pair,"remoteFiles":remote}));
    assert!(planned(&plan).is_empty(), "{plan}");
    assert!(plan["baselined"].as_array().unwrap().is_empty());

    // A failed download is recorded from the planned trees, not from a rescan,
    // and a completed one becomes the baseline.
    let remote = json!([
        telegram_file(31, Some("7c1e.tdenc"), "TDENC2", 800),
        telegram_file(32, Some("b.txt"), "b.txt", 15),
        telegram_file(33, Some("c.txt"), "", 7),
        telegram_file(34, Some("d.txt"), "", 9),
    ]);
    let plan = app.ok(json!({"command":"sync_cycle","pairId":pair,"remoteFiles":remote}));
    assert_eq!(
        planned(&plan),
        planned_pairs(&[("c.txt", "download"), ("d.txt", "download")])
    );
    std::fs::write(folder.join("c.txt"), b"c bytes").unwrap();
    let digest = {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(b"c bytes"))
    };
    app.ok(json!({"command":"sync_record","results":[
        {"relativePath":"c.txt","action":"download","success":true,"localHash":digest},
        {"relativePath":"d.txt","action":"download","success":false,"detail":"Transfer cancelled"},
    ]}));
    let states = sync_states(&mut app, pair);
    assert!(states.contains(&("c.txt".into(), "synced".into())));
    assert!(states.contains(&("d.txt".into(), "error".into())));
    let plan = app.ok(json!({"command":"sync_cycle","pairId":pair,"remoteFiles":remote}));
    assert_eq!(planned(&plan), planned_pairs(&[("d.txt", "download")]));
    app.stop();
}

#[test]
fn incremental_sync_scanning_reuses_recorded_hashes_and_still_sees_real_changes() {
    let fixture = Fixture::new();
    let folder = sync_folder(&fixture);
    for name in ["one.txt", "two.txt", "three.txt"] {
        std::fs::write(folder.join(name), format!("contents of {name}")).unwrap();
        settle(&folder.join(name));
    }
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"database"}));
    let pair = app.ok(json!({"command":"sync_seed_pair","folder":"library"}))["pairId"]
        .as_i64()
        .unwrap();
    let plan = app
        .ok(json!({"command":"sync_cycle","pairId":pair,"remoteFiles":[],"scanner":"incremental"}));
    assert_eq!(
        plan["hashed"], 3,
        "nothing is recorded yet, so every file is read"
    );
    assert_eq!(plan["reused"], 0);
    let remote = json!([
        telegram_file(51, Some("one.txt"), "one.txt", 19),
        telegram_file(52, Some("three.txt"), "three.txt", 21),
        telegram_file(53, Some("two.txt"), "two.txt", 19),
    ]);
    app.ok(json!({"command":"sync_record","results":[
        {"relativePath":"one.txt","action":"upload","success":true,"messageId":51},
        {"relativePath":"three.txt","action":"upload","success":true,"messageId":52},
        {"relativePath":"two.txt","action":"upload","success":true,"messageId":53},
    ],"uploadedFiles":remote}));
    app.stop();

    let mut app = Backend::start(&fixture.0);
    let plan = app.ok(
        json!({"command":"sync_cycle","pairId":pair,"remoteFiles":remote,"scanner":"incremental"}),
    );
    assert_eq!(
        (plan["hashed"].as_u64(), plan["reused"].as_u64()),
        (Some(0), Some(3))
    );
    assert!(planned(&plan).is_empty(), "{plan}");

    // A real edit changes the size or the timestamp and is hashed and planned.
    std::fs::write(folder.join("one.txt"), b"contents of one.txt, now longer").unwrap();
    let plan = app.ok(
        json!({"command":"sync_cycle","pairId":pair,"remoteFiles":remote,"scanner":"incremental"}),
    );
    assert_eq!(
        (plan["hashed"].as_u64(), plan["reused"].as_u64()),
        (Some(1), Some(2))
    );
    assert_eq!(planned(&plan), planned_pairs(&[("one.txt", "upload")]));

    // An edit that keeps both the size and the timestamp is invisible to the
    // incremental scanner by design, and is caught by the periodic full pass.
    let original = std::fs::metadata(folder.join("two.txt"))
        .unwrap()
        .modified()
        .unwrap();
    std::fs::write(folder.join("two.txt"), b"CONTENTS OF TWO.TXT").unwrap();
    std::fs::File::options()
        .write(true)
        .open(folder.join("two.txt"))
        .unwrap()
        .set_modified(original)
        .unwrap();
    let plan = app.ok(
        json!({"command":"sync_cycle","pairId":pair,"remoteFiles":remote,"scanner":"incremental"}),
    );
    assert_eq!(planned(&plan), planned_pairs(&[("one.txt", "upload")]));
    let plan =
        app.ok(json!({"command":"sync_cycle","pairId":pair,"remoteFiles":remote,"scanner":"full"}));
    assert_eq!(plan["hashed"], 3);
    assert_eq!(
        planned(&plan),
        planned_pairs(&[("one.txt", "upload"), ("two.txt", "upload")])
    );

    // A file written moments ago is recorded without a trusted timestamp, so
    // the next incremental scan reads it again instead of assuming it is unchanged.
    std::fs::write(folder.join("fresh.txt"), b"just written").unwrap();
    let plan = app.ok(
        json!({"command":"sync_cycle","pairId":pair,"remoteFiles":remote,"scanner":"incremental"}),
    );
    assert!(planned(&plan).contains(&("fresh.txt".into(), "upload".into())));
    app.ok(json!({"command":"sync_record","results":[
        {"relativePath":"fresh.txt","action":"upload","success":true,"messageId":54},
    ],"uploadedFiles":[telegram_file(54, Some("fresh.txt"), "fresh.txt", 12)]}));
    let states = app.ok(json!({"command":"sync_state","pairId":pair}));
    let fresh = states
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["path"] == "fresh.txt")
        .unwrap();
    assert_eq!(fresh["status"], "synced");
    assert_eq!(fresh["localMtimeRecorded"], false);
    app.stop();
}

#[test]
fn sync_refuses_a_mass_deletion_and_never_plans_from_it() {
    let fixture = Fixture::new();
    let folder = sync_folder(&fixture);
    let names = ["a.txt", "b.txt", "c.txt", "d.txt"];
    for name in names {
        std::fs::write(folder.join(name), name).unwrap();
    }
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"database"}));
    let pair = app.ok(json!({"command":"sync_seed_pair","folder":"library","preferences":{"propagateDeletions":true}}))["pairId"]
        .as_i64()
        .unwrap();
    app.ok(json!({"command":"sync_cycle","pairId":pair,"remoteFiles":[]}));
    let remote: Vec<Value> = names
        .iter()
        .enumerate()
        .map(|(index, name)| telegram_file(61 + index as i32, Some(name), name, 5))
        .collect();
    let results: Vec<Value> = names
        .iter()
        .enumerate()
        .map(|(index, name)| json!({"relativePath":name,"action":"upload","success":true,"messageId":61 + index as i32}))
        .collect();
    app.ok(json!({"command":"sync_record","results":results,"uploadedFiles":remote}));

    // Three of four local files vanish (an unmounted disk looks like this).
    for name in &names[..3] {
        std::fs::remove_file(folder.join(name)).unwrap();
    }
    assert!(app
        .failure(json!({"command":"sync_cycle","pairId":pair,"remoteFiles":remote}))
        .contains("mass deletion protection stopped 3 deletes across 4 synced files"));
    assert_eq!(
        sync_states(&mut app, pair).len(),
        4,
        "recorded state is untouched"
    );

    // A single deletion is within the guard and is planned for Telegram.
    for name in &names[1..3] {
        std::fs::write(folder.join(name), name).unwrap();
    }
    let plan = app.ok(json!({"command":"sync_cycle","pairId":pair,"remoteFiles":remote}));
    assert_eq!(planned(&plan), planned_pairs(&[("a.txt", "delete_remote")]));
    app.stop();
}

#[test]
fn the_log_file_is_redacted_bounded_and_named_in_the_startup_failure_report() {
    let fixture = Fixture::new();
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap();
    let mut app = Backend::start(&fixture.0);
    assert_eq!(
        app.ok(json!({"command":"log_start"}))["path"],
        "telegram-drive.log"
    );
    app.ok(json!({"command":"log_emit","level":"info","message":"opened folder Holiday Photos"}));
    app.ok(json!(
        {
            "command":"log_emit",
            "level":"warn",
            "message":format!(
        "retry for +15555550123 at {home}/Documents/report.pdf via http://localhost:14201/stream/home/9?token=abcDEF123456&credential=77 with session 0123456789abcdef0123456789abcdef0123"
    )
        }
    ));
    app.ok(json!({"command":"log_emit","level":"error","message":"two\nline failure api_hash=deadbeefcafe"}));
    let log = String::from_utf8(fixture.read("logs/telegram-drive.log")).unwrap();
    let lines: Vec<&str> = log.lines().collect();
    assert_eq!(
        lines.len(),
        2,
        "only warnings and errors are written by default: {log}"
    );
    assert!(lines[0].contains(" WARN "), "{log}");
    assert!(lines[1].contains(" ERROR "), "{log}");
    for secret in [
        "15555550123",
        "abcDEF123456",
        "credential=77",
        "deadbeefcafe",
        "0123456789abcdef0123456789abcdef0123",
        home.as_str(),
        "Holiday Photos",
    ] {
        assert!(
            !log.contains(secret),
            "{secret} must not reach the log file: {log}"
        );
    }
    assert!(log.contains("+<redacted>"));
    assert!(log.contains("~/Documents/report.pdf"));
    assert!(log.contains("token=<redacted>&credential=<redacted>"));
    assert!(log.contains("two line failure api_hash=<redacted>"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(fixture.path("logs/telegram-drive.log"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "the log file is private to the user");
    }

    // A real setup failure: this build refuses a database written by a later release.
    app.ok(json!({"command":"database"}));
    app.stop();
    let database = sqlite::open(fixture.path("shares.db")).unwrap();
    database
        .execute("INSERT INTO app_schema_migrations VALUES(999,'future','future',1,'future')")
        .unwrap();
    drop(database);
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"log_start"}));
    let failure = app.failure(json!({"command":"database"}));
    let report = app.ok(json!({"command":"startup_failure_message","error":format!("error encountered during setup hook: {failure}")}));
    let report = report.as_str().unwrap();
    assert!(report.contains("Reason: "), "{report}");
    assert!(
        report.contains("Database uses application schema version 999"),
        "{report}"
    );
    assert!(!report.contains("setup hook"), "{report}");
    assert!(report.contains("Nothing was deleted."), "{report}");
    assert!(
        report.contains("older version was installed over a newer one"),
        "{report}"
    );
    assert!(report.contains("telegram-drive.log"), "{report}");

    // The file is bounded: once it reaches its limit it rotates once.
    let filler = "bounded log line ".repeat(40);
    app.ok(json!({"command":"log_emit","level":"warn","message":filler,"repeat":2_000}));
    app.stop();
    let current = std::fs::metadata(fixture.path("logs/telegram-drive.log"))
        .unwrap()
        .len();
    let previous = std::fs::metadata(fixture.path("logs/telegram-drive.previous.log"))
        .unwrap()
        .len();
    assert!(current <= 1024 * 1024, "current log is {current} bytes");
    assert!(
        previous > 0 && previous <= 1024 * 1024 + 4096,
        "previous log is {previous} bytes"
    );
}

#[cfg(unix)]
#[test]
fn staging_files_live_in_a_private_directory_that_rejects_planted_links() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);

    // First use creates a directory only the current user can enter.
    let name = app.ok(json!({"command":"staging_root","parent":"shared-tmp"}))["name"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(name.starts_with("telegram-drive-staging-"), "{name}");
    let staging = fixture.path("shared-tmp").join(&name);
    assert_eq!(
        std::fs::metadata(&staging).unwrap().permissions().mode() & 0o777,
        0o700
    );

    // Permissions loosened by something else are tightened again on next use.
    std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o755)).unwrap();
    app.ok(json!({"command":"staging_root","parent":"shared-tmp"}));
    assert_eq!(
        std::fs::metadata(&staging).unwrap().permissions().mode() & 0o777,
        0o700
    );

    // Old leftovers are swept; anything recent, which may belong to a
    // resumable transfer, is kept.
    std::fs::write(staging.join("tg_drive_stale.tmp"), b"abandoned").unwrap();
    std::fs::create_dir(staging.join("archive_stale_extract")).unwrap();
    std::fs::write(staging.join("archive_stale_extract/entry"), b"abandoned").unwrap();
    for stale in ["tg_drive_stale.tmp", "archive_stale_extract"] {
        std::fs::File::open(staging.join(stale))
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(30 * 24 * 3_600))
            .unwrap();
    }
    std::fs::write(staging.join("tg_drive_active.tmp"), b"resumable").unwrap();
    app.ok(json!({"command":"staging_root","parent":"shared-tmp","sweepOlderThanSeconds":7 * 24 * 3_600}));
    assert!(!staging.join("tg_drive_stale.tmp").exists());
    assert!(!staging.join("archive_stale_extract").exists());
    assert_eq!(
        std::fs::read(staging.join("tg_drive_active.tmp")).unwrap(),
        b"resumable"
    );

    // A link planted at the staging name by another local user is refused, and
    // nothing is written through it.
    std::fs::create_dir(fixture.path("victim")).unwrap();
    std::fs::create_dir(fixture.path("hostile-tmp")).unwrap();
    std::os::unix::fs::symlink(
        fixture.path("victim"),
        fixture.path("hostile-tmp").join(&name),
    )
    .unwrap();
    assert!(app
        .failure(json!({"command":"staging_root","parent":"hostile-tmp"}))
        .contains("not a directory owned by this application"));
    assert_eq!(
        std::fs::read_dir(fixture.path("victim")).unwrap().count(),
        0
    );
    app.stop();
}

#[test]
fn connection_failures_retry_with_growing_waits_and_removed_transfers_expire() {
    let fixture = Fixture::new();
    let queued = |id: &str| {
        json!(
            {
                "id":id,
                "ownerId":"101",
                "direction":"upload",
                "kind":"local_upload",
                "status":"pending",
                "path":"/synthetic/report.pdf",
                "filename":"report.pdf",
                "progress":0,
                "transferredBytes":0,
                "totalBytes":50,
                "speedBytesPerSec":0,
                "queuePosition":0,
                "revision":1,
                "createdAt":1,
                "updatedAt":2
            }
        )
    };
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"save_transfer","job":queued("upload-1")}));

    // A dropped connection waits and retries on its own, a little longer each
    // time, and the budget survives a restart.
    let now = 1_000_000;
    let mut waits = Vec::new();
    for attempt in 1..=5 {
        let failed = app.ok(json!({"command":"fail_transfer","id":"upload-1","error":"Upload failed: connection reset by peer","now":now}));
        assert_eq!(failed["status"], "cooldown", "{failed}");
        assert_eq!(failed["errorCategory"], "network");
        assert_eq!(failed["networkRetries"], attempt);
        waits.push((failed["retryAt"].as_i64().unwrap() - now) / 1_000);
        if attempt == 2 {
            app.stop();
            app = Backend::start(&fixture.0);
            let recovered = app.ok(json!({"command":"transfers"}));
            assert_eq!(recovered[0]["status"], "cooldown");
            assert_eq!(recovered[0]["networkRetries"], 2);
        }
    }
    assert_eq!(waits, vec![5, 15, 45, 120, 300]);
    // The sixth consecutive failure is handed to the user.
    let failed = app.ok(json!({"command":"fail_transfer","id":"upload-1","error":"Upload failed: connection reset by peer","now":now}));
    assert_eq!(failed["status"], "failed");
    assert_eq!(failed["retryAt"], json!(null));

    // Other failures keep their existing outcomes.
    for (id, error, status, category) in [
        (
            "upload-2",
            "No such file or directory",
            "failed",
            "source_missing",
        ),
        ("upload-3", "FLOOD_WAIT_42", "cooldown", "rate_limit"),
        (
            "upload-4",
            "Client not connected",
            "waiting_for_network",
            "network",
        ),
        (
            "upload-5",
            "[VAULT_LOCKED] unlock the vault",
            "waiting_for_unlock",
            "unlock",
        ),
        ("upload-6", "Transfer cancelled", "cancelled", "cancelled"),
    ] {
        app.ok(json!({"command":"save_transfer","job":queued(id)}));
        let failed = app.ok(json!({"command":"fail_transfer","id":id,"error":error,"now":now}));
        assert_eq!(failed["status"], status, "{error}");
        assert_eq!(failed["errorCategory"], category, "{error}");
        assert_eq!(failed["networkRetries"], 0, "{error}");
    }
    let flood = app.ok(json!({"command":"transfers"}));
    let flood = flood
        .as_array()
        .unwrap()
        .iter()
        .find(|job| job["id"] == "upload-3")
        .unwrap();
    assert_eq!(flood["retryAt"], now + 42_000);

    // A removed job cannot be recreated by a late update, but its tombstone
    // does not live forever.
    app.ok(json!({"command":"remove_transfer","id":"upload-2"}));
    assert!(app
        .failure(json!({"command":"save_transfer","job":queued("upload-2")}))
        .contains("stale updates are rejected"));
    app.stop();
    let database = sqlite::open(fixture.path("transfers.db")).unwrap();
    database
        .execute("UPDATE transfer_tombstones SET removed_at = removed_at - 31 * 24 * 60 * 60")
        .unwrap();
    drop(database);
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"save_transfer","job":queued("upload-2")}));
    app.stop();

    // A queue written by an earlier release has undated tombstones. They are
    // dated on first use, keep protecting their ids, and nothing is lost.
    let database = sqlite::open(fixture.path("transfers.db")).unwrap();
    database
        .execute("DROP TABLE transfer_tombstones; CREATE TABLE transfer_tombstones (id TEXT PRIMARY KEY NOT NULL); INSERT INTO transfer_tombstones(id) VALUES ('legacy-removed')")
        .unwrap();
    drop(database);
    let mut app = Backend::start(&fixture.0);
    assert!(app
        .failure(json!({"command":"save_transfer","job":queued("legacy-removed")}))
        .contains("stale updates are rejected"));
    assert_eq!(
        app.ok(json!({"command":"transfers"}))
            .as_array()
            .unwrap()
            .len(),
        6
    );
    app.stop();
    let database = sqlite::open(fixture.path("transfers.db")).unwrap();
    let mut dated = database
        .prepare("SELECT COUNT(*) FROM transfer_tombstones WHERE id = 'legacy-removed' AND removed_at IS NOT NULL")
        .unwrap();
    dated.next().unwrap();
    assert_eq!(dated.read::<i64, _>(0).unwrap(), 1);
}

#[test]
fn each_account_keeps_its_own_folder_groups_across_switches_restarts_and_upgrade() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"database"}));

    // Account A organises two folders into a group.
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!({"command":"folder_scan","folders":[{"id":11,"name":"A photos"},{"id":12,"name":"A work"}]}));
    let work = app.ok(json!({"command":"group_create","name":"A projects"}));
    app.ok(json!({"command":"folder_assign","folder":12,"group":work}));
    let layout_a = json!({
        "folders":[{"id":12,"name":"A work","group":work},{"id":11,"name":"A photos","group":null}],
        "groups":["A projects"],
    });
    assert_eq!(app.ok(json!({"command":"folder_layout"})), layout_a);

    // After sign-out nothing of A is readable, by anyone.
    app.ok(json!({"command":"folder_sign_out"}));
    assert_eq!(app.ok(json!({"command":"folder_rows"})), json!([]));
    app.stop();

    // Account B signs in: it starts empty, and its scan cannot prune A's data.
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":202}));
    assert_eq!(
        app.ok(json!({"command":"folder_layout"})),
        json!({"folders":[],"groups":[]})
    );
    app.ok(json!({"command":"folder_scan","folders":[{"id":21,"name":"B archive"}]}));
    let archive = app.ok(json!({"command":"group_create","name":"B personal"}));
    app.ok(json!({"command":"folder_assign","folder":21,"group":archive}));
    let layout_b =
        json!({"folders":[{"id":21,"name":"B archive","group":archive}],"groups":["B personal"]});
    assert_eq!(app.ok(json!({"command":"folder_layout"})), layout_b);
    app.stop();

    // Switching back without an explicit sign-out (session replaced) still
    // restores A exactly, and B's layout is kept for its next sign-in.
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    assert_eq!(app.ok(json!({"command":"folder_layout"})), layout_a);
    app.stop();
    let mut app = Backend::start(&fixture.0);
    assert_eq!(
        app.ok(json!({"command":"folder_layout"})),
        layout_a,
        "unchanged across a restart"
    );
    app.ok(json!({"command":"seed_account","owner":202}));
    assert_eq!(app.ok(json!({"command":"folder_layout"})), layout_b);
    app.stop();

    // Upgrade path: a layout written by an earlier release has no owner
    // marker. It is kept for the account that is signed in.
    let upgraded = Fixture::new();
    let mut app = Backend::start(&upgraded.0);
    app.ok(json!({"command":"database"}));
    app.ok(json!({"command":"seed_folder_row","id":31,"name":"Existing folder"}));
    let group = app.ok(json!({"command":"group_create","name":"Existing group"}));
    app.ok(json!({"command":"folder_assign","folder":31,"group":group}));
    app.ok(json!({"command":"seed_account","owner":101}));
    let existing = json!({"folders":[{"id":31,"name":"Existing folder","group":group}],"groups":["Existing group"]});
    assert_eq!(app.ok(json!({"command":"folder_layout"})), existing);
    app.ok(json!({"command":"folder_sign_out"}));
    app.ok(json!({"command":"seed_account","owner":202}));
    assert_eq!(
        app.ok(json!({"command":"folder_layout"})),
        json!({"folders":[],"groups":[]})
    );
    app.ok(json!({"command":"seed_account","owner":101}));
    assert_eq!(app.ok(json!({"command":"folder_layout"})), existing);
    app.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rest_api_routes_authenticate_and_reach_their_own_handlers() {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"database"}));
    app.ok(json!({"command":"seed_account","owner":101}));
    let key = "synthetic-e2e-api-key";
    let url = app.ok(json!({"command":"start_api","key":key}))["url"]
        .as_str()
        .unwrap()
        .to_string();

    let health = client
        .get(format!("{url}/api/v1/health"))
        .send()
        .await
        .unwrap();
    assert_eq!(health.status(), 200);
    assert_eq!(health.json::<Value>().await.unwrap()["status"], "ok");

    for (header, message) in [
        (None, "Missing X-API-Key header"),
        (Some("wrong"), "Invalid API key"),
    ] {
        let mut request = client.get(format!("{url}/api/v1/files"));
        if let Some(header) = header {
            request = request.header("X-API-Key", header);
        }
        let response = request.send().await.unwrap();
        assert_eq!(response.status(), 401);
        let body = response.json::<Value>().await.unwrap();
        assert_eq!(body["error"]["code"], "UNAUTHORIZED");
        assert_eq!(body["error"]["message"], message);
    }

    // Search has its own handler. It must not be taken for a file identifier.
    let get = |path: &str| {
        client
            .get(format!("{url}{path}"))
            .header("X-API-Key", key)
            .send()
    };
    let search = get("/api/v1/files/search?q=report").await.unwrap();
    assert_eq!(
        search.status(),
        503,
        "search reaches its handler and reports the missing connection"
    );
    assert_eq!(
        search.json::<Value>().await.unwrap()["error"]["code"],
        "NOT_CONNECTED"
    );

    // Malformed input is refused before anything is read.
    for (path, code) in [
        ("/api/v1/files/search", "INVALID_QUERY"),
        ("/api/v1/files?created_after=yesterday", "INVALID_TIMESTAMP"),
        ("/api/v1/files?folder_id=photos", "INVALID_FOLDER_ID"),
        // 2^32 + 7 must not be read as file 7.
        ("/api/v1/files/4294967303", "INVALID_FILE_ID"),
        ("/api/v1/files/4294967303/download", "INVALID_FILE_ID"),
        ("/api/v1/files/0/thumbnail", "INVALID_FILE_ID"),
    ] {
        let response = get(path).await.unwrap();
        assert_eq!(response.status(), 400, "{path}");
        assert_eq!(
            response.json::<Value>().await.unwrap()["error"]["code"],
            code,
            "{path}"
        );
    }
    // Errors from malformed input have the documented shape too.
    for (request, status, code) in [
        (
            client.get(format!("{url}/api/v1/files?page=first")),
            400,
            "INVALID_QUERY",
        ),
        (
            client
                .post(format!("{url}/api/v1/folders"))
                .body("{")
                .header("content-type", "application/json"),
            400,
            "INVALID_BODY",
        ),
        (
            client.get(format!("{url}/api/v1/files/report.pdf")),
            404,
            "NOT_FOUND",
        ),
    ] {
        let response = request.header("X-API-Key", key).send().await.unwrap();
        assert_eq!(response.status(), status);
        assert_eq!(
            response.json::<Value>().await.unwrap()["error"]["code"],
            code
        );
    }
    let truncated = client
        .delete(format!("{url}/api/v1/files/4294967303"))
        .header("X-API-Key", key)
        .send()
        .await
        .unwrap();
    assert_eq!(
        truncated.status(),
        400,
        "an out-of-range identifier never deletes another file"
    );

    // The published contract names real routes: every documented operation
    // exists and, unless marked public, refuses a request without a key.
    let contract = client
        .get(format!("{url}/api/v1/openapi.json"))
        .send()
        .await
        .unwrap();
    assert_eq!(contract.status(), 200);
    let contract = contract.json::<Value>().await.unwrap();
    assert_eq!(contract["openapi"], "3.1.0");
    let mut operations = 0;
    for (path, methods) in contract["paths"].as_object().unwrap() {
        let concrete = path
            .replace("{message_id}", "5")
            .replace("{folder_id}", "7");
        for (method, operation) in methods.as_object().unwrap() {
            let public = operation["security"]
                .as_array()
                .is_some_and(|list| list.is_empty());
            let response = client
                .request(
                    method.to_uppercase().parse().unwrap(),
                    format!("{url}{concrete}"),
                )
                .send()
                .await
                .unwrap();
            let expected = if public { 200 } else { 401 };
            assert_eq!(response.status(), expected, "{method} {path}");
            operations += 1;
        }
    }
    assert_eq!(operations, 20, "the contract covers every route");
    app.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rest_api_lists_search_and_reports_cover_the_whole_library_of_one_account() {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .unwrap();
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"database"}));
    app.ok(json!({"command":"seed_account","owner":101}));
    let key = "synthetic-e2e-api-key";
    let url = app.ok(json!({"command":"start_api","key":key}))["url"]
        .as_str()
        .unwrap()
        .to_string();

    // 230 files in one folder: more than any page, and more than the 100 to
    // 200 newest messages earlier releases looked at.
    let mut files = Vec::new();
    for index in 1..=230_i64 {
        let day = 1 + (index - 1) / 24;
        let hour = (index - 1) % 24;
        files.push(json!({
            "id": index, "folder": 500,
            "name": format!("report-{index:03}.pdf"),
            "size": 1000 + index, "mime": "application/pdf",
            "created": format!("2026-03-{day:02}T{hour:02}:00:00Z"),
        }));
    }
    // A renamed copy of report-001 in another folder, an encrypted envelope,
    // and one file in Saved Messages.
    files.push(
        json!({"id": 1, "folder": 600, "name": "Renamed copy.pdf", "uploadedAs": "report-001.pdf",
        "size": 1001, "mime": "application/pdf", "created": "2026-04-02T08:30:00Z"}),
    );
    files.push(json!({"id": 2, "folder": 600, "name": "TDENC2", "uploadedAs": "c0ffee.tdenc", "encrypted": true,
        "size": 4096, "mime": "application/octet-stream", "created": "2026-04-03T09:00:00Z"}));
    files.push(json!({"id": 9, "folder": null, "name": "notes.txt",
        "size": 12, "mime": "text/plain", "created": "2026-01-15T12:00:00Z"}));
    app.ok(json!({"command":"api_seed_catalog",
        "folders":[{"id":null,"name":"Saved Messages"},{"id":500,"name":"Reports"},{"id":600,"name":"Other"},{"id":700,"name":"Unused"}],
        "files": files}));

    let get = |path: String| {
        let request = client.get(format!("{url}{path}")).header("X-API-Key", key);
        async move {
            let response = request.send().await.unwrap();
            let status = response.status().as_u16();
            (status, response.json::<Value>().await.unwrap())
        }
    };

    // Totals describe the whole match, and paging visits every file once.
    let mut seen = std::collections::BTreeSet::new();
    for page in 1..=3 {
        let (status, body) =
            get(format!("/api/v1/files?folder_id=500&limit=100&page={page}")).await;
        assert_eq!(status, 200);
        assert_eq!(body["total"], 230);
        assert_eq!(body["complete"], true);
        assert_eq!(body["pagination"]["total_pages"], 3);
        assert_eq!(body["pagination"]["has_next"], page < 3);
        assert_eq!(body["pagination"]["has_prev"], page > 1);
        assert_eq!(body["data"], body["files"]);
        for file in body["data"].as_array().unwrap() {
            assert!(
                seen.insert(file["id"].as_i64().unwrap()),
                "a file appeared on two pages"
            );
        }
    }
    assert_eq!(seen.len(), 230);
    assert_eq!((seen.first(), seen.last()), (Some(&1), Some(&230)));

    // Timestamps are RFC 3339 and filter as instants, in any accepted form.
    let (_, oldest) = get("/api/v1/files?folder_id=500&limit=1".into()).await;
    assert_eq!(
        oldest["data"][0],
        json!({
            "id": 1, "folder_id": 500, "name": "report-001.pdf", "size": 1001,
            "mime_type": "application/pdf", "created_at": "2026-03-01T00:00:00Z", "encrypted": false,
        })
    );
    for bounds in [
        "created_after=2026-03-02T00:00:00Z&created_before=2026-03-02T23:00:00Z",
        "created_after=2026-03-02&created_before=2026-03-02%2023:00:00%20UTC",
        "created_after=1772409600&created_before=2026-03-02T23:59:59Z",
    ] {
        let (status, body) = get(format!("/api/v1/files?folder_id=500&{bounds}")).await;
        assert_eq!(status, 200, "{bounds}");
        assert_eq!(body["total"], 24, "{bounds}");
    }

    // Filters, ordering and field selection apply to the whole library.
    let (_, largest) =
        get("/api/v1/files?sort=size&order=desc&limit=2&fields=id,size".into()).await;
    assert_eq!(largest["total"], 233);
    assert_eq!(
        largest["data"],
        json!([{"id": 2, "size": 4096}, {"id": 230, "size": 1230}])
    );
    let (_, text) = get("/api/v1/files?mime_type=text&size_max=100".into()).await;
    assert_eq!(text["total"], 1);
    assert_eq!(text["data"][0]["folder_id"], Value::Null);
    let (_, saved) = get("/api/v1/files?folder_id=null".into()).await;
    assert_eq!(saved["total"], 1);
    let (_, older) = get("/api/v1/files?folder_id=500&offset_id=11".into()).await;
    assert_eq!(older["total"], 10);

    // Search reads every file, not the newest few.
    let (status, found) = get("/api/v1/files/search?q=REPORT-00".into()).await;
    assert_eq!(status, 200);
    assert_eq!(found.as_array().unwrap().len(), 9);
    let (_, scoped) = get("/api/v1/files/search?q=copy&folder_id=600".into()).await;
    assert_eq!(scoped.as_array().unwrap().len(), 1);
    assert_eq!(scoped[0]["name"], "Renamed copy.pdf");

    // Reports agree with the listing.
    let (_, stats) = get("/api/v1/storage/stats".into()).await;
    assert_eq!(stats["total_file_count"], 233);
    let report_bytes: u64 = (1..=230).map(|index| 1000 + index).sum();
    assert_eq!(
        stats["total_storage_used_bytes"],
        report_bytes + 1001 + 4096 + 12
    );
    assert_eq!(stats["complete"], true);
    assert_eq!(
        stats["folders"],
        json!([
            {"id": null, "name": "Saved Messages", "file_count": 1, "size_bytes": 12},
            {"id": 500, "name": "Reports", "file_count": 230, "size_bytes": report_bytes},
            {"id": 600, "name": "Other", "file_count": 2, "size_bytes": 5097},
            {"id": 700, "name": "Unused", "file_count": 0, "size_bytes": 0},
        ])
    );
    assert_eq!(
        stats["mime_types"],
        json!([
            {"mime_type": "application/octet-stream", "file_count": 1, "size_bytes": 4096},
            {"mime_type": "application/pdf", "file_count": 231, "size_bytes": report_bytes + 1001},
            {"mime_type": "text/plain", "file_count": 1, "size_bytes": 12},
        ])
    );
    let (_, duplicates) = get("/api/v1/storage/duplicates".into()).await;
    assert_eq!(duplicates.as_array().unwrap().len(), 1);
    assert_eq!(duplicates[0]["name"], "report-001.pdf");
    assert_eq!(duplicates[0]["files"].as_array().unwrap().len(), 2);
    let (_, empty) = get("/api/v1/folders/empty".into()).await;
    assert_eq!(empty.as_array().unwrap().len(), 1);
    assert_eq!(empty[0]["id"], 700);

    // Another account never receives this library.
    app.ok(json!({"command":"seed_account","owner":202}));
    for path in [
        "/api/v1/files?folder_id=500",
        "/api/v1/files/search?q=report",
        "/api/v1/storage/stats",
        "/api/v1/storage/duplicates",
        "/api/v1/folders/empty",
    ] {
        let (status, body) = get(path.into()).await;
        assert_eq!(status, 503, "{path}");
        assert_eq!(body["error"]["code"], "NOT_CONNECTED", "{path}");
    }
    app.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn webdav_serves_only_its_token_and_only_the_signed_in_account() {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .unwrap();
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"database"}));
    let token = "a".repeat(64);
    let url = app.ok(json!({"command":"start_webdav","token":token,"writeEnabled":false}))["url"]
        .as_str()
        .unwrap()
        .to_string();
    let propfind = reqwest::Method::from_bytes(b"PROPFIND").unwrap();
    let mkcol = reqwest::Method::from_bytes(b"MKCOL").unwrap();

    // Nothing is disclosed without the connection token: every other path,
    // including a well-formed wrong token, looks like it does not exist.
    for path in [
        "/",
        "/dav/",
        "/dav/short/",
        &format!("/dav/{}/", "b".repeat(64)),
    ] {
        for method in [
            reqwest::Method::GET,
            reqwest::Method::OPTIONS,
            propfind.clone(),
        ] {
            let response = client
                .request(method.clone(), format!("{url}{path}"))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 404, "{method} {path}");
            assert!(response.headers().get("dav").is_none(), "{method} {path}");
        }
    }
    // A move or copy cannot name a destination outside the token either.
    let escape = client
        .request(
            reqwest::Method::from_bytes(b"MOVE").unwrap(),
            format!("{url}/dav/{token}/a.txt"),
        )
        .header("destination", format!("{url}/dav/{}/a.txt", "b".repeat(64)))
        .send()
        .await
        .unwrap();
    assert_eq!(escape.status(), 404);

    // With the token but nobody signed in, the drive is refused, not empty.
    let root = format!("{url}/dav/{token}/");
    let signed_out = client
        .request(propfind.clone(), &root)
        .header("depth", "1")
        .send()
        .await
        .unwrap();
    assert_eq!(signed_out.status(), 403);

    // Signed in: the root lists Saved Messages under the token prefix, which
    // Finder and Explorer need to follow child resources.
    app.ok(json!({"command":"seed_account","owner":101}));
    let options = client
        .request(reqwest::Method::OPTIONS, &root)
        .send()
        .await
        .unwrap();
    assert_eq!(options.status(), 200);
    assert!(options.headers().get("dav").is_some());
    let listing = client
        .request(propfind.clone(), &root)
        .header("depth", "1")
        .send()
        .await
        .unwrap();
    assert_eq!(listing.status(), 207);
    let body = listing.text().await.unwrap();
    assert!(
        body.contains(&format!("/dav/{token}/Saved%20Messages/")),
        "{body}"
    );

    // A read-only mount refuses uploads and new folders before reaching
    // Telegram. Deletion and renaming read the target from Telegram first, so
    // they need a live connection and are part of real-account acceptance.
    for (method, path) in [
        (reqwest::Method::PUT, "Saved%20Messages/new.txt"),
        (mkcol, "New%20folder/"),
    ] {
        let mut request = client.request(method.clone(), format!("{root}{path}"));
        if method == reqwest::Method::PUT {
            request = request.body("synthetic");
        }
        let response = request.send().await.unwrap();
        assert_eq!(response.status(), 403, "{method} {path}");
    }
    app.stop();
}

// ---------------------------------------------------------------------------
// Large uploads. The uploader, its record in the account's store, and the
// envelope of a protected file are production code in a separate process.
// Telegram's part storage is the controlled boundary: parts are kept as files
// and one part can be refused to interrupt the upload. Publishing the message
// and Telegram discarding old parts are not exercised.
// ---------------------------------------------------------------------------

fn synthetic_bytes(length: usize, seed: u64) -> Vec<u8> {
    let mut state = seed;
    (0..length)
        .map(|_| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 33) as u8
        })
        .collect()
}

fn set_modified(path: &Path, modified: SystemTime) {
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(modified)
        .unwrap();
}

const LARGE_UPLOAD_BYTES: usize = 11 * 1024 * 1024 + 300_123;
const LARGE_UPLOAD_PARTS: i32 = 23;

#[test]
fn a_large_upload_continues_where_it_stopped_and_starts_over_when_the_file_changed() {
    let fixture = Fixture::new();
    let source = synthetic_bytes(LARGE_UPLOAD_BYTES, 7);
    fixture.write("big.bin", &source);
    let modified = settle(&fixture.path("big.bin"));
    let upload = |fail_at: Option<i32>, publish: bool| json!({"command":"upload_resumable","source":"big.bin","sink":"telegram-parts","failAtPart":fail_at,"publish":publish});
    let session = json!({"command":"upload_session","source":"big.bin"});
    let all_parts: Vec<i32> = (0..LARGE_UPLOAD_PARTS).collect();

    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"database"}));
    app.ok(json!({"command":"seed_account","owner":101}));

    // The connection drops at the tenth part. Everything confirmed before it
    // is recorded, including parts that were already on their way.
    assert!(app
        .failure(upload(Some(9), false))
        .contains("connection reset"));
    let interrupted = app.ok(session.clone());
    assert_eq!(interrupted["totalParts"], LARGE_UPLOAD_PARTS);
    assert_eq!(interrupted["completedParts"], 9);
    app.stop();

    // A new process sends only what is missing, under the same identifier,
    // and the stored parts are exactly the file.
    let mut app = Backend::start(&fixture.0);
    let finished = app.ok(upload(None, false));
    assert_eq!(finished["fileId"], interrupted["fileId"]);
    assert_eq!(finished["resumedParts"], 9);
    assert_eq!(
        finished["savedParts"],
        json!((9..LARGE_UPLOAD_PARTS).collect::<Vec<_>>())
    );
    app.ok(json!({"command":"upload_assemble","sink":"telegram-parts","fileId":finished["fileId"],"totalParts":LARGE_UPLOAD_PARTS,"destination":"assembled.bin"}));
    assert!(
        fixture.read("assembled.bin") == source,
        "the stored parts differ from the file"
    );

    // If publishing the message failed, the next attempt has nothing to send.
    let nothing_left = app.ok(upload(None, true));
    assert_eq!(nothing_left["resumedParts"], LARGE_UPLOAD_PARTS);
    assert_eq!(nothing_left["savedParts"], json!([]));
    // Once published, the finished upload is forgotten.
    assert_eq!(app.ok(session.clone()), Value::Null);

    // The file is rewritten with the same size and timestamp while an upload
    // is interrupted. The sent part no longer matches, so nothing of the old
    // upload is reused.
    assert!(app
        .failure(upload(Some(5), false))
        .contains("connection reset"));
    let stale = app.ok(session.clone());
    assert_eq!(stale["completedParts"], 5);
    let rewritten = synthetic_bytes(LARGE_UPLOAD_BYTES, 8);
    fixture.write("big.bin", &rewritten);
    set_modified(&fixture.path("big.bin"), modified);
    let refused = app.ok(upload(None, false));
    assert!(
        refused["startsOver"]
            .as_str()
            .unwrap()
            .contains("no longer matches"),
        "{refused}"
    );
    assert_eq!(app.ok(session.clone()), Value::Null);
    let restarted = app.ok(upload(None, false));
    assert_eq!(restarted["resumedParts"], 0);
    assert_ne!(restarted["fileId"], stale["fileId"]);
    assert_eq!(restarted["savedParts"], json!(all_parts));
    app.ok(json!({"command":"upload_assemble","sink":"telegram-parts","fileId":restarted["fileId"],"totalParts":LARGE_UPLOAD_PARTS,"destination":"assembled.bin"}));
    assert!(fixture.read("assembled.bin") == rewritten);
    app.ok(upload(None, true));

    // A file whose timestamp changed is not even compared: it starts over.
    assert!(app
        .failure(upload(Some(5), false))
        .contains("connection reset"));
    let previous = app.ok(session.clone());
    set_modified(&fixture.path("big.bin"), modified - Duration::from_secs(60));
    assert_eq!(app.ok(session.clone()), Value::Null);
    let restarted = app.ok(upload(None, true));
    assert_eq!(restarted["resumedParts"], 0);
    assert_ne!(restarted["fileId"], previous["fileId"]);

    // A file modified moments before its upload began could change again
    // without its timestamp moving, so its progress is never reused.
    fixture.write("big.bin", &source);
    assert!(app
        .failure(upload(Some(5), false))
        .contains("connection reset"));
    assert_eq!(app.ok(session.clone()), Value::Null);
    assert_eq!(app.ok(upload(None, true))["resumedParts"], 0);

    // Progress belongs to the account that made it.
    settle(&fixture.path("big.bin"));
    assert!(app
        .failure(upload(Some(5), false))
        .contains("connection reset"));
    assert_eq!(app.ok(session.clone())["completedParts"], 5);
    app.ok(json!({"command":"seed_account","owner":202}));
    assert_eq!(app.ok(session.clone()), Value::Null);
    assert_eq!(app.ok(upload(None, true))["resumedParts"], 0);
    app.ok(json!({"command":"seed_account","owner":101}));
    assert_eq!(app.ok(session)["completedParts"], 5);
    app.stop();
}

#[test]
fn a_protected_upload_continues_its_envelope_and_folder_sync_places_it_by_its_stored_path() {
    let fixture = Fixture::new();
    let source = synthetic_bytes(LARGE_UPLOAD_BYTES, 21);
    fixture.write("report.bin", &source);
    settle(&fixture.path("report.bin"));
    let upload = |fail_at: Option<i32>, publish: bool| {
        json!({"command":"upload_resumable","source":"report.bin","sink":"telegram-parts","protected":true,
            "syncPath":"docs/2026/report.bin","failAtPart":fail_at,"publish":publish})
    };
    let session = json!({"command":"upload_session","source":"report.bin","protected":true});

    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"database"}));
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!({"command":"vault_create","passphrase":"synthetic vault passphrase"}));

    assert!(app
        .failure(upload(Some(7), false))
        .contains("connection reset"));
    let interrupted = app.ok(session.clone());
    assert_eq!(interrupted["protected"], true);
    assert_eq!(interrupted["completedParts"], 7);
    let remote_name = interrupted["remoteName"].as_str().unwrap().to_string();
    assert!(remote_name.ends_with(".tdenc"));
    app.stop();

    // After a restart the vault is locked; the upload waits for it. Unlocked,
    // it continues the same envelope: same identifier, same remote name, and
    // only the missing parts are sent.
    let mut app = Backend::start(&fixture.0);
    app.failure(upload(None, false));
    assert_eq!(app.ok(session.clone())["completedParts"], 7);
    app.ok(json!({"command":"vault_unlock","passphrase":"synthetic vault passphrase"}));
    let finished = app.ok(upload(None, false));
    assert_eq!(finished["continuedEnvelope"], true);
    assert_eq!(finished["fileId"], interrupted["fileId"]);
    assert_eq!(finished["remoteName"], remote_name.as_str());
    assert_eq!(finished["resumedParts"], 7);
    let total_parts = finished["totalParts"].as_i64().unwrap();
    assert_eq!(
        finished["savedParts"],
        json!((7..total_parts).collect::<Vec<_>>())
    );

    // The parts stored across both attempts form one valid envelope that
    // decrypts to the original file.
    app.ok(json!({"command":"upload_assemble","sink":"telegram-parts","fileId":finished["fileId"],"totalParts":total_parts,"destination":"envelope.tdenc"}));
    let envelope = fixture.read("envelope.tdenc");
    assert!(!envelope.windows(16).any(|window| window == &source[..16]));
    assert!(
        !envelope.windows(10).any(|window| window == b"report.bin"),
        "the name must stay inside the encrypted metadata"
    );
    assert!(
        !envelope.windows(9).any(|window| window == b"docs/2026"),
        "the sync path must stay inside the encrypted metadata"
    );
    assert_eq!(
        app.ok(
            json!({"command":"read_envelope","source":"envelope.tdenc","destination":"opened.bin"})
        )["plaintextBytes"],
        LARGE_UPLOAD_BYTES
    );
    assert!(fixture.read("opened.bin") == source);
    app.ok(upload(None, true));
    assert_eq!(app.ok(session.clone()), Value::Null);

    // Different content never continues an envelope: it gets new keys.
    assert!(app
        .failure(upload(Some(4), false))
        .contains("connection reset"));
    let previous = app.ok(session.clone());
    let modified = std::fs::metadata(fixture.path("report.bin"))
        .unwrap()
        .modified()
        .unwrap();
    fixture.write("report.bin", synthetic_bytes(LARGE_UPLOAD_BYTES, 22));
    set_modified(&fixture.path("report.bin"), modified);
    assert!(app.ok(upload(None, false))["startsOver"].is_string());
    let renewed = app.ok(upload(None, true));
    assert_eq!(renewed["continuedEnvelope"], false);
    assert_eq!(renewed["resumedParts"], 0);
    assert_ne!(renewed["remoteName"], previous["remoteName"]);

    // Folder Sync has no record of the uploaded file (a new device, or a
    // mapping that was removed and added again). Its caption and remote name
    // say nothing; the path comes out of the envelope with the vault.
    let folder = sync_folder(&fixture);
    std::fs::create_dir_all(folder.join("docs/2026")).unwrap();
    std::fs::write(folder.join("docs/2026/report.bin"), &source).unwrap();
    settle(&folder.join("docs/2026/report.bin"));
    let pair = app.ok(json!({"command":"sync_seed_pair","folder":"library"}))["pairId"]
        .as_i64()
        .unwrap();
    let mut protected_file = telegram_file(71, Some(&remote_name), "TDENC2", envelope.len() as u64);
    protected_file["envelope"] = json!("envelope.tdenc");
    let plan = app
        .ok(json!({"command":"sync_cycle","pairId":pair,"remoteFiles":[protected_file.clone()]}));
    assert_eq!(plan["remoteFiles"], 1);
    // Both copies are known to be the same path. Nothing is uploaded again;
    // the user is asked, or adoption records the pair as already in sync.
    assert_eq!(
        planned(&plan),
        vec![("docs/2026/report.bin".to_string(), "conflict".to_string())]
    );
    app.ok(json!({"command":"sync_set_preferences","pairId":pair,"preferences":{"adoptMatchingFiles":true}}));
    let plan = app.ok(json!({"command":"sync_cycle","pairId":pair,"remoteFiles":[protected_file]}));
    assert_eq!(plan["baselined"], json!(["docs/2026/report.bin"]));
    assert_eq!(planned(&plan), vec![]);
    let state = app.ok(json!({"command":"sync_state","pairId":pair}));
    assert_eq!(state[0]["path"], "docs/2026/report.bin");
    assert_eq!(state[0]["status"], "synced");
    assert_eq!(state[0]["messageId"], 71);

    // Once recorded, later cycles need neither the vault nor the envelope.
    app.ok(json!({"command":"vault_lock"}));
    let recorded = telegram_file(71, Some(&remote_name), "TDENC2", envelope.len() as u64);
    let plan =
        app.ok(json!({"command":"sync_cycle","pairId":pair,"remoteFiles":[recorded.clone()]}));
    assert_eq!(
        (plan["remoteFiles"].as_i64(), planned(&plan)),
        (Some(1), vec![])
    );

    // A protected file that Folder Sync did not upload carries no path and is
    // left alone, as before.
    app.ok(json!({"command":"vault_unlock","passphrase":"synthetic vault passphrase"}));
    fixture.write("private.bin", b"uploaded by hand, not by Folder Sync");
    app.ok(
        json!({"command":"fixture_encrypt","source":"private.bin","destination":"private.tdenc"}),
    );
    let mut unrelated = telegram_file(
        72,
        Some("tdrive_unrelated.tdenc"),
        "TDENC2",
        fixture.read("private.tdenc").len() as u64,
    );
    unrelated["envelope"] = json!("private.tdenc");
    let plan =
        app.ok(json!({"command":"sync_cycle","pairId":pair,"remoteFiles":[recorded, unrelated]}));
    assert_eq!(
        (plan["remoteFiles"].as_i64(), planned(&plan)),
        (Some(1), vec![])
    );
    app.stop();
}

#[test]
fn a_transfer_run_for_folder_sync_reports_back_and_never_outlives_its_owner() {
    let fixture = Fixture::new();
    let job = |id: &str, owned_by_sync: bool| {
        let mut job = json!(
            {
                "id":id,
                "ownerId":"101",
                "direction":"upload",
                "kind":"local_upload",
                "status":"uploading",
                "path":"/synthetic/report.pdf",
                "filename":"report.pdf",
                "progress":40,
                "transferredBytes":20,
                "totalBytes":50,
                "speedBytesPerSec":9,
                "queuePosition":0,
                "revision":1,
                "createdAt":1,
                "updatedAt":2
            }
        );
        if owned_by_sync {
            job["origin"] = json!("sync");
            job["syncPath"] = json!("docs/report.pdf");
        }
        job
    };
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"save_transfer","job":job("upload-1", false)}));
    let now = 1_000_000;
    let fail = |app: &mut Backend, error: &str| {
        app.ok(json!({"command":"fail_transfer","id":"upload-1","error":error,"now":now}))
    };

    // While the queue is retrying or cooling down, whoever waits keeps waiting.
    let retrying = fail(&mut app, "Upload failed: connection reset by peer");
    assert_eq!(
        (retrying["status"].as_str(), &retrying["supervision"]),
        (Some("cooldown"), &json!("working"))
    );
    let cooling = fail(&mut app, "FLOOD_WAIT_30");
    assert_eq!(
        (cooling["status"].as_str(), &cooling["supervision"]),
        (Some("cooldown"), &json!("working"))
    );

    // What only the user can resolve is handed back with its reason.
    let locked = fail(
        &mut app,
        "[VAULT_LOCKED] Unlock the vault before starting this upload",
    );
    assert_eq!(locked["status"], "waiting_for_unlock");
    assert!(locked["supervision"]["stopped"]
        .as_str()
        .unwrap()
        .contains("VAULT_LOCKED"));
    let account = fail(
        &mut app,
        "ACCOUNT_CHANGED: The transfer belongs to a different account",
    );
    assert_eq!(account["status"], "paused");
    assert!(account["supervision"]["stopped"]
        .as_str()
        .unwrap()
        .contains("ACCOUNT_CHANGED"));
    let cancelled = fail(&mut app, "Transfer cancelled");
    assert_eq!(
        cancelled["supervision"],
        json!({"stopped":"Transfer cancelled"})
    );
    // So is a connection that stays down after every automatic retry.
    let mut last = Value::Null;
    for _ in 0..5 {
        last = fail(&mut app, "Upload failed: connection reset by peer");
    }
    assert_eq!(last["status"], "failed");
    assert!(last["supervision"]["stopped"]
        .as_str()
        .unwrap()
        .contains("connection reset"));

    // A transfer Folder Sync was running when the application stopped is not
    // kept: it could otherwise be resumed by hand and finish with nobody to
    // record the result. The user's own transfers are kept as before.
    app.ok(json!({"command":"save_transfer","job":job("sync-7", true)}));
    app.stop();
    let mut app = Backend::start(&fixture.0);
    let kept = app.ok(json!({"command":"transfers"}));
    assert_eq!(kept.as_array().unwrap().len(), 1, "{kept}");
    assert_eq!(kept[0]["id"], "upload-1");
    app.stop();
}

#[test]
fn folder_scan_keeps_peers_readable_and_publishes_only_complete_current_account_snapshots() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account", "owner":101}));
    app.ok(json!({"command":"peer_scan_seed"}));
    app.ok(json!({"command":"peer_scan_start"}));
    assert_eq!(
        app.ok(json!({"command":"peer_scan_read"})),
        json!({"1":"existing"})
    );
    assert_eq!(app.ok(json!({"command":"peer_scan_finish"})), 2);
    assert_eq!(
        app.ok(json!({"command":"peer_scan_read"})),
        json!({"2":"discovered", "3":"complete"})
    );
    app.ok(json!({"command":"peer_scan_start", "fail":true}));
    assert!(app
        .failure(json!({"command":"peer_scan_finish"}))
        .contains("interrupted"));
    assert_eq!(
        app.ok(json!({"command":"peer_scan_read"})),
        json!({"2":"discovered", "3":"complete"})
    );
    app.ok(json!({"command":"peer_scan_start"}));
    app.ok(json!({"command":"seed_account", "owner":202}));
    assert!(app
        .failure(json!({"command":"peer_scan_finish"}))
        .contains("ACCOUNT_CHANGED"));
    assert_eq!(
        app.ok(json!({"command":"peer_scan_read"})),
        json!({"2":"discovered", "3":"complete"})
    );
    app.stop();
}

#[tokio::test]
async fn media_http_ranges_filenames_failures_and_account_revocation() {
    let fixture = Fixture::new();
    let content: Vec<u8> = (0..1_200_000).map(|i| (i % 251) as u8).collect();
    fixture.write("media.bin", &content);
    fixture.write("empty.bin", []);
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account", "owner":101}));
    let filename = "résumé \"test\"\r\nheader\\.mp4";
    let started =
        app.ok(json!({"command":"start_media_fixture", "source":"media.bin", "filename":filename}));
    let url = started["url"].as_str().unwrap();
    let client = reqwest::Client::new();
    for (range, start, end) in [
        ("bytes=-17", content.len() - 17, content.len() - 1),
        ("bytes=524287-524301", 524287, 524301),
        ("bytes=1199985-", 1199985, content.len() - 1),
        ("bytes=1199990-9999999", 1199990, content.len() - 1),
        ("bytes=-9999999", 0, content.len() - 1),
    ] {
        let response = client.get(url).header("Range", range).send().await.unwrap();
        assert_eq!(response.status(), 206, "{range}");
        assert_eq!(
            response.headers()["content-range"],
            format!("bytes {start}-{end}/{}", content.len())
        );
        assert_eq!(response.content_length(), Some((end - start + 1) as u64));
        let disposition = response.headers()["content-disposition"].to_str().unwrap();
        assert!(
            disposition.contains("filename*=UTF-8''r%C3%A9sum%C3%A9"),
            "{disposition}"
        );
        assert!(!disposition.contains('\r') && !disposition.contains('\n'));
        assert_eq!(
            response.bytes().await.unwrap().as_ref(),
            &content[start..=end]
        );
    }
    for range in ["bytes=1200000-", "bytes=-0"] {
        let response = client.get(url).header("Range", range).send().await.unwrap();
        assert_eq!(response.status(), 416, "{range}");
        assert_eq!(response.headers()["content-range"], "bytes */1200000");
    }
    // Unsupported units, malformed syntax and multipart ranges are ignored.
    for range in [
        "items=0-1",
        "bytes=abc-def",
        "bytes=0-1,4-5",
        "bytes=0-1-2",
        "bytes=5-2",
    ] {
        let response = client.get(url).header("Range", range).send().await.unwrap();
        assert_eq!(response.status(), 200, "{range}");
        assert_eq!(response.bytes().await.unwrap().as_ref(), content.as_slice());
    }
    let empty =
        app.ok(json!({"command":"start_media_fixture", "source":"empty.bin", "filename":"empty"}));
    let response = client
        .get(empty["url"].as_str().unwrap())
        .header("Range", "bytes=0-")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 416);
    assert_eq!(response.headers()["content-range"], "bytes */0");
    let response = client
        .get(empty["url"].as_str().unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert!(response.bytes().await.unwrap().is_empty());
    for suffix in ["&failAfter=65536&delayMs=5", "&declaredSize=1300000"] {
        let result = client.get(format!("{url}{suffix}")).send().await;
        if let Ok(response) = result {
            assert!(response.bytes().await.is_err());
        }
    }
    let mut response = client
        .get(format!("{url}&delayMs=20"))
        .send()
        .await
        .unwrap();
    assert!(!response.chunk().await.unwrap().unwrap().is_empty());
    app.ok(json!({"command":"seed_account", "owner":202}));
    assert!(response.bytes().await.is_err());
    assert_eq!(client.get(url).send().await.unwrap().status(), 404);
    let fresh = app.ok(
        json!({"command":"start_media_fixture","source":"media.bin","filename":"owner-202.bin"}),
    );
    let response = client
        .get(fresh["url"].as_str().unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.bytes().await.unwrap().as_ref(), content.as_slice());
    app.stop();
}

#[tokio::test]
async fn protected_media_http_delivers_full_and_seeked_authenticated_content_without_the_four_mib_cutoff(
) {
    let fixture = Fixture::new();
    let content: Vec<u8> = (0..5_300_123).map(|i| (i % 251) as u8).collect();
    fixture.write("plain.bin", &content);
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account", "owner":101}));
    app.ok(json!({"command":"vault_create", "passphrase":"private fixture passphrase"}));
    app.ok(
        json!({"command":"fixture_encrypt", "source":"plain.bin", "destination":"protected.bin"}),
    );
    let started = app.ok(json!({"command":"start_media_fixture", "source":"protected.bin", "filename":"private.bin"}));
    let url = format!(
        "{}&protected=true&credential={}",
        started["url"].as_str().unwrap(),
        started["credential"].as_str().unwrap()
    );
    let client = reqwest::Client::new();
    let response = client.get(&url).send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.content_length(), Some(content.len() as u64));
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(response.bytes().await.unwrap().as_ref(), content.as_slice());
    for (range, start, end) in [
        ("bytes=-23", content.len() - 23, content.len() - 1),
        ("bytes=1048567-2097167", 1048567, 2097167),
        ("bytes=13-", 13, content.len() - 1),
    ] {
        let response = client
            .get(&url)
            .header("Range", range)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 206, "{range}");
        assert_eq!(
            response.headers()["content-range"],
            format!("bytes {start}-{end}/{}", content.len())
        );
        assert_eq!(
            response.bytes().await.unwrap().as_ref(),
            &content[start..=end]
        );
    }
    let response = client
        .get(&url)
        .header("Range", "bytes=9999999-")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 416);
    assert_eq!(
        response.headers()["content-range"],
        format!("bytes */{}", content.len())
    );
    // Corrupt a later record: authenticated prefix can be delivered but the
    // corrupted record must never be exposed as successful plaintext.
    let original = fixture.read("protected.bin");
    let mut corrupted = original.clone();
    let middle = 2_200_000;
    corrupted[middle] ^= 0x80;
    fixture.write("protected.bin", corrupted);
    let response = client.get(&url).send().await;
    if let Ok(response) = response {
        assert!(response.bytes().await.is_err());
    }
    fixture.write("protected.bin", &original[..original.len() - 1000]);
    let response = client.get(&url).send().await;
    if let Ok(response) = response {
        assert!(response.bytes().await.is_err());
    }
    let mut corrupt_footer = original.clone();
    *corrupt_footer.last_mut().unwrap() ^= 1;
    fixture.write("protected.bin", corrupt_footer);
    let response = client.get(&url).send().await;
    if let Ok(response) = response {
        assert!(response.bytes().await.is_err());
    }
    fixture.write("protected.bin", &original[..original.len() - 68]);
    let response = client.get(&url).send().await;
    if let Ok(response) = response {
        assert!(response.bytes().await.is_err());
    }
    let response = client
        .get(&url)
        .header("Range", "bytes=-23")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 206);
    assert_eq!(
        response.bytes().await.unwrap().as_ref(),
        &content[content.len() - 23..]
    );
    fixture.write("protected.bin", &original);
    let response = client
        .get(format!("{url}&failAfter=1300000&delayMs=1"))
        .send()
        .await;
    if let Ok(response) = response {
        assert!(response.bytes().await.is_err());
    }
    let mut response = client
        .get(format!("{url}&delayMs=10"))
        .send()
        .await
        .unwrap();
    assert!(!response.chunk().await.unwrap().unwrap().is_empty());
    app.ok(json!({"command":"vault_lock"}));
    assert!(response.bytes().await.is_err());
    assert_eq!(client.get(&url).send().await.unwrap().status(), 423);
    app.stop();
}

#[tokio::test]
async fn media_resolution_cache_reuses_concurrent_seeks_and_rejects_a_stale_account_session() {
    let fixture = Fixture::new();
    fixture.write("media.bin", vec![5; 100_000]);
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account", "owner":101}));
    let started = app
        .ok(json!({"command":"start_media_fixture", "source":"media.bin", "filename":"movie.bin"}));
    let url = started["url"].as_str().unwrap();
    let stats_url = format!("{}/resolutions", url.split("/media").next().unwrap());
    let client = reqwest::Client::new();
    let before = client
        .get(format!("{url}&resolveFailure=true"))
        .send()
        .await
        .unwrap();
    assert_eq!(before.status(), 502);
    let responses = futures::future::join_all((0..8).map(|i| {
        let client = client.clone();
        let url = url.to_string();
        async move {
            let response = client
                .get(&url)
                .header("Range", format!("bytes={i}-{}", i + 20))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 206);
            assert_eq!(response.bytes().await.unwrap().len(), 21);
        }
    }))
    .await;
    assert_eq!(responses.len(), 8);
    let resolutions: u64 = client
        .get(&stats_url)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        resolutions, 2,
        "failed lookup is retried; concurrent seeks share one successful lookup"
    );
    let response = client
        .get(url)
        .header("Range", "bytes=-11")
        .send()
        .await
        .unwrap();
    assert_eq!(response.bytes().await.unwrap().len(), 11);
    let resolutions: u64 = client
        .get(&stats_url)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resolutions, 2);
    app.ok(json!({"command":"seed_account", "owner":202}));
    assert_eq!(client.get(url).send().await.unwrap().status(), 404);
    let other_url = url.replace("owner=101", "owner=202");
    assert_eq!(client.get(&other_url).send().await.unwrap().status(), 200);
    let resolutions: u64 = client
        .get(&stats_url)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resolutions, 3);
    // Even a return to the same owner after sign-out starts a fresh session.
    app.ok(json!({"command":"workspace_suspend"}));
    app.ok(json!({"command":"workspace_resume"}));
    assert_eq!(client.get(&other_url).send().await.unwrap().status(), 200);
    let resolutions: u64 = client
        .get(&stats_url)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resolutions, 4);
    app.ok(json!({"command":"seed_account", "owner":101}));
    assert_eq!(client.get(url).send().await.unwrap().status(), 200);
    let resolutions: u64 = client
        .get(&stats_url)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resolutions, 5);
    app.stop();
}

#[tokio::test]
async fn media_resolution_cache_expires_evicts_and_discards_lookup_completed_after_account_switch()
{
    let fixture = Fixture::new();
    fixture.write("media.bin", vec![8; 100]);
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account", "owner":101}));
    let started = app.ok(json!({"command":"start_media_fixture", "source":"media.bin", "filename":"clip.bin", "cacheEntries":2, "cacheTtlMs":500}));
    let url = started["url"].as_str().unwrap();
    let base_url = url.split("/media").next().unwrap();
    let stats_url = format!("{base_url}/resolutions");
    let client = reqwest::Client::new();
    for id in [0, 1, 2, 0] {
        let response = client
            .get(format!("{url}&messageId={id}"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(response.bytes().await.unwrap().len(), 100);
    }
    let count: u64 = client
        .get(&stats_url)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(count, 4, "oldest entry was evicted at the size bound");
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(client.get(url).send().await.unwrap().status(), 200);
    let count: u64 = client
        .get(&stats_url)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(count, 5, "expired snapshot was refreshed");
    let pending_client = client.clone();
    let pending_url = format!("{url}&messageId=99&waitForResolution=true");
    let pending =
        tokio::spawn(async move { pending_client.get(pending_url).send().await.unwrap() });
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let count: u64 = client
            .get(&stats_url)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if count == 6 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "lookup never entered the resolution gate"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    app.ok(json!({"command":"seed_account", "owner":202}));
    assert_eq!(
        client
            .post(format!("{base_url}/resolve-continue"))
            .send()
            .await
            .unwrap()
            .status(),
        204
    );
    assert_eq!(pending.await.unwrap().status(), 404);
    assert_eq!(
        client
            .get(url.replace("owner=101", "owner=202"))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    let count: u64 = client
        .get(&stats_url)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(count, 7);
    app.stop();
}

#[test]
fn workspace_connections_are_reused_exclusively_and_never_cross_account_or_root() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"workspace_marker", "owner":101, "create":true}));
    assert_eq!(
        app.ok(json!({"command":"workspace_marker", "owner":101})),
        "lease"
    );
    assert!(app
        .failure(json!({"command":"workspace_marker", "owner":202}))
        .contains("no such table"));
    assert!(app
        .failure(json!({"command":"workspace_marker", "owner":101, "root":"other-root"}))
        .contains("no such table"));
    app.ok(json!({"command":"workspace_tx_start"}));
    assert_eq!(
        app.ok(json!({"command":"workspace_tx_read"})),
        json!([null, null]),
        "another lease must not observe uncommitted writes"
    );
    app.ok(json!({"command":"workspace_tx_finish"}));
    assert_eq!(
        app.ok(json!({"command":"workspace_tx_read"})),
        json!([1, 2])
    );
    app.ok(json!({"command":"workspace_tx_start", "fail":true}));
    assert!(app
        .failure(json!({"command":"workspace_tx_finish"}))
        .contains("interrupted"));
    assert_eq!(
        app.ok(json!({"command":"workspace_tx_read"})),
        json!([1, 2])
    );
    {
        let future = sqlite::open(fixture.0.join("workspace/101/workspace.db")).unwrap();
        future.execute("PRAGMA user_version=99").unwrap();
    }
    assert!(app
        .failure(json!({"command":"workspace_marker", "owner":101}))
        .contains("newer than this application"));
    app.stop();
}

#[test]
fn shares_database_uses_wal_and_accepts_a_writer_while_another_process_holds_a_read_snapshot() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    let mode = app.ok(json!({"command":"database_mode"}));
    assert_eq!(mode["journal"], "wal");
    assert_eq!(mode["busyMs"], 5000);
    let reader = sqlite::open(fixture.path("shares.db")).unwrap();
    reader.execute("BEGIN").unwrap();
    let mut query = reader
        .prepare("SELECT COUNT(*) FROM folder_metadata")
        .unwrap();
    query.next().unwrap();
    assert_eq!(query.read::<i64, _>(0).unwrap(), 0);
    drop(query);
    app.ok(json!({"command":"seed_folder_row", "id":11, "name":"concurrent writer"}));
    let mut query = reader
        .prepare("SELECT COUNT(*) FROM folder_metadata")
        .unwrap();
    query.next().unwrap();
    assert_eq!(
        query.read::<i64, _>(0).unwrap(),
        0,
        "read transaction stays consistent"
    );
    drop(query);
    reader.execute("COMMIT").unwrap();
    let mut query = reader
        .prepare("SELECT COUNT(*) FROM folder_metadata")
        .unwrap();
    query.next().unwrap();
    assert_eq!(query.read::<i64, _>(0).unwrap(), 1);
    app.stop();
}

#[test]
fn workspace_pages_bound_payloads_and_keep_a_consistent_inventory_during_writes_and_account_changes(
) {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account", "owner":101}));
    let file = |id: usize, name: String| {
        json!(
            {
                "id":id,
                "folder_id":null,
                "name":name,
                "size":100,
                "mime_type":"image/jpeg",
                "file_ext":"jpg",
                "created_at":"2026-10-01T00:00:00Z",
                "icon_type":"image",
                "encryption_state":if id==700 {
                    "encrypted_unlocked"
                }else {
                    "plain"
                }
            }
        )
    };
    let files: Vec<_> = (1..=700)
        .map(|id| file(id, format!("Photo {id}.jpg")))
        .collect();
    app.ok(json!({"command":"workspace_seed_files", "owner":"101", "files":files, "hidden":["saved:1","saved:2"], "tagged":"saved:699"}));
    let first = app.ok(json!({"command":"workspace_page", "owner":"101", "limit":256}));
    assert_eq!(first["files"].as_array().unwrap().len(), 256);
    assert_eq!(first["totalFiles"], 698);
    app.ok(json!({"command":"workspace_seed_files", "owner":"101", "files":[file(699,"Renamed outside page snapshot.jpg".into()),file(701,"New file.jpg".into())]}));
    let mut all = first["files"].as_array().unwrap().clone();
    let mut cursor = first["nextCursor"].clone();
    while let Some(token) = cursor.as_str() {
        let page =
            app.ok(json!({"command":"workspace_page", "owner":"101", "limit":256, "cursor":token}));
        assert!(page["files"].as_array().unwrap().len() <= 256);
        assert_eq!(page["totalFiles"], 698);
        all.extend_from_slice(page["files"].as_array().unwrap());
        cursor = page["nextCursor"].clone();
    }
    assert_eq!(all.len(), 698);
    let keys: std::collections::HashSet<_> = all
        .iter()
        .map(|file| file["key"].as_str().unwrap())
        .collect();
    assert_eq!(keys.len(), 698);
    assert!(!keys.contains("saved:1") && !keys.contains("saved:2") && !keys.contains("saved:701"));
    let tagged = all.iter().find(|file| file["id"] == 699).unwrap();
    assert_eq!(tagged["name"], "Photo 699.jpg");
    assert_eq!(tagged["tags"], json!(["page-tag"]));
    assert_eq!(
        all.iter().find(|file| file["id"] == 700).unwrap()["name"],
        "Protected file"
    );
    let fresh = app.ok(json!({"command":"workspace_page", "owner":"101", "limit":256}));
    assert_eq!(fresh["totalFiles"], 699);
    let stale = fresh["nextCursor"].as_str().unwrap();
    app.ok(json!({"command":"seed_account", "owner":202}));
    assert!(app
        .failure(json!({"command":"workspace_page", "owner":"202", "cursor":stale}))
        .contains("ACCOUNT_CHANGED"));
    let other = app.ok(json!({"command":"workspace_page", "owner":"202"}));
    assert_eq!(other["totalFiles"], 0);
    app.stop();
}

#[test]
fn workspace_paging_stays_responsive_with_large_removal_history() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!({"command":"workspace_seed_files","owner":"101","files":[]}));
    // Persist a realistic large library and deletion history in another process's
    // database connection, then ask the application's paged IPC boundary to read it.
    let database = sqlite::open(fixture.path("workspace/101/workspace.db")).unwrap();
    database.execute("BEGIN").unwrap();
    for id in 1..=10_000 {
        let metadata = json!(
            {
                "id":id,
                "folder_id":null,
                "name":format!("Document {id}.pdf"),
                "size":100,
                "mime_type":"application/pdf",
                "file_ext":"pdf",
                "created_at":"2026-10-01T00:00:00Z",
                "icon_type":"file",
                "encryption_state":"plain"
            }
        )
        .to_string();
        let mut insert = database.prepare("INSERT INTO workspace_files(key,folder,metadata,folder_name) VALUES(?,'saved',?,'Saved Messages')").unwrap();
        insert.bind((1, format!("saved:{id}").as_str())).unwrap();
        insert.bind((2, metadata.as_str())).unwrap();
        insert.next().unwrap();
    }
    for id in 1..=1_000 {
        let key = format!("saved:{id}");
        let value = json!({"key":key,"status":"pending"}).to_string();
        let mut insert = database
            .prepare("INSERT INTO workspace_records(kind,id,value,updated) VALUES('removal',?,?,1)")
            .unwrap();
        insert.bind((1, key.as_str())).unwrap();
        insert.bind((2, value.as_str())).unwrap();
        insert.next().unwrap();
    }
    database.execute("COMMIT").unwrap();
    drop(database);
    let start = Instant::now();
    let page = app.ok(json!({"command":"workspace_page","owner":"101","limit":256}));
    assert_eq!(page["totalFiles"], 9000);
    assert_eq!(page["files"].as_array().unwrap().len(), 256);
    assert!(page["files"]
        .as_array()
        .unwrap()
        .iter()
        .all(|file| file["id"].as_i64().unwrap() > 1000));
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "paged read with deletion history took {:?}",
        start.elapsed()
    );
    app.stop();
}

#[test]
fn archive_staging_propagates_download_failures_instead_of_accepting_a_complete_prefix() {
    let fixture = Fixture::new();
    fixture.write("archive.bin", vec![3; 32_777]);
    let mut app = Backend::start(&fixture.0);
    assert_eq!(
        app.ok(json!({"command":"archive_stage_fixture", "source":"archive.bin"})),
        32777
    );
    assert!(app
        .failure(json!({"command":"archive_stage_fixture", "source":"archive.bin", "fail":true}))
        .contains("transport interrupted"));
    let staging = std::fs::read_dir(&fixture.0)
        .unwrap()
        .filter_map(Result::ok)
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("telegram-drive-staging")
        })
        .unwrap()
        .path();
    assert_eq!(std::fs::read_dir(staging).unwrap().count(), 0);
    assert!(app
        .failure(json!({"command":"archive_stage_fixture", "source":"archive.bin", "maxBytes":100}))
        .contains("limit"));
    app.stop();
}

#[test]
fn zip_preview_and_extraction_use_private_files_verify_crc_limits_and_cleanup_cancelled_or_stale_staging(
) {
    let fixture = Fixture::new();
    let payload: Vec<u8> = (0..2_100_019).map(|index| (index % 251) as u8).collect();
    let write_zip = |name: &str, compression: zip::CompressionMethod, body: &[u8]| {
        let mut archive = zip::ZipWriter::new(std::fs::File::create(fixture.path(name)).unwrap());
        archive
            .start_file(
                "../../nested/document.bin",
                zip::write::SimpleFileOptions::default().compression_method(compression),
            )
            .unwrap();
        archive.write_all(body).unwrap();
        archive.finish().unwrap();
    };
    write_zip("document.zip", zip::CompressionMethod::Stored, &payload);
    write_zip(
        "bomb.zip",
        zip::CompressionMethod::Deflated,
        &vec![0; 2_100_019],
    );
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let listed =
        app.ok(json!({"command":"archive_zip_fixture","owner":"101","source":"document.zip"}));
    assert_eq!(listed[0]["size"], payload.len());
    let staging = std::fs::read_dir(&fixture.0)
        .unwrap()
        .filter_map(Result::ok)
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("telegram-drive-staging")
        })
        .unwrap()
        .path()
        .canonicalize()
        .unwrap();
    assert_eq!(std::fs::read_dir(&staging).unwrap().count(), 0);
    let extracted = app.ok(
        json!({"command":"archive_zip_fixture","owner":"101","source":"document.zip","entry":0}),
    );
    assert_eq!(extracted["filename"], "document.bin");
    let extracted_path = PathBuf::from(extracted["temp_path"].as_str().unwrap());
    assert_eq!(extracted_path.parent(), Some(staging.as_path()));
    assert_eq!(std::fs::read(&extracted_path).unwrap(), payload);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&extracted_path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    app.ok(json!({"command":"archive_delete_fixture","path":extracted["temp_path"]}));
    let original = fixture.read("document.zip");
    let mut damaged = original.clone();
    let data_offset = {
        let mut zip =
            zip::ZipArchive::new(std::fs::File::open(fixture.path("document.zip")).unwrap())
                .unwrap();
        let offset = zip
            .by_index(0)
            .unwrap()
            .data_start()
            .expect("stored ZIP fixture has a data offset") as usize;
        offset
    };
    damaged[data_offset + 100] ^= 1;
    fixture.write("damaged.zip", damaged);
    assert!(app
        .failure(
            json!({"command":"archive_zip_fixture","owner":"101","source":"damaged.zip","entry":0})
        )
        .contains("extraction failed"));
    assert!(app
        .failure(json!({"command":"archive_zip_fixture","owner":"101","source":"bomb.zip"}))
        .contains("compression ratio"));
    fixture.write("truncated.zip", &original[..original.len() - 30]);
    app.failure(json!({"command":"archive_zip_fixture","owner":"101","source":"truncated.zip"}));
    assert_eq!(std::fs::read_dir(&staging).unwrap().count(), 0);
    app.ok(json!({"command":"archive_stage_start"}));
    assert_eq!(std::fs::read_dir(&staging).unwrap().count(), 2);
    app.ok(json!({"command":"archive_stage_cancel"}));
    assert_eq!(std::fs::read_dir(&staging).unwrap().count(), 0);
    app.ok(json!({"command":"archive_stage_start"}));
    app.ok(json!({"command":"seed_account","owner":202}));
    assert!(app
        .failure(json!({"command":"archive_stage_finish"}))
        .contains("ACCOUNT_CHANGED"));
    assert_eq!(std::fs::read_dir(&staging).unwrap().count(), 0);
    app.stop();
}

#[test]
fn incremental_inventory_reconciles_external_changes_and_repairs_cold_walk_races_on_audit() {
    let fixture = Fixture::new();
    let write =
        |value: Value| fixture.write("remote-inventory.json", serde_json::to_vec(&value).unwrap());
    write(
        json!({"highwater":251,"rows":(1..=250).map(|id|json!({"id":id,"name":format!("Remote {id}"),"protected":id==250})).collect::<Vec<_>>() }),
    );
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let read = |audit: bool, poll: bool| json!({"command":"inventory_list","owner":"101","source":"remote-inventory.json","audit":audit,"poll":poll});
    let first = app.ok(read(false, false));
    assert_eq!(first["rows"].as_array().unwrap().len(), 250);
    assert_eq!(first["historyWalks"], 1);
    assert_eq!(first["lookupBatches"], 0);
    assert_eq!(app.ok(read(false, false))["lookupBatches"], 0);
    write(
        json!({"highwater":252,"rows":[{"id":1,"name":"Outside rename"},{"id":252,"name":"Outside addition"}]}),
    );
    let catchup = app.ok(read(false, true));
    assert_eq!(
        catchup["rows"].as_array().unwrap().len(),
        251,
        "poll only catches up new messages before audit"
    );
    let audited = app.ok(read(true, false));
    assert_eq!(
        audited["rows"],
        json!([{"id":252,"name":"Outside addition","protected":false},{"id":1,"name":"Outside rename","protected":false}])
    );
    assert_eq!(
        audited["historyWalks"], 1,
        "steady state must not walk old history"
    );
    write(json!({"highwater":252,"rows":[{"id":1,"name":"Keep"}],"lookupError":true}));
    assert!(app
        .failure(read(true, false))
        .contains("verification fixture unavailable"));
    let store = sqlite::open(fixture.path("workspace/101/workspace.db")).unwrap();
    let mut query = store
        .prepare("SELECT value FROM workspace_records WHERE kind='file-inventory-v1'")
        .unwrap();
    query.next().unwrap();
    let durable: Value = serde_json::from_str(&query.read::<String, _>(0).unwrap()).unwrap();
    assert_eq!(durable["ids"], json!([1, 252]));
    assert!(
        !query.read::<String, _>(0).unwrap().contains("Outside"),
        "persistent inventory is identifiers only"
    );
    drop(query);
    drop(store);
    write(
        json!({"highwater":253,"rows":[{"id":1,"name":"Keep"},{"id":253,"name":"After newest deletion"}]}),
    );
    app.stop();
    let mut app = Backend::start(&fixture.0);
    let restarted = app.ok(read(false, false));
    assert_eq!(restarted["historyWalks"], 1);
    assert_eq!(restarted["rows"][0]["id"], 253);
    app.ok(json!({"command":"seed_account","owner":202}));
    assert!(app.failure(read(false, false)).contains("ACCOUNT_CHANGED"));
    write(json!(
        {
            "highwater":2,
            "rows":[
                {
                    "id":1,
                    "name":"Before walk"
                },
                {
                    "id":2,
                    "name":"Deleted during walk"
                }
            ],
            "bootstrapRace":{
                "highwater":3,
                "rows":[
                    {
                        "id":1,
                        "name":"Edited during walk"
                    },
                    {
                        "id":3,
                        "name":"Added during walk"
                    }
                ]
            }
        }
    ));
    let raced =
        app.ok(json!({"command":"inventory_list","owner":"202","source":"remote-inventory.json"}));
    assert_eq!(raced["rows"][0]["name"], "Deleted during walk");
    let other = app.ok(json!({"command":"inventory_list","owner":"202","source":"remote-inventory.json","audit":true}));
    assert_eq!(
        other["rows"],
        json!([{"id":3,"name":"Added during walk","protected":false},{"id":1,"name":"Edited during walk","protected":false}])
    );
    app.stop();
}

#[test]
fn protected_folder_metadata_is_redacted_in_the_legacy_persistent_inventory_too() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!(
        {
            "command":"legacy_inventory_write",
            "file":{
                "id":1,
                "folder_id":null,
                "name":"Secret personal filename.pdf",
                "size":100,
                "mime_type":"application/pdf",
                "file_ext":"pdf",
                "created_at":"2026-10-01T00:00:00Z",
                "icon_type":"file",
                "encryption_state":"encrypted_unlocked"
            }
        }
    ));
    let database = sqlite::open(fixture.path("shares.db")).unwrap();
    let mut query = database
        .prepare("SELECT file_name,mime_type,file_ext,encryption_state FROM file_inventory")
        .unwrap();
    query.next().unwrap();
    assert_eq!(query.read::<String, _>(0).unwrap(), "Protected file");
    assert_eq!(
        query.read::<String, _>(1).unwrap(),
        "application/octet-stream"
    );
    assert_eq!(query.read::<Option<String>, _>(2).unwrap(), None);
    assert_eq!(query.read::<String, _>(3).unwrap(), "encrypted_locked");
    drop(query);
    drop(database);
    app.stop();
}

#[test]
fn webdav_upload_transport_reopens_the_source_and_bounds_retries_without_publishing_failed_attempts(
) {
    let fixture = Fixture::new();
    fixture.write("dav-upload.bin", vec![7; 512_019]);
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let sent=app.ok(json!({"command":"webdav_upload_fixture","owner":"101","source":"dav-upload.bin","target":"dav-sent.bin","failAttempts":1}));
    assert_eq!(sent, json!({"attempts":2,"error":null}));
    assert_eq!(fixture.read("dav-sent.bin"), fixture.read("dav-upload.bin"));
    let failure=app.ok(json!({"command":"webdav_upload_fixture","owner":"101","source":"dav-upload.bin","target":"dav-failed.bin","failAttempts":5}));
    assert_eq!(failure, json!({"attempts":3,"error":"IsRemote"}));
    assert!(!fixture.path("dav-failed.bin").exists());
    let forbidden=app.ok(json!({"command":"webdav_upload_fixture","owner":"101","source":"dav-upload.bin","target":"dav-forbidden.bin","forbidden":true}));
    assert_eq!(forbidden, json!({"attempts":1,"error":"Forbidden"}));
    assert!(!fixture.path("dav-forbidden.bin").exists());
    app.stop();
}

#[test]
fn inventory_single_flights_concurrent_consumers_and_keeps_same_owner_roots_separate() {
    let fixture = Fixture::new();
    fixture.write(
        "remote-a.json",
        serde_json::to_vec(&json!({"highwater":2,"rows":[{"id":1,"name":"First root"}]})).unwrap(),
    );
    fixture.write(
        "remote-b.json",
        serde_json::to_vec(&json!({"highwater":2,"rows":[{"id":1,"name":"Other root"}]})).unwrap(),
    );
    std::fs::create_dir(fixture.path("other-root")).unwrap();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let first =
        app.ok(json!({"command":"inventory_parallel","owner":"101","source":"remote-a.json"}));
    assert_eq!(first["historyWalks"], 1);
    assert_eq!(first["rows"][0]["name"], "First root");
    app.ok(json!({"command":"seed_account","owner":101,"root":"other-root"}));
    let other=app.ok(json!({"command":"inventory_list","owner":"101","source":"remote-b.json","root":"other-root"}));
    assert_eq!(other["rows"][0]["name"], "Other root");
    assert_eq!(other["historyWalks"], 2);
    let again = app.ok(json!({"command":"inventory_list","owner":"101","source":"remote-a.json"}));
    assert_eq!(again["rows"][0]["name"], "First root");
    fixture.write(
        "remote-a.json",
        serde_json::to_vec(
            &json!({"highwater":2,"rows":[{"id":1,"name":"Committed local rename"}]}),
        )
        .unwrap(),
    );
    app.ok(json!({"command":"listing_rename","owner":"101","name":"Committed local rename"}));
    let changed =
        app.ok(json!({"command":"inventory_list","owner":"101","source":"remote-a.json"}));
    assert_eq!(changed["rows"][0]["name"], "Committed local rename");
    assert_eq!(changed["historyWalks"], 2);
    app.stop();
}

#[test]
fn folder_publication_rejects_a_local_mutation_after_hydration_without_overwriting_it() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let file = json!(
        {
            "id":1,
            "folder_id":null,
            "name":"Old inventory name.pdf",
            "size":100,
            "mime_type":"application/pdf",
            "file_ext":"pdf",
            "created_at":"2026-10-01T00:00:00Z",
            "icon_type":"file",
            "encryption_state":"plain"
        }
    );
    app.ok(json!({"command":"workspace_seed_files","owner":"101","files":[file.clone()]}));
    app.ok(json!({"command":"publication_start","owner":"101","file":file}));
    app.ok(json!({"command":"listing_rename","owner":"101","name":"Committed local rename.pdf"}));
    assert!(app
        .failure(json!({"command":"publication_finish"}))
        .contains("INVENTORY_CHANGED"));
    let page = app.ok(json!({"command":"workspace_page","owner":"101"}));
    assert_eq!(page["files"][0]["name"], "Committed local rename.pdf");
    app.stop();
}

#[test]
fn folder_publication_rejects_hydrated_protected_names_after_the_vault_locks() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!({"command":"vault_create","passphrase":"listing-vault-fixture-passphrase"}));
    let file = json!(
        {
            "id":1,
            "folder_id":null,
            "name":"Private decrypted filename.pdf",
            "size":100,
            "mime_type":"application/pdf",
            "file_ext":"pdf",
            "created_at":"2026-10-01T00:00:00Z",
            "icon_type":"file",
            "encryption_state":"encrypted_unlocked"
        }
    );
    app.ok(json!({"command":"publication_start","owner":"101","file":file}));
    app.ok(json!({"command":"vault_lock"}));
    assert!(app
        .failure(json!({"command":"publication_finish"}))
        .contains("VAULT_LOCKED"));
    let page = app.ok(json!({"command":"workspace_page","owner":"101"}));
    assert_eq!(page["files"][0]["name"], "Protected file");
    app.stop();
}

#[test]
fn inventory_at_the_capacity_limit_audits_an_external_delete_before_accepting_its_replacement() {
    let fixture = Fixture::new();
    let rows = |start: i32, end: i32| json!({"highwater":end,"rows":(start..=end).map(|id|json!({"id":id,"name":format!("File {id}")})).collect::<Vec<_>>()});
    fixture.write(
        "capacity.json",
        serde_json::to_vec(&rows(1, 50_000)).unwrap(),
    );
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let first = app.ok(json!({"command":"inventory_list","owner":"101","source":"capacity.json"}));
    assert_eq!(first["rows"].as_array().unwrap().len(), 50_000);
    fixture.write(
        "capacity.json",
        serde_json::to_vec(&rows(2, 50_001)).unwrap(),
    );
    let replacement = app
        .ok(json!({"command":"inventory_list","owner":"101","source":"capacity.json","poll":true}));
    assert_eq!(replacement["rows"].as_array().unwrap().len(), 50_000);
    assert_eq!(replacement["rows"][0]["id"], 50_001);
    assert!(replacement["rows"]
        .as_array()
        .unwrap()
        .iter()
        .all(|row| row["id"] != 1));
    assert_eq!(replacement["historyWalks"], 1);
    app.stop();
}

#[test]
fn inventory_reconciliation_survives_an_abandoned_waiter_without_restarting_the_history_walk() {
    let fixture = Fixture::new();
    fixture.write("slow-inventory.json",serde_json::to_vec(&json!({"highwater":1,"historyDelayMs":50,"rows":[{"id":1,"name":"Completed shared bootstrap"}]})).unwrap());
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let completed =
        app.ok(json!({"command":"inventory_waiter","owner":"101","source":"slow-inventory.json"}));
    assert_eq!(
        completed["historyWalks"], 1,
        "abandoned waiter restarted the shared history walk"
    );
    assert_eq!(completed["rows"][0]["name"], "Completed shared bootstrap");
    let database = sqlite::open(fixture.path("workspace/101/workspace.db")).unwrap();
    let mut query = database
        .prepare("SELECT value FROM workspace_records WHERE kind='file-inventory-v1'")
        .unwrap();
    query.next().unwrap();
    let cursor: Value = serde_json::from_str(&query.read::<String, _>(0).unwrap()).unwrap();
    assert_eq!(cursor["ids"], json!([1]));
    drop(query);
    drop(database);
    app.stop();
}

#[test]
fn inventory_background_polling_detects_external_renames_emits_only_changes_and_does_not_renew_idle_retention(
) {
    let fixture = Fixture::new();
    for (file, name) in [
        ("timer-initial.json", "Initial name"),
        ("timer-changed.json", "Outside rename"),
        ("timer-cold.json", "After idle"),
    ] {
        fixture.write(
            file,
            serde_json::to_vec(&json!({"highwater":1,"rows":[{"id":1,"name":name}]})).unwrap(),
        );
    }
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let result=app.ok(json!({"command":"inventory_timer_lifecycle","owner":"101","initial":"timer-initial.json","changed":"timer-changed.json","cold":"timer-cold.json"}));
    assert_eq!(result["unchangedEvents"], 0);
    assert_eq!(result["changedEvents"], 1);
    assert_eq!(result["changedName"], "Outside rename");
    assert_eq!(result["coldName"], "After idle");
    assert_eq!(
        result["coldLookupBatches"], 0,
        "idle slot uses one cold history walk without redundant identifier hydration"
    );
    app.stop();
}

#[test]
fn inventory_deadline_failure_never_certifies_a_partial_bootstrap() {
    let fixture = Fixture::new();
    fixture.write(
        "deadline.json",
        serde_json::to_vec(
            &json!({"highwater":1,"historyDelayMs":100,"rows":[{"id":1,"name":"Never certified"}]}),
        )
        .unwrap(),
    );
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let result =
        app.ok(json!({"command":"inventory_deadline","owner":"101","source":"deadline.json"}));
    assert!(result["error"]
        .as_str()
        .unwrap()
        .contains("INVENTORY_STALE"));
    assert!(result["cursor"].is_null());
    app.stop();
}

#[test]
fn account_changes_cancel_a_shared_inventory_job_and_leave_no_completed_old_owner_cursor() {
    let fixture = Fixture::new();
    fixture.write(
        "watch-old.json",
        serde_json::to_vec(
            &json!({"highwater":1,"historyGate":true,"rows":[{"id":1,"name":"Old owner data"}]}),
        )
        .unwrap(),
    );
    fixture.write(
        "watch-new.json",
        serde_json::to_vec(&json!({"highwater":1,"rows":[{"id":1,"name":"New owner data"}]}))
            .unwrap(),
    );
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!({"command":"inventory_watch_start","owner":"101","source":"watch-old.json"}));
    // Exceed the old delay while the remote history remains genuinely unfinished.
    std::thread::sleep(Duration::from_millis(1200));
    let inventory_rows = || {
        let database = sqlite::open(fixture.path("workspace/101/workspace.db")).unwrap();
        let mut query = database
            .prepare("SELECT COUNT(*) FROM workspace_records WHERE kind='file-inventory-v1'")
            .unwrap();
        query.next().unwrap();
        query.read::<i64, _>(0).unwrap()
    };
    assert_eq!(
        inventory_rows(),
        0,
        "History must stay gated before the switch"
    );
    app.ok(json!({"command":"seed_account","owner":202}));
    fixture.write("watch-old.release", b"release");
    assert!(app
        .failure(json!({"command":"inventory_watch_finish"}))
        .contains("ACCOUNT_CHANGED"));
    assert_eq!(inventory_rows(), 0);
    let current =
        app.ok(json!({"command":"inventory_list","owner":"202","source":"watch-new.json"}));
    assert_eq!(current["rows"][0]["name"], "New owner data");
    app.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn webdav_maps_remote_permissions_missing_files_and_transport_failures_to_http_statuses() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let client = reqwest::Client::new();
    let token = "c".repeat(64);
    for (error, status) in [
        ("INVENTORY_STALE: CHANNEL_PRIVATE", 403),
        ("MESSAGE_ID_INVALID", 404),
        ("Connection reset", 502),
    ] {
        let url = app.ok(json!({"command":"start_webdav","token":token,"remoteError":error}))
            ["url"]
            .as_str()
            .unwrap()
            .to_string();
        let response = client
            .request(
                reqwest::Method::GET,
                format!("{url}/dav/{token}/Saved%20Messages/file.txt"),
            )
            .header("depth", "1")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), status, "{error}");
    }
    app.stop();
}

#[test]
fn completed_inventory_jobs_are_delivered_before_another_expired_audit_is_started() {
    let fixture = Fixture::new();
    fixture.write("completed.json",serde_json::to_vec(&json!({"highwater":1,"historyDelayMs":50,"rows":[{"id":1,"name":"Completed while nobody waited"}]})).unwrap());
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let result=app.ok(json!({"command":"inventory_waiter","owner":"101","source":"completed.json","completeBeforeWait":true}));
    assert_eq!(result["rows"][0]["name"], "Completed while nobody waited");
    assert_eq!(
        result["lookupBatches"], 0,
        "completed cold walk must not introduce redundant verification"
    );
    assert_eq!(
        result["historyCalls"], 1,
        "completed reconciliation was discarded and restarted"
    );
    app.stop();
}

#[test]
fn an_api_consumer_cannot_consume_the_desktop_inventory_change_notification() {
    let fixture = Fixture::new();
    for (path, name) in [
        ("initial.json", "Original"),
        ("changed.json", "Renamed elsewhere"),
        ("cold.json", "Idle"),
    ] {
        fixture.write(
            path,
            serde_json::to_vec(&json!({"highwater":1,"rows":[{"id":1,"name":name}]})).unwrap(),
        );
    }
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let result=app.ok(json!({"command":"inventory_timer_lifecycle","owner":"101","initial":"initial.json","changed":"changed.json","cold":"cold.json","consumerBeforePoll":true}));
    assert_eq!(
        result["changedEvents"], 1,
        "API read consumed the broadcast serial"
    );
    assert_eq!(result["changedName"], "Renamed elsewhere");
    app.stop();
}

#[test]
fn slow_api_inventory_reconciliation_preserves_desktop_change_notifications() {
    let fixture = Fixture::new();
    for (path, name, delay) in [
        ("initial.json", "Original", 0),
        ("changed.json", "Renamed elsewhere", 1100),
        ("cold.json", "Idle", 0),
    ] {
        fixture.write(
            path,
            serde_json::to_vec(&json!({
                "highwater": 1,
                "rows": [{"id": 1, "name": name}],
                "historyDelayMs": delay,
            }))
            .unwrap(),
        );
    }
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let result = app.ok(json!({
        "command": "inventory_timer_lifecycle",
        "owner": "101",
        "initial": "initial.json",
        "changed": "changed.json",
        "cold": "cold.json",
        "consumerBeforePoll": true,
    }));
    assert_eq!(result["unchangedEvents"], 0);
    assert_eq!(result["changedEvents"], 1);
    assert_eq!(result["changedName"], "Renamed elsewhere");
    assert_eq!(result["coldName"], "Idle");
    assert_eq!(result["coldLookupBatches"], 0);
    app.stop();
}

#[test]
fn inventory_twenty_small_folders_remain_monitored_without_count_based_eviction() {
    let fixture = Fixture::new();
    fixture.write(
        "churn.json",
        serde_json::to_vec(&json!({"highwater":1,"rows":[{"id":1,"name":"Folder row"}]})).unwrap(),
    );
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    for folder in 1..=20 {
        app.ok(
            json!({"command":"inventory_list","owner":"101","folder":folder,"source":"churn.json"}),
        );
    }
    assert_eq!(
        app.ok(json!({"command":"inventory_touch","owner":"101","folder":1})),
        true
    );
    let result =
        app.ok(json!({"command":"inventory_list","owner":"101","folder":1,"source":"churn.json"}));
    assert_eq!(result["rows"][0]["name"], "Folder row");
    assert_eq!(
        result["historyWalks"], 20,
        "small folders must remain resident within the total row budget"
    );
    assert_eq!(
        app.ok(json!({"command":"inventory_touch","owner":"101","folder":1})),
        true
    );
    app.stop();
}

#[test]
fn concurrent_thumbnail_requests_decode_real_images_under_one_global_capacity_limit() {
    let fixture = Fixture::new();
    image::RgbImage::from_pixel(1600, 900, image::Rgb([30, 80, 150]))
        .save(fixture.path("thumbnail-source.png"))
        .unwrap();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let result =
        app.ok(json!({"command":"thumbnail_batch","owner":"101","source":"thumbnail-source.png"}));
    assert!(
        result["peak"].as_u64().unwrap() <= 3,
        "unbounded decoders: {result}"
    );
    for size in result["sizes"].as_array().unwrap() {
        assert!(size["width"].as_u64().unwrap() <= 480 && size["height"].as_u64().unwrap() <= 360);
    }
    for index in 0..6 {
        let meta = std::fs::metadata(fixture.path(&format!("rendered-{index}.jpg"))).unwrap();
        assert!(meta.len() > 0 && meta.len() <= 1024 * 1024);
    }
    app.stop();
}

#[test]
fn the_shared_asset_cache_binds_source_identity_account_and_protection_without_requiring_indexed_files(
) {
    let fixture = Fixture::new();
    fixture.write("asset-a.bin", b"first source bytes");
    fixture.write("asset-b.bin", b"other source bytes");
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let first = app.ok(json!({"command":"asset_read","owner":"101","source":"asset-a.bin"}));
    assert_eq!(
        std::fs::read(first["path"].as_str().unwrap()).unwrap(),
        b"first source bytes"
    );
    let cached = app.ok(json!({"command":"asset_read","owner":"101","source":"asset-a.bin"}));
    assert_eq!(cached["downloads"], 1);
    assert_eq!(cached["path"], first["path"]);
    let changed = app.ok(json!({"command":"asset_read","owner":"101","source":"asset-b.bin"}));
    assert_eq!(changed["downloads"], 2);
    assert_ne!(changed["path"], first["path"]);
    assert_eq!(
        std::fs::read(changed["path"].as_str().unwrap()).unwrap(),
        b"other source bytes"
    );
    let database = sqlite::open(fixture.path("workspace/101/workspace.db")).unwrap();
    let mut query = database
        .prepare("SELECT COUNT(*) FROM workspace_files")
        .unwrap();
    query.next().unwrap();
    assert_eq!(
        query.read::<i64, _>(0).unwrap(),
        0,
        "previewing must not index an arbitrary chat message"
    );
    drop(query);
    drop(database);
    app.ok(json!({"command":"seed_account","owner":202}));
    let other = app.ok(json!({"command":"asset_read","owner":"202","source":"asset-a.bin"}));
    assert_eq!(other["downloads"], 3);
    assert_ne!(other["path"], first["path"]);
    assert!(app
        .failure(
            json!({"command":"asset_read","owner":"202","source":"asset-b.bin","sourceOwner":101})
        )
        .contains("ACCOUNT_CHANGED"));
    assert!(app.failure(json!({"command":"asset_read","owner":"202","source":"asset-b.bin","id":999,"protection":"encrypted_unlocked"})).contains("ENCRYPTED_PREVIEW_UNAVAILABLE"));
    app.stop();
}

#[test]
fn failed_asset_completion_does_not_leave_a_target_which_the_next_request_certifies_as_cached() {
    let fixture = Fixture::new();
    fixture.write(
        "failed-asset.bin",
        b"complete bytes before transport rejection",
    );
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    assert!(app
        .failure(
            json!({"command":"asset_read","owner":"101","source":"failed-asset.bin","fail":true})
        )
        .contains("NETWORK_UNAVAILABLE"));
    let directory = fixture.path("asset-cache/previews/workspace/101/previews");
    assert_eq!(
        std::fs::read_dir(&directory).unwrap().count(),
        0,
        "failed target was published"
    );
    let retried = app.ok(json!({"command":"asset_read","owner":"101","source":"failed-asset.bin"}));
    assert_eq!(
        retried["downloads"], 2,
        "failed bytes were certified as a completed cache entry"
    );
    app.stop();
}

#[test]
fn pinned_shared_previews_survive_size_eviction_and_explicit_disposable_cache_clearing() {
    let fixture = Fixture::new();
    fixture.write("pinned.bin", vec![1; 10_000]);
    fixture.write("disposable.bin", vec![2; 10_000]);
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let pinned = app.ok(json!({"command":"asset_read","owner":"101","source":"pinned.bin"}));
    let pinned = PathBuf::from(pinned["path"].as_str().unwrap());
    assert_eq!(app.ok(json!({"command":"asset_pin","owner":"101"})), true);
    assert_eq!(
        app.ok(json!({"command":"asset_cached","owner":"101"})),
        pinned.to_string_lossy().as_ref()
    );
    app.ok(json!({"command":"asset_limits","previews":15_000,"thumbnails":2_000_000}));
    let disposable =
        app.ok(json!({"command":"asset_read","owner":"101","source":"disposable.bin","id":2}));
    assert!(
        pinned.is_file(),
        "size eviction removed a deliberately kept preview"
    );
    app.ok(json!({"command":"asset_clear","owner":"101","category":"previews"}));
    assert!(
        pinned.is_file() && pinned.with_extension("pin").is_file(),
        "clear removed a deliberately kept preview"
    );
    assert!(!Path::new(disposable["path"].as_str().unwrap()).exists());
    app.ok(json!({"command":"asset_delete","owner":"101"}));
    assert!(pinned.is_file());
    app.ok(json!({"command":"asset_pin","owner":"101","pinned":false}));
    app.ok(json!({"command":"asset_delete","owner":"101"}));
    assert!(!pinned.exists());
    app.stop();
}

#[test]
fn cancellation_during_asset_lookup_is_registered_and_cannot_publish_a_later_result() {
    let fixture = Fixture::new();
    fixture.write("delayed-asset.bin", b"later source data");
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!({"command":"asset_start","owner":"101","source":"delayed-asset.bin","requestId":"lookup-cancellation","lookupDelayMs":1000}));
    assert_eq!(
        app.ok(json!({"command":"asset_cancel","owner":"101","requestId":"lookup-cancellation"})),
        true,
        "lookup was outside request registration"
    );
    assert!(app
        .failure(json!({"command":"asset_finish"}))
        .contains("CANCELLED"));
    let completed =
        app.ok(json!({"command":"asset_read","owner":"101","source":"delayed-asset.bin"}));
    assert_eq!(completed["downloads"], 1);
    assert_eq!(
        std::fs::read(completed["path"].as_str().unwrap()).unwrap(),
        b"later source data"
    );
    app.stop();
}

#[test]
fn canonical_cache_root_publishes_reuses_and_clears_a_native_preview() {
    let fixture = Fixture::new();
    let root = fixture.0.canonicalize().unwrap();
    let mut app = Backend::start(&root);
    app.ok(json!({"command":"seed_account","owner":101}));
    let request = json!({
        "command": "native_preview_prepare", "filename": "canonical.bin",
        "size": 1024, "limit": 4096,
    });
    let prepared = app.ok(request.clone());
    let partial = PathBuf::from(prepared["partial"].as_str().unwrap());
    assert!(partial.starts_with(&root));
    std::fs::write(&partial, vec![7; 1024]).unwrap();
    let published = app.ok(json!({"command":"native_preview_finish"}));
    let cached = app.ok(request);
    assert_eq!(cached["cached"], published);
    let path = PathBuf::from(published.as_str().unwrap());
    assert_eq!(std::fs::read(&path).unwrap(), vec![7; 1024]);
    app.ok(json!({"command":"native_preview_clear"}));
    assert!(!path.exists());
    app.stop();
}

#[test]
fn native_and_async_preview_writers_share_capacity_and_clear_generations() {
    let fixture = Fixture::new();
    fixture.write("cross-adapter.bin", vec![7; 10_000]);
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let lease=app.ok(json!({"command":"native_preview_prepare","filename":"native.bin","size":10_000,"limit":15_000}));
    std::fs::write(lease["partial"].as_str().unwrap(), vec![3; 10_000]).unwrap();
    app.ok(json!({"command":"asset_limits","previews":15_000,"thumbnails":2_000_000}));
    assert!(app
        .failure(json!({"command":"asset_read","owner":"101","source":"cross-adapter.bin"}))
        .contains("CACHE_BUSY"));
    app.ok(json!({"command":"asset_clear","owner":"101","category":"previews"}));
    assert!(app
        .failure(json!({"command":"native_preview_finish"}))
        .contains("cleared"));
    app.ok(
        json!({"command":"asset_start","owner":"101","source":"cross-adapter.bin","delayMs":1000}),
    );
    std::thread::sleep(std::time::Duration::from_millis(150));
    app.ok(json!({"command":"native_preview_clear"}));
    assert!(app
        .failure(json!({"command":"asset_finish"}))
        .contains("CANCELLED"));
    let retried =
        app.ok(json!({"command":"asset_read","owner":"101","source":"cross-adapter.bin"}));
    assert_eq!(
        std::fs::read(retried["path"].as_str().unwrap()).unwrap(),
        vec![7; 10_000]
    );
    app.stop();
}

#[test]
fn cache_pool_scope_does_not_adopt_similarly_named_parent_directories() {
    let fixture = Fixture::new();
    std::fs::create_dir_all(fixture.path("previews/thumbnails/instance")).unwrap();
    fixture.write("previews/unrelated.bin", vec![8; 10_000]);
    fixture.write(
        "previews/thumbnails/instance/.native-e2e-fixture",
        b"telegram-drive-synthetic-e2e\n",
    );
    fixture.write("previews/thumbnails/instance/source.bin", vec![7; 10_000]);
    let mut app = Backend::start(&fixture.path("previews/thumbnails/instance"));
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!({"command":"asset_limits","previews":15_000,"thumbnails":15_000}));
    app.ok(json!({"command":"asset_read","owner":"101","source":"source.bin"}));
    assert_eq!(
        std::fs::read(fixture.path("previews/unrelated.bin")).unwrap(),
        vec![8; 10_000]
    );
    app.stop();
}

fn ffmpeg_helper_name() -> &'static str {
    if cfg!(windows) {
        "ffmpeg-helper.exe"
    } else {
        "ffmpeg-helper"
    }
}

const VIDEO_FIXTURE_READINESS: Duration = Duration::from_secs(300);
fn thumbnail_video_fixture(fixture: &Fixture, mode: &str) {
    fixture.write("video.bin", b"controlled video boundary");
    fixture.write("ffmpeg-mode", mode);
    image::RgbImage::from_pixel(320, 240, image::Rgb([15, 80, 150]))
        .save(fixture.path("ffmpeg-frame.jpg"))
        .unwrap();
    std::fs::copy(
        env!("CARGO_BIN_EXE_native-e2e-heic-helper"),
        fixture.path(ffmpeg_helper_name()),
    )
    .unwrap();
}
#[test]
fn telegram_provided_video_thumbnail_is_preferred_over_frame_extraction() {
    let fixture = Fixture::new();
    thumbnail_video_fixture(&fixture, "fail");
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let result = app.ok(json!(
        {
            "command":"asset_read",
            "owner":"101",
            "source":"video.bin",
            "thumbnail":true,
            "thumbnailSource":"ffmpeg-frame.jpg",
            "video":true,
            "ffmpeg":ffmpeg_helper_name()
        }
    ));
    assert_eq!(
        image::open(result["path"].as_str().unwrap())
            .unwrap()
            .width(),
        480
    );
    assert!(!fixture.path("ffmpeg-pid").exists());
    app.stop();
}
#[test]
fn video_frame_child_process_success_and_failure_preserve_cache_integrity() {
    let fixture = Fixture::new();
    thumbnail_video_fixture(&fixture, "ok");
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let result=app.ok(json!({"command":"asset_read","owner":"101","source":"video.bin","thumbnail":true,"video":true,"ffmpeg":ffmpeg_helper_name()}));
    assert_eq!(
        image::open(result["path"].as_str().unwrap())
            .unwrap()
            .height(),
        360
    );
    assert!(fixture.path("ffmpeg-pid").is_file());
    fixture.write("ffmpeg-mode", "fail");
    assert!(app
        .failure(json!(
            {
                "command":"asset_read",
                "owner":"101",
                "source":"video.bin",
                "id":2,
                "thumbnail":true,
                "video":true,
                "ffmpeg":ffmpeg_helper_name()
            }
        ))
        .contains("THUMBNAIL_UNAVAILABLE"));
    assert_eq!(
        std::fs::read_dir(fixture.path("asset-cache/previews/workspace/101/thumbnails"))
            .unwrap()
            .count(),
        1
    );
    app.stop();
}
#[test]
fn clearing_thumbnails_waits_for_blocking_child_cleanup_and_preserves_next_request() {
    let fixture = Fixture::new();
    thumbnail_video_fixture(&fixture, "wait");
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!({"command":"asset_start","owner":"101","source":"video.bin","thumbnail":true,"video":true,"ffmpeg":ffmpeg_helper_name(),"fixtureVideoClock":true}));
    let deadline = Instant::now() + VIDEO_FIXTURE_READINESS;
    while !fixture.path("ffmpeg-pid").exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    app.ok(json!({"command":"asset_clear","owner":"101","category":"thumbnails"}));
    assert!(app
        .failure(json!({"command":"asset_finish"}))
        .contains("CANCELLED"));
    assert_eq!(
        std::fs::read_dir(fixture.path("asset-cache/previews/workspace/101/thumbnails"))
            .unwrap()
            .count(),
        0
    );
    #[cfg(unix)]
    {
        let pid = std::fs::read_to_string(fixture.path("ffmpeg-pid")).unwrap();
        assert!(
            !Command::new("kill")
                .args(["-0", pid.trim()])
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success(),
            "extraction child survived cache clear"
        );
    }
    fixture.write("ffmpeg-mode", "ok");
    let result=app.ok(json!({"command":"asset_read","owner":"101","source":"video.bin","thumbnail":true,"video":true,"ffmpeg":ffmpeg_helper_name()}));
    assert!(Path::new(result["path"].as_str().unwrap()).is_file());
    app.stop();
}

#[test]
fn live_frame_partials_are_counted_once_when_another_thumbnail_is_admitted() {
    let fixture = Fixture::new();
    thumbnail_video_fixture(&fixture, "frame-wait");
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let limit = 2 * 1024 * 1024
        + std::fs::metadata(fixture.path("video.bin")).unwrap().len()
        + std::fs::metadata(fixture.path("ffmpeg-frame.jpg"))
            .unwrap()
            .len();
    app.ok(json!({"command":"asset_limits","previews":1_000_000,"thumbnails":limit}));
    app.ok(json!({"command":"asset_start","owner":"101","source":"video.bin","thumbnail":true,"video":true,"ffmpeg":ffmpeg_helper_name(),"fixtureVideoClock":true}));
    let deadline = Instant::now() + VIDEO_FIXTURE_READINESS;
    while !fixture.path("ffmpeg-frame-ready").exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_video_fixture_child_alive(&fixture);
    let second = app.ok(json!(
        {
            "command":"asset_read",
            "owner":"101",
            "id":2,
            "source":"video.bin",
            "thumbnail":true,
            "thumbnailSource":"ffmpeg-frame.jpg",
            "video":true,
            "ffmpeg":ffmpeg_helper_name()
        }
    ));
    assert!(Path::new(second["path"].as_str().unwrap()).is_file());
    assert_video_fixture_child_alive(&fixture);
    app.ok(json!({"command":"asset_clear","owner":"101","category":"thumbnails"}));
    assert!(app
        .failure(json!({"command":"asset_finish"}))
        .contains("CANCELLED"));
    app.stop();
}

fn assert_video_fixture_child_alive(fixture: &Fixture) {
    let pid = std::fs::read_to_string(fixture.path("ffmpeg-pid")).unwrap();
    #[cfg(unix)]
    assert!(
        Command::new("kill")
            .args(["-0", pid.trim()])
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success(),
        "controlled extraction child is no longer alive"
    );
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::{
            Foundation::CloseHandle,
            System::Threading::{
                GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
            },
        };
        let handle = OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION,
            0,
            pid.trim().parse().unwrap(),
        );
        assert!(!handle.is_null());
        let mut code = 0;
        let alive = GetExitCodeProcess(handle, &mut code) != 0 && code == 259;
        CloseHandle(handle);
        assert!(alive, "controlled extraction child is no longer alive");
    }
}
#[test]
fn delayed_video_fixture_reaches_a_live_partial_before_cancellation() {
    let fixture = Fixture::new();
    thumbnail_video_fixture(&fixture, "frame-wait");
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let started = Instant::now();
    app.ok(json!({
        "command":"asset_start", "owner":"101", "source":"video.bin",
        "thumbnail":true, "video":true, "ffmpeg":ffmpeg_helper_name(),
        "fixtureVideoClock":true, "fixtureVideoReadyDelayMs":21_000
    }));
    let deadline = Instant::now() + VIDEO_FIXTURE_READINESS;
    while !fixture.path("ffmpeg-frame-ready").exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let reached = fixture.path("ffmpeg-frame-ready").is_file();
    let elapsed = started.elapsed();
    if reached {
        assert_video_fixture_child_alive(&fixture);
    }
    app.ok(json!({"command":"asset_clear","owner":"101","category":"thumbnails"}));
    let stopped = app.failure(json!({"command":"asset_finish"}));
    assert!(stopped.contains("CANCELLED"), "{stopped}");
    assert!(reached, "delayed fixture did not reach its partial stage");
    assert!(
        elapsed >= Duration::from_secs(21),
        "controlled readiness delay was not exercised: {elapsed:?}"
    );
    assert_eq!(
        std::fs::read_dir(fixture.path("asset-cache/previews/workspace/101/thumbnails"))
            .unwrap()
            .count(),
        0
    );
    #[cfg(unix)]
    {
        let pid = std::fs::read_to_string(fixture.path("ffmpeg-pid")).unwrap();
        assert!(
            !Command::new("kill")
                .args(["-0", pid.trim()])
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success(),
            "delayed extraction child survived clear"
        );
    }
    fixture.write("ffmpeg-mode", "ok");
    let next = app.ok(json!({"command":"asset_read","owner":"101","source":"video.bin","thumbnail":true,"video":true,"ffmpeg":ffmpeg_helper_name()}));
    assert!(Path::new(next["path"].as_str().unwrap()).is_file());
}

#[test]
fn video_frame_deadline_kills_and_reaps_the_extraction_child() {
    let fixture = Fixture::new();
    thumbnail_video_fixture(&fixture, "wait");
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let start = Instant::now();
    assert!(app.failure(json!({"command":"asset_read","owner":"101","source":"video.bin","thumbnail":true,"video":true,"ffmpeg":ffmpeg_helper_name()})).contains("CANCELLED"));
    assert!(
        start.elapsed() >= Duration::from_secs(19) && start.elapsed() < Duration::from_secs(35)
    );
    #[cfg(unix)]
    {
        let pid = std::fs::read_to_string(fixture.path("ffmpeg-pid")).unwrap();
        assert!(!Command::new("kill")
            .args(["-0", pid.trim()])
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success());
    }
    assert_eq!(
        std::fs::read_dir(fixture.path("asset-cache/previews/workspace/101/thumbnails"))
            .unwrap()
            .count(),
        0
    );
    app.stop();
}

#[tokio::test]
async fn rest_thumbnails_use_the_shared_decoder_cache_account_scope_and_protection_gate() {
    let fixture = Fixture::new();
    image::RgbImage::from_pixel(1024, 768, image::Rgb([20, 100, 180]))
        .save(fixture.path("http-thumbnail.png"))
        .unwrap();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!({"command":"api_thumbnail_seed","owner":"101","source":"http-thumbnail.png"}));
    let server = app.ok(json!({"command":"start_api","key":"thumbnail-journey"}));
    let url = format!(
        "{}/api/v1/files/1/thumbnail",
        server["url"].as_str().unwrap()
    );
    let http = reqwest::Client::new();
    let response = http
        .get(&url)
        .header("X-API-Key", "thumbnail-journey")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "image/jpeg");
    let bytes = response.bytes().await.unwrap();
    assert_eq!(image::load_from_memory(&bytes).unwrap().width(), 480);
    let second = http
        .get(&url)
        .header("X-API-Key", "thumbnail-journey")
        .send()
        .await
        .unwrap();
    assert_eq!(second.bytes().await.unwrap(), bytes);
    app.ok(json!({"command":"seed_account","owner":202}));
    app.ok(json!({"command":"api_thumbnail_seed","owner":"202","source":"http-thumbnail.png","protection":"encrypted_unlocked"}));
    let denied = http
        .get(&url)
        .header("X-API-Key", "thumbnail-journey")
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 403);
    assert!(fixture
        .path("previews/workspace/202/thumbnails")
        .read_dir()
        .unwrap()
        .next()
        .is_none());
    app.stop();
}
#[test]
fn shared_preview_status_and_offline_listing_preserve_legacy_pins() {
    let fixture = Fixture::new();
    fixture.write("offline.bin", b"shared offline bytes");
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!({"command":"asset_offline_seed","owner":"101","source":"offline.bin"}));
    let shared = app.ok(json!({"command":"asset_read","owner":"101","source":"offline.bin"}));
    app.ok(json!({"command":"asset_pin","owner":"101"}));
    let rows = app.ok(json!({"command":"asset_offline_read","owner":"101"}));
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(rows[0]["size"], 20);
    let base = fixture.path("asset-cache/previews");
    std::fs::write(base.join("101_home_2.bin"), b"legacy kept").unwrap();
    std::fs::write(base.join("101_home_2.pin"), b"pinned").unwrap();
    let status = app.ok(json!({"command":"asset_status"}));
    assert_eq!(status["file_count"], 2);
    assert_eq!(status["total_bytes"], 31);
    app.ok(json!({"command":"asset_clear_all","owner":"101"}));
    assert!(
        Path::new(shared["path"].as_str().unwrap()).is_file()
            && base.join("101_home_2.bin").is_file()
    );
    app.ok(json!({"command":"seed_account","owner":202}));
    assert_eq!(
        app.ok(json!({"command":"asset_offline_read","owner":"202"})),
        json!([])
    );
    app.stop();
}

#[test]
fn asset_identity_serializes_a_delayed_old_thumbnail_against_a_new_preview() {
    let fixture = Fixture::new();
    for (name, color) in [
        ("old.png", image::Rgb([10, 20, 30])),
        ("new.png", image::Rgb([80, 90, 100])),
    ] {
        image::RgbImage::from_pixel(320, 240, color)
            .save(fixture.path(name))
            .unwrap();
    }
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!({"command":"asset_start","owner":"101","source":"old.png","thumbnail":true,"lookupDelayMs":500}));
    let latest = app.ok(json!({"command":"asset_read","owner":"101","source":"new.png"}));
    app.ok(json!({"command":"asset_finish"}));
    assert_eq!(
        app.ok(json!({"command":"asset_cached","owner":"101"})),
        latest["path"],
        "old thumbnail identity displaced the current preview"
    );
    app.stop();
}
#[test]
fn asset_identity_cancellation_while_sqlite_is_busy_cannot_commit_abandoned_metadata() {
    let fixture = Fixture::new();
    fixture.write("identity-old.bin", b"old bytes");
    fixture.write("identity-new.bin", b"new bytes");
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let original =
        app.ok(json!({"command":"asset_read","owner":"101","source":"identity-old.bin"}));
    let database = sqlite::open(fixture.path("workspace/101/workspace.db")).unwrap();
    database.execute("BEGIN IMMEDIATE").unwrap();
    app.ok(json!({"command":"asset_start","owner":"101","source":"identity-new.bin","requestId":"busy-identity"}));
    let deadline = Instant::now() + Duration::from_secs(15);
    while app
        .ok(json!({"command":"asset_metadata_started"}))
        .as_u64()
        .unwrap()
        < 2
    {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    app.ok(json!({"command":"asset_cancel","owner":"101","requestId":"busy-identity"}));
    assert!(app
        .failure(json!({"command":"asset_finish"}))
        .contains("CANCELLED"));
    database.execute("ROLLBACK").unwrap();
    drop(database);
    let deadline = Instant::now() + Duration::from_secs(15);
    while app
        .ok(json!({"command":"asset_metadata_finished"}))
        .as_u64()
        .unwrap()
        < 2
    {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        app.ok(json!({"command":"asset_cached","owner":"101"})),
        original["path"],
        "abandoned SQLite writer replaced the committed identity"
    );
    app.stop();
}
#[test]
fn a_legacy_pin_is_atomic_with_shared_cache_admission() {
    let fixture = Fixture::new();
    std::fs::create_dir_all(fixture.path("asset-cache/previews")).unwrap();
    fixture.write("asset-cache/previews/101_home_9.bin", vec![9; 10_000]);
    fixture.write("incoming.bin", vec![1; 10_000]);
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!({"command":"asset_limits","previews":15_000,"thumbnails":2_000_000}));
    app.ok(json!({"command":"legacy_pin_start"}));
    let held = app
        .ok(json!({"command":"asset_core_busy"}))
        .as_bool()
        .unwrap();
    if held {
        fixture.write("legacy-pin-release", b"release");
    }
    app.ok(json!({"command":"asset_start","owner":"101","source":"incoming.bin"}));
    if !held {
        app.ok(json!({"command":"asset_finish"}));
    }
    fixture.write("legacy-pin-release", b"release");
    app.ok(json!({"command":"legacy_pin_finish"}));
    if held {
        app.ok(json!({"command":"asset_finish"}));
    }
    assert!(
        fixture
            .path("asset-cache/previews/101_home_9.bin")
            .is_file(),
        "successful pin refers to an evicted file"
    );
    assert!(fixture
        .path("asset-cache/previews/101_home_9.pin")
        .is_file());
    app.stop();
}

#[test]
fn local_fts_search_reads_owned_inventory_without_a_persistent_index_or_other_chats() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let file = |id, folder, name: &str| {
        json!(
            {
                "id":id,
                "folder_id":folder,
                "name":name,
                "size":20000,
                "mime_type":"application/pdf",
                "file_ext":"pdf",
                "created_at":"2026-10-02T00:00:00Z",
                "icon_type":"file",
                "encryption_state":"plain",
                "is_favorite":false,
                "is_pinned":false
            }
        )
    };
    app.ok(json!({"command":"workspace_seed_files","owner":"101","files":[file(1,42,"Café budget-2026.pdf"),file(2,777,"Café private chat.pdf"),file(3,42,"会议记录2026.pdf")]}));
    app.ok(json!({"command":"search_index_build","owner":"101","folders":[42]}));
    let found =
        app.ok(json!({"command":"search_index_query","query":{"query":"cafe","type":"document"}}));
    assert_eq!(found["total"], 1);
    assert_eq!(found["files"][0]["folder_id"], 42);
    let cjk = app.ok(json!({"command":"search_index_query","query":{"query":"记录"}}));
    assert_eq!(cjk["files"][0]["id"], 3);
    let database = sqlite::open(fixture.path("workspace/101/workspace.db")).unwrap();
    let mut tables = database
        .prepare("SELECT count(*) FROM sqlite_master WHERE name LIKE 'search_names%'")
        .unwrap();
    tables.next().unwrap();
    assert_eq!(tables.read::<i64, _>(0).unwrap(), 0);
    drop(tables);
    let mut version = database.prepare("PRAGMA user_version").unwrap();
    version.next().unwrap();
    assert_eq!(version.read::<i64, _>(0).unwrap(), 1);
    drop(version);
    drop(database);
    app.ok(json!({"command":"seed_account","owner":202}));
    assert!(app
        .failure(json!({"command":"search_index_query","query":{"query":"cafe"}}))
        .contains("ACCOUNT_CHANGED"));
    app.stop();
}

#[test]
fn local_search_runtime_hides_removals_and_fences_uncommitted_metadata() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let file = |id| {
        json!(
            {
                "id":id,
                "folder_id":42,
                "name":format!("Report {id}.pdf"),
                "size":20000,
                "mime_type":"application/pdf",
                "file_ext":"pdf",
                "created_at":"2026-10-01",
                "is_favorite":false,
                "is_pinned":false,
                "encryption_state":"plain",
                "icon_type":"file"
            }
        )
    };
    app.ok(json!({"command":"workspace_seed_files","owner":"101","files":[file(1),file(2)],"hidden":["42:2"]}));
    let search =
        json!({"command":"search_runtime","owner":"101","folders":[42],"query":{"query":"Report"}});
    let found = app.ok(search.clone());
    assert_eq!(found["total"], 1, "hidden removal must not reappear");
    app.ok(json!({"command":"workspace_tx_start","metadata":true}));
    assert!(
        app.failure(search.clone()).contains("SEARCH_CHANGED"),
        "search must wait through the whole transaction"
    );
    app.ok(json!({"command":"workspace_tx_finish"}));
    let changed = app.ok(search);
    assert_ne!(found["indexId"], changed["indexId"]);
    assert_eq!(changed["files"][0]["is_favorite"], true);
    app.stop();
}

#[test]
fn local_search_runtime_filters_before_paging_and_reuses_saved_rules() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let file = |id, folder, name: &str, size, ext: &str| {
        json!(
            {
                "id":id,
                "folder_id":folder,
                "name":name,
                "size":size,
                "mime_type":"application/octet-stream",
                "file_ext":ext,
                "created_at":chrono::Utc::now().to_rfc3339(),
                "is_favorite":false,
                "is_pinned":false,
                "encryption_state":"plain",
                "icon_type":"file"
            }
        )
    };
    app.ok(json!(
        {
            "command":"workspace_seed_files",
            "owner":"101",
            "files":[
                file(1,
                42,
                "Café report.pdf",
                20000,
                "pdf"),
                file(2,
                42,
                "Café report-final.pdf",
                20000,
                "pdf"),
                file(3,
                42,
                "Café archive.zip",
                200_000_000,
                "zip"),
                file(4,
                777,
                "Café outside.pdf",
                20000,
                "pdf"),
                file(5,
                42,
                "会议记录.pdf",
                20000,
                "pdf")
            ]
        }
    ));
    app.ok(json!({"command":"search_record","kind":"collection","id":"reports","value":{"id":"reports","name":"Reports","color":"blue","icon":"book","coverKey":null}}));
    for id in [1, 2] {
        app.ok(json!({"command":"search_tag","key":format!("42:{id}"),"tag":"Résumé"}));
        app.ok(json!({"command":"search_assign","key":format!("42:{id}"),"collection":"reports"}));
        app.ok(json!({"command":"search_record","kind":"favorite","id":format!("42:{id}"),"value":true}));
    }
    let query = json!(
        {
            "query":"cafe",
            "type":"document",
            "size":"small",
            "date":"7d",
            "folderKey":"42",
            "tags":[
                "résumé"
            ],
            "collectionId":"reports",
            "favoritesOnly":true,
            "protection":"plain",
            "limit":1
        }
    );
    let first =
        app.ok(json!({"command":"search_runtime","owner":"101","folders":[42],"query":query}));
    assert_eq!(first["total"], 2);
    assert_eq!(first["offline"], true);
    assert_eq!(first["complete"], false);
    let mut next = query.clone();
    next["offset"] = json!(1);
    next["indexId"] = first["indexId"].clone();
    let second =
        app.ok(json!({"command":"search_runtime","owner":"101","folders":[42],"query":next}));
    assert_ne!(first["files"][0]["id"], second["files"][0]["id"]);
    assert_eq!(second["nextOffset"], Value::Null);
    app.ok(json!(
        {
            "command":"search_record",
            "kind":"search",
            "id":"saved",
            "value":{
                "id":"saved",
                "name":"Reports",
                "query":"cafe",
                "filters":{
                    "scope":"all",
                    "type":"document",
                    "size":"small",
                    "date":"7d",
                    "protection":"plain"
                },
                "folderKey":"42",
                "tags":[
                    "résumé"
                ],
                "collectionId":"reports",
                "favoritesOnly":true
            }
        }
    ));
    let saved =
        app.ok(json!({"command":"search_runtime","owner":"101","folders":[42],"saved":"saved"}));
    assert_eq!(saved["total"], 2);
    let generation = saved["indexId"].clone();
    app.ok(json!({"command":"search_tag","key":"42:2","tag":"Résumé","add":false}));
    let changed =
        app.ok(json!({"command":"search_runtime","owner":"101","folders":[42],"query":query}));
    assert_eq!(changed["total"], 1);
    assert_ne!(generation, changed["indexId"]);
    assert!(app
        .failure(json!({"command":"search_runtime","owner":"101","folders":[42],"query":next}))
        .contains("SEARCH_CHANGED"));
    assert!(app
        .failure(
            json!({"command":"search_runtime","owner":"101","folders":[42],"query":{"offset":1}})
        )
        .contains("SEARCH_CHANGED"));
    assert!(app.failure(json!({"command":"search_runtime","owner":"101","folders":[42],"query":{"folderKey":"777"}})).contains("NOT_DRIVE_FOLDER"));
    assert_eq!(app.ok(json!({"command":"search_runtime","owner":"101","folders":[42],"query":{"query":"记录"}}))["files"][0]["id"],5);
    assert_eq!(app.ok(json!({"command":"search_runtime","owner":"101","folders":[42],"query":{"query":"report-final"}}))["total"],1);
    assert!(app.failure(json!({"command":"search_runtime","owner":"101","folders":[42],"query":{"query":"x".repeat(501)}})).contains("INVALID_SEARCH"));
    app.stop();
}

#[test]
fn local_search_runtime_canceled_build_retains_its_capacity_and_cannot_publish() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let query = json!({"command":"search_runtime","owner":"101","folders":[42],"query":{}});
    let mut start = query.clone();
    start["command"] = json!("search_start");
    app.ok(start);
    assert_eq!(
        app.ok(json!({"command":"search_abort"})),
        2,
        "owned blocking build keeps its permit after async abort"
    );
    fixture.write("search-release", b"release");
    let finished = app.ok(query);
    assert_eq!(finished["total"], 0);
    assert_eq!(app.ok(json!({"command":"search_available"})), 3);
    app.stop();
}

#[test]
fn local_search_runtime_inventory_vault_and_account_changes_reject_old_generations() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!(
        {
            "command":"workspace_seed_files",
            "owner":"101",
            "files":[
                {
                    "id":1,
                    "folder_id":42,
                    "name":"Secret project.pdf",
                    "size":1000,
                    "mime_type":"application/pdf",
                    "file_ext":"pdf",
                    "created_at":"2026-10-01",
                    "icon_type":"file",
                    "is_favorite":false,
                    "is_pinned":false,
                    "encryption_state":"encrypted_unlocked"
                }
            ]
        }
    ));
    app.ok(json!({"command":"vault_create","passphrase":"search fixture passphrase"}));
    let query = json!({"command":"search_runtime","owner":"101","folders":[42],"query":{"query":"Secret","protection":"unlocked"},"names":{"42:1":"Secret project.pdf"}});
    let found = app.ok(query.clone());
    assert_eq!(found["total"], 1);
    assert!(
        !String::from_utf8_lossy(&fixture.read("workspace/101/workspace.db"))
            .contains("Secret project")
    );
    app.ok(json!({"command":"search_inventory_changed"}));
    let changed = app.ok(query.clone());
    assert_ne!(found["indexId"], changed["indexId"]);
    app.ok(json!({"command":"vault_lock"}));
    let locked = app.ok(
        json!({"command":"search_runtime","owner":"101","folders":[42],"query":{"query":"Secret"}}),
    );
    assert_eq!(locked["total"], 0);
    assert_ne!(changed["indexId"], locked["indexId"]);
    let mut old = query;
    old["query"]["indexId"] = changed["indexId"].clone();
    old["names"] = json!({});
    assert!(app.failure(old).contains("SEARCH_CHANGED"));
    app.ok(json!({"command":"seed_account","owner":202}));
    assert!(app
        .failure(json!({"command":"search_runtime","owner":"101","folders":[42],"query":{}}))
        .contains("ACCOUNT_CHANGED"));
    app.stop();
}

#[test]
fn local_search_runtime_bounds_metadata_before_indexing_and_scopes_large_accounts() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!(
        {
            "command":"workspace_seed_files",
            "owner":"101",
            "files":[
                {
                    "id":1,
                    "folder_id":777,
                    "name":"x".repeat(33*1024*1024),
                    "size":1,
                    "created_at":"2026-10-01",
                    "icon_type":"file",
                    "is_favorite":false,
                    "is_pinned":false,
                    "encryption_state":"plain"
                }
            ]
        }
    ));
    assert_eq!(app.ok(json!({"command":"search_runtime","owner":"101","folders":[42,777],"query":{"folderKey":"42"}}))["total"],0);
    assert!(app
        .failure(json!({"command":"search_runtime","owner":"101","folders":[42,777],"query":{}}))
        .contains("SEARCH_INDEX_LIMIT"));
    app.stop();
}

#[test]
fn local_search_inventory_bootstraps_more_than_sixteen_folders_and_keeps_live_only_organization() {
    let fixture = Fixture::new();
    fixture.write(
        "remote-search.json",
        json!({"highwater":1,"rows":[{"id":1,"name":"Report live.pdf"}]}).to_string(),
    );
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let folders: Vec<_> = (42..62).collect();
    let query = json!({"command":"search_inventory","owner":"101","source":"remote-search.json","folders":folders,"query":{"query":"Report"}});
    let all = app.ok(query.clone());
    assert_eq!(all["total"], 20);
    assert_eq!(all["complete"], true);
    assert_eq!(all["offline"], false);
    assert_eq!(app.ok(query.clone())["indexId"], all["indexId"]);
    app.ok(json!({"command":"search_record","kind":"favorite","id":"42:1","value":true}));
    app.ok(json!({"command":"search_tag","key":"42:1","tag":"Live"}));
    let mut favorite = query.clone();
    favorite["query"] = json!({"query":"Report","favoritesOnly":true,"tags":["Live"]});
    let found = app.ok(favorite);
    assert_eq!(found["total"], 1);
    assert_eq!(found["files"][0]["folder_id"], 42);
    assert_eq!(
        app.ok(json!({"command":"workspace","owner":"101"}))["files"]
            .as_array()
            .unwrap()
            .len(),
        0,
        "transient names must not be persisted just to carry organization"
    );
    app.ok(json!({"command":"search_record","kind":"removal","id":"42:1","value":{"key":"42:1","status":"deleted"}}));
    assert_eq!(app.ok(query)["total"], 19);
    app.stop();
}

#[test]
fn local_search_live_overlay_preserves_embedded_flags_and_explicit_record_overrides() {
    let fixture = Fixture::new();
    fixture.write(
        "flags.json",
        json!({"highwater":1,"rows":[{"id":1,"name":"Existing report.pdf"}]}).to_string(),
    );
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!(
        {
            "command":"workspace_seed_files",
            "owner":"101",
            "files":[
                {
                    "id":1,
                    "folder_id":42,
                    "name":"Existing report.pdf",
                    "size":1000,
                    "mime_type":"application/pdf",
                    "file_ext":"pdf",
                    "created_at":"2026-10-01",
                    "icon_type":"file",
                    "encryption_state":"plain",
                    "is_favorite":true,
                    "is_pinned":true
                }
            ]
        }
    ));
    let query = json!({"command":"search_inventory","owner":"101","source":"flags.json","folders":[42],"query":{"favoritesOnly":true}});
    let found = app.ok(query.clone());
    assert_eq!(found["total"], 1);
    assert_eq!(found["files"][0]["is_pinned"], true);
    app.ok(json!({"command":"search_record","kind":"favorite","id":"42:1","value":false}));
    assert_eq!(app.ok(query)["total"], 0);
    app.stop();
}

#[test]
fn local_search_workspace_asset_lookup_opens_unindexed_plain_media_without_persisting_names() {
    let fixture = Fixture::new();
    image::RgbaImage::from_pixel(800, 600, image::Rgba([12, 42, 83, 255]))
        .save(fixture.path("live.png"))
        .unwrap();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let request = json!({"command":"workspace_asset_read","owner":"101","folder":42,"id":1,"source":"live.png"});
    let preview = app.ok(request.clone());
    assert_eq!(
        std::fs::read(preview["path"].as_str().unwrap()).unwrap(),
        fixture.read("live.png")
    );
    let mut thumbnail = request.clone();
    thumbnail["thumbnail"] = json!(true);
    let thumb = app.ok(thumbnail);
    assert_eq!(
        image::image_dimensions(thumb["path"].as_str().unwrap()).unwrap(),
        (480, 360)
    );
    assert_eq!(
        app.ok(json!({"command":"workspace","owner":"101"}))["files"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    let mut foreign = request.clone();
    foreign["owner"] = json!("202");
    assert!(app.failure(foreign).contains("ACCOUNT_CHANGED"));
    let mut invalid = request;
    invalid["id"] = json!(-1);
    assert!(app.failure(invalid).contains("INVALID_FILE_KEY"));
    app.stop();
}

#[test]
fn local_search_date_window_is_frozen_across_pages_of_one_generation() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let boundary = (chrono::Utc::now() - chrono::Duration::days(7) + chrono::Duration::seconds(3))
        .to_rfc3339();
    let files: Vec<_> = (1..=2)
        .map(|id| {
            json!(
                {
                    "id":id,
                    "folder_id":42,
                    "name":format!("Boundary report{id}.pdf"),
                    "size":1000,
                    "file_ext":"pdf",
                    "created_at":boundary,
                    "icon_type":"file",
                    "encryption_state":"plain",
                    "is_favorite":false,
                    "is_pinned":false
                }
            )
        })
        .collect();
    app.ok(json!({"command":"workspace_seed_files","owner":"101","files":files}));
    let first=app.ok(json!({"command":"search_runtime","owner":"101","folders":[42],"query":{"query":"Boundary","date":"7d","limit":1}}));
    assert_eq!(first["total"], 2);
    std::thread::sleep(Duration::from_secs(4));
    let next=app.ok(json!({"command":"search_runtime","owner":"101","folders":[42],"query":{"query":"Boundary","date":"7d","limit":1,"offset":1,"indexId":first["indexId"]}}));
    assert_eq!(next["total"], 2);
    assert_ne!(next["files"][0]["id"], first["files"][0]["id"]);
    app.stop();
}

#[test]
fn configurable_weekly_quota_uses_the_saved_limit_and_releases_failed_transfer_holds() {
    use chrono::Datelike;
    let fixture = Fixture::new();
    let today = chrono::Local::now().date_naive();
    let monday = today - chrono::Duration::days(i64::from(today.weekday().num_days_from_monday()));
    fixture.write("bandwidth.json",json!({"date":monday.to_string(),"up_bytes":0,"down_bytes":0,"limit_bytes":131072,"period":"weekly"}).to_string());
    let mut app = Backend::start(&fixture.0);
    assert!(
        app.failure(json!({"command":"bandwidth_hold","bytes":262144,"upload":true}))
            .contains("limit"),
        "saved weekly quota must be authoritative"
    );
    let held = app.ok(json!({"command":"bandwidth_hold","bytes":65536,"upload":true}));
    assert_eq!(held["limit_bytes"], 131072);
    assert_eq!(app.ok(json!({"command":"bandwidth_cancel"}))["up_bytes"], 0);
    app.ok(json!({"command":"bandwidth_hold","bytes":65536,"upload":true}));
    app.ok(json!({"command":"bandwidth_commit"}));
    app.stop();
    let mut restarted = Backend::start(&fixture.0);
    assert_eq!(
        restarted.ok(json!({"command":"bandwidth_read"}))["up_bytes"],
        65536
    );
    assert!(restarted
        .failure(json!({"command":"bandwidth_hold","bytes":131072}))
        .contains("limit"));
    restarted.stop();
}

#[test]
fn a_crashed_quota_hold_is_not_committed_or_double_charged_after_restart() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"bandwidth_hold","bytes":65536,"upload":true}));
    app.child.kill().unwrap();
    app.child.wait().unwrap();
    let mut restarted = Backend::start(&fixture.0);
    assert_eq!(
        restarted.ok(json!({"command":"bandwidth_read"}))["up_bytes"],
        0,
        "a killed pending transfer must not become successful quota usage"
    );
    restarted.ok(json!({"command":"bandwidth_hold","bytes":65536,"upload":true}));
    restarted.ok(json!({"command":"bandwidth_commit"}));
    restarted.stop();
    let mut again = Backend::start(&fixture.0);
    assert_eq!(
        again.ok(json!({"command":"bandwidth_read"}))["up_bytes"],
        65536
    );
    again.stop();
}

#[test]
fn quota_rollover_keeps_admission_holds_and_late_cancellation_cannot_refund_new_usage() {
    let fixture = Fixture::new();
    fixture.write(
        "bandwidth.json",
        json!({"date":"2026-09-28","up_bytes":0,"down_bytes":0,
            "limit_bytes":131072,"period":"weekly"})
        .to_string(),
    );
    let mut app = Backend::start(&fixture.0);
    let initial = app.ok(json!({"command":"bandwidth_set_date","date":"2026-10-04"}));
    assert_eq!(initial["date"], "2026-09-28");
    assert_eq!(initial["up_bytes"], 0);
    assert_eq!(initial["down_bytes"], 0);
    app.ok(json!({"command":"bandwidth_set_limit","bytes":131072}));
    app.ok(json!({"command":"bandwidth_hold","bytes":65536,"upload":true}));
    app.ok(json!({"command":"bandwidth_set_date","date":"2026-10-05"}));
    assert!(app
        .failure(json!({"command":"bandwidth_commit_other","bytes":98304}))
        .contains("limit"));
    app.ok(json!({"command":"bandwidth_commit_other","bytes":32768}));
    let after = app.ok(json!({"command":"bandwidth_cancel"}));
    assert_eq!(after["up_bytes"], 0);
    assert_eq!(after["down_bytes"], 32768);
    assert_eq!(after["date"], "2026-10-05");
    app.ok(json!({"command":"bandwidth_hold","bytes":32768,"upload":true}));
    app.ok(json!({"command":"bandwidth_resize","bytes":65536}));
    app.ok(json!({"command":"bandwidth_commit"}));
    assert_eq!(
        app.ok(json!({"command":"bandwidth_read"}))["up_bytes"],
        65536
    );
    app.stop();
}

#[test]
fn quota_persistence_failure_keeps_committed_usage_and_blocks_admission_until_repaired() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"bandwidth_hold","bytes":65536,"upload":true}));
    std::fs::remove_file(fixture.path("bandwidth.json")).unwrap();
    std::fs::create_dir(fixture.path("bandwidth.json")).unwrap();
    assert_eq!(
        app.ok(json!({"command":"bandwidth_commit"}))["up_bytes"],
        65536
    );
    app.failure(json!({"command":"bandwidth_hold","bytes":1}));
    std::fs::remove_dir(fixture.path("bandwidth.json")).unwrap();
    assert_eq!(
        app.ok(json!({"command":"bandwidth_read"}))["up_bytes"],
        65536
    );
    app.stop();
    let mut restarted = Backend::start(&fixture.0);
    assert_eq!(
        restarted.ok(json!({"command":"bandwidth_read"}))["up_bytes"],
        65536
    );
    restarted.stop();
}

#[test]
fn quota_can_shrink_an_admitted_hold_after_the_user_lowers_the_limit() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"bandwidth_set_limit","bytes":131072}));
    app.ok(json!({"command":"bandwidth_hold","bytes":98304,"upload":true}));
    app.ok(json!({"command":"bandwidth_set_limit","bytes":32768}));
    assert!(app
        .failure(json!({"command":"bandwidth_resize","bytes":114688}))
        .contains("limit"));
    app.ok(json!({"command":"bandwidth_resize","bytes":65536}));
    app.ok(json!({"command":"bandwidth_commit"}));
    assert_eq!(
        app.ok(json!({"command":"bandwidth_read"}))["up_bytes"],
        65536
    );
    app.stop();
}

#[test]
fn quota_backward_week_clock_does_not_reset_successful_usage() {
    let fixture = Fixture::new();
    fixture.write(
        "bandwidth.json",
        json!({"date":"2026-10-05","up_bytes":0,"down_bytes":0,
            "limit_bytes":131072,"period":"weekly"})
        .to_string(),
    );
    let mut app = Backend::start(&fixture.0);
    let initial = app.ok(json!({"command":"bandwidth_set_date","date":"2026-10-05"}));
    assert_eq!(initial["date"], "2026-10-05");
    assert_eq!(initial["up_bytes"], 0);
    assert_eq!(initial["down_bytes"], 0);
    app.ok(json!({"command":"bandwidth_set_limit","bytes":131072}));
    app.ok(json!({"command":"bandwidth_commit_other","bytes":98304}));
    let reversed = app.ok(json!({"command":"bandwidth_set_date","date":"2026-10-04"}));
    assert_eq!(
        reversed["down_bytes"], 98304,
        "backward Monday boundary must retain usage"
    );
    assert!(app
        .failure(json!({"command":"bandwidth_hold","bytes":1}))
        .contains("clock"));
    app.ok(json!({"command":"bandwidth_set_date","date":"2026-10-05"}));
    app.ok(json!({"command":"bandwidth_hold","bytes":32768}));
    app.ok(json!({"command":"bandwidth_cancel"}));
    app.stop();
}

#[test]
fn quota_corrupt_existing_file_is_preserved_and_cannot_reset_the_limit_or_usage() {
    let fixture = Fixture::new();
    let corrupt = "{truncated accounting";
    fixture.write("bandwidth.json", corrupt);
    let mut app = Backend::start(&fixture.0);
    app.failure(json!({"command":"bandwidth_hold","bytes":1}));
    assert_eq!(fixture.read("bandwidth.json"), corrupt.as_bytes());
    use chrono::Datelike;
    let today = chrono::Local::now().date_naive();
    let monday = today - chrono::Duration::days(i64::from(today.weekday().num_days_from_monday()));
    fixture.write("bandwidth.json",json!({"date":monday.to_string(),"up_bytes":32768,"down_bytes":0,"limit_bytes":65536,"period":"weekly"}).to_string());
    assert!(app
        .failure(json!({"command":"bandwidth_hold","bytes":65536}))
        .contains("limit"));
    assert_eq!(
        app.ok(json!({"command":"bandwidth_read"}))["up_bytes"],
        32768
    );
    app.stop();
}

#[tokio::test]
async fn traffic_download_limit_applies_outside_vpn_and_is_shared_by_http_streams() {
    let fixture = Fixture::new();
    fixture.write("media.bin", vec![42; 131072]);
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!({"command":"traffic_configure","vpn":false,"downKbs":128}));
    let started = app
        .ok(json!({"command":"start_media_fixture","source":"media.bin","filename":"media.bin"}));
    let url = started["url"].as_str().unwrap();
    let client = reqwest::Client::new();
    let started_at = Instant::now();
    let fetch = || async { client.get(url).send().await.unwrap().bytes().await.unwrap() };
    let (a, b) = tokio::join!(fetch(), fetch());
    assert_eq!(a.len(), 131072);
    assert_eq!(b.len(), 131072);
    assert!(
        started_at.elapsed() >= Duration::from_millis(1850),
        "Two streams must share one 128 KiB/s allowance outside VPN; elapsed {:?}",
        started_at.elapsed()
    );
    app.stop();
}

#[test]
fn traffic_upload_throttle_is_flagged_and_paces_only_unconfirmed_parts() {
    let fixture = Fixture::new();
    fixture.write("source.bin", synthetic_bytes(1048576, 3));
    settle(&fixture.path("source.bin"));
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!({"command":"traffic_configure","upKbs":512}));
    let upload = |fail| json!({"command":"upload_resumable","source":"source.bin","sink":"parts","failAtPart":fail});
    let mut first = upload(None::<i32>);
    first["publish"] = json!(true);
    let started = Instant::now();
    app.ok(first);
    assert!(started.elapsed() < Duration::from_secs(2));
    app.ok(json!({"command":"traffic_configure","upKbs":512,"bandwidth_schedule":true}));
    let started = Instant::now();
    app.failure(upload(Some(1)));
    assert!(started.elapsed() >= Duration::from_millis(1850));
    let started = Instant::now();
    let completed = app.ok(upload(None::<i32>));
    assert_eq!(completed["resumedParts"], 1);
    assert!(started.elapsed() >= Duration::from_millis(850));
    assert!(
        started.elapsed() < Duration::from_millis(1850),
        "The verified local prefix must not be throttled again: {:?}",
        started.elapsed()
    );
    let before = fixture.read("network_settings.json");
    app.stop();
    let mut restarted = Backend::start(&fixture.0);
    let snapshot = restarted.ok(json!({"command":"traffic_snapshot"}));
    assert_eq!(snapshot["vpn"]["bandwidth_schedule"], true);
    assert_eq!(snapshot["vpn"]["bandwidth_limit_up_kbs"], 512);
    assert_eq!(fixture.read("network_settings.json"), before);
    restarted.stop();
}

#[tokio::test]
async fn traffic_overnight_pause_has_half_open_boundaries_and_overlap_uses_the_lower_rate() {
    let fixture = Fixture::new();
    fixture.write("media.bin", vec![8; 131072]);
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!({"command":"traffic_clock","time":"2026-10-02 23:50"}));
    let windows = json!([
        {"days":[4],"start_minute":1380,"end_minute":30,"up_kbs":0,"down_kbs":0,"pause":true},
        {"days":[5],"start_minute":30,"end_minute":60,"up_kbs":0,"down_kbs":64,"pause":false},
        {"days":[5],"start_minute":0,"end_minute":60,"up_kbs":0,"down_kbs":96,"pause":false}
    ]);
    app.ok(json!({"command":"traffic_configure","downKbs":128,"bandwidth_schedule":true,"windows":windows}));
    let started = app
        .ok(json!({"command":"start_media_fixture","source":"media.bin","filename":"media.bin"}));
    let url = started["url"].as_str().unwrap().to_owned();
    let request =
        tokio::spawn(async move { reqwest::get(url).await.unwrap().bytes().await.unwrap() });
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert!(!request.is_finished());
    app.ok(json!({"command":"traffic_clock","time":"2026-10-03 00:29"}));
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert!(
        !request.is_finished(),
        "Overnight pause belongs to the previous weekday"
    );
    let resumed = Instant::now();
    app.ok(json!({"command":"traffic_clock","time":"2026-10-03 00:30"}));
    assert_eq!(request.await.unwrap().len(), 131072);
    assert!(
        resumed.elapsed() >= Duration::from_millis(1850),
        "Overlapping 64/96/base128 limits must use 64 KiB/s without a pause burst"
    );
    app.ok(json!({"command":"traffic_clock","time":"2026-10-03 01:00"}));
    let resumed = Instant::now();
    let bytes = reqwest::get(started["url"].as_str().unwrap())
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(bytes.len(), 131072);
    assert!(resumed.elapsed() >= Duration::from_millis(850));
    assert!(resumed.elapsed() < Duration::from_millis(1850));
    app.stop();
}

#[tokio::test]
async fn traffic_disabled_schedule_ignores_windows_and_pause_withdraws_on_account_change() {
    let fixture = Fixture::new();
    fixture.write("media.bin", vec![4; 65536]);
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!({"command":"traffic_clock","time":"2026-10-02 12:00"}));
    let windows = json!([{"days":[4],"start_minute":0,"end_minute":1440,"up_kbs":0,"down_kbs":0,"pause":true}]);
    app.ok(json!({"command":"traffic_configure","windows":windows}));
    let started = app
        .ok(json!({"command":"start_media_fixture","source":"media.bin","filename":"media.bin"}));
    let url = started["url"].as_str().unwrap();
    let bytes = tokio::time::timeout(Duration::from_secs(2), async {
        reqwest::get(url).await.unwrap().bytes().await.unwrap()
    })
    .await
    .unwrap();
    assert_eq!(bytes.len(), 65536);
    app.ok(json!({"command":"traffic_configure","bandwidth_schedule":true,"windows":windows}));
    let url = url.to_owned();
    let request = tokio::spawn(async move {
        match reqwest::get(url).await {
            Ok(response) => response.bytes().await.map(|bytes| bytes.len()),
            Err(error) => Err(error),
        }
    });
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert!(!request.is_finished());
    app.ok(json!({"command":"seed_account","owner":202}));
    let result = tokio::time::timeout(Duration::from_secs(2), request)
        .await
        .unwrap()
        .unwrap();
    assert!(result.is_err());
    app.stop();
}

#[tokio::test]
async fn traffic_failed_settings_write_preserves_runtime_limits_and_the_saved_configuration() {
    let fixture = Fixture::new();
    fixture.write("media.bin", vec![4; 65536]);
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!({"command":"traffic_configure","downKbs":64}));
    let saved = fixture.read("network_settings.json");
    std::fs::remove_file(fixture.path("network_settings.json")).unwrap();
    std::fs::create_dir(fixture.path("network_settings.json")).unwrap();
    app.failure(json!({"command":"traffic_configure","downKbs":0,"bandwidth_schedule":true}));
    let snapshot = app.ok(json!({"command":"traffic_snapshot"}));
    assert_eq!(snapshot["vpn"]["bandwidth_limit_down_kbs"], 64);
    assert_eq!(snapshot["vpn"]["bandwidth_schedule"], false);
    let started = app
        .ok(json!({"command":"start_media_fixture","source":"media.bin","filename":"media.bin"}));
    let time = Instant::now();
    let bytes = reqwest::get(started["url"].as_str().unwrap())
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(bytes.len(), 65536);
    assert!(time.elapsed() >= Duration::from_millis(850));
    std::fs::remove_dir(fixture.path("network_settings.json")).unwrap();
    fixture.write("network_settings.json", saved);
    app.stop();
    let mut restarted = Backend::start(&fixture.0);
    assert_eq!(
        restarted.ok(json!({"command":"traffic_snapshot"}))["vpn"]["bandwidth_limit_down_kbs"],
        64
    );
    restarted.stop();
}

#[tokio::test]
async fn quota_http_ranges_charge_successful_response_bytes_and_failed_bodies_release_the_hold() {
    let fixture = Fixture::new();
    fixture.write("media.bin", vec![4; 131072]);
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!({"command":"bandwidth_set_limit","bytes":131072}));
    let started = app
        .ok(json!({"command":"start_media_fixture","source":"media.bin","filename":"media.bin"}));
    let url = started["url"].as_str().unwrap();
    let client = reqwest::Client::new();
    let failed = client.get(format!("{url}&failAfter=0")).send().await;
    if let Ok(response) = failed {
        assert!(response.bytes().await.is_err());
    }
    assert_eq!(app.ok(json!({"command":"bandwidth_read"}))["down_bytes"], 0);
    let mut partial = client
        .get(format!("{url}&failAfter=65536"))
        .send()
        .await
        .unwrap();
    assert!(!partial.chunk().await.unwrap().unwrap().is_empty());
    assert!(partial.bytes().await.is_err());
    assert_eq!(
        app.ok(json!({"command":"bandwidth_read"}))["down_bytes"],
        0,
        "Partial delivery must release the whole failed body hold"
    );
    let bytes = client
        .get(url)
        .header("Range", "bytes=0-32767")
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(bytes.len(), 32768);
    assert_eq!(
        app.ok(json!({"command":"bandwidth_read"}))["down_bytes"],
        32768
    );
    assert_eq!(client.get(url).send().await.unwrap().status(), 429);
    app.stop();
    let mut restarted = Backend::start(&fixture.0);
    assert_eq!(
        restarted.ok(json!({"command":"bandwidth_read"}))["down_bytes"],
        32768
    );
    restarted.stop();
}

#[tokio::test]
async fn traffic_paused_protected_stream_releases_without_plaintext_when_the_vault_locks() {
    let fixture = Fixture::new();
    fixture.write("plain.bin", vec![4; 65536]);
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!({"command":"vault_create","passphrase":"private fixture passphrase"}));
    app.ok(json!({"command":"fixture_encrypt","source":"plain.bin","destination":"protected.bin"}));
    app.ok(json!({"command":"traffic_clock","time":"2026-10-02 12:00"}));
    app.ok(json!({"command":"traffic_configure","bandwidth_schedule":true,"windows":[{"days":[4],"start_minute":0,"end_minute":1440,"up_kbs":0,"down_kbs":0,"pause":true}]}));
    let started = app.ok(
        json!({"command":"start_media_fixture","source":"protected.bin","filename":"private.bin"}),
    );
    let url = format!(
        "{}&protected=true&credential={}",
        started["url"].as_str().unwrap(),
        started["credential"].as_str().unwrap()
    );
    let pending = tokio::spawn(async move {
        match reqwest::get(url).await {
            Ok(response) => response.bytes().await.map(|bytes| bytes.len()),
            Err(error) => Err(error),
        }
    });
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert!(!pending.is_finished());
    app.ok(json!({"command":"vault_lock"}));
    let result = tokio::time::timeout(Duration::from_secs(2), pending)
        .await
        .unwrap()
        .unwrap();
    assert!(result.is_err());
    app.stop();
}

#[cfg(unix)]
fn abortive_close(socket: std::net::TcpStream) {
    use std::os::fd::AsRawFd;
    let linger = libc::linger {
        l_onoff: 1,
        l_linger: 0,
    };
    // The fixture owns this socket; reset both directions instead of an HTTP
    // half-close, which legitimately keeps an active response alive.
    assert_eq!(
        unsafe {
            libc::setsockopt(
                socket.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_LINGER,
                std::ptr::from_ref(&linger).cast(),
                std::mem::size_of_val(&linger) as libc::socklen_t,
            )
        },
        0
    );
    drop(socket);
}
#[cfg(windows)]
fn abortive_close(socket: std::net::TcpStream) {
    use std::os::windows::io::AsRawSocket;
    #[repr(C)]
    struct Linger {
        on: u16,
        seconds: u16,
    }
    #[link(name = "ws2_32")]
    extern "system" {
        fn setsockopt(
            socket: usize,
            level: i32,
            name: i32,
            value: *const std::ffi::c_char,
            len: i32,
        ) -> i32;
    }
    let linger = Linger { on: 1, seconds: 0 };
    assert_eq!(
        unsafe {
            setsockopt(
                socket.as_raw_socket() as usize,
                0xffff,
                0x0080,
                std::ptr::from_ref(&linger).cast(),
                std::mem::size_of_val(&linger) as i32,
            )
        },
        0
    );
    drop(socket);
}

#[tokio::test]
async fn traffic_disconnected_http_waiters_do_not_leave_future_delay_for_a_new_transfer() {
    let fixture = Fixture::new();
    fixture.write("media.bin", vec![4; 65536]);
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!({"command":"traffic_configure","downKbs":32}));
    let started = app
        .ok(json!({"command":"start_media_fixture","source":"media.bin","filename":"media.bin"}));
    let url = started["url"].as_str().unwrap().to_owned();
    let parsed = reqwest::Url::parse(&url).unwrap();
    let address = format!("{}:{}", parsed.host_str().unwrap(), parsed.port().unwrap());
    let request = format!(
        "GET {}?{} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        parsed.path(),
        parsed.query().unwrap(),
        address
    );
    let mut a = std::net::TcpStream::connect(&address).unwrap();
    let mut b = std::net::TcpStream::connect(&address).unwrap();
    a.write_all(request.as_bytes()).unwrap();
    b.write_all(request.as_bytes()).unwrap();
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert_eq!(
        app.ok(json!({"command":"traffic_pending"})),
        2,
        "Both bodies must be waiting for shared allowance"
    );
    abortive_close(a);
    abortive_close(b);
    let deadline = Instant::now() + Duration::from_secs(1);
    while app.ok(json!({"command":"traffic_pending"})) != 0 {
        assert!(
            Instant::now() < deadline,
            "Server did not withdraw reset sockets"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let start = Instant::now();
    let bytes = tokio::time::timeout(Duration::from_millis(2900), async {
        reqwest::get(url).await.unwrap().bytes().await.unwrap()
    })
    .await
    .expect("Canceled HTTP waits left future bandwidth debt");
    assert_eq!(bytes.len(), 65536);
    assert!(start.elapsed() >= Duration::from_millis(1850));
    app.stop();
}

#[test]
fn traffic_proxy_save_failure_restores_the_old_credential_and_runtime_configuration() {
    let fixture = Fixture::new();
    fixture.write("proxy-credential", "old synthetic proxy secret");
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"traffic_proxy_patch","host":"old.example"}));
    let saved = fixture.read("network_settings.json");
    std::fs::remove_file(fixture.path("network_settings.json")).unwrap();
    std::fs::create_dir(fixture.path("network_settings.json")).unwrap();
    app.failure(json!({"command":"traffic_proxy_patch","host":"new.example","password":"new synthetic proxy secret"}));
    assert_eq!(
        fixture.read("proxy-credential"),
        b"old synthetic proxy secret"
    );
    assert_eq!(
        app.ok(json!({"command":"traffic_snapshot"}))["proxy"]["host"],
        "old.example"
    );
    assert_eq!(
        app.ok(json!({"command":"traffic_proxy_matches","password":"old synthetic proxy secret"})),
        true
    );
    app.failure(json!({"command":"traffic_proxy_patch","clear":true}));
    assert_eq!(
        fixture.read("proxy-credential"),
        b"old synthetic proxy secret"
    );
    std::fs::remove_dir(fixture.path("network_settings.json")).unwrap();
    fixture.write("network_settings.json", saved);
    app.stop();
    let mut restarted = Backend::start(&fixture.0);
    assert_eq!(
        restarted
            .ok(json!({"command":"traffic_proxy_matches","password":"old synthetic proxy secret"})),
        true
    );
    restarted.stop();
}

#[test]
fn traffic_proxy_validation_failure_cannot_mutate_a_saved_credential() {
    let fixture = Fixture::new();
    fixture.write("proxy-credential", "old synthetic proxy secret");
    let mut app = Backend::start(&fixture.0);
    assert!(app.failure(json!({"command":"traffic_proxy_patch","password":"new synthetic proxy secret","invalidWindow":true})).contains("schedule"));
    assert_eq!(
        fixture.read("proxy-credential"),
        b"old synthetic proxy secret"
    );
    assert!(!fixture.path("network_settings.json").exists());
    app.stop();
}

#[test]
fn traffic_ordinary_upload_readers_share_the_allowance_across_disk_to_disk_transfers() {
    let fixture = Fixture::new();
    let content = synthetic_bytes(65536, 8);
    fixture.write("source.bin", &content);
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!({"command":"traffic_configure","upKbs":64,"bandwidth_schedule":true}));
    let started = Instant::now();
    app.ok(json!({"command":"reader_copy_start","source":"source.bin","destination":"copy","copies":2}));
    assert_eq!(
        app.ok(json!({"command":"reader_copy_finish"})),
        json!([65536, 65536])
    );
    assert!(started.elapsed() >= Duration::from_millis(1850));
    assert_eq!(fixture.read("copy-0"), content);
    assert_eq!(fixture.read("copy-1"), content);
    assert_eq!(
        app.ok(json!({"command":"bandwidth_read"}))["up_bytes"],
        131072
    );
    app.stop();
}

#[test]
fn traffic_canceling_a_paused_upload_reader_removes_staging_and_quota_before_a_fresh_copy() {
    let fixture = Fixture::new();
    let content = synthetic_bytes(65536, 8);
    fixture.write("source.bin", &content);
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!({"command":"traffic_clock","time":"2026-10-02 12:00"}));
    app.ok(json!({"command":"traffic_configure","bandwidth_schedule":true,"windows":[{"days":[4],"start_minute":0,"end_minute":1440,"up_kbs":0,"down_kbs":0,"pause":true}]}));
    app.ok(json!({"command":"reader_copy_start","source":"source.bin","destination":"paused"}));
    std::thread::sleep(Duration::from_millis(350));
    assert_eq!(
        app.ok(json!({"command":"bandwidth_read"}))["up_bytes"],
        65536
    );
    assert_eq!(
        app.ok(json!({"command":"reader_copy_cancel"}))["up_bytes"],
        0
    );
    assert!(!fixture.path("paused-0").exists());
    assert!(!std::fs::read_dir(&fixture.0).unwrap().any(|entry| entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with("paused-0.")));
    app.ok(json!({"command":"traffic_configure"}));
    app.ok(json!({"command":"reader_copy_start","source":"source.bin","destination":"fresh"}));
    app.ok(json!({"command":"reader_copy_finish"}));
    assert_eq!(fixture.read("fresh-0"), content);
    app.stop();
    let mut restarted = Backend::start(&fixture.0);
    assert_eq!(
        restarted.ok(json!({"command":"bandwidth_read"}))["up_bytes"],
        65536
    );
    restarted.stop();
}

#[tokio::test]
async fn traffic_credential_storage_wait_cannot_block_an_existing_http_transfer() {
    let fixture = Fixture::new();
    fixture.write("media.bin", vec![4; 65536]);
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!({"command":"traffic_configure","downKbs":32}));
    let started = app
        .ok(json!({"command":"start_media_fixture","source":"media.bin","filename":"media.bin"}));
    let url = started["url"].as_str().unwrap().to_owned();
    let request =
        tokio::spawn(async move { reqwest::get(url).await.unwrap().bytes().await.unwrap() });
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert_eq!(app.ok(json!({"command":"traffic_pending"})), 1);
    app.ok(json!({"command":"traffic_proxy_start_slow"}));
    let result = tokio::time::timeout(Duration::from_millis(2500), request).await;
    fixture.write("proxy-io-release", b"release");
    app.ok(json!({"command":"traffic_proxy_finish_slow"}));
    assert_eq!(
        result
            .expect("Credential IO held configuration locks and blocked shared pacing")
            .unwrap()
            .len(),
        65536
    );
    assert_eq!(
        app.ok(json!({"command":"traffic_snapshot"}))["proxy"]["host"],
        "new.fixture"
    );
    app.stop();
}

#[tokio::test]
async fn traffic_delivered_protected_media_renews_activity_and_idle_media_does_not() {
    let fixture = Fixture::new();
    fixture.write("plain.bin", vec![4; 65536]);
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!({"command":"vault_create","passphrase":"private fixture passphrase"}));
    app.ok(json!({"command":"fixture_encrypt","source":"plain.bin","destination":"protected.bin"}));
    app.ok(json!({"command":"vault_auto_timeout","milliseconds":3000}));
    let started = app.ok(
        json!({"command":"start_media_fixture","source":"protected.bin","filename":"private.bin"}),
    );
    let url = format!(
        "{}&protected=true&credential={}&delayMs=1000",
        started["url"].as_str().unwrap(),
        started["credential"].as_str().unwrap()
    );
    assert_eq!(
        reqwest::get(url)
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap()
            .len(),
        65536
    );
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(
        app.ok(json!({"command":"vault_auto_due"})),
        false,
        "Delivered protected bytes remain recent vault activity"
    );
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(
        app.ok(json!({"command":"vault_auto_due"})),
        true,
        "An idle completed stream must not renew the vault"
    );
    app.stop();
}

#[tokio::test]
async fn sponsor_origins_only_hide_requests_after_verified_active_or_grace_access() {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use ed25519_dalek::{Signer, SigningKey};
    use sha2::{Digest, Sha256};
    for separate_mode in [false, true] {
        let fixture = Fixture::new();
        let mut app = Backend::start(&fixture.0);
        let occupied = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let taken = occupied.local_addr().unwrap().port();
        let media = app.ok(json!({"command":"start_stream_server","preferredPort":taken}));
        assert_ne!(media["port"], json!(taken));
        let separate = if separate_mode {
            app.ok(json!({"command":"start_ad_server"}))
        } else {
            media.clone()
        };
        if separate_mode {
            assert_ne!(media["port"], separate["port"]);
            assert_eq!(separate["publishedStreamPort"], media["port"]);
        }
        let client = reqwest::Client::new();
        let key = SigningKey::from_bytes(&[73; 32]);
        let public = URL_SAFE_NO_PAD.encode(key.verifying_key().as_bytes());
        let device = "synthetic-sponsor-device";
        let now = chrono::Utc::now().timestamp();
        let token =
            |expires: i64, until: i64| {
                let header = URL_SAFE_NO_PAD.encode(
                    serde_json::to_vec(&json!({"alg":"EdDSA","typ":"TD-SUPPORTER","kid":"v1"}))
                        .unwrap(),
                );
                let payload=URL_SAFE_NO_PAD.encode(serde_json::to_vec(&json!(
                {
                    "iss":"telegram-drive-supporter",
                    "aud":"telegram-drive-desktop",
                    "entitlement_id":"fixture-sponsor",
                    "device_key_hash":URL_SAFE_NO_PAD.encode(Sha256::digest(device.as_bytes())),
                    "terms_version":"2026-08-11",
                    "issued_at":now-1000,
                    "expires_at":expires,
                    "offline_until":until
                }
            )).unwrap());
                let signed = format!("{header}.{payload}");
                format!(
                    "{signed}.{}",
                    URL_SAFE_NO_PAD.encode(key.sign(signed.as_bytes()).to_bytes())
                )
            };
        for origin in [&separate] {
            let response = client
                .get(format!("{}/ad-banner", origin["url"].as_str().unwrap()))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 200);
            let csp = response.headers()["content-security-policy"]
                .to_str()
                .unwrap();
            assert!(csp.contains(&format!("http://localhost:{}/ad-script", origin["port"])));
            assert!(response
                .text()
                .await
                .unwrap()
                .contains("highperformanceformat.com"));
        }
        let invalid = format!(
            "{}.invalid",
            token(now + 100, now + 200).rsplit_once('.').unwrap().0
        );
        app.failure(json!({"command":"sponsor_load_verified","token":invalid,"devicePublicKey":device,"servicePublicKey":public,"now":now}));
        assert_eq!(
            client
                .get(format!("{}/ad-banner", separate["url"].as_str().unwrap()))
                .send()
                .await
                .unwrap()
                .status(),
            200
        );
        for expires in [now + 100, now - 10] {
            app.ok(json!({"command":"sponsor_load_verified","token":token(expires,now+200),"devicePublicKey":device,"servicePublicKey":public,"now":now}));
            for origin in [&separate] {
                for path in ["ad-banner", "ad-script"] {
                    let response = client
                        .get(format!("{}/{path}", origin["url"].as_str().unwrap()))
                        .send()
                        .await
                        .unwrap();
                    assert_eq!(
                        response.status(),
                        204,
                        "{path} must be suppressed before provider IO"
                    );
                    assert_eq!(response.headers()["cache-control"], "no-store");
                    assert!(response.bytes().await.unwrap().is_empty());
                }
            }
        }
        app.ok(json!({"command":"sponsor_load_verified","token":token(now-20,now-10),"devicePublicKey":device,"servicePublicKey":public,"now":now}));
        assert_eq!(
            client
                .get(format!("{}/ad-banner", separate["url"].as_str().unwrap()))
                .send()
                .await
                .unwrap()
                .status(),
            200
        );
        if separate_mode {
            for path in ["ad-banner", "ad-script"] {
                assert_eq!(
                    client
                        .get(format!("{}/{path}", media["url"].as_str().unwrap()))
                        .send()
                        .await
                        .unwrap()
                        .status(),
                    404
                );
            }
            for path in [
                "stream/1/2",
                "share/not-a-share",
                "hls/not-a-video/index.m3u8",
            ] {
                assert_eq!(
                    client
                        .get(format!("{}/{path}", separate["url"].as_str().unwrap()))
                        .send()
                        .await
                        .unwrap()
                        .status(),
                    404,
                    "ad origin must not expose media or shares"
                );
            }
        }
        if separate_mode {
            assert_eq!(
                app.ok(json!({"command":"stop_ad_server"}))["sponsorPort"],
                0,
                "stopped selected listener must not be advertised"
            );
        }
        app.stop();
    }
}

#[tokio::test]
async fn supporter_verification_suppresses_inflight_and_stale_ad_relay_bodies_until_signed_deadline(
) {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use ed25519_dalek::{Signer, SigningKey};
    use sha2::{Digest, Sha256};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (requests_tx, mut requests_rx) = tokio::sync::mpsc::channel::<String>(4);
    let (release_tx, mut release_rx) = tokio::sync::mpsc::channel::<bool>(4);
    let provider = tokio::spawn(async move {
        for _ in 0..3 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut headers = Vec::new();
            loop {
                let mut byte = [0; 1];
                socket.read_exact(&mut byte).await.unwrap();
                headers.push(byte[0]);
                if headers.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            requests_tx
                .send(String::from_utf8(headers).unwrap())
                .await
                .unwrap();
            let success = release_rx.recv().await.unwrap();
            let body = if success {
                format!("/* atOptions currentScript {} */", "fixture".repeat(200))
            } else {
                "outage".into()
            };
            let status = if success {
                "200 OK"
            } else {
                "503 Service Unavailable"
            };
            socket.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
        }
    });
    let origin = app
        .ok(json!({"command":"start_ad_server","fixtureUrl":format!("http://{address}/loader")}));
    let url = format!("{}/ad-script", origin["url"].as_str().unwrap());
    let referer = format!("http://localhost:{}/ad-banner", origin["port"]);
    let spawn_request = |user_agent: Option<&str>, referer: &str| {
        let mut request = reqwest::Client::new().get(&url).header("referer", referer);
        if let Some(ua) = user_agent {
            request = request.header("user-agent", ua);
        }
        tokio::spawn(async move { request.send().await.unwrap() })
    };
    let warming = spawn_request(None, &referer);
    let headers = requests_rx.recv().await.unwrap();
    assert!(headers.contains("Chrome/154.0.0.0"));
    assert!(headers.contains(&format!("referer: {referer}")));
    release_tx.send(true).await.unwrap();
    let warmed = warming.await.unwrap();
    assert_eq!(warmed.status(), 200);
    assert_eq!(warmed.headers()["x-telegram-drive-ad-cache"], "network");
    assert!(warmed.text().await.unwrap().contains("currentScript"));
    let key = SigningKey::from_bytes(&[74; 32]);
    let public = URL_SAFE_NO_PAD.encode(key.verifying_key().as_bytes());
    let device = "inflight-sponsor-device";
    let load_access = |app: &mut Backend| {
        let now = chrono::Utc::now().timestamp();
        let header = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&json!({"alg":"EdDSA","typ":"TD-SUPPORTER","kid":"v1"})).unwrap(),
        );
        let payload = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&json!(
                {
                    "iss":"telegram-drive-supporter",
                    "aud":"telegram-drive-desktop",
                    "entitlement_id":"fixture-inflight",
                    "device_key_hash":URL_SAFE_NO_PAD.encode(Sha256::digest(device.as_bytes())),
                    "terms_version":"2026-08-11",
                    "issued_at":now-100,
                    "expires_at":now-10,
                    "offline_until":now+2
                }
            ))
            .unwrap(),
        );
        let signed = format!("{header}.{payload}");
        let token = format!(
            "{signed}.{}",
            URL_SAFE_NO_PAD.encode(key.sign(signed.as_bytes()).to_bytes())
        );
        app.ok(json!({"command":"sponsor_load_verified","token":token,"devicePublicKey":device,"servicePublicKey":public,"now":now}));
    };
    for success in [true, false] {
        let pending = spawn_request(Some("FixtureWebView/42"), "http://localhost:1/ad-banner");
        let headers = requests_rx.recv().await.unwrap();
        assert!(headers.contains("user-agent: FixtureWebView/42"));
        assert!(!headers.contains("referer:"));
        load_access(&mut app);
        release_tx.send(success).await.unwrap();
        let response = pending.await.unwrap();
        assert_eq!(response.status(), 204);
        assert!(response.bytes().await.unwrap().is_empty());
        app.failure(json!(
            {
                "command":"sponsor_load_verified",
                "token":"unavailable-fixture",
                "devicePublicKey":device,
                "servicePublicKey":public,
                "now":chrono::Utc::now().timestamp()
            }
        ));
        assert_eq!(
            reqwest::get(&url).await.unwrap().status(),
            204,
            "temporary verification failure retains signed grace"
        );
        tokio::time::sleep(Duration::from_secs(3)).await;
        assert_eq!(
            reqwest::get(format!("{}/ad-banner", origin["url"].as_str().unwrap()))
                .await
                .unwrap()
                .status(),
            200,
            "suppression expires at the signed offline deadline"
        );
    }
    provider.await.unwrap();
    app.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn share_password_http_negotiates_language_weights_aliases_and_exact_query_fields() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!({"command":"database"}));
    let db = sqlite::open(fixture.path("shares.db")).unwrap();
    db.execute("INSERT INTO shared_links(id,owner_id,message_id,file_name,password_hash,created_at) VALUES('language',101,42,'A<&\"private.pdf','synthetic-hash',1)").unwrap();
    drop(db);
    let url = app.ok(json!({"command":"start_http"}))["url"]
        .as_str()
        .unwrap()
        .to_string();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    for (query, accept, expected) in [
        ("", "fr;q=1,ar;q=0.1", "fr"),
        ("", "en-US;q=0,en-GB;q=1,fr;q=0.5", "en"),
        ("", "es-MX;q=0.1,es-ES;q=0.9,fr;q=0.5", "es"),
        ("", "es;q=0,*;q=1", "en"),
        ("", "zh;q=0,zh-HK;q=1", "zh-TW"),
        ("", "ar;q=0,ja;q=0.8", "ja"),
        ("", "de;q=0.5,es;q=0.5", "de"),
        ("?notlang=ar", "es", "es"),
        ("?lang=arabic", "ko", "ko"),
        ("?lang=zh%2DHant%2DHK", "en", "zh-TW"),
        ("?lang=ur_PK", "en", "ur-PK"),
        ("", "in-ID", "id"),
        ("", "tl-PH", "fil-PH"),
        ("", "xx;q=1,pt-PT;q=0.7", "pt-BR"),
        ("", "ar;q=oops,ja;q=0.6", "ja"),
        ("", "en;q=0,*;q=0.5", "es"),
    ] {
        let response = client
            .get(format!("{url}/d/language{query}"))
            .header("Accept-Language", accept)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()["cache-control"], "no-store");
        let html = response.text().await.unwrap();
        assert!(
            html.contains(&format!("<html lang=\"{expected}\"")),
            "wrong language for {query}/{accept}"
        );
        assert!(html.contains("A&lt;&amp;&quot;private.pdf"));
        assert!(!html.contains("A<&\"private.pdf"));
        let dir = if ["ar", "fa-IR", "ur-PK"].contains(&expected) {
            "rtl"
        } else {
            "ltr"
        };
        assert!(html.contains(&format!("dir=\"{dir}\"")));
    }
    app.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn share_password_http_renders_all_24_catalogs_and_retains_language_through_verification() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!({"command":"database"}));
    let db = sqlite::open(fixture.path("shares.db")).unwrap();
    let hash = bcrypt::hash("language journey password", 4).unwrap();
    let languages = [
        "en", "es", "ru", "uk-UA", "pl-PL", "fa-IR", "ur-PK", "ms-MY", "zh-CN", "zh-TW", "fr",
        "it", "ar", "pt-BR", "de", "hi", "bn-BD", "id", "fil-PH", "tr", "th-TH", "ja", "ko", "vi",
    ];
    for lang in languages {
        let mut statement = db.prepare("INSERT INTO shared_links(id,owner_id,message_id,file_name,password_hash,created_at) VALUES(?,101,42,'language.pdf',?,1)").unwrap();
        statement
            .bind((1, format!("language-{lang}").as_str()))
            .unwrap();
        statement.bind((2, hash.as_str())).unwrap();
        statement.next().unwrap();
    }
    drop(db);
    let url = app.ok(json!({"command":"start_http"}))["url"]
        .as_str()
        .unwrap()
        .to_string();
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    for lang in languages {
        let token = format!("language-{lang}");
        let html = client
            .get(format!("{url}/d/{token}?lang={lang}"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(
            html.contains(&format!("<html lang=\"{lang}\"")),
            "missing language {lang}"
        );
        assert!(html.contains(&format!("action=\"/d/{token}/verify?lang={lang}\"")));
        assert!(html.contains(&format!("name=\"lang\" value=\"{lang}\"")));
        let catalog: Value = serde_json::from_slice(
            &std::fs::read(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join(format!("../src/i18n/locales/{lang}.json")),
            )
            .unwrap(),
        )
        .unwrap();
        let escape = |text: &str| {
            text.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('"', "&quot;")
                .replace('\'', "&#x27;")
        };
        for key in ["title", "heading", "description", "verify"] {
            assert!(
                html.contains(&escape(catalog["share_page"][key].as_str().unwrap())),
                "missing {lang}/{key}"
            );
        }
        let wrong = client
            .post(format!("{url}/d/{token}/verify"))
            .header("Accept-Language", "en")
            .form(&[("password", "wrong"), ("lang", lang)])
            .send()
            .await
            .unwrap();
        assert_eq!(wrong.status(), 200);
        assert!(!wrong.headers().contains_key("set-cookie"));
        let wrong_html = wrong.text().await.unwrap();
        assert!(wrong_html.contains(&format!("<html lang=\"{lang}\"")));
        assert!(wrong_html.contains(&escape(
            catalog["share_page"]["incorrect"].as_str().unwrap()
        )));
        let verified = client
            .post(format!("{url}/d/{token}/verify"))
            .form(&[("password", "language journey password"), ("lang", lang)])
            .send()
            .await
            .unwrap();
        assert_eq!(verified.status(), 302);
        assert_eq!(
            verified.headers()["location"],
            format!("/d/{token}?lang={lang}")
        );
        let cookie = verified.headers()["set-cookie"].to_str().unwrap();
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Strict"));
        assert!(cookie.contains(&format!("Path=/d/{token}")));
    }
    app.ok(json!({"command":"seed_account","owner":202}));
    assert_eq!(
        client
            .get(format!("{url}/d/language-es?lang=es"))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    app.stop();
}

fn notification_fixture_job(id: &str) -> Value {
    json!(
        {
            "id":id,
            "ownerId":"101",
            "direction":"upload",
            "kind":"local_upload",
            "status":"pending",
            "path":"/synthetic/private-folder/secret-report.pdf",
            "filename":"/synthetic/private-folder/secret-report.pdf",
            "progress":0,
            "transferredBytes":0,
            "totalBytes":50,
            "speedBytesPerSec":0,
            "queuePosition":0,
            "revision":1,
            "createdAt":1,
            "updatedAt":2
        }
    )
}
fn wait_for_notification_count(app: &mut Backend, count: usize) -> Value {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let delivered = app.ok(json!({"command":"notification_deliveries"}));
        if delivered.as_array().unwrap().len() >= count || Instant::now() >= deadline {
            return delivered;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn desktop_notifications_localize_stored_transfer_outcomes_and_preserve_receipts_after_restart() {
    let fixture = Fixture::new();
    fixture.write("settings.json", br#"{"settings":{"language":"es"}}"#);
    fixture.write(
        "notification-preferences.json",
        concat!(
            r#"{"notificationsEnabled":true,"notifyCompleted":true,"#,
            r#""notifyFailed":true,"notifyPaused":true,"notifyAttention":true,"#,
            r#""showFilenamesInNotifications":false}"#,
        )
        .as_bytes(),
    );
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!({"command":"save_transfer","job":notification_fixture_job("complete")}));
    app.ok(json!({"command":"save_transfer","job":notification_fixture_job("fail")}));
    app.ok(json!({"command":"notifications_start"}));
    app.ok(json!({"command":"notification_transition","id":"complete","status":"completed"}));
    let delivered = wait_for_notification_count(&mut app, 1);
    assert_eq!(delivered.as_array().unwrap().len(), 1);
    assert_eq!(delivered[0]["title"], "Transferencia completada");
    assert_eq!(delivered[0]["body"], "Se completó una subida.");
    assert!(!delivered.to_string().contains("secret-report"));
    let receipts: Value =
        serde_json::from_slice(&fixture.read("desktop-notification-receipts.v1.json")).unwrap();
    assert_eq!(receipts.as_array().unwrap().len(), 1);
    assert_eq!(receipts[0]["revision"], 2);
    app.stop();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"notifications_start"}));
    app.ok(json!({"command":"notification_replay","id":"complete"}));
    std::thread::sleep(Duration::from_millis(1100));
    assert_eq!(
        app.ok(json!({"command":"notification_deliveries"}))
            .as_array()
            .unwrap()
            .len(),
        1
    );
    app.ok(json!({"command":"notification_transition","id":"fail","error":"synthetic permanent failure"}));
    let delivered = wait_for_notification_count(&mut app, 2);
    assert_eq!(delivered.as_array().unwrap().len(), 2);
    assert_eq!(delivered[1]["title"], "Transferencia fallida");
    assert!(!delivered.to_string().contains("secret-report"));
    app.stop();
}

#[test]
fn desktop_notifications_deliver_an_arrival_during_aggregation_handoff_without_another_event() {
    let fixture = Fixture::new();
    fixture.write(
        "notification-preferences.json",
        br#"{"notificationsEnabled":true}"#,
    );
    fixture.write("notification-drain-hold", b"hold");
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    for id in ["first", "during-handoff"] {
        app.ok(json!({"command":"save_transfer","job":notification_fixture_job(id)}));
    }
    app.ok(json!({"command":"notifications_start"}));
    app.ok(json!({"command":"notification_transition","id":"first","status":"completed"}));
    let deadline = Instant::now() + Duration::from_secs(15);
    while !fixture.path("notification-drain-entered").exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(fixture.path("notification-drain-entered").exists());
    app.ok(json!({"command":"notification_transition","id":"during-handoff","status":"completed"}));
    std::fs::remove_file(fixture.path("notification-drain-hold")).unwrap();
    let delivered = wait_for_notification_count(&mut app, 2);
    assert_eq!(
        delivered.as_array().unwrap().len(),
        2,
        "arrival was stranded until a later transfer event"
    );
    let receipts: Value =
        serde_json::from_slice(&fixture.read("desktop-notification-receipts.v1.json")).unwrap();
    assert_eq!(receipts.as_array().unwrap().len(), 2);
    app.stop();
}

#[test]
fn desktop_notifications_tray_tracks_saved_and_runtime_language_without_a_transfer_event() {
    let fixture = Fixture::new();
    let saved = br#"{"settings":{"language":"es"}}"#;
    fixture.write("settings.json", saved);
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let mut job = notification_fixture_job("paused");
    job["status"] = json!("paused");
    app.ok(json!({"command":"save_transfer","job":job}));
    app.ok(json!({"command":"notifications_start"}));
    let tray = app.ok(json!({"command":"notification_tray"}));
    assert_eq!(tray["status"], "Telegram Drive — En pausa: 1");
    assert_eq!(tray["open"], "Abrir Telegram Drive");
    assert_eq!(tray["quit"], "Salir de Telegram Drive");
    assert_eq!(tray["resume_enabled"], true);
    assert_eq!(tray["pause_enabled"], false);
    app.ok(json!({"command":"notification_language","language":"ar"}));
    let tray = app.ok(json!({"command":"notification_tray"}));
    assert_eq!(tray["status"], "Telegram Drive — المتوقفة مؤقتًا: 1");
    assert_eq!(tray["quit"], "إنهاء Telegram Drive");
    app.failure(json!({"command":"notification_language","language":"arabic-invalid"}));
    assert_eq!(app.ok(json!({"command":"notification_tray"})), tray);
    assert_eq!(fixture.read("settings.json"), saved);
    assert_eq!(
        app.ok(json!({"command":"notification_deliveries"})),
        json!([])
    );
    app.stop();
}

fn await_notification_fixture_file(fixture: &Fixture, name: &str) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !fixture.path(name).exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        fixture.path(name).exists(),
        "notification fixture did not reach {name}"
    );
}

#[test]
fn desktop_notifications_tray_rejects_a_delayed_older_transfer_summary() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    for id in ["first", "newer"] {
        app.ok(json!({"command":"save_transfer","job":notification_fixture_job(id)}));
    }
    app.ok(json!({"command":"notifications_start"}));
    fixture.write("notification-tray-hold", b"hold");
    app.ok(json!({"command":"notification_transition","id":"first","status":"paused","background":true}));
    await_notification_fixture_file(&fixture, "notification-tray-entered");
    app.ok(json!({"command":"notification_transition","id":"newer","status":"paused"}));
    assert_eq!(
        app.ok(json!({"command":"notification_tray"}))["status"],
        "Telegram Drive — 2 paused"
    );
    std::fs::remove_file(fixture.path("notification-tray-hold")).unwrap();
    await_notification_fixture_file(&fixture, "notification-tray-returned");
    assert_eq!(
        app.ok(json!({"command":"notification_tray"}))["status"],
        "Telegram Drive — 2 paused",
        "older publication replaced the current transfer summary"
    );
    app.stop();
}

#[test]
fn desktop_notifications_serialize_receipt_writes_so_restart_cannot_replay_a_lost_newer_receipt() {
    let fixture = Fixture::new();
    fixture.write(
        "notification-preferences.json",
        br#"{"notificationsEnabled":true}"#,
    );
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    for id in ["first", "newer"] {
        app.ok(json!({"command":"save_transfer","job":notification_fixture_job(id)}));
    }
    app.ok(json!({"command":"notifications_start"}));
    fixture.write("notification-receipt-hold", b"hold");
    app.ok(json!({"command":"notification_transition","id":"first","status":"completed"}));
    await_notification_fixture_file(&fixture, "notification-receipt-entered");
    app.ok(json!({"command":"notification_transition","id":"newer","status":"completed"}));
    let deadline = Instant::now() + Duration::from_millis(1500);
    while !fixture.path("notification-receipt-second-written").exists() && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(10));
    }
    std::fs::remove_file(fixture.path("notification-receipt-hold")).unwrap();
    assert_eq!(
        wait_for_notification_count(&mut app, 2)
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let receipts: Value =
        serde_json::from_slice(&fixture.read("desktop-notification-receipts.v1.json")).unwrap();
    assert_eq!(
        receipts.as_array().unwrap().len(),
        2,
        "an older snapshot overwrote the newer durable receipt"
    );
    app.stop();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"notifications_start"}));
    for id in ["first", "newer"] {
        app.ok(json!({"command":"notification_replay","id":id}));
    }
    std::thread::sleep(Duration::from_millis(1100));
    assert_eq!(
        app.ok(json!({"command":"notification_deliveries"}))
            .as_array()
            .unwrap()
            .len(),
        2
    );
    app.stop();
}

#[test]
fn desktop_notifications_recheck_disabled_preferences_before_delivering_a_queued_outcome() {
    let fixture = Fixture::new();
    fixture.write(
        "notification-preferences.json",
        br#"{"notificationsEnabled":true}"#,
    );
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!({"command":"save_transfer","job":notification_fixture_job("queued")}));
    app.ok(json!({"command":"notifications_start"}));
    app.ok(json!({"command":"notification_transition","id":"queued","status":"completed"}));
    fixture.write(
        "notification-preferences.json",
        br#"{"notificationsEnabled":false}"#,
    );
    std::thread::sleep(Duration::from_millis(1100));
    assert_eq!(
        app.ok(json!({"command":"notification_deliveries"})),
        json!([])
    );
    assert!(!fixture
        .path("desktop-notification-receipts.v1.json")
        .exists());
    app.stop();
}

#[test]
fn desktop_notifications_preserve_aggregation_foreground_filename_privacy_and_sync_exclusion() {
    let fixture = Fixture::new();
    fixture.write(
        "notification-preferences.json",
        br#"{"notificationsEnabled":true}"#,
    );
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    for id in ["visible", "sync", "name", "batch-a", "batch-b"] {
        let mut job = notification_fixture_job(id);
        if id == "name" {
            job["filename"] = json!("/synthetic/private-folder/{{count}}-report.pdf\n\u{0000}");
        }
        app.ok(json!({"command":"save_transfer","job":job}));
    }
    app.ok(json!({"command":"notifications_start"}));
    fixture.write("notification-visible", b"visible");
    app.ok(json!({"command":"notification_transition","id":"visible","status":"completed"}));
    std::fs::remove_file(fixture.path("notification-visible")).unwrap();
    app.ok(json!({"command":"notification_transition","id":"sync","status":"completed","origin":"sync"}));
    std::thread::sleep(Duration::from_millis(1100));
    assert_eq!(
        app.ok(json!({"command":"notification_deliveries"})),
        json!([])
    );
    assert!(!fixture
        .path("desktop-notification-receipts.v1.json")
        .exists());
    fixture.write(
        "notification-preferences.json",
        br#"{"notificationsEnabled":true,"showFilenamesInNotifications":true}"#,
    );
    app.ok(json!({"command":"notification_transition","id":"name","status":"completed"}));
    let delivered = wait_for_notification_count(&mut app, 1);
    assert_eq!(delivered[0]["body"], "{{count}}-report.pdf completed.");
    assert!(!delivered.to_string().contains("private-folder"));
    assert!(!delivered[0]["body"]
        .as_str()
        .unwrap()
        .contains(['\n', '\0']));
    fixture.write(
        "notification-preferences.json",
        br#"{"notificationsEnabled":true,"showFilenamesInNotifications":false}"#,
    );
    for id in ["batch-a", "batch-b"] {
        app.ok(json!({"command":"notification_transition","id":id,"status":"completed"}));
    }
    let delivered = wait_for_notification_count(&mut app, 2);
    assert_eq!(delivered.as_array().unwrap().len(), 2);
    assert_eq!(delivered[1]["body"], "Completed transfers: 2.");
    assert!(!delivered.to_string().contains("secret-report"));
    let receipts: Value =
        serde_json::from_slice(&fixture.read("desktop-notification-receipts.v1.json")).unwrap();
    assert_eq!(receipts.as_array().unwrap().len(), 3);
    app.stop();
}

#[test]
fn desktop_notifications_render_all_native_catalogs_and_preserve_malformed_or_system_preferences() {
    let fixture = Fixture::new();
    fixture.write(
        "notification-preferences.json",
        br#"{"notificationsEnabled":true}"#,
    );
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let languages = [
        "en", "es", "ru", "uk-UA", "pl-PL", "fa-IR", "ur-PK", "ms-MY", "zh-CN", "zh-TW", "fr",
        "it", "ar", "pt-BR", "de", "hi", "bn-BD", "id", "fil-PH", "tr", "th-TH", "ja", "ko", "vi",
    ];
    for language in languages {
        app.ok(json!({"command":"save_transfer","job":notification_fixture_job(language)}));
    }
    for (index, language) in languages.into_iter().enumerate() {
        let saved = serde_json::to_vec(&json!({"settings":{"language":language}})).unwrap();
        fixture.write("settings.json", &saved);
        app.ok(json!({"command":"notifications_start"}));
        app.ok(json!({"command":"notification_transition","id":language,"status":"completed"}));
        let delivered = wait_for_notification_count(&mut app, index + 1);
        assert_eq!(delivered.as_array().unwrap().len(), index + 1);
        let catalog: Value = serde_json::from_slice(
            &std::fs::read(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join(format!("../src/i18n/locales/{language}.json")),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            delivered[index]["title"],
            catalog["native_notifications"]["title_completed"]
        );
        assert_eq!(
            delivered[index]["body"],
            catalog["native_notifications"]["upload_completed"]
        );
        assert_eq!(fixture.read("settings.json"), saved);
    }
    fixture.write("settings.json", b"{malformed");
    app.ok(json!({"command":"notifications_start","systemLanguage":"zh_Hant_HK"}));
    assert_eq!(
        app.ok(json!({"command":"notification_tray"}))["quit"],
        "結束Telegram Drive"
    );
    assert_eq!(fixture.read("settings.json"), b"{malformed");
    let saved = br#"{"settings":{"language":"system"}}"#;
    fixture.write("settings.json", saved);
    app.ok(json!({"command":"notifications_start","systemLanguage":"ur_PK"}));
    assert_eq!(
        app.ok(json!({"command":"notification_tray"}))["quit"],
        "Telegram Drive بند کریں"
    );
    assert_eq!(fixture.read("settings.json"), saved);
    app.stop();
}

#[test]
fn desktop_notifications_recheck_disable_after_a_blocked_durable_receipt_write() {
    let fixture = Fixture::new();
    fixture.write(
        "notification-preferences.json",
        br#"{"notificationsEnabled":true}"#,
    );
    fixture.write("notification-receipt-hold", b"hold");
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!({"command":"save_transfer","job":notification_fixture_job("held")}));
    app.ok(json!({"command":"notifications_start"}));
    app.ok(json!({"command":"notification_transition","id":"held","status":"completed"}));
    await_notification_fixture_file(&fixture, "notification-receipt-entered");
    fixture.write(
        "notification-preferences.json",
        br#"{"notificationsEnabled":false}"#,
    );
    std::fs::remove_file(fixture.path("notification-receipt-hold")).unwrap();
    await_notification_fixture_file(&fixture, "notification-flush-complete");
    assert_eq!(
        app.ok(json!({"command":"notification_deliveries"})),
        json!([]),
        "disabled preference was ignored after a slow receipt write"
    );
    let receipts: Value =
        serde_json::from_slice(&fixture.read("desktop-notification-receipts.v1.json")).unwrap();
    assert_eq!(receipts.as_array().unwrap().len(), 1);
    app.stop();
}

#[test]
fn desktop_notifications_remove_opted_out_filenames_after_a_blocked_durable_receipt_write() {
    let fixture = Fixture::new();
    fixture.write(
        "notification-preferences.json",
        br#"{"notificationsEnabled":true,"showFilenamesInNotifications":true}"#,
    );
    fixture.write("notification-receipt-hold", b"hold");
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!({"command":"save_transfer","job":notification_fixture_job("held")}));
    app.ok(json!({"command":"notifications_start"}));
    app.ok(json!({"command":"notification_transition","id":"held","status":"completed"}));
    await_notification_fixture_file(&fixture, "notification-receipt-entered");
    fixture.write(
        "notification-preferences.json",
        br#"{"notificationsEnabled":true,"showFilenamesInNotifications":false}"#,
    );
    std::fs::remove_file(fixture.path("notification-receipt-hold")).unwrap();
    let delivered = wait_for_notification_count(&mut app, 1);
    assert_eq!(delivered[0]["body"], "An upload completed.");
    assert!(!delivered.to_string().contains("secret-report"));
    app.stop();
}

#[test]
fn release_inventory_cold_restart_and_unchanged_cursor_cost_one_walk_without_lookups() {
    let fixture = Fixture::new();
    fixture.write("release-inventory.json", json!({"highwater":250,"rows":(1..=250).map(|id|json!({"id":id,"name":format!("Report {id}")})).collect::<Vec<_>>()} ).to_string());
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let read = json!({"command":"inventory_list","owner":"101","source":"release-inventory.json"});
    let first = app.ok(read.clone());
    assert_eq!(
        first["historyCalls"], 1,
        "cold listing must use one history call"
    );
    assert_eq!(
        first["lookupBatches"], 0,
        "cold history is already verified by Telegram"
    );
    app.stop();
    let mut app = Backend::start(&fixture.0);
    let restarted = app.ok(read);
    assert_eq!(restarted["historyCalls"], 1);
    assert_eq!(restarted["lookupBatches"], 0);
    app.stop();
}

#[test]
fn release_global_search_twenty_folders_reuses_generation_after_thirty_one_seconds() {
    let fixture = Fixture::new();
    fixture.write(
        "release-search.json",
        json!({"highwater":250,"rows":(1..=250).map(|id|json!({"id":id,"name":format!("Report {id}.pdf")})).collect::<Vec<_>>()}).to_string(),
    );
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let query = json!({"command":"search_inventory","owner":"101","source":"release-search.json","folders":(1..=20).collect::<Vec<_>>(),"query":{"query":"Report"}});
    let first = app.ok(query.clone());
    assert_eq!(first["total"], 5000);
    std::thread::sleep(std::time::Duration::from_secs(31));
    let second = app.ok(query);
    assert_eq!(second["indexId"], first["indexId"]);
    assert_eq!(second["historyCalls"], first["historyCalls"]);
    assert_eq!(second["lookupBatches"], first["lookupBatches"]);
    println!("twenty-folder global search first={first}; second={second}");
    app.stop();
}

#[test]
fn release_inventory_capacity_returns_newest_partial_listing() {
    let fixture = Fixture::new();
    fixture.write("release-capacity.json", json!({"highwater":50001,"rows":(1..=50001).map(|id|json!({"id":id,"name":format!("File {id}")})).collect::<Vec<_>>()} ).to_string());
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let result =
        app.ok(json!({"command":"inventory_list","owner":"101","source":"release-capacity.json"}));
    assert_eq!(result["rows"].as_array().unwrap().len(), 50000);
    assert_eq!(result["rows"][0]["id"], 50001);
    assert_eq!(result["rows"][49999]["id"], 2);
    assert_eq!(result["complete"], false);
    app.stop();
}

#[test]
fn release_public_folder_counters_refresh_transport_references_without_listing_churn() {
    let fixture = Fixture::new();
    let write = |row: Value| {
        fixture.write(
            "raw-inventory.json",
            json!({"highwater":1,"rows":[row]}).to_string(),
        )
    };
    write(json!({"id":1,"name":"Report.pdf","views":1,"reference":1}));
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let query = json!({"command":"inventory_raw_list","owner":"101","source":"raw-inventory.json"});
    let first = app.ok(query.clone());
    write(json!({"id":1,"name":"Report.pdf","views":20,"forwards":5,"reference":2}));
    let mut audit = query.clone();
    audit["audit"] = json!(true);
    let refreshed = app.ok(audit.clone());
    assert_eq!(
        refreshed["serial"], first["serial"],
        "counters and access references are not listing changes"
    );
    assert_eq!(refreshed["rows"][0]["reference"], json!([2]));
    assert_eq!(refreshed["rows"][0]["views"], 20);
    write(json!({"id":1,"name":"Report.pdf","caption":"Renamed","reference":3}));
    let renamed = app.ok(audit.clone());
    assert_ne!(renamed["serial"], first["serial"]);
    write(json!({"id":1,"name":"Report.pdf","caption":"Renamed","documentId":2,"reference":4}));
    assert_ne!(
        app.ok(audit)["serial"],
        renamed["serial"],
        "a replacement file invalidates media and search"
    );
    app.stop();
}

#[test]
fn release_targeted_peer_resolution_keeps_cache_readable_during_dialog_discovery() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!({"command":"peer_scan_seed"}));
    app.ok(json!({"command":"peer_scan_start","resolve":true}));
    assert_eq!(
        app.ok(json!({"command":"peer_scan_read"})),
        json!({"1":"existing"})
    );
    assert_eq!(app.ok(json!({"command":"peer_scan_finish"})), 1);
    assert_eq!(app.ok(json!({"command":"peer_scan_read"}))["3"], "complete");
    app.ok(json!({"command":"peer_scan_start","resolve":true,"fail":true,"target":4}));
    app.ok(json!({"command":"seed_account","owner":202}));
    assert!(app
        .failure(json!({"command":"peer_scan_finish"}))
        .contains("ACCOUNT_CHANGED"));
    app.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retained_session_release_keeps_http_alive_and_fences_account_replacement() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let seed = |name| {
        json!({
            "command":"api_seed_catalog", "folders":[{"id":null,"name":"Saved Messages"}],
            "complete":true, "files":[{"id":1,"folder":null,"name":name,"size":100,
                "created":"2026-10-01T00:00:00Z","mime":"application/pdf"}],
        })
    };
    app.ok(seed("Before.pdf"));
    let token = "e".repeat(64);
    let dav = app.ok(json!({"command":"start_webdav","token":token,
        "catalogFixture":true,"retainSession":true}));
    let url = format!(
        "{}/dav/{token}/Saved%20Messages/",
        dav["url"].as_str().unwrap()
    );
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    let propfind = reqwest::Method::from_bytes(b"PROPFIND").unwrap();
    let before = client
        .request(propfind.clone(), &url)
        .header("depth", "1")
        .send()
        .await
        .unwrap();
    assert_eq!(before.status(), 207);
    assert!(before.text().await.unwrap().contains("Before.pdf"));
    app.ok(json!({"command":"capture_account","owner":"101"}));
    #[cfg(windows)]
    {
        assert!(app
            .failure(json!({"command":"seed_account","owner":202}))
            .contains("os error 32"));
        assert_eq!(app.ok(json!({"command":"validate_account"})), true);
    }
    assert_eq!(app.ok(json!({"command":"release_retained_sessions"})), 1);
    assert_eq!(app.ok(json!({"command":"validate_account"})), true);
    app.ok(json!({"command":"seed_account","owner":202}));
    assert!(app
        .failure(json!({"command":"validate_account"}))
        .contains("ACCOUNT_CHANGED"));
    app.ok(seed("After.pdf"));
    let after = client
        .request(propfind, &url)
        .header("depth", "1")
        .send()
        .await
        .unwrap();
    assert_eq!(after.status(), 207);
    let body = after.text().await.unwrap();
    assert!(body.contains("After.pdf"));
    assert!(!body.contains("Before.pdf"));
    drop(client);
    app.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn release_partial_catalog_serves_rest_and_dav_without_pruning_known_older_files() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let seed = |start, end, complete| {
        json!(
            {
                "command":"api_seed_catalog",
                "folders":[
                    {
                        "id":null,
                        "name":"Saved Messages"
                    }
                ],
                "complete":complete,
                "files":(start..=end).map(|id|json!({
                    "id":id,
                    "folder":null,
                    "name":format!("File {id}.pdf"),
                    "size":100,
                    "created":"2026-10-01T00:00:00Z",
                    "mime":"application/pdf"
                })).collect::<Vec<_>>()
            }
        )
    };
    app.ok(seed(1, 2, true));
    let token = "d".repeat(64);
    let dav = app.ok(
        json!({"command":"start_webdav","token":token,"catalogFixture":true,"retainSession":true}),
    );
    let api = app.ok(json!({"command":"start_api","key":"partial-catalog-key"}));
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .unwrap();
    let url = format!(
        "{}/dav/{token}/Saved%20Messages/",
        dav["url"].as_str().unwrap()
    );
    let propfind = reqwest::Method::from_bytes(b"PROPFIND").unwrap();
    let first = client
        .request(propfind.clone(), &url)
        .header("depth", "1")
        .send()
        .await
        .unwrap();
    assert_eq!(first.status(), 207);
    assert!(first.text().await.unwrap().contains("File%201.pdf"));
    app.ok(seed(2, 50001, false));
    let response = client
        .get(format!(
            "{}/api/v1/files?per_page=100",
            api["url"].as_str().unwrap()
        ))
        .header("X-API-Key", "partial-catalog-key")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["complete"], false);
    assert_eq!(body["total"], 50000);
    let before = app.ok(json!({"command":"account_validation_status"}));
    let fallback_before = before["fallbackReads"].as_u64().unwrap();
    let validations_before = before["validations"].as_u64().unwrap();
    let body_started = Instant::now();
    let partial = client
        .request(propfind, &url)
        .header("depth", "1")
        .send()
        .await
        .unwrap();
    assert_eq!(partial.status(), 207);
    let body = partial.text().await;
    let after = app.ok(json!({"command":"account_validation_status"}));
    let fallback_after = after["fallbackReads"].as_u64().unwrap();
    let validations_after = after["validations"].as_u64().unwrap();
    eprintln!(
        "50k DAV body: {:.3}s, {} fallback SQLite session reads, {} account validations",
        body_started.elapsed().as_secs_f64(),
        fallback_after - fallback_before,
        validations_after - validations_before
    );
    let body = body.unwrap();
    assert_eq!(
        fallback_after, fallback_before,
        "A live runner retains its session connection"
    );
    assert!(
        validations_after - validations_before >= 100_002,
        "Every entry still validates its account"
    );
    assert!(body.contains("File%2050001.pdf"));
    assert!(
        body.contains("File%201.pdf"),
        "an incomplete DAV listing must retain an older known entry"
    );
    app.ok(json!({"command":"capture_account","owner":"101"}));
    #[cfg(windows)]
    {
        // Windows refuses replacing a database held by the disconnected runner.
        // Keep the HTTP server alive while explicitly closing that fixture session.
        assert!(app
            .failure(json!({"command":"seed_account","owner":202}))
            .contains("os error 32"));
        assert_eq!(app.ok(json!({"command":"validate_account"})), true);
        assert_eq!(app.ok(json!({"command":"release_retained_sessions"})), 1);
    }
    app.ok(json!({"command":"seed_account","owner":202}));
    app.failure(json!({"command":"validate_account"}));
    let switched = client
        .request(reqwest::Method::from_bytes(b"PROPFIND").unwrap(), &url)
        .header("depth", "1")
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        !switched.contains("File%201.pdf") && !switched.contains("File%2050001.pdf"),
        "The previous account must not be exposed after session replacement"
    );
    app.stop();
}

#[test]
fn release_global_search_over_the_row_bound_returns_partial_and_folder_discovery_changes_its_generation(
) {
    let fixture = Fixture::new();
    fixture.write("large-search.json", json!({"highwater":40000,"rows":(1..=40000).map(|id|json!({"id":id,"name":format!("Report {id}.pdf")})).collect::<Vec<_>>()} ).to_string());
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let query = json!({"command":"search_inventory","owner":"101","source":"large-search.json","folders":[1,2,3],"query":{"query":"Report","limit":10}});
    // These bulk fixture requests build up to the retained-row cap under load.
    // Completeness, counts and generation assertions below remain unchanged.
    const BULK_SEARCH_FIXTURE_WAIT: Duration = Duration::from_secs(300);
    let mut read = |query: Value, phase: &str| {
        let started = Instant::now();
        let result = app.ok_before(query, BULK_SEARCH_FIXTURE_WAIT);
        println!(
            "bulk search fixture phase={phase}; wallSeconds={}",
            started.elapsed().as_secs_f64()
        );
        result
    };
    let first = read(query.clone(), "cold");
    assert_eq!(first["complete"], false);
    assert!(first["total"].as_u64().unwrap() >= 40000);
    assert!(first["total"].as_u64().unwrap() < 80000);
    assert_eq!(read(query.clone(), "cached")["indexId"], first["indexId"]);
    let mut scoped = query.clone();
    scoped["query"]["folderKey"] = json!("3");
    let third = read(scoped, "scoped");
    assert_eq!(third["complete"], true);
    assert_eq!(third["total"], 40000);
    let mut changed = query;
    changed["folders"] = json!([3]);
    let reply = read(changed, "discovery");
    assert_ne!(reply["indexId"], first["indexId"]);
    assert_eq!(reply["complete"], true);
    assert_eq!(reply["total"], 40000);
    app.stop();
}

#[test]
fn release_forced_refresh_joining_a_cold_listing_performs_a_later_catchup() {
    let fixture = Fixture::new();
    fixture.write(
        "overlap.json",
        json!(
            {
                "highwater":1,
                "historyDelayMs":50,
                "rows":[
                    {
                        "id":1,
                        "name":"Before"
                    }
                ],
                "bootstrapRace":{
                    "highwater":2,
                    "rows":[
                        {
                            "id":1,
                            "name":"Before"
                        },
                        {
                            "id":2,
                            "name":"Uploaded"
                        }
                    ]
                }
            }
        )
        .to_string(),
    );
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let result = app
        .ok(json!({"command":"inventory_refresh_overlap","owner":"101","source":"overlap.json"}));
    assert_eq!(result["first"].as_array().unwrap().len(), 1);
    assert_eq!(result["refreshed"].as_array().unwrap().len(), 2);
    assert_eq!(result["historyCalls"], 2);
    assert_eq!(result["lookupBatches"], 0);
    app.stop();
}

#[test]
fn release_audit_rounds_bound_work_and_do_not_rewrite_an_unchanged_cursor() {
    let fixture = Fixture::new();
    fixture.write("audit-large.json", json!({"highwater":50000,"rows":(1..=50000).map(|id|json!({"id":id,"name":format!("File {id}")})).collect::<Vec<_>>()} ).to_string());
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let result = app.ok(json!({"command":"inventory_audit_rounds","owner":"101","large":"audit-large.json","durationMs":600}));
    assert_eq!(result["coldHistoryCalls"], 1);
    assert_eq!(result["coldLookupBatches"], 0);
    assert_eq!(result["writesChanged"], false);
    assert!(result["maxBatchesPerWindow"].as_u64().unwrap() <= 25);
    assert!(
        result["lookupBatches"].as_u64().unwrap() < 500,
        "a ten-minute accelerated window must not finish an unbounded 500-batch audit"
    );
    println!("accelerated ten-minute cost: {result}");
    app.stop();
}

#[test]
fn release_a_failing_folder_does_not_block_later_healthy_audit_rounds() {
    let fixture = Fixture::new();
    fixture.write(
        "audit-small.json",
        json!({"highwater":1,"rows":[{"id":1,"name":"Initial name"}]}).to_string(),
    );
    fixture.write(
        "audit-failure.json",
        json!({"highwater":1,"lookupError":true,"rows":[{"id":1,"name":"Unavailable"}]})
            .to_string(),
    );
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let result = app.ok(json!({"command":"inventory_audit_rounds","owner":"101","small":"audit-small.json","failure":"audit-failure.json","durationMs":1200,"editTwice":true}));
    assert_eq!(result["smallName"], "Second outside rename");
    assert!(result["maxBatchesPerWindow"].as_u64().unwrap() <= 25);
    assert!(result["events"].as_u64().unwrap() >= 1);
    app.stop();
}

#[test]
fn release_verified_uploads_and_renames_only_reconcile_the_destination_folder() {
    let fixture = Fixture::new();
    let rows = |highwater| json!({"highwater":highwater,"rows":(1..=highwater).map(|id|json!({"id":id,"name":format!("Report {id}")})).collect::<Vec<_>>()});
    fixture.write("scoped-cost.json", rows(250).to_string());
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let query = json!({"command":"search_inventory","owner":"101","source":"scoped-cost.json","folders":(1..=20).collect::<Vec<_>>(),"query":{"query":"Report"}});
    let first = app.ok(query.clone());
    assert_eq!(first["historyCalls"], 20);
    assert_eq!(first["lookupBatches"], 0);
    fixture.write("scoped-cost.json", rows(251).to_string());
    app.ok(json!({"command":"inventory_upload_published","owner":"101","folder":1}));
    let uploaded = app.ok(query.clone());
    assert_eq!(uploaded["total"], 5001);
    assert_eq!(uploaded["historyCalls"], 21);
    assert_eq!(uploaded["lookupBatches"], 0);
    let mut rename = rows(251);
    rename["rows"][0]["name"] = json!("Report renamed");
    fixture.write("scoped-cost.json", rename.to_string());
    app.ok(json!({"command":"listing_rename","owner":"101","folder":1,"name":"Report renamed"}));
    let renamed = app.ok(query);
    assert_eq!(renamed["historyCalls"], 22);
    assert_eq!(renamed["lookupBatches"], 1);
    let folder = app.ok(
        json!({"command":"inventory_list","owner":"101","folder":1,"source":"scoped-cost.json"}),
    );
    assert_eq!(folder["rows"][250]["name"], "Report renamed");
    app.stop();
}

#[test]
fn release_queued_old_peer_discovery_cannot_survive_cache_clear() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    let result = app.ok(json!({"command":"peer_queued_clear"}));
    assert_eq!(result["first"], true);
    assert_eq!(result["second"], true);
    assert_eq!(result["cache"], json!({}));
    app.stop();
}

#[test]
fn release_reordered_folder_discovery_preserves_index_and_storage() {
    let fixture = Fixture::new();
    fixture.write(
        "order.json",
        json!({"highwater":1,"rows":[{"id":1,"name":"Report"}]}).to_string(),
    );
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let query = json!({"command":"search_inventory","owner":"101","source":"order.json","folders":[1,2,3],"query":{"query":"Report"}});
    let first = app.ok(query.clone());
    let db = sqlite::open(fixture.path("workspace/101/workspace.db")).unwrap();
    let version = || {
        let mut q = db.prepare("PRAGMA data_version").unwrap();
        q.next().unwrap();
        q.read::<i64, _>(0).unwrap()
    };
    let before = version();
    let mut reordered = query;
    reordered["folders"] = json!([3, 1, 2, 1]);
    let second = app.ok(reordered);
    assert_eq!(second["indexId"], first["indexId"]);
    assert_eq!(second["historyCalls"], first["historyCalls"]);
    assert_eq!(version(), before);
    app.stop();
}

#[test]
fn release_recovered_large_audit_finishes_while_small_folder_keeps_polling() {
    let fixture = Fixture::new();
    fixture.write(
        "recovered-large.json",
        json!(
            {
                "highwater":5000,
                "lookupError":true,
                "rows":(1..=5000).map(|id|json!({
                    "id":id,
                    "name":format!("File {id}")
                })).collect::<Vec<_>>()
            }
        )
        .to_string(),
    );
    fixture.write(
        "recovered-small.json",
        json!({"highwater":1,"rows":[{"id":1,"name":"Small"}]}).to_string(),
    );
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let result = app.ok(json!(
        {
            "command":"inventory_audit_rounds",
            "owner":"101",
            "large":"recovered-large.json",
            "small":"recovered-small.json",
            "durationMs":2600,
            "recoverLarge":true
        }
    ));
    assert_eq!(result["large"]["last"], "Recovered last row");
    assert_eq!(result["large"]["complete"], true);
    assert!(result["maxBatchesPerWindow"].as_u64().unwrap() <= 25);
    app.stop();
}

#[test]
fn release_search_text_overflow_keeps_the_authenticated_fitting_prefix() {
    let fixture = Fixture::new();
    let name = format!("Report {}", "x".repeat(120_000));
    fixture.write("text-overflow.json", json!({"highwater":400,"rows":(1..=400).map(|id|json!({"id":id,"name":name})).collect::<Vec<_>>()} ).to_string());
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let reply = app.ok(json!({"command":"search_inventory","owner":"101","source":"text-overflow.json","folders":[1],"query":{"query":"Report","limit":1}}));
    assert_eq!(reply["complete"], false);
    assert!(reply["total"].as_u64().unwrap() > 100);
    assert!(reply["total"].as_u64().unwrap() < 400);
    app.stop();
}

#[test]
fn release_maximum_retained_audit_set_rechecks_every_id_within_one_hour() {
    let fixture = Fixture::new();
    fixture.write("bound-large.json", json!({"highwater":49000,"rows":(1..=49000).map(|id|json!({"id":id,"name":format!("Large {id}")})).collect::<Vec<_>>()} ).to_string());
    fixture.write("bound-small.json", json!({"highwater":100,"rows":(1..=100).map(|id|json!({"id":id,"name":format!("Small {id}")})).collect::<Vec<_>>()} ).to_string());
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    // One IPC request compresses 92 virtual minutes over 100,000 rows.
    // The 120s IPC wait expired under the sixteen-worker loaded full suite.
    // Virtual-time audit bounds and every per-ID/rate assertion remain unchanged.
    const COMPRESSED_AUDIT_DEADLINE: Duration = Duration::from_secs(300);
    let started = Instant::now();
    let result = app.ok_before(
        json!({"command":"inventory_detection_bound","owner":"101","large":"bound-large.json","small":"bound-small.json"}),
        COMPRESSED_AUDIT_DEADLINE,
    );
    println!(
        "maximum retained audit detection: {result}; wallSeconds={}",
        started.elapsed().as_secs_f64()
    );
    assert_eq!(result["seen"], 100000);
    assert!(result["repeated"].as_u64().unwrap() >= 100000);
    assert!(result["minimumChecks"].as_u64().unwrap() >= 1);
    assert!(result["minimumOriginalChecks"].as_u64().unwrap() >= 2);
    assert!(result["maxBatchesPerMinute"].as_u64().unwrap() <= 25);
    assert!(result["maxGapSeconds"].as_u64().unwrap() <= 3600);
    app.stop();
}

#[test]
fn release_ten_minutes_of_an_open_large_folder_has_bounded_cost_and_no_cursor_writes() {
    let fixture = Fixture::new();
    fixture.write("ten-minute-large.json", json!({"highwater":50000,"rows":(1..=50000).map(|id|json!({"id":id,"name":format!("File {id}")})).collect::<Vec<_>>()} ).to_string());
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let result = app.ok(json!({"command":"inventory_detection_bound","owner":"101","large":"ten-minute-large.json","largeFolders":1,"smallFolders":0,"ticks":20,"grow":false}));
    println!("ten-minute production interval cost: {result}");
    assert_eq!(result["coldHistoryCalls"], 1);
    assert_eq!(result["coldLookupBatches"], 0);
    assert_eq!(result["historyCalls"], 20);
    assert!(result["lookupBatches"].as_u64().unwrap() <= 175);
    assert_eq!(result["writesChanged"], false);
    assert!(result["maxBatchesPerMinute"].as_u64().unwrap() <= 25);
    app.stop();
}

#[test]
fn encryption_beta_derivation_keeps_vault_status_responsive() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"vault_create","passphrase":"synthetic initial passphrase"}));
    app.ok(json!({"command":"vault_lock"}));
    app.ok(json!({"command":"vault_prepare_start","id":"unlock","passphrase":"synthetic initial passphrase","held":true}));
    let status = app.ok(json!({"command":"vault_status_responsive"}));
    fixture.write("prepare-unlock.release", b"continue");
    app.ok(json!({"command":"vault_prepare_finish","id":"unlock"}));
    assert_eq!(status["responsive"], true);
    app.stop();
}

#[test]
fn encryption_beta_wrong_current_passphrase_cannot_change_an_unlocked_vault() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"vault_create","passphrase":"synthetic initial passphrase"}));
    let bytes = fixture.read("crypto.vault");
    let identity = app.ok(json!({"command":"vault_identity"}));
    app.failure(json!({"command":"vault_change_passphrase","currentPassphrase":"synthetic wrong passphrase","passphrase":"synthetic replacement passphrase"}));
    assert_eq!(fixture.read("crypto.vault"), bytes);
    assert_eq!(app.ok(json!({"command":"vault_identity"})), identity);
    app.stop();
}

#[test]
fn encryption_beta_committed_replacement_remains_unlocked_when_backup_cleanup_fails() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"vault_create","passphrase":"synthetic initial passphrase"}));
    let identity = app.ok(json!({"command":"vault_identity"}));
    app.ok(json!({"command":"vault_cleanup_fault","fail":true}));
    app.ok(json!({"command":"vault_change_passphrase","currentPassphrase":"synthetic initial passphrase","passphrase":"synthetic replacement passphrase"}));
    assert_eq!(app.ok(json!({"command":"vault_identity"})), identity);
    app.stop();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"vault_unlock","passphrase":"synthetic replacement passphrase"}));
    assert_eq!(app.ok(json!({"command":"vault_identity"})), identity);
    app.stop();
}

#[test]
fn encryption_beta_recovery_does_not_read_an_unused_corrupt_backup() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"vault_create","passphrase":"synthetic initial passphrase"}));
    app.stop();
    fixture.write("crypto.vault.bak", vec![0u8; 2 * 1024 * 1024]);
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"vault_unlock","passphrase":"synthetic initial passphrase"}));
    assert_ne!(app.ok(json!({"command":"vault_identity"})), Value::Null);
    app.stop();
}

#[test]
fn encryption_beta_recovery_export_and_verification_survive_disk_damage_while_unlocked() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"vault_create","passphrase":"synthetic initial passphrase"}));
    fixture.write("crypto.vault", vec![0u8; 2 * 1024 * 1024]);
    app.ok(json!({"command":"vault_export","passphrase":"synthetic recovery passphrase"}));
    let result = app.ok(
        json!({"command":"vault_verify_recovery","passphrase":"synthetic recovery passphrase"}),
    );
    assert_eq!(result["matches_vault_key"], true);
    app.stop();
}

#[test]
fn encryption_beta_recovery_can_archive_and_replace_oversized_corrupt_disk() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"vault_create","passphrase":"synthetic initial passphrase"}));
    let identity = app.ok(json!({"command":"vault_identity"}));
    app.ok(json!({"command":"vault_export","passphrase":"synthetic recovery passphrase"}));
    app.stop();
    let corrupt = vec![0u8; 2 * 1024 * 1024];
    fixture.write("crypto.vault", &corrupt);
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"vault_recover","passphrase":"synthetic recovery passphrase","vaultPassphrase":"synthetic restored passphrase","replaceExisting":true}));
    assert_eq!(app.ok(json!({"command":"vault_identity"})), identity);
    let archives = std::fs::read_dir(&fixture.0)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("crypto.vault.replaced-")
        })
        .collect::<Vec<_>>();
    assert_eq!(archives.len(), 1);
    assert_eq!(std::fs::read(&archives[0]).unwrap(), corrupt);
    app.stop();
}

fn wait_crypto_preparation(fixture: &Fixture, id: &str) {
    let path = fixture.path(&format!("prepare-{id}.finished"));
    let deadline = Instant::now() + Duration::from_secs(30);
    while !path.is_file() {
        assert!(
            Instant::now() < deadline,
            "Detached crypto preparation did not finish"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn encryption_beta_repeated_lock_fences_an_already_locked_unlock_preparation() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"vault_create","passphrase":"synthetic original passphrase"}));
    let before = fixture.read("crypto.vault");
    app.ok(json!({"command":"vault_lock"}));
    app.ok(json!({"command":"vault_prepare_start","id":"locked","passphrase":"synthetic original passphrase","held":true}));
    app.ok(json!({"command":"vault_lock"}));
    fixture.write("prepare-locked.release", b"continue");
    assert!(app
        .failure(json!({"command":"vault_prepare_finish","id":"locked"}))
        .contains("VAULT_CHANGED"));
    assert_eq!(app.ok(json!({"command":"vault_identity"})), Value::Null);
    assert_eq!(fixture.read("crypto.vault"), before);
    app.stop();
}

#[test]
fn encryption_beta_competing_unlocks_publish_only_the_current_ticket() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"vault_create","passphrase":"synthetic original passphrase"}));
    let identity = app.ok(json!({"command":"vault_identity"}));
    app.ok(json!({"command":"vault_lock"}));
    app.ok(json!({"command":"vault_prepare_start","id":"first","passphrase":"synthetic original passphrase","held":true}));
    app.ok(json!({"command":"vault_prepare_start","id":"second","passphrase":"synthetic original passphrase"}));
    app.ok(json!({"command":"vault_prepare_finish","id":"second"}));
    fixture.write("prepare-first.release", b"continue");
    assert!(app
        .failure(json!({"command":"vault_prepare_finish","id":"first"}))
        .contains("VAULT_CHANGED"));
    assert_eq!(app.ok(json!({"command":"vault_identity"})), identity);
    app.stop();
}

#[test]
fn encryption_beta_competing_changes_keep_the_winning_passphrase_and_keys() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"vault_create","passphrase":"synthetic original passphrase"}));
    let identity = app.ok(json!({"command":"vault_identity"}));
    app.ok(json!(
        {
            "command":"vault_prepare_start",
            "id":"change",
            "operation":"change",
            "currentPassphrase":"synthetic original passphrase",
            "passphrase":"synthetic losing passphrase",
            "held":true
        }
    ));
    app.ok(json!({"command":"vault_change_passphrase","currentPassphrase":"synthetic original passphrase","passphrase":"synthetic winning passphrase"}));
    let winning = fixture.read("crypto.vault");
    fixture.write("prepare-change.release", b"continue");
    let failure = app.failure(json!({"command":"vault_prepare_finish","id":"change"}));
    assert!(failure.contains("VAULT_CHANGED"), "{failure}");
    assert_eq!(fixture.read("crypto.vault"), winning);
    app.stop();
    let mut app = Backend::start(&fixture.0);
    app.failure(json!({"command":"vault_unlock","passphrase":"synthetic losing passphrase"}));
    app.ok(json!({"command":"vault_unlock","passphrase":"synthetic winning passphrase"}));
    assert_eq!(app.ok(json!({"command":"vault_identity"})), identity);
    app.stop();
}

#[test]
fn encryption_beta_profile_mutation_fences_prepared_recovery_import() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"vault_create","passphrase":"synthetic original passphrase"}));
    app.ok(json!({"command":"vault_export","passphrase":"synthetic recovery passphrase"}));
    app.ok(json!({"command":"vault_prepare_start","id":"import","operation":"import","passphrase":"synthetic recovery passphrase","held":true}));
    app.ok(json!({"command":"vault_save_profile","profile":"added-after-capture"}));
    let current = fixture.read("crypto.vault");
    fixture.write("prepare-import.release", b"continue");
    assert!(app
        .failure(json!({"command":"vault_prepare_finish","id":"import"}))
        .contains("VAULT_CHANGED"));
    assert_eq!(fixture.read("crypto.vault"), current);
    assert_eq!(
        app.ok(
            json!({"command":"vault_verify_recovery","passphrase":"synthetic recovery passphrase"})
        )["missing_profiles"],
        1
    );
    app.stop();
}

#[test]
fn encryption_beta_canceled_preparation_cannot_commit_after_its_worker_finishes() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"vault_create","passphrase":"synthetic original passphrase"}));
    let before = fixture.read("crypto.vault");
    let identity = app.ok(json!({"command":"vault_identity"}));
    app.ok(json!(
        {
            "command":"vault_prepare_start",
            "id":"canceled",
            "operation":"change",
            "currentPassphrase":"synthetic original passphrase",
            "passphrase":"synthetic replacement passphrase",
            "held":true
        }
    ));
    app.ok(json!({"command":"vault_prepare_abort","id":"canceled"}));
    fixture.write("prepare-canceled.release", b"continue");
    wait_crypto_preparation(&fixture, "canceled");
    assert_eq!(fixture.read("crypto.vault"), before);
    assert_eq!(app.ok(json!({"command":"vault_identity"})), identity);
    app.stop();
}

#[test]
fn encryption_beta_new_vault_creation_cannot_be_replaced_by_an_unconfirmed_import() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"vault_create","passphrase":"synthetic exported passphrase"}));
    app.ok(json!({"command":"vault_export","passphrase":"synthetic recovery passphrase"}));
    app.stop();
    std::fs::remove_file(fixture.path("crypto.vault")).unwrap();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!(
        {
            "command":"vault_prepare_start",
            "id":"new-import",
            "operation":"import",
            "passphrase":"synthetic recovery passphrase",
            "vaultPassphrase":"synthetic restored passphrase",
            "replaceExisting":false,
            "held":true
        }
    ));
    app.ok(json!({"command":"vault_create","passphrase":"synthetic winning passphrase"}));
    let winning = fixture.read("crypto.vault");
    fixture.write("prepare-new-import.release", b"continue");
    assert!(app
        .failure(json!({"command":"vault_prepare_finish","id":"new-import"}))
        .contains("VAULT_CHANGED"));
    assert_eq!(fixture.read("crypto.vault"), winning);
    assert!(app
        .failure(json!(
            {
                "command":"vault_recover",
                "passphrase":"synthetic recovery passphrase",
                "vaultPassphrase":"synthetic restored passphrase",
                "replaceExisting":false
            }
        ))
        .contains("RECOVERY_CONFIRMATION_REQUIRED"));
    app.stop();
}

#[test]
fn encryption_beta_disk_change_and_precommit_io_failure_preserve_live_material() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"vault_create","passphrase":"synthetic original passphrase"}));
    let before = fixture.read("crypto.vault");
    app.ok(json!({"command":"vault_lock"}));
    app.ok(json!({"command":"vault_prepare_start","id":"disk","passphrase":"synthetic original passphrase","held":true}));
    let mut changed = before.clone();
    let last = changed.len() - 1;
    changed[last] ^= 1;
    fixture.write("crypto.vault", &changed);
    fixture.write("prepare-disk.release", b"continue");
    assert!(app
        .failure(json!({"command":"vault_prepare_finish","id":"disk"}))
        .contains("VAULT_CHANGED"));
    assert_eq!(fixture.read("crypto.vault"), changed);
    assert_eq!(app.ok(json!({"command":"vault_identity"})), Value::Null);
    fixture.write("crypto.vault", &before);
    app.ok(json!({"command":"vault_unlock","passphrase":"synthetic original passphrase"}));
    let identity = app.ok(json!({"command":"vault_identity"}));
    std::fs::create_dir(fixture.path("crypto.vault.part")).unwrap();
    app.failure(json!({"command":"vault_change_passphrase","currentPassphrase":"synthetic original passphrase","passphrase":"synthetic replacement passphrase"}));
    assert_eq!(fixture.read("crypto.vault"), before);
    assert_eq!(app.ok(json!({"command":"vault_identity"})), identity);
    app.stop();
}

#[test]
fn encryption_beta_file_passphrase_upload_resumes_and_decrypts_without_a_vault() {
    let fixture = Fixture::new();
    let source = synthetic_bytes(LARGE_UPLOAD_BYTES, 39);
    fixture.write("file-passphrase.bin", &source);
    settle(&fixture.path("file-passphrase.bin"));
    let request = |fail| {
        json!(
            {
                "command":"upload_resumable",
                "source":"file-passphrase.bin",
                "sink":"file-key-parts",
                "protected":true,
                "protectionMode":"passphrase",
                "filePassphrase":"synthetic file passphrase",
                "failAtPart":fail,
                "publish":false
            }
        )
    };
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"database"}));
    app.ok(json!({"command":"seed_account","owner":101}));
    assert!(app.failure(request(Some(7))).contains("connection reset"));
    let interrupted =
        app.ok(json!({"command":"upload_session","source":"file-passphrase.bin","protected":true}));
    app.stop();
    let mut app = Backend::start(&fixture.0);
    let finished = app.ok(request(None));
    assert_eq!(finished["continuedEnvelope"], true);
    assert_eq!(finished["fileId"], interrupted["fileId"]);
    app.ok(json!({"command":"upload_assemble","sink":"file-key-parts","fileId":finished["fileId"],"totalParts":finished["totalParts"],"destination":"file-key.tdenc"}));
    app.failure(json!(
        {
            "command":"read_envelope",
            "source":"file-key.tdenc",
            "destination":"rejected.bin",
            "filePassphrase":"synthetic wrong passphrase"
        }
    ));
    assert!(!fixture.path("rejected.bin").exists());
    app.ok(json!(
        {
            "command":"read_envelope",
            "source":"file-key.tdenc",
            "destination":"opened.bin",
            "filePassphrase":"synthetic file passphrase"
        }
    ));
    assert_eq!(fixture.read("opened.bin"), source);
    app.stop();
}

#[test]
fn tdenc2_known_answers_survive_restart_and_recovery() {
    let fixture = Fixture::new();
    let vectors = Path::new(env!("CARGO_MANIFEST_DIR")).join("test-support/crypto-vectors");
    fixture.write(
        "crypto.vault",
        std::fs::read(vectors.join("v3.9.8-vault.bin")).unwrap(),
    );
    fixture.write(
        "recovery.bundle",
        std::fs::read(vectors.join("v3.9.8-recovery.bin")).unwrap(),
    );
    let plaintext: Vec<u8> = (0..1_048_593).map(|n| (n % 251) as u8).collect();
    fixture.write("fixed-answer.bin", &plaintext);
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"vault_unlock","passphrase":"public v3.9.8 vault fixture passphrase"}));
    for (sync, golden) in [
        (false, "v3.9.8-envelope.tdenc"),
        (true, "current-envelope.tdenc"),
    ] {
        app.ok(
            json!({"command":"envelope_known_answer","syncPath":sync,"destination":"answer.tdenc"}),
        );
        let expected = std::fs::read(vectors.join(golden)).unwrap();
        assert_eq!(fixture.read("answer.tdenc"), expected);
        app.ok(json!({"command":"envelope_known_answer","syncPath":sync,"continue":true,"destination":"continued.tdenc"}));
        assert_eq!(fixture.read("continued.tdenc"), expected);
        app.ok(
            json!({"command":"read_envelope","source":"answer.tdenc","destination":"decoded.bin"}),
        );
        assert_eq!(fixture.read("decoded.bin"), plaintext);
    }
    app.ok(json!({"command":"vault_verify_recovery","passphrase":"public v3.9.8 recovery fixture passphrase"}));
    app.ok(json!({"command":"vault_recover","passphrase":"public v3.9.8 recovery fixture passphrase","vaultPassphrase":"public replacement fixture passphrase"}));
    drop(app);
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"vault_unlock","passphrase":"public replacement fixture passphrase"}));
    app.ok(json!({"command":"read_envelope","source":"answer.tdenc","destination":"after-recovery.bin"}));
    assert_eq!(fixture.read("after-recovery.bin"), plaintext);
}

#[test]
fn tdenc2_golden_corruption_never_publishes_plaintext() {
    let fixture = Fixture::new();
    let vectors = Path::new(env!("CARGO_MANIFEST_DIR")).join("test-support/crypto-vectors");
    fixture.write(
        "crypto.vault",
        std::fs::read(vectors.join("v3.9.8-vault.bin")).unwrap(),
    );
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"vault_unlock","passphrase":"public v3.9.8 vault fixture passphrase"}));
    for golden in ["v3.9.8-envelope.tdenc", "current-envelope.tdenc"] {
        let bytes = std::fs::read(vectors.join(golden)).unwrap();
        let header = u32::from_le_bytes(bytes[26..30].try_into().unwrap()) as usize;
        let mut variants = vec![
            bytes[..20].to_vec(),
            bytes[..header - 1].to_vec(),
            bytes[..bytes.len() - 1].to_vec(),
        ];
        for offset in [8, 66, 150, header - 1, header + 9, bytes.len() - 9] {
            let mut corrupt = bytes.clone();
            corrupt[offset] ^= 1;
            variants.push(corrupt);
        }
        let split = header + 1_048_576 + 16;
        let end = bytes.len() - 68;
        let mut reordered = bytes[..header].to_vec();
        reordered.extend_from_slice(&bytes[split..end]);
        reordered.extend_from_slice(&bytes[header..split]);
        reordered.extend_from_slice(&bytes[end..]);
        variants.push(reordered);
        let mut trailing = bytes.clone();
        trailing.push(0);
        variants.push(trailing);
        for (index, corrupt) in variants.iter().enumerate() {
            fixture.write("corrupt.tdenc", corrupt);
            let destination = format!("rejected-{index}.bin");
            app.failure(json!({"command":"read_envelope","source":"corrupt.tdenc","destination":destination}));
            assert!(!fixture.path(&destination).exists());
        }
    }
    drop(app);
    let wrong = Fixture::new();
    wrong.write(
        "golden.tdenc",
        std::fs::read(vectors.join("v3.9.8-envelope.tdenc")).unwrap(),
    );
    let mut other = Backend::start(&wrong.0);
    other.ok(
        json!({"command":"vault_create","passphrase":"public unrelated key fixture passphrase"}),
    );
    other.failure(
        json!({"command":"read_envelope","source":"golden.tdenc","destination":"rejected.bin"}),
    );
    assert!(!wrong.path("rejected.bin").exists());
}

#[test]
fn release_small_storage_insights_include_older_inventory_and_reuse_generations() {
    let fixture = Fixture::new();
    fixture.write("inventory.json", serde_json::to_vec(&json!({"rows":(1..=600).map(|id| json!({"id":id,"name":format!("older-file-{id}.pdf"),"text":"","raw":0})).collect::<Vec<_>>() })).unwrap());
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    for view in ["large", "duplicates", "old"] {
        let reply = app.ok(json!({"command":"storage_insight_inventory","owner":"101","folders":[1,2],"source":"inventory.json","view":view}));
        assert_eq!(reply["scanned_count"], 1200);
        assert_eq!(reply["complete"], true);
        assert_eq!(reply["historyCalls"], 2);
        assert_eq!(reply["lookupBatches"], 0);
        if view == "large" {
            assert_eq!(reply["files"].as_array().unwrap().len(), 1000);
        }
        if view == "duplicates" {
            assert_eq!(reply["duplicate_groups"], 600);
        }
    }
}

fn reuse_download() -> (Fixture, Backend) {
    let fixture = Fixture::new();
    fixture.write("original.part", b"verified file bytes");
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!({"command":"capture_account","owner":"101"}));
    app.ok(json!({"command":"publish_download","source":"original.part","destination":"verified.bin","policy":"replace"}));
    (fixture, app)
}
fn opening_hash_bytes(app: &mut Backend, path: &str, success: bool) -> u64 {
    let before = app.ok(json!({"command":"external_file_status"}))[1]
        .as_u64()
        .unwrap();
    let reply = app.request(json!({"command":"external_file_open","path":path}));
    assert_eq!(reply["ok"], success, "{reply}");
    app.ok(json!({"command":"external_file_status"}))[1]
        .as_u64()
        .unwrap()
        - before
}
fn reuse_hash_bytes(app: &mut Backend, path: &str) -> u64 {
    let status = app.ok(json!({"command":"external_file_identity_status","path":path}));
    if status["eligible"] == true {
        0
    } else {
        status["bytes"].as_u64().unwrap()
    }
}

#[test]
fn hash_reuse_unchanged_second_open_and_restart_dispatch_without_reading_contents() {
    let (fixture, mut app) = reuse_download();
    app.ok(json!({"command":"external_file_open","path":"verified.bin"}));
    let opened = fixture.read("opened-path");
    assert_eq!(
        opening_hash_bytes(&mut app, "verified.bin", true),
        reuse_hash_bytes(&mut app, "verified.bin")
    );
    assert_eq!(fixture.read("opened-path"), opened);
    drop(app);
    let mut app = Backend::start(&fixture.0);
    assert_eq!(
        opening_hash_bytes(&mut app, "verified.bin", true),
        reuse_hash_bytes(&mut app, "verified.bin")
    );
    assert_eq!(fixture.read("opened-path"), opened);
}
#[test]
fn hash_reuse_same_size_rewrite_with_restored_mtime_is_hashed_and_refused() {
    let (fixture, mut app) = reuse_download();
    let path = fixture.path("verified.bin");
    let before = std::fs::metadata(&path).unwrap().modified().unwrap();
    std::thread::sleep(Duration::from_millis(5));
    std::fs::write(&path, vec![b'x'; b"verified file bytes".len()]).unwrap();
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(before))
        .unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().modified().unwrap(),
        before
    );
    assert_eq!(
        opening_hash_bytes(&mut app, "verified.bin", false),
        b"verified file bytes".len() as u64
    );
    assert!(!fixture.path("opened-path").exists());
}
#[test]
fn hash_reuse_rename_replacement_with_different_bytes_is_hashed_and_refused() {
    let (fixture, mut app) = reuse_download();
    fixture.write("replacement.bin", vec![b'x'; b"verified file bytes".len()]);
    std::fs::rename(
        fixture.path("replacement.bin"),
        fixture.path("verified.bin"),
    )
    .unwrap();
    assert_eq!(
        opening_hash_bytes(&mut app, "verified.bin", false),
        b"verified file bytes".len() as u64
    );
    assert!(!fixture.path("opened-path").exists());
}
#[cfg(unix)]
#[test]
fn hash_reuse_symlink_substitution_remains_refused_before_hashing() {
    let (fixture, mut app) = reuse_download();
    fixture.write("planted.bin", b"verified file bytes");
    std::fs::remove_file(fixture.path("verified.bin")).unwrap();
    std::os::unix::fs::symlink(fixture.path("planted.bin"), fixture.path("verified.bin")).unwrap();
    assert_eq!(opening_hash_bytes(&mut app, "verified.bin", false), 0);
    assert!(!fixture.path("opened-path").exists());
}
#[test]
fn hash_reuse_changed_identity_matching_contents_refreshes_then_skips_the_hash() {
    let (fixture, mut app) = reuse_download();
    fixture.write("replacement.bin", b"verified file bytes");
    std::fs::rename(
        fixture.path("replacement.bin"),
        fixture.path("verified.bin"),
    )
    .unwrap();
    assert_eq!(
        opening_hash_bytes(&mut app, "verified.bin", true),
        b"verified file bytes".len() as u64
    );
    assert_eq!(
        opening_hash_bytes(&mut app, "verified.bin", true),
        reuse_hash_bytes(&mut app, "verified.bin")
    );
    drop(app);
    let mut app = Backend::start(&fixture.0);
    assert_eq!(
        opening_hash_bytes(&mut app, "verified.bin", true),
        reuse_hash_bytes(&mut app, "verified.bin")
    );
}
#[test]
fn hash_reuse_account_switch_away_and_back_requires_verification_again() {
    let (_fixture, mut app) = reuse_download();
    app.ok(json!({"command":"external_file_open","path":"verified.bin"}));
    app.ok(json!({"command":"seed_account","owner":202}));
    opening_hash_bytes(&mut app, "verified.bin", false);
    app.ok(json!({"command":"seed_account","owner":101}));
    assert_eq!(
        opening_hash_bytes(&mut app, "verified.bin", true),
        b"verified file bytes".len() as u64
    );
    assert_eq!(
        opening_hash_bytes(&mut app, "verified.bin", true),
        reuse_hash_bytes(&mut app, "verified.bin")
    );
}
#[test]
fn hash_reuse_registered_legacy_cache_rechecks_protection_and_removal() {
    for change in ["protection", "removal"] {
        let fixture = Fixture::new();
        fixture.write("legacy.bin", b"verified legacy file");
        let mut app = Backend::start(&fixture.0);
        app.ok(json!({"command":"seed_account","owner":101}));
        let file = app.ok(json!({"command":"legacy_external_seed","owner":"101","source":"legacy.bin","category":"offline"}));
        app.ok(json!({"command":"external_file_open","path":file["path"]}));
        std::fs::remove_file(fixture.path("opened-path")).unwrap();
        if change == "protection" {
            app.ok(json!({"command":"legacy_external_seed","owner":"101","source":"legacy.bin","updateOnly":true,"protection":"vault"}));
        } else {
            app.ok(json!({"command":"workspace_seed_files","owner":"101","files":[],"hidden":["saved:1"]}));
        }
        opening_hash_bytes(&mut app, file["path"].as_str().unwrap(), false);
        assert!(!fixture.path("opened-path").exists());
    }
}
#[test]
fn hash_reuse_clear_cache_invalidates_a_pinned_survivor() {
    let fixture = Fixture::new();
    fixture.write("source.bin", b"cached preview bytes");
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let file = app.ok(json!({"command":"asset_read","owner":"101","source":"source.bin"}));
    let path = PathBuf::from(file["path"].as_str().unwrap());
    let relative = path
        .strip_prefix(fixture.0.canonicalize().unwrap())
        .unwrap()
        .to_string_lossy();
    app.ok(json!({"command":"asset_pin","owner":"101","pinned":true}));
    app.ok(json!({"command":"external_file_open","path":relative}));
    assert_eq!(
        opening_hash_bytes(&mut app, &relative, true),
        reuse_hash_bytes(&mut app, &relative)
    );
    app.ok(json!({"command":"asset_clear","owner":"101","category":"previews"}));
    assert!(path.is_file());
    assert_eq!(
        opening_hash_bytes(&mut app, &relative, true),
        b"cached preview bytes".len() as u64
    );
    assert_eq!(
        opening_hash_bytes(&mut app, &relative, true),
        reuse_hash_bytes(&mut app, &relative)
    );
}
#[test]
fn hash_reuse_legacy_first_open_hashes_once_then_reuses() {
    let fixture = Fixture::new();
    fixture.write("legacy.bin", b"verified legacy file");
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let file = app.ok(json!({"command":"legacy_external_seed","owner":"101","source":"legacy.bin","category":"offline"}));
    assert_eq!(
        opening_hash_bytes(&mut app, file["path"].as_str().unwrap(), true),
        b"verified legacy file".len() as u64
            + reuse_hash_bytes(&mut app, file["path"].as_str().unwrap())
    );
    assert_eq!(
        opening_hash_bytes(&mut app, file["path"].as_str().unwrap(), true),
        reuse_hash_bytes(&mut app, file["path"].as_str().unwrap())
    );
}
#[test]
fn hash_reuse_unstable_identity_seam_hashes_every_open() {
    let (_fixture, mut app) = reuse_download();
    app.ok(json!({"command":"external_file_identity_mode","path":"verified.bin","unstable":true}));
    for _ in 0..2 {
        assert_eq!(
            opening_hash_bytes(&mut app, "verified.bin", true),
            b"verified file bytes".len() as u64
        );
    }
}

#[test]
fn hash_reuse_old_registration_without_identity_or_proof_is_verified_before_upgrade() {
    for cached in [false, true] {
        let (fixture, mut app, path, bytes) = if cached {
            let fixture = Fixture::new();
            fixture.write("source.bin", b"cached preview bytes");
            let mut app = Backend::start(&fixture.0);
            app.ok(json!({"command":"seed_account","owner":101}));
            let file = app.ok(json!({"command":"asset_read","owner":"101","source":"source.bin"}));
            let path = PathBuf::from(file["path"].as_str().unwrap())
                .strip_prefix(fixture.0.canonicalize().unwrap())
                .unwrap()
                .to_string_lossy()
                .into_owned();
            (fixture, app, path, b"cached preview bytes".len() as u64)
        } else {
            let (fixture, app) = reuse_download();
            (
                fixture,
                app,
                "verified.bin".into(),
                b"verified file bytes".len() as u64,
            )
        };
        app.ok(json!({"command":"external_file_old_registration","path":path}));
        drop(app);
        let mut app = Backend::start(&fixture.0);
        assert_eq!(opening_hash_bytes(&mut app, &path, true), bytes);
        assert_eq!(
            opening_hash_bytes(&mut app, &path, true),
            reuse_hash_bytes(&mut app, &path)
        );
    }
}
#[test]
fn hash_reuse_handoff_rechecks_cache_epoch_and_protection_after_verification() {
    for change in ["clear", "protection"] {
        let fixture = Fixture::new();
        fixture.write("legacy.bin", b"verified legacy file");
        let mut app = Backend::start(&fixture.0);
        app.ok(json!({"command":"seed_account","owner":101}));
        let file = app.ok(json!({"command":"legacy_external_seed","owner":"101","source":"legacy.bin","category":"offline"}));
        app.ok(json!({"command":"external_file_open","path":file["path"]}));
        app.ok(json!({"command":"external_file_start","id":"handoff","path":file["path"],"legacy":true,"postHash":true}));
        if change == "clear" {
            app.ok(json!({"command":"asset_clear","owner":"101","category":"previews"}));
        } else {
            app.ok(json!({"command":"legacy_external_seed","owner":"101","source":"legacy.bin","updateOnly":true,"protection":"vault"}));
        }
        fixture.write("file-handoff.release", b"go");
        app.failure(json!({"command":"vault_prepare_finish","id":"handoff"}));
    }
}

#[cfg(unix)]
#[test]
fn hash_reuse_aliased_cache_root_cannot_bypass_old_registration_protection() {
    let fixture = Fixture::new();
    fixture.write("legacy.bin", b"verified legacy file");
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let file =
        app.ok(json!({"command":"legacy_external_seed","owner":"101","source":"legacy.bin"}));
    app.ok(json!({"command":"external_file_open","path":file["path"]}));
    app.ok(json!({"command":"external_file_old_registration","path":file["path"]}));
    app.ok(json!({"command":"legacy_external_seed","owner":"101","source":"legacy.bin","updateOnly":true,"protection":"vault"}));
    std::os::unix::fs::symlink(fixture.0.canonicalize().unwrap(), fixture.path("alias")).unwrap();
    std::fs::remove_file(fixture.path("opened-path")).unwrap();
    app.failure(json!({"command":"external_file_open","path":file["path"],"cacheRoot":"alias"}));
    assert!(!fixture.path("opened-path").exists());
}
#[test]
fn hash_reuse_heic_decoder_input_keeps_full_verification() {
    let (fixture, mut app) = heic_fixture();
    fixture.write("heic-mode", "wait");
    let mut request = heic_request();
    request["command"] = json!("asset_start");
    request["display"] = json!(true);
    app.ok(request);
    wait_heic_process(&fixture);
    let hashed = app.ok(json!({"command":"external_file_status"}))[1]
        .as_u64()
        .unwrap();
    let size = fixture.read("synthetic.heic").len() as u64;
    app.ok(json!({"command":"asset_abort"}));
    app.ok(json!({"command":"asset_clear","owner":"101","category":"previews"}));
    assert!(
        hashed >= 2 * size,
        "Production and pre-decode verification must both hash: {hashed}/{size}"
    );
}

#[test]
fn release_small_external_open_requires_an_app_produced_file_and_survives_restart() {
    let fixture = Fixture::new();
    fixture.write("arbitrary.txt", b"not produced by app");
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!({"command":"capture_account","owner":"101"}));
    app.failure(json!({"command":"external_file_open","path":"arbitrary.txt"}));
    assert!(!fixture.path("opened-path").exists());
    fixture.write("partial.bin", b"verified download");
    app.ok(json!({"command":"publish_download","source":"partial.bin","destination":"download.bin","policy":"replace"}));
    app.ok(json!({"command":"external_file_open","path":"download.bin"}));
    drop(app);
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"external_file_open","path":"download.bin"}));
    fixture.write("download.bin", b"substituted different bytes");
    app.failure(json!({"command":"external_file_open","path":"download.bin"}));
    #[cfg(unix)]
    {
        std::fs::remove_file(fixture.path("download.bin")).unwrap();
        std::os::unix::fs::symlink(fixture.path("arbitrary.txt"), fixture.path("download.bin"))
            .unwrap();
        app.failure(json!({"command":"external_file_open","path":"download.bin"}));
    }
}

#[test]
fn release_small_proxy_rpc_response_proves_transport_but_disconnect_does_not() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let worker = std::thread::spawn(move || {
        for body in ["{\"rpc\":\"API_ID_INVALID\"}", "not-json"] {
            let (mut socket, _) = listener.accept().unwrap();
            let mut bytes = [0; 2048];
            std::io::Read::read(&mut socket, &mut bytes).unwrap();
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        }
    });
    let reply = app
        .ok(json!({"command":"proxy_transport_fixture","url":format!("http://{address}/probe")}));
    assert_eq!(reply["transportWorks"], true);
    let malformed = app
        .ok(json!({"command":"proxy_transport_fixture","url":format!("http://{address}/probe")}));
    assert_eq!(malformed["transportWorks"], false);
    worker.join().unwrap();
    let disconnected = app
        .ok(json!({"command":"proxy_transport_fixture","url":format!("http://{address}/probe")}));
    assert_eq!(disconnected["transportWorks"], false);
}

#[test]
fn release_external_modified_cache_is_refused_without_deleting_bytes() {
    let fixture = Fixture::new();
    fixture.write("source.bin", b"authentic cached preview");
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let request =
        json!({"command":"asset_read","owner":"101","source":"source.bin","thumbnail":false});
    let first = app.ok(request.clone());
    let path = PathBuf::from(first["path"].as_str().unwrap());
    std::fs::write(&path, vec![b'x'; b"authentic cached preview".len()]).unwrap();
    assert!(app.failure(request).contains("FILE_OPEN_REFUSED"));
    assert!(path.is_file(), "Rejected cache bytes must not be deleted");
}

#[test]
fn release_external_large_file_workers_leave_runtime_responsive_and_bound_reads() {
    let fixture = Fixture::new();
    fixture.write("large.part", vec![7u8; 32 * 1024 * 1024]);
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!({"command":"capture_account","owner":"101"}));
    app.ok(json!({"command":"publish_download","source":"large.part","destination":"large.bin","policy":"replace"}));
    for id in ["first", "second"] {
        app.ok(json!({"command":"external_file_start","id":id,"path":"large.bin"}));
    }
    let status = app.ok(json!({"command":"external_file_status"}));
    assert_eq!(status[0], 0);
    assert_eq!(status[1], 32 * 1024 * 1024);
    assert_eq!(
        app.ok(json!({"command":"vault_status_responsive"}))["responsive"],
        true
    );
    fixture.write("file-first.release", b"release");
    fixture.write("file-second.release", b"release");
    for id in ["first", "second"] {
        app.ok(json!({"command":"vault_prepare_finish","id":id}));
    }
    let status = app.ok(json!({"command":"external_file_status"}));
    assert_eq!(status[0], 2);
    assert_eq!(status[1], 96 * 1024 * 1024);
}

#[test]
fn release_external_legacy_kept_cache_remains_available_without_deleting_bytes() {
    for offline in [true, false] {
        let fixture = Fixture::new();
        fixture.write("source.bin", b"kept legacy cache must survive upgrade");
        let mut app = Backend::start(&fixture.0);
        app.ok(json!({"command":"seed_account","owner":101}));
        let request =
            json!({"command":"asset_read","owner":"101","source":"source.bin","thumbnail":false});
        let first = app.ok(request.clone());
        let path = PathBuf::from(first["path"].as_str().unwrap());
        let relative = path
            .strip_prefix(fixture.0.canonicalize().unwrap())
            .unwrap()
            .to_string_lossy()
            .to_string();
        app.ok(json!({"command":"asset_pin","owner":"101","pinned":true}));
        app.ok(json!({"command":"external_file_forget","path":relative}));
        if !offline {
            app.ok(json!({"command":"asset_limits","previews":1,"thumbnails":10000}));
        }
        let second = app.ok(json!({"command":"asset_read","owner":"101","source":"source.bin","thumbnail":false,"noSource":offline}));
        assert_eq!(second["path"], first["path"]);
        assert_eq!(second["downloads"], first["downloads"]);
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"kept legacy cache must survive upgrade"
        );
        app.failure(json!({"command":"external_file_open","path":relative}));
    }
}

#[test]
fn release_external_post_hash_replacement_is_refused_at_launch() {
    let fixture = Fixture::new();
    fixture.write("authentic.part", b"authenticated app-produced bytes");
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!({"command":"capture_account","owner":"101"}));
    app.ok(json!({"command":"publish_download","source":"authentic.part","destination":"download.bin","policy":"replace"}));
    app.ok(json!({"command":"external_file_start","id":"replaced","path":"download.bin","postHash":true}));
    fixture.write("replacement.bin", b"unverified replacement bytes");
    std::fs::rename(
        fixture.path("replacement.bin"),
        fixture.path("download.bin"),
    )
    .unwrap();
    fixture.write("file-replaced.release", b"release");
    assert!(app
        .failure(json!({"command":"vault_prepare_finish","id":"replaced"}))
        .contains("FILE_OPEN_REFUSED"));
    assert!(!fixture.path("opened-path").exists());
}

#[test]
fn release_external_post_hash_cache_replacement_cannot_inherit_registration() {
    let fixture = Fixture::new();
    fixture.write("authentic.part", b"authenticated app-produced bytes");
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!({"command":"capture_account","owner":"101"}));
    app.ok(json!({"command":"publish_download","source":"authentic.part","destination":"download.bin","policy":"replace"}));
    app.ok(json!({"command":"external_file_start","id":"replaced","path":"download.bin","postHash":true,"reuseCache":true}));
    fixture.write("replacement.bin", b"unauthenticated substituted file");
    assert_eq!(
        fixture.read("replacement.bin").len(),
        fixture.read("download.bin").len()
    );
    std::fs::rename(
        fixture.path("replacement.bin"),
        fixture.path("download.bin"),
    )
    .unwrap();
    fixture.write("file-replaced.release", b"release");
    assert!(app
        .failure(json!({"command":"vault_prepare_finish","id":"replaced"}))
        .contains("FILE_OPEN_REFUSED"));
    assert_eq!(
        fixture.read("download.bin"),
        b"unauthenticated substituted file"
    );
    app.failure(json!({"command":"external_file_open","path":"download.bin"}));
}

#[test]
fn legacy_external_upgrade_records_restore_previews_thumbnails_and_offline_opening() {
    let fixture = Fixture::new();
    fixture.write("legacy.bin", b"historical app cache contents");
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let mut paths = Vec::new();
    for (id, category, current) in [
        (1, "previews", false),
        (2, "thumbnails", false),
        (3, "offline", false),
        (4, "previews", true),
    ] {
        let record = app.ok(json!({"command":"legacy_external_seed","owner":"101","source":"legacy.bin","id":id,"category":category,"current":current}));
        paths.push(record["path"].as_str().unwrap().to_string());
    }
    drop(app);
    let mut app = Backend::start(&fixture.0);
    for path in &paths {
        app.ok(json!({"command":"external_file_open","path":path}));
        assert_eq!(
            fixture.read("opened-path"),
            fixture
                .path(path)
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .as_bytes()
        );
    }
    assert_eq!(
        app.ok(json!({"command":"legacy_external_status"}))["migrated"],
        4
    );
    drop(app);
    let mut app = Backend::start(&fixture.0);
    for path in &paths {
        app.ok(json!({"command":"external_file_open","path":path}));
    }
    assert_eq!(
        app.ok(json!({"command":"legacy_external_status"}))["migrated"],
        4
    );
}

#[test]
fn legacy_external_migration_refuses_unrecorded_siblings_symlinks_protected_and_other_accounts() {
    let fixture = Fixture::new();
    fixture.write("legacy.bin", b"known historical cache");
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let known =
        app.ok(json!({"command":"legacy_external_seed","owner":"101","source":"legacy.bin"}));
    let path = known["path"].as_str().unwrap();
    let planted = fixture.path(path).with_extension("pdf");
    std::fs::write(&planted, b"planted sibling").unwrap();
    let relative = planted.strip_prefix(&fixture.0).unwrap().to_string_lossy();
    app.failure(json!({"command":"external_file_open","path":relative}));
    fixture.write("download.bin", b"user chosen download");
    app.failure(json!({"command":"external_file_open","path":"download.bin"}));
    let protected = app.ok(json!({"command":"legacy_external_seed","owner":"101","source":"legacy.bin","id":2,"protection":"vault"}));
    app.failure(json!({"command":"external_file_open","path":protected["path"]}));
    #[cfg(unix)]
    {
        std::fs::remove_file(fixture.path(path)).unwrap();
        std::os::unix::fs::symlink(fixture.path("legacy.bin"), fixture.path(path)).unwrap();
        app.failure(json!({"command":"external_file_open","path":path}));
        std::fs::remove_file(fixture.path(path)).unwrap();
        std::fs::hard_link(fixture.path("legacy.bin"), fixture.path(path)).unwrap();
    }
    app.ok(json!({"command":"external_file_open","path":path}));
    std::fs::write(fixture.path(path), b"modified same length!!").unwrap();
    app.failure(json!({"command":"external_file_open","path":path}));
    app.ok(json!({"command":"seed_account","owner":202}));
    app.failure(json!({"command":"external_file_open","path":path}));
}

#[test]
fn legacy_external_upgrade_keeps_historical_previews_discoverable_offline() {
    let fixture = Fixture::new();
    fixture.write("legacy.bin", b"historical offline preview");
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!({"command":"legacy_external_seed","owner":"101","source":"legacy.bin","id":9}));
    drop(app);
    let mut app = Backend::start(&fixture.0);
    let files = app.ok(json!({"command":"asset_offline_read","owner":"101"}));
    assert_eq!(files.as_array().unwrap().len(), 1);
    assert_eq!(files[0]["id"], 9);
}

#[test]
fn legacy_external_upgrade_restores_flat_desktop_command_caches() {
    let fixture = Fixture::new();
    fixture.write("legacy.bin", b"recorded desktop cache");
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let preview = app.ok(json!({"command":"legacy_external_seed","owner":"101","source":"legacy.bin","id":51,"category":"flat-preview"}));
    let thumb = app.ok(json!({"command":"legacy_external_seed","owner":"101","source":"legacy.bin","id":52,"category":"flat-thumbnail"}));
    drop(app);
    let mut app = Backend::start(&fixture.0);
    for path in [&preview["path"], &thumb["path"]] {
        app.ok(json!({"command":"external_file_open","path":path}));
    }
    fixture.write(
        "asset-cache/previews/101_home_51.pdf",
        b"recorded desktop cache",
    );
    fixture.write(
        "asset-cache/previews/101_home_999.png",
        b"recorded desktop cache",
    );
    for path in [
        "asset-cache/previews/101_home_51.pdf",
        "asset-cache/previews/101_home_999.png",
    ] {
        app.failure(json!({"command":"external_file_open","path":path}));
    }
    assert_eq!(
        app.ok(json!({"command":"legacy_external_status"}))["migrated"],
        2
    );
}

#[test]
fn legacy_external_concurrent_first_opens_register_once_and_never_remigrate() {
    let fixture = Fixture::new();
    fixture.write("legacy.bin", b"concurrent recorded cache");
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let record =
        app.ok(json!({"command":"legacy_external_seed","owner":"101","source":"legacy.bin"}));
    let path = &record["path"];
    app.ok(json!({"command":"external_file_start","id":"first","path":path,"legacy":true}));
    app.ok(json!({"command":"external_file_start","id":"second","path":path,"legacy":true,"hold":false}));
    fixture.write("file-first.release", b"go");
    for id in ["first", "second"] {
        app.ok(json!({"command":"vault_prepare_finish","id":id}));
    }
    assert_eq!(
        app.ok(json!({"command":"legacy_external_status"}))["migrated"],
        1
    );
    assert_eq!(
        app.ok(json!({"command":"external_file_status"}))[1],
        b"concurrent recorded cache".len() as u64
            + 2 * reuse_hash_bytes(&mut app, path.as_str().unwrap())
    );
    app.ok(json!({"command":"external_file_forget","path":path}));
    app.failure(json!({"command":"external_file_open","path":path}));
}

#[test]
fn legacy_external_hashing_rechecks_protection_and_account_before_registration() {
    for change in ["protection", "account"] {
        let fixture = Fixture::new();
        fixture.write("legacy.bin", b"recorded cache pending migration");
        let mut app = Backend::start(&fixture.0);
        app.ok(json!({"command":"seed_account","owner":101}));
        let record =
            app.ok(json!({"command":"legacy_external_seed","owner":"101","source":"legacy.bin"}));
        app.ok(json!({"command":"external_file_start","id":"held","path":record["path"],"legacy":true}));
        if change == "protection" {
            app.ok(json!({"command":"legacy_external_seed","owner":"101","source":"legacy.bin","updateOnly":true,"protection":"vault"}));
        } else {
            app.ok(json!({"command":"seed_account","owner":202}));
        }
        fixture.write("file-held.release", b"go");
        app.failure(json!({"command":"vault_prepare_finish","id":"held"}));
        app.ok(json!({"command":"seed_account","owner":101}));
        assert_eq!(
            app.ok(json!({"command":"legacy_external_status"}))["migrated"],
            0
        );
    }
}

#[cfg(unix)]
#[test]
fn legacy_external_refuses_symlink_cache_directories() {
    let fixture = Fixture::new();
    fixture.write("legacy.bin", b"recorded cache");
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let record =
        app.ok(json!({"command":"legacy_external_seed","owner":"101","source":"legacy.bin"}));
    let path = record["path"].as_str().unwrap();
    let parent = fixture.path(path).parent().unwrap().to_path_buf();
    let moved = fixture.path("redirected");
    std::fs::rename(&parent, &moved).unwrap();
    std::os::unix::fs::symlink(&moved, &parent).unwrap();
    app.failure(json!({"command":"external_file_open","path":path}));
    assert_eq!(
        app.ok(json!({"command":"legacy_external_status"}))["migrated"],
        0
    );
}

#[test]
fn legacy_external_command_resolves_and_opens_recorded_cache_without_telegram() {
    for category in ["previews", "flat-preview", "thumbnails", "flat-thumbnail"] {
        let fixture = Fixture::new();
        fixture.write("legacy.bin", b"recorded cache without network");
        let mut app = Backend::start(&fixture.0);
        app.ok(json!({"command":"seed_account","owner":101}));
        let file = app.ok(json!({"command":"legacy_external_seed","owner":"101","source":"legacy.bin","category":category}));
        drop(app);
        let mut app = Backend::start(&fixture.0);
        let resolved = app.ok(
            json!({"command":"legacy_external_resolve","thumbnail":category.contains("thumbnail")}),
        );
        assert_eq!(
            PathBuf::from(resolved.as_str().unwrap()),
            fixture
                .path(file["path"].as_str().unwrap())
                .canonicalize()
                .unwrap()
        );
        app.ok(json!({"command":"external_file_open","path":file["path"]}));
        assert_eq!(
            app.ok(json!({"command":"legacy_external_status"}))["migrated"],
            1
        );
    }
}

#[test]
fn legacy_external_stale_plain_offline_pack_cannot_override_current_protection_or_removal() {
    for change in ["protection", "removal"] {
        let fixture = Fixture::new();
        fixture.write("legacy.bin", b"ready plain offline pack");
        let mut app = Backend::start(&fixture.0);
        app.ok(json!({"command":"seed_account","owner":101}));
        let file = app.ok(json!({"command":"legacy_external_seed","owner":"101","source":"legacy.bin","category":"offline"}));
        if change == "protection" {
            app.ok(json!({"command":"legacy_external_seed","owner":"101","source":"legacy.bin","updateOnly":true,"protection":"vault"}));
        } else {
            app.ok(json!({"command":"workspace_seed_files","owner":"101","files":[],"hidden":["saved:1"]}));
        }
        assert!(app
            .failure(json!({"command":"external_file_open","path":file["path"]}))
            .contains("FILE_OPEN_REFUSED"));
        assert_eq!(
            app.ok(json!({"command":"legacy_external_status"}))["migrated"],
            0
        );
    }
}

#[test]
fn legacy_external_ready_pack_survives_missing_current_inventory() {
    let fixture = Fixture::new();
    fixture.write("legacy.bin", b"ready pack absent from inventory");
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let file = app.ok(json!({"command":"legacy_external_seed","owner":"101","source":"legacy.bin","category":"offline","removeMetadata":true}));
    app.ok(json!({"command":"external_file_open","path":file["path"]}));
    assert_eq!(
        app.ok(json!({"command":"legacy_external_status"}))["migrated"],
        1
    );
}

#[test]
fn legacy_external_aborted_preview_hash_retains_cache_guards_until_clear_can_finish() {
    for category in ["previews", "flat-preview"] {
        let fixture = Fixture::new();
        fixture.write("legacy.bin", b"legacy cache with held digest");
        let mut app = Backend::start(&fixture.0);
        app.ok(json!({"command":"seed_account","owner":101}));
        let file = app.ok(json!({"command":"legacy_external_seed","owner":"101","source":"legacy.bin","category":category}));
        let path = file["path"].as_str().unwrap();
        app.ok(json!({"command":"external_file_open","path":path}));
        app.ok(json!({"command":"legacy_external_resolve_start","task":"resolve"}));
        app.ok(json!({"command":"vault_prepare_abort","id":"resolve"}));
        app.ok(json!({"command":"legacy_external_clear_start","task":"clear"}));
        std::thread::sleep(Duration::from_millis(150));
        assert!(
            !fixture.path("legacy-clear.done").exists(),
            "Clear must retain the detached hashing worker's cache lifetime"
        );
        assert!(fixture.path(path).is_file());
        fixture.write("file-resolve.release", b"release");
        app.ok(json!({"command":"vault_prepare_finish","id":"clear"}));
        assert!(!fixture.path(path).exists());
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.ok(json!({"command":"external_file_status"}))[0] != 2 {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            !fixture.path(path).exists(),
            "No worker may recreate cleared cache bytes"
        );
    }
}

#[test]
fn heic_display_pipeline_produces_a_jpeg_and_keeps_original_for_external_opening() {
    let fixture = Fixture::new();
    fixture.write(
        "synthetic.heic",
        include_bytes!("../test-support/fixtures/heic/tiled-12mp.heic"),
    );
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let request = json!({"command":"asset_display_read","owner":"101","source":"synthetic.heic","filename":"Synthetic.HEIC","extension":"heic","mime":"image/heic"});
    let display = app.ok(request);
    let path = PathBuf::from(display["path"].as_str().unwrap());
    assert_eq!(
        path.extension().unwrap(),
        "jpg",
        "HEIC display must use a separate JPEG rendition"
    );
    assert_eq!(image::image_dimensions(&path).unwrap(), (4032, 3024));
    let original = app.ok(json!({"command":"asset_read","owner":"101","source":"synthetic.heic","filename":"Synthetic.HEIC","extension":"heic","mime":"image/heic"}));
    let original_path = PathBuf::from(original["path"].as_str().unwrap());
    assert_ne!(original_path, path);
    assert_eq!(
        std::fs::read(&original_path).unwrap(),
        include_bytes!("../test-support/fixtures/heic/tiled-12mp.heic")
    );
    app.ok(json!({"command":"external_file_open","path":original_path.strip_prefix(fixture.0.canonicalize().unwrap()).unwrap().to_string_lossy()}));
}

#[test]
fn heic_thumbnail_fallback_uses_a_full_primary_image_when_telegram_has_no_thumbnail() {
    let fixture = Fixture::new();
    fixture.write(
        "synthetic.heic",
        include_bytes!("../test-support/fixtures/heic/tiled-12mp.heic"),
    );
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    let image = app.ok(json!({"command":"asset_read","owner":"101","source":"synthetic.heic","filename":"Synthetic.heic","extension":"heic","mime":"image/heic","thumbnail":true}));
    assert_eq!(
        image::image_dimensions(image["path"].as_str().unwrap()).unwrap(),
        (480, 360)
    );
}

#[test]
fn controlled_media_helper_accepts_equivalent_paths_and_refuses_outside_outputs() {
    let fixture = Fixture::new();
    let helper = fixture.path(if cfg!(windows) {
        "heic-helper.exe"
    } else {
        "heic-helper"
    });
    std::fs::copy(env!("CARGO_BIN_EXE_native-e2e-heic-helper"), &helper).unwrap();
    let alias = fixture.0.join("..").join(fixture.0.file_name().unwrap());
    let probe = Command::new(&helper)
        .arg("-version")
        .env("TD_E2E_COMPLETE", alias.join("probe.complete"))
        .output()
        .unwrap();
    assert!(
        probe.status.success(),
        "{}",
        String::from_utf8_lossy(&probe.stderr)
    );
    assert!(String::from_utf8_lossy(&probe.stdout).contains("ffmpeg version 8.1.2"));
    assert!(fixture.path("probe.complete").is_file());
    fixture.write(
        "heic-payload-480x360.jpg",
        include_bytes!("../test-support/fixtures/heic/small-reference.jpg"),
    );
    let child = Command::new(&helper)
        .args(["-s", "480x360"])
        .arg(alias.join("display.jpg"))
        .env("TD_E2E_CONVERSION_READY", alias.join("conversion.ready"))
        .env("TD_E2E_COMPLETE", alias.join("conversion.complete"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let pid = child.id().to_string();
    let conversion = child.wait_with_output().unwrap();
    assert!(
        conversion.status.success(),
        "{}",
        String::from_utf8_lossy(&conversion.stderr)
    );
    assert_eq!(fixture.read("conversion.ready"), pid.as_bytes());
    assert_eq!(fixture.read("conversion.complete"), pid.as_bytes());
    assert_eq!(
        fixture.read("display.jpg"),
        fixture.read("heic-payload-480x360.jpg")
    );
    let outside = Fixture::new();
    let escaped = fixture
        .0
        .join("..")
        .join(outside.0.file_name().unwrap())
        .join("display.jpg");
    let refused = Command::new(&helper)
        .args(["-s", "480x360"])
        .arg(escaped)
        .output()
        .unwrap();
    assert!(!refused.status.success());
    assert!(!outside.path("display.jpg").exists());
}

fn heic_fixture() -> (Fixture, Backend) {
    let fixture = Fixture::new();
    fixture.write(
        "synthetic.heic",
        include_bytes!("../test-support/fixtures/heic/small.heic"),
    );
    fixture.write(
        "heic-frame.jpg",
        include_bytes!("../test-support/fixtures/heic/small-reference.jpg"),
    );
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    (fixture, app)
}
fn heic_request() -> Value {
    json!({"command":"asset_display_read","owner":"101","source":"synthetic.heic","filename":"Synthetic.heic","extension":"heic","mime":"image/heic"})
}
fn wait_heic_process(fixture: &Fixture) -> u32 {
    let deadline = Instant::now() + Duration::from_secs(300);
    loop {
        if let (Ok(ready), Ok(entered)) = (
            std::fs::read_to_string(fixture.path("heic-ready")),
            std::fs::read_to_string(fixture.path("heic-pid")),
        ) {
            if ready == entered {
                if let Ok(pid) = ready.parse::<u32>() {
                    if pid > 0 {
                        return pid;
                    }
                }
            }
        }
        assert!(
            Instant::now() < deadline,
            "Controlled decoder did not start"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn heic_fixture_completed_exit_keeps_real_status_and_refuses_unmonitored_work() {
    let mut results = Vec::new();
    for mode in ["completed", "failed-exit", "active"] {
        let (fixture, mut app) = heic_fixture();
        let mut request = heic_request();
        request["fixtureMonitorLoss"] = json!(if mode == "active" {
            "before-completion"
        } else {
            "after-completion"
        });
        request["fixtureExitDelayMs"] = json!(100);
        if mode == "failed-exit" {
            request["fixtureExitFailure"] = json!(true);
        }
        if mode == "active" {
            fixture.write("heic-mode", "wait");
            request["fixtureDeadlineMs"] = json!(200);
        }
        let reply = app.request(request);
        let status = app.ok(json!({"command":"heic_status"}));
        app.ok(json!({"command":"asset_clear","owner":"101","category":"previews"}));
        results.push((mode, reply, status));
    }
    for (mode, reply, status) in results {
        if mode == "active" {
            assert!(
                reply["error"]
                    .as_str()
                    .unwrap()
                    .contains("MEMORY_MONITOR_UNAVAILABLE"),
                "{reply}; {status}"
            );
            assert!(status["decodes"].as_array().unwrap().is_empty());
        } else {
            let process = status["processes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|item| item["version_probe"] == false)
                .unwrap();
            assert_eq!(process["completed_exit_wait"], true, "{process}");
            if mode == "completed" {
                assert_eq!(reply["ok"], true, "{reply}; {status}");
            } else {
                assert!(
                    reply["error"]
                        .as_str()
                        .unwrap()
                        .contains("HEIC_DECODE_FAILED"),
                    "{reply}; {status}"
                );
                assert!(status["decodes"].as_array().unwrap().is_empty());
            }
        }
    }
}

#[test]
fn heic_fixture_conversion_readiness_precedes_its_deadline_and_helper_stays_small() {
    let (fixture, mut app) = heic_fixture();
    fixture.write("heic-mode", "wait");
    let mut request = heic_request();
    request["fixtureReadyDelayMs"] = json!(1200);
    request["fixtureDeadlineMs"] = json!(200);
    let failure = app.failure(request);
    let status = app.ok(json!({"command":"heic_status"}));
    app.ok(json!({"command":"asset_clear","owner":"101","category":"previews"}));
    assert!(failure.contains("DEADLINE"), "{failure}; {status}");
    assert!(
        fixture.path("heic-ready").is_file(),
        "Actual conversion never signaled readiness: {status}"
    );
    let process = status["processes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["version_probe"] == false && item["outcome"] == "HEIC_DECODE_DEADLINE")
        .unwrap();
    assert!(
        process["readiness_seconds"].as_f64().unwrap() >= 1.2,
        "{process}"
    );
    assert!(
        process["clock_seconds"].as_f64().unwrap() >= 0.2,
        "{process}"
    );
    let helper_size = std::fs::metadata(fixture.path(if cfg!(windows) {
        "heic-helper.exe"
    } else {
        "heic-helper"
    }))
    .unwrap()
    .len();
    let driver_size = std::fs::metadata(env!("CARGO_BIN_EXE_native-e2e-driver"))
        .unwrap()
        .len();
    assert!(
        helper_size < 16 * 1024 * 1024 && helper_size < driver_size / 4,
        "Helper: {helper_size}; driver: {driver_size}"
    );
    println!("HEIC readiness containment: {status}");
    assert!(status["decodes"].as_array().unwrap().is_empty());
}

#[test]
fn heic_version_banners_gate_the_real_asset_pipeline_and_keep_original_actions() {
    let mut failures = Vec::new();
    for (version, eligible) in [
        ("8.1.2", true),
        ("8.1-full_build-www.gyan.dev", true),
        ("n8.1-3-g0123abcd", true),
        ("9.0", true),
        ("8.0.1", false),
        ("7.1.1-1ubuntu1", false),
        ("N-121234-g0123abcd-20260901", false),
        ("8.1foo", false),
    ] {
        let (fixture, mut app) = heic_fixture();
        fixture.write("heic-version", version);
        let reply = app.request(heic_request());
        if reply["ok"] != eligible {
            failures.push(format!(
                "{version}: expected eligible={eligible}, got {reply}"
            ));
        }
        if reply["ok"] == true {
            assert_eq!(
                image::image_dimensions(reply["value"]["path"].as_str().unwrap()).unwrap(),
                (240, 180)
            );
            assert!(fixture.path("heic-pid").is_file());
        } else {
            let diagnostics = app.ok(json!({"command":"heic_status"}));
            assert!(
                reply["error"]
                    .as_str()
                    .unwrap()
                    .contains("HEIC_FFMPEG_VERSION_UNSUPPORTED"),
                "{version}: {reply}; diagnostics={diagnostics}"
            );
            assert!(
                !fixture.path("heic-pid").exists(),
                "Refused version must not decode"
            );
        }
        let original = app.ok(json!({"command":"asset_cached","owner":"101"}));
        app.ok(json!({"command":"external_file_open","path":Path::new(original.as_str().unwrap()).strip_prefix(fixture.0.canonicalize().unwrap()).unwrap().to_string_lossy()}));
        assert_eq!(
            std::fs::read(original.as_str().unwrap()).unwrap(),
            include_bytes!("../test-support/fixtures/heic/small.heic")
        );
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn heic_rejects_corrupt_oversized_old_missing_and_partial_tile_decoders() {
    for mode in [
        "corrupt",
        "oversized",
        "old",
        "missing",
        "tile",
        "oversize",
        "fail",
    ] {
        let (fixture, mut app) = heic_fixture();
        fixture.write("heic-mode", mode);
        if mode == "corrupt" {
            fixture.write(
                "synthetic.heic",
                include_bytes!("../test-support/fixtures/heic/corrupt.heic"),
            );
        }
        if mode == "oversized" {
            fixture.write(
                "synthetic.heic",
                include_bytes!("../test-support/fixtures/heic/oversized.heic"),
            );
        }
        if mode == "tile" {
            fixture.write(
                "synthetic.heic",
                include_bytes!("../test-support/fixtures/heic/tiled-12mp.heic"),
            );
            fixture.write(
                "heic-frame.jpg",
                include_bytes!("../test-support/fixtures/heic/tiled-12mp-reference.jpg"),
            );
        }
        let mut request = heic_request();
        if mode == "missing" {
            request["heicTools"] = json!("missing");
        }
        let failure = app.failure(request);
        assert!(failure.starts_with("HEIC_"), "{mode}: {failure}");
        let original = app.ok(json!({"command":"asset_cached","owner":"101"}));
        for entry in
            std::fs::read_dir(Path::new(original.as_str().unwrap()).parent().unwrap()).unwrap()
        {
            let path = entry.unwrap().path();
            assert!(
                !path
                    .extension()
                    .is_some_and(|ext| matches!(ext.to_str(), Some("part" | "source" | "jpg"))),
                "Failed decode left bytes: {path:?}"
            );
        }
        assert!(original.as_str().unwrap().ends_with(".heic"));
        app.ok(json!({"command":"external_file_open","path":Path::new(original.as_str().unwrap()).strip_prefix(fixture.0.canonicalize().unwrap()).unwrap().to_string_lossy()}));
    }
}
#[test]
fn heic_reuse_pin_unpin_and_clear_preserve_the_original_relationship() {
    let (_fixture, mut app) = heic_fixture();
    let first = app.ok(heic_request());
    let rendition = PathBuf::from(first["path"].as_str().unwrap());
    let second = app.ok(heic_request());
    assert_eq!(first, second);
    let status = app.ok(json!({"command":"heic_status"}));
    assert_eq!(status["decodes"].as_array().unwrap().len(), 1);
    app.ok(json!({"command":"asset_pin","owner":"101","pinned":true}));
    assert!(rendition.with_extension("pin").is_file());
    app.ok(json!({"command":"asset_limits","previews":1024,"thumbnails":1024}));
    assert_eq!(
        app.ok(heic_request())["path"],
        first["path"],
        "Pinned cache hits need no output reservation"
    );
    app.ok(json!({"command":"asset_clear","owner":"101","category":"previews"}));
    assert!(rendition.is_file());
    app.ok(json!({"command":"asset_pin","owner":"101","pinned":false}));
    assert!(!rendition.with_extension("pin").exists());
    app.ok(json!({"command":"asset_clear","owner":"101","category":"previews"}));
    assert!(!rendition.exists());
}
struct HeicProcess {
    pid: u32,
    #[cfg(windows)]
    handle: std::os::windows::io::OwnedHandle,
}
impl HeicProcess {
    fn capture(pid: u32) -> Self {
        #[cfg(windows)]
        let handle = {
            use std::os::windows::io::FromRawHandle;
            use windows_sys::Win32::System::Threading::{
                OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
            };
            let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
            assert!(
                !handle.is_null(),
                "Cannot open decoder pid={pid}: {}",
                std::io::Error::last_os_error()
            );
            unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(handle) }
        };
        let process = Self {
            pid,
            #[cfg(windows)]
            handle,
        };
        assert!(
            process.alive().expect("Cannot query decoder"),
            "Decoder pid={} must be live",
            process.pid
        );
        process
    }
    fn alive(&self) -> std::io::Result<bool> {
        #[cfg(unix)]
        {
            let pid = i32::try_from(self.pid).expect("Invalid decoder PID");
            if unsafe { libc::kill(pid, 0) } == 0 {
                return Ok(true);
            }
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ESRCH) {
                return Ok(false);
            }
            Err(error)
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::System::Threading::GetExitCodeProcess;
            let mut code = 0;
            if unsafe { GetExitCodeProcess(self.handle.as_raw_handle(), &mut code) } == 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(code == 259)
        }
    }
}
struct HeicLifecycleCleanup<'a> {
    app: &'a mut Backend,
    fixture: &'a Fixture,
    process: &'a HeicProcess,
}
impl Drop for HeicLifecycleCleanup<'_> {
    fn drop(&mut self) {
        // Release before any assertion unwinds into Backend's process teardown.
        let _ = std::fs::write(self.fixture.path("heic-sample.release"), b"release");
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.app.request(json!({"command":"asset_abort"}))
        }));
        let until = Instant::now() + Duration::from_secs(30);
        while !matches!(self.process.alive(), Ok(false)) && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
#[test]
fn heic_caller_abort_clear_and_account_change_terminate_the_decoder() {
    for operation in ["abort", "clear", "account"] {
        let (fixture, mut app) = heic_fixture();
        fixture.write("heic-mode", "wait");
        let mut request = heic_request();
        request["command"] = json!("asset_start");
        request["display"] = json!(true);
        request["requestId"] = json!("heic-lifecycle");
        let sample_race = operation != "clear";
        request["fixtureSampleGate"] = json!(sample_race);
        if sample_race {
            request["fixtureMonitorLoss"] = json!("before-completion");
        }
        app.ok(request);
        let pid = wait_heic_process(&fixture);
        let process = HeicProcess::capture(pid);
        let cleanup = HeicLifecycleCleanup {
            app: &mut app,
            fixture: &fixture,
            process: &process,
        };
        let app = &mut *cleanup.app;
        if sample_race {
            let until = Instant::now() + Duration::from_secs(30);
            while !std::fs::read_to_string(fixture.path("heic-sample.started"))
                .is_ok_and(|value| value == pid.to_string())
            {
                assert!(Instant::now() < until, "Controlled RSS read did not start");
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(
                process
                    .alive()
                    .expect("Cannot query decoder during the RSS read"),
                "Decoder remains live during the RSS read"
            );
        }
        if operation == "abort" {
            app.ok(json!({"command":"asset_abort"}));
            fixture.write("heic-sample.release", b"release");
        }
        if operation == "account" {
            app.ok(json!({"command":"seed_account","owner":202}));
            fixture.write("heic-sample.release", b"release");
            assert!(app
                .failure(json!({"command":"asset_finish"}))
                .contains("ACCOUNT_CHANGED"));
        }
        if operation != "account" {
            app.ok(json!({"command":"asset_clear","owner":"101","category":"previews"}));
        }
        if operation == "clear" {
            assert!(app
                .failure(json!({"command":"asset_finish"}))
                .contains("CANCELLED"));
        }
        let status = app.ok(json!({"command":"heic_status"}));
        eprintln!("HEIC lifecycle operation={operation}; pid={pid}; status={status}");
        assert!(status["decodes"].as_array().unwrap().is_empty());
        assert!(
            status["processes"].as_array().unwrap().iter().any(|item| {
                item["pid"] == pid
                    && (item["outcome"] == "CANCELLED"
                        || item["outcome"]
                            .as_str()
                            .is_some_and(|v| v.contains("ACCOUNT_CHANGED")))
            }),
            "operation={operation}; pid={pid}; status={status}"
        );
        assert!(
            !process.alive().expect("Cannot verify decoder termination"),
            "Decoder must be dead after {operation}"
        );
        let directory = fixture.path("asset-cache/previews/workspace/101/previews");
        assert!(
            std::fs::read_dir(directory).unwrap().all(|entry| {
                !matches!(
                    entry.unwrap().path().extension().and_then(|v| v.to_str()),
                    Some("jpg" | "part")
                )
            }),
            "Cancelled decoder must not leave a rendition or partial"
        );
        let database = sqlite::open(fixture.path("workspace/101/workspace.db")).unwrap();
        let mut query = database
            .prepare("SELECT COUNT(*) FROM workspace_records WHERE kind='heic-rendition-v1'")
            .unwrap();
        query.next().unwrap();
        assert_eq!(query.read::<i64, _>(0).unwrap(), 0);
    }
}
#[test]
fn heic_source_mutation_during_decode_never_publishes_a_rendition() {
    let (fixture, mut app) = heic_fixture();
    fixture.write("heic-mode", "gate");
    let mut request = heic_request();
    request["command"] = json!("asset_start");
    request["display"] = json!(true);
    app.ok(request);
    wait_heic_process(&fixture);
    let original = app.ok(json!({"command":"asset_cached","owner":"101"}));
    std::fs::write(original.as_str().unwrap(), b"modified after verification").unwrap();
    fixture.write("heic-release", b"ready");
    assert!(app
        .failure(json!({"command":"asset_finish"}))
        .contains("modified"));
    assert_eq!(
        app.ok(json!({"command":"heic_status"}))["decodes"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "The decoder completed; publication failed on its source digest"
    );
    app.ok(json!({"command":"asset_clear","owner":"101","category":"previews"}));
}
#[test]
fn heic_memory_threshold_terminates_a_touching_decoder_process() {
    let (fixture, mut app) = heic_fixture();
    fixture.write("heic-mode", "memory");
    let failure = app.failure(heic_request());
    assert!(
        failure.contains("MEMORY_LIMIT") || failure.contains("HEIC_DECODE_FAILED"),
        "{failure}"
    );
    let status = app.ok(json!({"command":"heic_status"}));
    assert_eq!(status["decoderEntered"], true, "{status}");
    assert!(
        fixture.path("heic-ready").is_file(),
        "Memory process was never ready: {status}"
    );
    #[cfg(target_os = "macos")]
    assert!(status["processes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["outcome"] == "HEIC_MEMORY_LIMIT"
            && item["metrics"]["sampled_peak_bytes"].as_u64().unwrap() > 1024 * 1024 * 1024));
    #[cfg(not(target_os = "macos"))]
    {
        let threshold = status["processes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["outcome"] == "HEIC_MEMORY_LIMIT");
        let denied = fixture.path("heic-memory-denied");
        assert!(
            threshold || denied.is_file(),
            "The controlled process must be stopped by a memory control"
        );
        if denied.is_file() {
            let attempted = std::fs::read_to_string(denied)
                .unwrap()
                .parse::<u64>()
                .unwrap();
            let cap = if cfg!(windows) {
                1024 * 1024 * 1024
            } else {
                2 * 1024 * 1024 * 1024
            };
            assert!(attempted <= cap + 8 * 1024 * 1024);
        }
    }
    println!("HEIC memory containment: {status}");
    assert!(status["decodes"].as_array().unwrap().is_empty());
}
#[test]
fn heic_telegram_thumbnail_wins_without_a_decoder_and_protected_sources_are_refused() {
    let (fixture, mut app) = heic_fixture();
    let mut request = heic_request();
    request["command"] = json!("asset_read");
    request["thumbnail"] = json!(true);
    request["thumbnailSource"] = json!("heic-frame.jpg");
    request["heicTools"] = json!("missing");
    let result = app.ok(request);
    assert_eq!(
        image::image_dimensions(result["path"].as_str().unwrap()).unwrap(),
        (480, 360)
    );
    let mut request = heic_request();
    request["protection"] = json!("vault");
    request["id"] = json!(2);
    assert!(app
        .failure(request)
        .starts_with("ENCRYPTED_PREVIEW_UNAVAILABLE"));
    assert!(app.ok(json!({"command":"heic_status"}))["decodes"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(!fixture.path("heic-pid").exists());
}
#[test]
fn heic_original_bytes_remain_charged_to_the_preview_budget() {
    let (_fixture, mut app) = heic_fixture();
    // Reserve16MiB for output plus retained original bytes; a16MiB total is insufficient.
    app.ok(json!({"command":"asset_limits","previews":16 * 1024 * 1024,"thumbnails":1024 * 1024}));
    let failure = app.failure(heic_request());
    assert!(
        failure.contains("cache") || failure.contains("CACHE"),
        "{failure}"
    );
    assert!(app.ok(json!({"command":"heic_status"}))["decodes"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn heic_offline_pack_sources_require_exact_plain_account_records() {
    let (fixture, mut app) = heic_fixture();
    let record = app.ok(json!({"command":"legacy_external_seed","owner":"101","source":"synthetic.heic","category":"offline","filename":"Synthetic.heic","extension":"heic","mime":"image/heic"}));
    let path = record["path"].as_str().unwrap();
    let displayed = app.ok(json!({"command":"asset_display_local","owner":"101","path":path}));
    assert_eq!(
        image::image_dimensions(displayed.as_str().unwrap()).unwrap(),
        (240, 180)
    );
    assert!(app
        .failure(json!({"command":"asset_display_local","owner":"101","path":path,"id":2}))
        .contains("SOURCE_REFUSED"));
    app.ok(json!({"command":"legacy_external_seed","owner":"101","source":"synthetic.heic","protection":"vault","updateOnly":true}));
    assert!(app
        .failure(json!({"command":"asset_display_local","owner":"101","path":path}))
        .contains("ENCRYPTED_PREVIEW_UNAVAILABLE"));
    assert!(fixture.path("heic-frame.jpg").is_file());
}
#[test]
fn heic_explicit_cancellation_and_deadline_remove_partial_outputs() {
    for operation in ["cancel", "deadline"] {
        let (fixture, mut app) = heic_fixture();
        fixture.write("heic-mode", "wait");
        let mut request = heic_request();
        request["command"] = json!("asset_start");
        request["display"] = json!(true);
        request["requestId"] = json!("heic-cancel");
        if operation == "deadline" {
            request["fixtureDeadlineMs"] = json!(2000);
        }
        app.ok(request);
        wait_heic_process(&fixture);
        if operation == "cancel" {
            app.ok(json!({"command":"asset_cancel","owner":"101","requestId":"heic-cancel"}));
        }
        let failure = app.failure(json!({"command":"asset_finish"}));
        assert!(
            failure.contains(if operation == "cancel" {
                "CANCELLED"
            } else {
                "DEADLINE"
            }),
            "{failure}"
        );
        app.ok(json!({"command":"asset_clear","owner":"101","category":"previews"}));
        assert!(app.ok(json!({"command":"heic_status"}))["decodes"]
            .as_array()
            .unwrap()
            .is_empty());
    }
}

#[test]
fn heic_review_source_limit_refuses_a_recorded_large_file_before_hashing() {
    let (fixture, mut app) = heic_fixture();
    std::fs::OpenOptions::new()
        .write(true)
        .open(fixture.path("synthetic.heic"))
        .unwrap()
        .set_len(64 * 1024 * 1024 + 1)
        .unwrap();
    let record = app.ok(json!({"command":"legacy_external_seed","owner":"101","source":"synthetic.heic","category":"offline","filename":"Synthetic.heic","extension":"heic","mime":"image/heic"}));
    let before = app.ok(json!({"command":"external_file_status"}));
    assert!(app
        .failure(json!({"command":"asset_display_local","owner":"101","path":record["path"]}))
        .starts_with("HEIC_"));
    let after = app.ok(json!({"command":"external_file_status"}));
    assert_eq!(
        before[1].as_u64().unwrap(),
        after[1].as_u64().unwrap(),
        "A rejected source must not be fully hashed"
    );
}
#[test]
fn heic_review_offline_view_does_not_orphan_an_online_rendition_pin() {
    let (_fixture, mut app) = heic_fixture();
    let online = app.ok(heic_request());
    let rendition = PathBuf::from(online["path"].as_str().unwrap());
    app.ok(json!({"command":"asset_pin","owner":"101","pinned":true}));
    assert!(rendition.with_extension("pin").exists());
    let record = app.ok(json!({"command":"legacy_external_seed","owner":"101","source":"synthetic.heic","category":"offline","filename":"Synthetic.heic","extension":"heic","mime":"image/heic"}));
    app.ok(json!({"command":"asset_display_local","owner":"101","path":record["path"]}));
    app.ok(json!({"command":"asset_pin","owner":"101","pinned":false}));
    assert!(
        !rendition.with_extension("pin").exists(),
        "Offline viewing must not replace the online pin association"
    );
    app.ok(json!({"command":"asset_clear","owner":"101","category":"previews"}));
    assert!(!rendition.exists());
}

#[test]
fn heic_pin_inheritance_cannot_recreate_a_pin_after_concurrent_unpinning() {
    let (fixture, mut app) = heic_fixture();
    let mut original = heic_request();
    original["command"] = json!("asset_read");
    app.ok(original);
    app.ok(json!({"command":"asset_pin","owner":"101","pinned":true}));
    app.ok(json!({"command":"heic_pin_gate"}));
    let mut request = heic_request();
    request["command"] = json!("asset_start");
    request["display"] = json!(true);
    app.ok(request);
    let deadline = Instant::now() + Duration::from_secs(300);
    while !fixture.path("heic-pin.checked").is_file() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    app.ok(json!({"command":"heic_pin_start","owner":"101"}));
    // Give the competing operation time to finish if it is not serialized.
    std::thread::sleep(Duration::from_millis(300));
    fixture.write("heic-pin.release", b"ready");
    let result = app.ok(json!({"command":"asset_finish"}));
    let rendition = PathBuf::from(result.as_str().unwrap());
    app.ok(json!({"command":"vault_prepare_finish","id":"heic-pin"}));
    assert!(
        !rendition.with_extension("pin").exists(),
        "A completed unpin must not be undone by late rendition publication"
    );
    app.ok(json!({"command":"asset_clear","owner":"101","category":"previews"}));
    assert!(!rendition.exists());
}

#[test]
fn heic_pin_markers_do_not_survive_canceled_rendition_publication() {
    let (fixture, mut app) = heic_fixture();
    let mut original = heic_request();
    original["command"] = json!("asset_read");
    let source = app.ok(original);
    let source = PathBuf::from(source["path"].as_str().unwrap());
    app.ok(json!({"command":"asset_pin","owner":"101","pinned":true}));
    app.ok(json!({"command":"heic_pin_gate"}));
    let mut request = heic_request();
    request["command"] = json!("asset_start");
    request["display"] = json!(true);
    request["requestId"] = json!("heic-publish-cancel");
    app.ok(request);
    let deadline = Instant::now() + Duration::from_secs(300);
    while !fixture.path("heic-pin.checked").is_file() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    app.ok(json!({"command":"asset_cancel","owner":"101","requestId":"heic-publish-cancel"}));
    fixture.write("heic-pin.release", b"ready");
    assert!(app
        .failure(json!({"command":"asset_finish"}))
        .contains("CANCELLED"));
    let pins: Vec<_> = std::fs::read_dir(source.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "pin"))
        .collect();
    assert_eq!(
        pins,
        vec![source.with_extension("pin")],
        "A failed rendition must leave no orphan pin marker"
    );
    app.ok(json!({"command":"asset_pin","owner":"101","pinned":false}));
    app.ok(json!({"command":"asset_clear","owner":"101","category":"previews"}));
}

#[test]
fn video_caption_choice_survives_interrupted_upload_restart() {
    let fixture = Fixture::new();
    fixture.write("Holiday video.mp4", b"synthetic video boundary fixture");
    let mut app = Backend::start(&fixture.0);
    for (id, enabled) in [("caption-on", true), ("caption-off", false)] {
        app.ok(json!({"command":"save_transfer","job":{
            "id":id,"ownerId":"101","direction":"upload","kind":"local_upload",
            "status":"uploading","path":fixture.path("Holiday video.mp4"),
            "filename":"Holiday video.mp4","videoUploadMode":"media","videoFilenameCaption":enabled,
            "protectionMode":"standard","progress":50,"transferredBytes":15,"totalBytes":30,
            "speedBytesPerSec":10,"queuePosition":0,"revision":1,"createdAt":1,"updatedAt":2
        }}));
    }
    app.stop();
    let mut app = Backend::start(&fixture.0);
    let recovered = app.ok(json!({"command":"transfers"}));
    for (id, enabled) in [("caption-on", true), ("caption-off", false)] {
        let job = recovered
            .as_array()
            .unwrap()
            .iter()
            .find(|job| job["id"] == id)
            .unwrap();
        assert_eq!(job["videoFilenameCaption"], enabled);
        assert_eq!(job["videoUploadMode"], "media");
        assert_eq!(job["status"], "paused");
        assert_eq!(job["ownerId"], "101");
    }
    app.stop();
}

// These are valid, short H.264 MP4 containers. Sparse ISO-BMFF `free` padding
// exercises real large-file preparation without checking in 210 MiB of video.
fn video_upload_fixture(fixture: &Fixture, name: &str, large: bool, late_moov: bool) {
    use std::io::{Seek, SeekFrom};
    let bytes: &[u8] = if late_moov {
        include_bytes!("fixtures/video-upload/synthetic-h264-moov-late.mp4")
    } else {
        include_bytes!("fixtures/video-upload/synthetic-h264.mp4")
    };
    let mut file = std::fs::File::create(fixture.path(name)).unwrap();
    if !large {
        file.write_all(bytes).unwrap();
        return;
    }
    let (prefix, suffix) = if late_moov {
        let mut offset = 0;
        let moov = loop {
            let size = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
            assert!(size >= 8);
            if &bytes[offset + 4..offset + 8] == b"moov" {
                break offset;
            }
            offset += size;
            assert!(offset < bytes.len());
        };
        (&bytes[..moov], &bytes[moov..])
    } else {
        (bytes, &[][..])
    };
    file.write_all(prefix).unwrap();
    const PADDING: u32 = 210 * 1024 * 1024;
    file.write_all(&PADDING.to_be_bytes()).unwrap();
    file.write_all(b"free").unwrap();
    file.seek(SeekFrom::Current(i64::from(PADDING) - 8))
        .unwrap();
    file.write_all(suffix).unwrap();
    if suffix.is_empty() {
        file.set_len(bytes.len() as u64 + u64::from(PADDING))
            .unwrap();
    }
    assert!(file.metadata().unwrap().len() > 200 * 1024 * 1024);
}

fn uploaded_video_wire(value: &Value) -> grammers_tl_types::types::InputMediaUploadedDocument {
    use grammers_tl_types::Deserializable;
    let hex = value["serializedTl"]
        .as_str()
        .expect("No captured Telegram attachment");
    assert_eq!(hex.len() % 2, 0);
    let bytes: Vec<u8> = hex
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect();
    let wire = grammers_tl_types::enums::InputMedia::from_bytes(&bytes).unwrap();
    let grammers_tl_types::enums::InputMedia::UploadedDocument(document) = wire else {
        panic!("Media upload was serialized as a different Telegram type")
    };
    document
}

#[test]
fn video_upload_media_includes_a_real_thumbnail_for_small_and_large_mp4_containers() {
    for (name, large, late) in [
        ("large-faststart.mp4", true, false),
        ("large-moov-late.mp4", true, true),
        ("small.mp4", false, false),
    ] {
        let fixture = Fixture::new();
        video_upload_fixture(&fixture, name, large, late);
        let mut app = start_video_upload_backend(&fixture);
        let result = app.ok(
            json!({"command":"video_upload_prepare","owner":"101","source":name,
            "mode":"media","filenameCaption":true}),
        );
        assert_eq!(
            result["result"]["receipt"],
            "controlled-telegram-message-303"
        );
        assert_eq!(result["thumbnailRequests"], 1);
        assert_eq!(result["sendAttempts"], 1);
        assert_eq!(result["pacerPending"], 0);
        let wire = uploaded_video_wire(&result);
        assert!(!wire.force_file, "Media was forced to a generic file");
        assert_eq!(wire.mime_type, "video/mp4");
        assert!(matches!(wire.file, grammers_tl_types::enums::InputFile::Big(_)) == large);
        let video = wire
            .attributes
            .iter()
            .find_map(|attribute| match attribute {
                grammers_tl_types::enums::DocumentAttribute::Video(video) => Some(video),
                _ => None,
            })
            .expect("Missing DocumentAttributeVideo in the actual Telegram payload");
        assert_eq!((video.w, video.h), (640, 360));
        assert!(
            (video.duration - 2.0).abs() < 0.1,
            "Unexpected video duration: {}",
            video.duration
        );
        assert!(
            video.supports_streaming,
            "Validated MP4 layouts accepted by Telegram Desktop must remain streamable"
        );
        assert!(wire.attributes.iter().any(|attribute| matches!(attribute,
            grammers_tl_types::enums::DocumentAttribute::Filename(filename) if filename.file_name == name)));
        assert_eq!(result["caption"], name);
        assert!(
            wire.thumb.is_some(),
            "Missing uploaded thumbnail for {name}"
        );
        let thumbnail = fixture.read(
            result["thumbnail"]
                .as_str()
                .expect("No generated thumbnail file"),
        );
        assert!(thumbnail.len() <= 200 * 1024 && thumbnail.len() > 100);
        assert_eq!(
            image::guess_format(&thumbnail).unwrap(),
            image::ImageFormat::Jpeg
        );
        let decoded = image::load_from_memory(&thumbnail)
            .expect("Thumbnail must be an actual decodable JPEG");
        assert!(decoded.width() > 0 && decoded.height() > 0);
        assert!(decoded.width() <= 320 && decoded.height() <= 320);
        assert_eq!(result["upBytes"], thumbnail.len() as u64);
        assert_video_upload_scratch_empty(&fixture);
        app.stop();
    }
}

fn start_video_upload_backend(fixture: &Fixture) -> Backend {
    fixture.write(".video-upload-fixture", b"synthetic\n");
    std::fs::create_dir(fixture.path("video-upload-scratch")).unwrap();
    // Native Windows success cannot silently be provided by FFmpeg from PATH.
    #[cfg(target_os = "windows")]
    fixture.write(".video-upload-empty-path", b"native Windows codec only");
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app
}
fn assert_video_upload_scratch_empty(fixture: &Fixture) {
    for entry in std::fs::read_dir(fixture.path("video-upload-scratch")).unwrap() {
        let entry = entry.unwrap();
        assert!(
            entry.file_type().unwrap().is_dir()
                && entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("telegram-drive-staging"),
            "Unexpected decoder scratch artifact: {:?}",
            entry.path()
        );
        assert!(
            std::fs::read_dir(entry.path()).unwrap().next().is_none(),
            "Plaintext decoder staging was not cleaned: {:?}",
            entry.path()
        );
    }
}
fn wait_for_video_thumbnail_transport(app: &mut Backend) -> Value {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let status = app.ok(json!({"command":"video_upload_observe"}));
        if status["thumbnailRequests"] == 1 {
            return status;
        }
        assert!(
            status["finished"] != true,
            "Operation finished without uploading a thumbnail: {status}"
        );
        assert!(
            Instant::now() < deadline,
            "Thumbnail upload was not reached: {status}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn video_upload_file_mode_skips_media_inspection_for_opaque_mp4_files() {
    let fixture = Fixture::new();
    fixture.write("private.mp4", b"private opaque content is not a valid MP4");
    let mut app = start_video_upload_backend(&fixture);
    let result=app.ok(json!({"command":"video_upload_prepare","owner":"101","source":"private.mp4","mode":"file"}));
    assert_eq!(result["result"]["video"], false);
    assert_eq!(result["thumbnailRequests"], 0);
    assert_eq!(result["sendAttempts"], 0);
    assert_eq!(result["upBytes"], 0);
    assert!(!fixture.path("video-upload-thumbnail.jpg").exists());
    assert_video_upload_scratch_empty(&fixture);
    app.stop();
}

#[test]
fn video_upload_invalid_media_and_other_accounts_do_not_publish_thumbnail_artifacts() {
    let fixture = Fixture::new();
    fixture.write("broken.mp4", b"not an MP4");
    let mut app = start_video_upload_backend(&fixture);
    let result=app.ok(json!({"command":"video_upload_prepare","owner":"101","source":"broken.mp4","mode":"media"}));
    assert!(result["result"]["error"]
        .as_str()
        .unwrap()
        .contains("VIDEO_METADATA_UNAVAILABLE"));
    assert_eq!(result["thumbnailRequests"], 0);
    assert_eq!(result["sendAttempts"], 0);
    assert_eq!(result["upBytes"], 0);
    assert!(!fixture.path("video-upload-thumbnail.jpg").exists());
    let rejected=app.request(json!({"command":"video_upload_prepare","owner":"202","source":"broken.mp4","mode":"media"}));
    assert_eq!(rejected["ok"], false);
    assert!(!fixture.path("video-upload-thumbnail.jpg").exists());
    assert_video_upload_scratch_empty(&fixture);
    app.stop();
}

#[test]
fn video_upload_thumbnail_failure_prevents_publication_and_releases_admission_for_retry() {
    let fixture = Fixture::new();
    video_upload_fixture(&fixture, "retry.mp4", false, false);
    let mut app = start_video_upload_backend(&fixture);
    let failure=app.ok(json!({"command":"video_upload_prepare","owner":"101","source":"retry.mp4","mode":"media","failThumbnail":true}));
    assert!(failure["result"]["error"]
        .as_str()
        .unwrap()
        .contains("thumbnail upload failure"));
    assert_eq!(failure["thumbnailRequests"], 1);
    assert_eq!(failure["sendAttempts"], 0);
    assert_eq!(failure["upBytes"], 0);
    assert_eq!(failure["pacerPending"], 0);
    assert_video_upload_scratch_empty(&fixture);
    let retry=app.ok(json!({"command":"video_upload_prepare","owner":"101","source":"retry.mp4","mode":"media","filenameCaption":false}));
    assert_eq!(
        retry["result"]["receipt"],
        "controlled-telegram-message-303"
    );
    assert_eq!(retry["thumbnailRequests"], 1);
    assert_eq!(retry["sendAttempts"], 1);
    assert_eq!(retry["caption"], "");
    assert!(uploaded_video_wire(&retry).thumb.is_some());
    let thumbnail = fixture.read("video-upload-thumbnail.jpg");
    assert_eq!(retry["upBytes"], thumbnail.len() as u64);
    assert_video_upload_scratch_empty(&fixture);
    app.stop();
}

#[test]
fn video_upload_cancellation_during_thumbnail_transport_never_sends_a_message() {
    let fixture = Fixture::new();
    video_upload_fixture(&fixture, "cancel.mp4", false, false);
    let mut app = start_video_upload_backend(&fixture);
    app.ok(json!({"command":"video_upload_start","owner":"101","source":"cancel.mp4","mode":"media","blockThumbnail":true}));
    let blocked = wait_for_video_thumbnail_transport(&mut app);
    assert!(
        blocked["upBytes"].as_u64().unwrap() > 0,
        "Thumbnail must hold real bandwidth admission"
    );
    app.ok(json!({"command":"video_upload_cancel"}));
    let result = app.ok(json!({"command":"video_upload_finish"}));
    assert!(result["result"]["error"]
        .as_str()
        .unwrap()
        .contains("cancelled"));
    assert_eq!(result["sendAttempts"], 0);
    assert_eq!(result["upBytes"], 0);
    assert_eq!(result["pacerPending"], 0);
    assert!(!fixture.path("video-upload-thumbnail.jpg").exists());
    assert_video_upload_scratch_empty(&fixture);
    app.stop();
}

#[test]
fn video_upload_account_switch_during_thumbnail_transport_fences_publication() {
    let fixture = Fixture::new();
    video_upload_fixture(&fixture, "account.mp4", false, false);
    let mut app = start_video_upload_backend(&fixture);
    app.ok(json!({"command":"video_upload_start","owner":"101","source":"account.mp4","mode":"media","blockThumbnail":true}));
    wait_for_video_thumbnail_transport(&mut app);
    app.ok(json!({"command":"seed_account","owner":202}));
    let result = app.ok(json!({"command":"video_upload_finish"}));
    assert!(
        result["result"]["error"].is_string(),
        "Account switch must reject the operation"
    );
    assert_eq!(result["sendAttempts"], 0);
    assert_eq!(result["upBytes"], 0);
    assert_eq!(result["pacerPending"], 0);
    assert!(!fixture.path("video-upload-thumbnail.jpg").exists());
    assert_video_upload_scratch_empty(&fixture);
    app.stop();
}

#[test]
fn video_upload_publication_failure_releases_thumbnail_admission_without_committing_usage() {
    let fixture = Fixture::new();
    video_upload_fixture(&fixture, "publish.mp4", false, false);
    let mut app = start_video_upload_backend(&fixture);
    let result=app.ok(json!({"command":"video_upload_prepare","owner":"101","source":"publish.mp4","mode":"media","failPublish":true}));
    assert!(result["result"]["error"]
        .as_str()
        .unwrap()
        .contains("publication failure"));
    assert_eq!(result["thumbnailRequests"], 1);
    assert_eq!(result["sendAttempts"], 1);
    assert_eq!(result["caption"], "publish.mp4");
    assert!(uploaded_video_wire(&result).thumb.is_some());
    assert_eq!(
        result["upBytes"], 0,
        "Only a successful declared upload commits quota"
    );
    assert_eq!(result["pacerPending"], 0);
    assert_video_upload_scratch_empty(&fixture);
    app.stop();
}

#[cfg(not(target_os = "windows"))]
#[test]
fn video_upload_unavailable_decoder_reports_file_fallback_without_publication_or_quota() {
    let fixture = Fixture::new();
    video_upload_fixture(&fixture, "unsupported.mp4", false, false);
    fixture.write(
        ".video-upload-empty-path",
        b"No external decoder is available",
    );
    let mut app = start_video_upload_backend(&fixture);
    let result=app.ok(json!({"command":"video_upload_prepare","owner":"101","source":"unsupported.mp4","mode":"media"}));
    let message = result["result"]["error"].as_str().unwrap();
    assert!(message.contains("VIDEO_THUMBNAIL_UNAVAILABLE"), "{message}");
    assert!(message.contains("File mode"), "{message}");
    assert_eq!(result["thumbnailRequests"], 0);
    assert_eq!(result["sendAttempts"], 0);
    assert_eq!(result["upBytes"], 0);
    assert_eq!(result["pacerPending"], 0);
    assert_video_upload_scratch_empty(&fixture);
    let fallback=app.ok(json!({"command":"video_upload_prepare","owner":"101","source":"unsupported.mp4","mode":"file"}));
    assert_eq!(fallback["result"]["video"], false);
    assert_eq!(fallback["sendAttempts"], 0);
    assert_video_upload_scratch_empty(&fixture);
    app.stop();
}

#[test]
fn video_upload_source_replacement_during_thumbnail_transport_prevents_publication() {
    let fixture = Fixture::new();
    video_upload_fixture(&fixture, "replace.mp4", false, false);
    let mut app = start_video_upload_backend(&fixture);
    app.ok(json!({"command":"video_upload_start","owner":"101","source":"replace.mp4","mode":"media","blockThumbnail":true}));
    wait_for_video_thumbnail_transport(&mut app);
    fixture.write(
        "replace.mp4",
        b"another file replaced the source after its preview was prepared",
    );
    app.ok(json!({"command":"video_upload_release"}));
    let result = app.ok(json!({"command":"video_upload_finish"}));
    assert!(
        result["result"]["error"]
            .as_str()
            .unwrap()
            .contains("SOURCE_CHANGED"),
        "{result}"
    );
    assert_eq!(result["thumbnailRequests"], 1);
    assert_eq!(result["sendAttempts"], 0);
    assert_eq!(result["upBytes"], 0);
    assert_eq!(result["pacerPending"], 0);
    assert_video_upload_scratch_empty(&fixture);
    app.stop();
}

// These lifecycle journeys control the external codec process, while the other
// video journeys decode a real H.264 stream with FFmpeg or Windows WinRT.
#[cfg(unix)]
fn video_upload_stalled_codec(fixture: &Fixture) {
    use std::os::unix::fs::PermissionsExt;
    let directory = fixture.path("video-upload-codec");
    std::fs::create_dir(&directory).unwrap();
    let marker = fixture.path("decoder.pid");
    let script=format!("#!/bin/sh\nprintf '%s' \"$$\" > '{}'\nfor output in \"$@\"; do :; done\nprintf 'unfinished preview' > \"$output\"\nexec /bin/sleep 300\n",marker.display());
    let executable = directory.join("ffmpeg");
    std::fs::write(&executable, script).unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    fixture.write(
        ".video-upload-helper-path",
        b"Owned blocking external codec fixture",
    );
}
#[cfg(unix)]
fn wait_for_owned_video_decoder(fixture: &Fixture) -> i32 {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(marker) = std::fs::read_to_string(fixture.path("decoder.pid")) {
            if let Ok(pid) = marker.parse() {
                return pid;
            }
        }
        assert!(
            Instant::now() < deadline,
            "Controlled decoder process did not start"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}
#[cfg(unix)]
fn assert_owned_video_decoder_stopped(fixture: &Fixture, pid: i32) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        // Read-only existence check of the exact process created by this journey.
        let running = unsafe { libc::kill(pid, 0) } == 0;
        let scratch = fixture.path("video-upload-scratch");
        let active_scratch = std::fs::read_dir(&scratch).unwrap().any(|entry| {
            let entry = entry.unwrap();
            entry.file_type().unwrap().is_dir()
                && std::fs::read_dir(entry.path()).unwrap().next().is_some()
        });
        if !running && !active_scratch {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Owned codec process or preview survived cleanup: pid={pid}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_video_upload_scratch_empty(fixture);
}

#[cfg(unix)]
#[test]
fn video_upload_cancels_a_blocked_decoder_process_and_removes_its_plaintext_staging() {
    let fixture = Fixture::new();
    video_upload_fixture(&fixture, "decode-cancel.mp4", false, false);
    video_upload_stalled_codec(&fixture);
    let mut app = start_video_upload_backend(&fixture);
    app.ok(json!({"command":"video_upload_start","owner":"101","source":"decode-cancel.mp4","mode":"media"}));
    let pid = wait_for_owned_video_decoder(&fixture);
    app.ok(json!({"command":"video_upload_cancel"}));
    let result = app.ok(json!({"command":"video_upload_finish"}));
    assert!(
        result["result"]["error"]
            .as_str()
            .unwrap()
            .contains("cancelled"),
        "{result}"
    );
    assert_eq!(result["thumbnailRequests"], 0);
    assert_eq!(result["sendAttempts"], 0);
    assert_eq!(result["upBytes"], 0);
    assert_owned_video_decoder_stopped(&fixture, pid);
    app.stop();
}

#[cfg(unix)]
#[test]
fn video_upload_account_switch_stops_a_blocked_decoder_before_thumbnail_transport() {
    let fixture = Fixture::new();
    video_upload_fixture(&fixture, "decode-account.mp4", false, false);
    video_upload_stalled_codec(&fixture);
    let mut app = start_video_upload_backend(&fixture);
    app.ok(json!({"command":"video_upload_start","owner":"101","source":"decode-account.mp4","mode":"media"}));
    let pid = wait_for_owned_video_decoder(&fixture);
    app.ok(json!({"command":"seed_account","owner":202}));
    let result = app.ok(json!({"command":"video_upload_finish"}));
    assert!(result["result"]["error"].is_string(), "{result}");
    assert_eq!(result["thumbnailRequests"], 0);
    assert_eq!(result["sendAttempts"], 0);
    assert_eq!(result["upBytes"], 0);
    assert_owned_video_decoder_stopped(&fixture, pid);
    app.stop();
}

#[cfg(unix)]
#[test]
fn video_upload_decoder_deadline_kills_the_owned_process_without_publication_or_quota() {
    let fixture = Fixture::new();
    video_upload_fixture(&fixture, "decode-deadline.mp4", false, false);
    video_upload_stalled_codec(&fixture);
    let mut app = start_video_upload_backend(&fixture);
    let started = Instant::now();
    app.ok(json!({"command":"video_upload_start","owner":"101","source":"decode-deadline.mp4","mode":"media"}));
    let pid = wait_for_owned_video_decoder(&fixture);
    let result = app.ok_before(
        json!({"command":"video_upload_finish"}),
        Duration::from_secs(75),
    );
    assert!(
        result["result"]["error"]
            .as_str()
            .unwrap()
            .contains("VIDEO_THUMBNAIL_UNAVAILABLE"),
        "{result}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(70),
        "Decoder exceeded its complete operation bound"
    );
    assert_eq!(result["thumbnailRequests"], 0);
    assert_eq!(result["sendAttempts"], 0);
    assert_eq!(result["upBytes"], 0);
    assert_owned_video_decoder_stopped(&fixture, pid);
    app.stop();
}

#[test]
fn video_upload_near_quota_limit_refuses_preview_admission_before_main_file_upload() {
    let fixture = Fixture::new();
    video_upload_fixture(&fixture, "quota.mp4", false, false);
    let mut app = start_video_upload_backend(&fixture);
    app.ok(json!({"command":"bandwidth_set_limit","bytes":1024}));
    app.ok(json!({"command":"bandwidth_hold","bytes":900,"upload":true}));
    let result = app.ok(
        json!({"command":"video_upload_prepare","owner":"101","source":"quota.mp4","mode":"media"}),
    );
    assert!(
        result["result"]["error"]
            .as_str()
            .unwrap()
            .contains("bandwidth limit"),
        "{result}"
    );
    assert_eq!(result["preparationCompleted"],false,"Preview quota must be admitted during preparation, before handing back a main-file upload plan");
    assert_eq!(result["thumbnailRequests"], 0);
    assert_eq!(result["sendAttempts"], 0);
    assert_eq!(
        result["upBytes"], 900,
        "Failed preview admission must release its own hold while preserving other operations"
    );
    assert_eq!(result["pacerPending"], 0);
    assert_video_upload_scratch_empty(&fixture);
    let stats = app.ok(json!({"command":"bandwidth_cancel"}));
    assert_eq!(stats["up_bytes"], 0);
    app.stop();
}

#[test]
fn video_upload_unused_prepared_preview_admission_is_released_when_main_upload_is_interrupted() {
    for switch_account in [false, true] {
        let fixture = Fixture::new();
        video_upload_fixture(&fixture, "main-boundary.mp4", false, false);
        let mut app = start_video_upload_backend(&fixture);
        app.ok(json!({"command":"video_upload_start","owner":"101","source":"main-boundary.mp4","mode":"media","blockMainUpload":true}));
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let observed = app.ok(json!({"command":"video_upload_observe"}));
            if observed["mainUploadRequests"] == 1 {
                assert_eq!(observed["preparationCompleted"], true);
                assert!(
                    observed["upBytes"].as_u64().unwrap() > 0,
                    "Preview quota must be admitted before the main upload"
                );
                assert_eq!(observed["thumbnailRequests"], 0);
                assert_eq!(observed["sendAttempts"], 0);
                break;
            }
            assert!(
                observed["finished"] != true,
                "Main upload handoff ended prematurely: {observed}"
            );
            assert!(
                Instant::now() < deadline,
                "Main upload was never reached: {observed}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        if switch_account {
            app.ok(json!({"command":"seed_account","owner":202}));
        } else {
            app.ok(json!({"command":"video_upload_cancel"}));
        }
        let result = app.ok(json!({"command":"video_upload_finish"}));
        assert!(result["result"]["error"].is_string(), "{result}");
        assert_eq!(result["mainUploadRequests"], 1);
        assert_eq!(result["thumbnailRequests"], 0);
        assert_eq!(result["sendAttempts"], 0);
        assert_eq!(
            result["upBytes"], 0,
            "Dropping an interrupted upload plan must release unused preview quota"
        );
        assert_eq!(result["pacerPending"], 0);
        assert!(!fixture.path("video-upload-thumbnail.jpg").exists());
        assert_video_upload_scratch_empty(&fixture);
        app.stop();
    }
}

fn retry_upload_fixture(id: &str, owner: &str, status: &str) -> Value {
    json!({
        "id":id,"ownerId":owner,"direction":"upload","kind":"local_upload",
        "status":status,"path":format!("/synthetic/{id}.mp4"),"filename":format!("{id}.mp4"),
        "folderId":91,"protectionMode":"vault_and_passphrase","protectMetadata":true,
        "videoUploadMode":"file","videoFilenameCaption":true,"tempZipPath":"/synthetic/retained.zip",
        "progress":81,"transferredBytes":810,"totalBytes":1000,"speedBytesPerSec":24,
        "error":"synthetic network failure","retryAt":1000,"networkRetries":5,"queuePosition":3,
        "revision":7,"createdAt":1,"updatedAt":2
    })
}

#[test]
fn retry_failed_uploads_preserves_other_states_account_scope_and_durable_inputs() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    for status in [
        "failed",
        "cancelled",
        "paused",
        "waiting_for_unlock",
        "waiting_for_network",
        "cooldown",
        "pending",
        "completed",
    ] {
        app.ok(json!({"command":"save_transfer","job":retry_upload_fixture(status,"101",status)}));
    }
    let mut download = retry_upload_fixture("download", "101", "failed");
    download["direction"] = json!("download");
    download["kind"] = json!("download");
    app.ok(json!({"command":"save_transfer","job":download}));
    app.ok(json!({"command":"save_transfer","job":retry_upload_fixture("foreign","202","failed")}));
    app.ok(json!({"command":"capture_account","owner":"101"}));
    let accepted = app
        .ok(json!({"command":"retry_failed_upload","id":"failed","owner":"101","promptToken":53}));
    assert_eq!(accepted["retried"], true);
    assert_eq!(accepted["job"]["status"], "pending");
    assert_eq!(accepted["job"]["revision"], 8);
    assert_eq!(accepted["job"]["progress"], 0);
    assert_eq!(accepted["job"]["transferredBytes"], 0);
    assert_eq!(accepted["job"]["speedBytesPerSec"], 0);
    assert_eq!(accepted["job"]["networkRetries"], 0);
    assert_eq!(accepted["hasPromptToken"], true);
    let original = retry_upload_fixture("failed", "101", "failed");
    for key in [
        "id",
        "ownerId",
        "path",
        "folderId",
        "filename",
        "protectionMode",
        "protectMetadata",
        "videoUploadMode",
        "videoFilenameCaption",
        "tempZipPath",
        "queuePosition",
        "totalBytes",
        "createdAt",
    ] {
        assert_eq!(accepted["job"][key], original[key], "retry changed {key}");
    }
    let repeated = app
        .ok(json!({"command":"retry_failed_upload","id":"failed","owner":"101","promptToken":99}));
    assert_eq!(repeated["retried"], false);
    assert_eq!(
        repeated["job"]["revision"], 8,
        "a repeated click requeued an accepted item"
    );
    for status in [
        "cancelled",
        "paused",
        "waiting_for_unlock",
        "waiting_for_network",
        "cooldown",
        "pending",
        "completed",
    ] {
        let result = app.ok(
            json!({"command":"retry_failed_upload","id":status,"owner":"101","promptToken":99}),
        );
        assert_eq!(result["retried"], false);
        assert_eq!(result["job"]["status"], status);
        assert_eq!(result["job"]["revision"], 7);
        assert_eq!(result["hasPromptToken"], false);
    }
    let result = app.ok(
        json!({"command":"retry_failed_upload","id":"download","owner":"101","promptToken":99}),
    );
    assert_eq!(result["retried"], false);
    assert_eq!(result["job"]["status"], "failed");
    assert_eq!(result["hasPromptToken"], false);
    assert!(app
        .failure(json!({"command":"retry_failed_upload","id":"foreign","owner":"101"}))
        .contains("ACCOUNT_CHANGED"));
    app.ok(json!({"command":"seed_account","owner":202}));
    assert!(app
        .failure(json!({"command":"retry_failed_upload","id":"foreign","owner":"101"}))
        .contains("ACCOUNT_CHANGED"));
    assert!(app.failure(json!({"command":"retry_failed_upload","id":"foreign","owner":"202","capturedAccount":true})).contains("ACCOUNT_CHANGED"));
    app.stop();
    let raw = sqlite::open(fixture.path("transfers.db")).unwrap();
    let mut query = raw
        .prepare("SELECT payload_json FROM transfer_jobs WHERE id='failed'")
        .unwrap();
    assert_eq!(query.next().unwrap(), sqlite::State::Row);
    let payload = query.read::<String, _>(0).unwrap();
    assert!(
        !payload.contains("promptToken"),
        "a credential handle was persisted"
    );
    drop(query);
    drop(raw);
    let mut app = Backend::start(&fixture.0);
    let persisted = app.ok(json!({"command":"transfers"}));
    let failed = persisted
        .as_array()
        .unwrap()
        .iter()
        .find(|job| job["id"] == "failed")
        .unwrap();
    assert_eq!(failed["status"], "pending");
    assert_eq!(failed["path"], original["path"]);
    app.ok(json!({"command":"seed_account","owner":101}));
    let unchanged = app.ok(json!({"command":"retry_failed_upload","id":"failed","owner":"101"}));
    assert_eq!(unchanged["hasPromptToken"], false);
    app.stop();
}

#[test]
fn retry_failed_uploads_rolls_back_storage_failure_and_recovers_only_pending_metadata() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(
        json!({"command":"save_transfer","job":retry_upload_fixture("rollback","101","failed")}),
    );
    app.ok(json!({"command":"save_transfer","job":retry_upload_fixture("receipt","101","failed")}));
    let connection = sqlite::open(fixture.path("transfers.db")).unwrap();
    connection.execute("CREATE TRIGGER deny_retry BEFORE UPDATE ON transfer_jobs WHEN NEW.id='rollback' AND NEW.status='pending' BEGIN SELECT RAISE(ABORT,'fixture disk failure'); END;").unwrap();
    assert!(app
        .failure(
            json!({"command":"retry_failed_upload","id":"rollback","owner":"101","promptToken":55})
        )
        .contains("PERSISTENCE_ERROR"));
    let snapshot = app.ok(json!({"command":"transfer_retry_snapshot","id":"rollback"}));
    assert_eq!(snapshot["job"]["status"], "failed");
    assert_eq!(snapshot["job"]["revision"], 7);
    assert_eq!(snapshot["hasPromptToken"], false);
    connection
        .execute(concat!(
            "DROP TRIGGER deny_retry; ",
            "CREATE TRIGGER deny_receipt BEFORE UPDATE ON transfer_jobs ",
            "WHEN NEW.id='receipt' AND NEW.status='completed' ",
            "BEGIN SELECT RAISE(ABORT,'fixture checkpoint failure'); END;"
        ))
        .unwrap();
    let mut completed = retry_upload_fixture("receipt", "101", "completed");
    completed["progress"] = json!(100);
    completed["revision"] = json!(8);
    let pending = app.ok(json!({"command":"record_transfer_result","job":completed}));
    assert_eq!(pending["persistencePending"], true);
    connection.execute("DROP TRIGGER deny_receipt").unwrap();
    drop(connection);
    let recovered = app
        .ok(json!({"command":"retry_failed_upload","id":"receipt","owner":"101","promptToken":56}));
    assert_eq!(
        recovered["job"]["status"], "completed",
        "published data must not be enqueued again"
    );
    assert_eq!(recovered["retried"], false);
    assert_eq!(recovered["job"]["persistencePending"], false);
    assert_eq!(recovered["hasPromptToken"], false);
    app.stop();
    let mut app = Backend::start(&fixture.0);
    let persisted = app.ok(json!({"command":"transfers"}));
    let jobs = persisted.as_array().unwrap();
    assert_eq!(
        jobs.iter().find(|job| job["id"] == "receipt").unwrap()["status"],
        "completed"
    );
    assert_eq!(
        jobs.iter().find(|job| job["id"] == "rollback").unwrap()["status"],
        "failed"
    );
    app.stop();
}

#[test]
fn retry_failed_uploads_keeps_a_hundred_item_batch_durable_across_process_restart() {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    for index in 0..100 {
        app.ok(json!({"command":"save_transfer","job":retry_upload_fixture(&format!("batch-{index}"),"101","failed")}));
    }
    let started = Instant::now();
    for index in 0..50 {
        let result = app.ok(json!({"command":"retry_failed_upload","id":format!("batch-{index}"),"owner":"101","promptToken":1000+index}));
        assert_eq!(result["job"]["status"], "pending");
        assert_eq!(result["job"]["revision"], 8);
    }
    app.stop();
    let mut app = Backend::start(&fixture.0);
    // Replaying the accepted half must not create a second queue attempt.
    for index in 0..100 {
        let result = app.ok(json!({"command":"retry_failed_upload","id":format!("batch-{index}"),"owner":"101","promptToken":2000+index}));
        assert_eq!(result["job"]["status"], "pending");
        assert_eq!(result["job"]["revision"], 8);
        assert_eq!(result["retried"], index >= 50);
        assert_eq!(result["hasPromptToken"], index >= 50);
    }
    println!(
        "100 failed-upload queue acceptances plus restart/replay took {:?}",
        started.elapsed()
    );
    app.stop();
    let mut app = Backend::start(&fixture.0);
    let jobs = app.ok(json!({"command":"transfers"}));
    assert_eq!(jobs.as_array().unwrap().len(), 100);
    assert!(jobs
        .as_array()
        .unwrap()
        .iter()
        .all(|job| job["status"] == "pending" && job["revision"] == 8));
    app.stop();
}

fn folder_sync_restart_waits_for_telegram_without_pausing_active_mappings_journey(scanner: &str) {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!({"command":"sync_seed","owner":"101","scanner":scanner}));
    app.stop();

    // A new native process restores actual persisted mappings before the
    // controlled Telegram boundary becomes usable. No user toggles a mapping.
    let mut app = Backend::start(&fixture.0);
    assert_eq!(app.ok(json!({"command":"sync_start"}))["scanner"], scanner);
    let state = app.ok(json!({"command":"sync_status","waitFor":"waiting"}));
    for pair in state["status"]["pairs"].as_array().unwrap() {
        if pair["pairId"] == 3 {
            assert_eq!(pair["phase"], "paused", "{state}");
            assert_eq!(pair["lastError"], "Paused by you", "{state}");
        } else {
            assert_eq!(pair["phase"], "waiting", "{state}");
            assert_eq!(pair["waitReason"], "telegram", "{state}");
            assert!(pair["lastError"].is_null(), "{state}");
        }
    }
    assert_eq!(state["scans"], 0, "Waiting must not read remote trees");
    assert!(app.ok(json!({"command":"sync_telegram","owner":101}))["accepted"] == true);
    let ready = app.ok(json!({"command":"sync_status","waitFor":"ready"}));
    assert_eq!(ready["status"]["activePairs"], 2, "{ready}");
    assert!(
        ready["scans"].as_u64().unwrap() >= 2,
        "Both active mappings reconcile after readiness: {ready}"
    );
    assert_eq!(ready["status"]["pairs"][2]["phase"], "paused", "{ready}");
    assert_eq!(
        fixture.read("mapping-1/keep.txt"),
        b"Keep the existing synced file across startup and reconnect.\n"
    );

    let disconnected = app.ok(json!({"command":"sync_telegram"}));
    let waiting = app.ok(json!({"command":"sync_status","waitFor":"waiting"}));
    assert_eq!(
        waiting["scans"], ready["scans"],
        "Disconnect must not scan remote trees"
    );
    let old_generation = disconnected["generation"].as_u64().unwrap() - 1;
    assert_eq!(
        app.ok(json!({"command":"sync_ready_reply","generation":old_generation,"owner":101}))
            ["accepted"],
        false
    );
    let stale = app.ok(json!({"command":"sync_status","waitFor":"waiting"}));
    assert_eq!(
        stale["scans"], ready["scans"],
        "A stale login reply must not resume sync"
    );
    app.ok(json!({"command":"sync_telegram","owner":101}));
    let reconnected = app.ok(json!({"command":"sync_status","waitFor":"ready"}));
    assert_eq!(
        reconnected["status"]["pairs"][2]["phase"], "paused",
        "{reconnected}"
    );
    for status in reconnected["history"].as_array().unwrap() {
        for pair in status["pairs"].as_array().unwrap() {
            if pair["pairId"] != 3 {
                assert_ne!(
                    pair["phase"], "paused",
                    "Startup and reconnect must never mislabel active mappings: {status}"
                );
            }
        }
    }
    app.ok(json!({"command":"sync_stop"}));
    app.stop();
}

fn folder_sync_accepts_ready_before_start_and_rejects_account_replacement_journey(scanner: &str) {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!({"command":"sync_seed","owner":"101","scanner":scanner}));
    assert_eq!(
        app.ok(json!({"command":"sync_start","readyOwner":101}))["scanner"],
        scanner
    );
    let ready = app.ok(json!({"command":"sync_status","waitFor":"ready"}));
    assert_eq!(ready["status"]["pairs"][0]["phase"], "ready", "{ready}");
    app.ok(json!({"command":"seed_account","owner":202}));
    app.ok(json!({"command":"sync_wake"}));
    let switched = app.ok(json!({"command":"sync_status","waitFor":"error"}));
    assert!(
        switched["status"]["lastError"]
            .as_str()
            .unwrap()
            .starts_with("ACCOUNT_CHANGED:"),
        "{switched}"
    );
    assert_eq!(
        switched["scans"], ready["scans"],
        "Old mappings must not read the new account"
    );
    assert_eq!(
        fixture.read("mapping-1/keep.txt"),
        b"Keep the existing synced file across startup and reconnect.\n"
    );
    app.stop();
}

fn folder_sync_recovers_via_readiness_probe_and_stops_while_waiting_journey(scanner: &str) {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!({"command":"sync_seed","owner":"101","scanner":scanner}));
    assert_eq!(app.ok(json!({"command":"sync_start"}))["scanner"], scanner);
    let waiting = app.ok(json!({"command":"sync_status","waitFor":"waiting"}));
    assert_eq!(waiting["scans"], 0, "{waiting}");
    // The fixture becomes available without publishing a ready notification.
    // Only the production readiness retry can recover before the 30s poll.
    app.ok(json!({"command":"sync_telegram","owner":101,"silent":true}));
    let ready = app.ok(json!({"command":"sync_status","waitFor":"ready"}));
    assert!(ready["probes"].as_u64().unwrap() > 0, "{ready}");
    app.ok(json!({"command":"sync_telegram"}));
    app.ok(json!({"command":"sync_status","waitFor":"waiting"}));
    // A successful incremental pass already set its five-minute idle deadline.
    // A missed reconnect notification must still recover on the short retry.
    app.ok(json!({"command":"sync_telegram","owner":101,"silent":true}));
    let recovered = app.ok(json!({"command":"sync_status","waitFor":"ready"}));
    assert!(
        recovered["scans"].as_u64().unwrap() > ready["scans"].as_u64().unwrap(),
        "{recovered}"
    );
    app.ok(json!({"command":"sync_telegram"}));
    app.ok(json!({"command":"sync_status","waitFor":"waiting"}));
    app.ok(json!({"command":"sync_stop"}));
    app.stop();
}

fn folder_sync_waiting_preserves_pending_remote_cleanup_journal_journey(scanner: &str) {
    let fixture = Fixture::new();
    let mut app = Backend::start(&fixture.0);
    app.ok(json!({"command":"seed_account","owner":101}));
    app.ok(json!({"command":"sync_seed","owner":"101","scanner":scanner}));
    app.ok(json!({"command":"sync_seed_cleanup"}));
    let before = app.ok(json!({"command":"sync_pending_cleanup"}));
    assert_eq!(app.ok(json!({"command":"sync_start"}))["scanner"], scanner);
    let waiting = app.ok(json!({"command":"sync_status","waitFor":"waiting"}));
    assert_eq!(
        waiting["status"]["pairs"][1]["phase"], "waiting",
        "{waiting}"
    );
    assert!(
        waiting["status"]["pairs"][1]["lastError"].is_null(),
        "{waiting}"
    );
    assert_eq!(app.ok(json!({"command":"sync_pending_cleanup"})), before);
    assert_eq!(waiting["scans"], 0, "{waiting}");
    app.stop();
}

#[test]
fn folder_sync_full_restart_waits_for_telegram_without_pausing_active_mappings() {
    folder_sync_restart_waits_for_telegram_without_pausing_active_mappings_journey("full");
}

#[test]
fn folder_sync_incremental_restart_waits_for_telegram_without_pausing_active_mappings() {
    folder_sync_restart_waits_for_telegram_without_pausing_active_mappings_journey("incremental");
}

#[test]
fn folder_sync_full_accepts_ready_before_start_and_rejects_account_replacement() {
    folder_sync_accepts_ready_before_start_and_rejects_account_replacement_journey("full");
}

#[test]
fn folder_sync_incremental_accepts_ready_before_start_and_rejects_account_replacement() {
    folder_sync_accepts_ready_before_start_and_rejects_account_replacement_journey("incremental");
}

#[test]
fn folder_sync_full_recovers_via_readiness_probe_and_stops_while_waiting() {
    folder_sync_recovers_via_readiness_probe_and_stops_while_waiting_journey("full");
}

#[test]
fn folder_sync_incremental_recovers_via_readiness_probe_and_stops_while_waiting() {
    folder_sync_recovers_via_readiness_probe_and_stops_while_waiting_journey("incremental");
}

#[test]
fn folder_sync_full_waiting_preserves_pending_remote_cleanup_journal() {
    folder_sync_waiting_preserves_pending_remote_cleanup_journal_journey("full");
}

#[test]
fn folder_sync_incremental_waiting_preserves_pending_remote_cleanup_journal() {
    folder_sync_waiting_preserves_pending_remote_cleanup_journal_journey("incremental");
}
