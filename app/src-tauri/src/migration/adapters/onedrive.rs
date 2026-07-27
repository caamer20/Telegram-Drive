use crate::migration::db::MigrationDb;
use crate::migration::events::{emit_item_progress, now_millis, ItemProgressPayload};
use crate::migration::microsoft::{self, MicrosoftSession};
use crate::migration::pipeline::stages::SourceDownloader;

use reqwest::Client;
use sha2::Digest;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex as TokioMutex;

pub struct OneDriveDownloader {
    http: Client,
    ms_session: Arc<TokioMutex<Option<MicrosoftSession>>>,
    db: MigrationDb,
    base_url: String,
    cancel_token: tokio_util::sync::CancellationToken,
    app_handle: Option<tauri::AppHandle>,
}

impl OneDriveDownloader {
    pub fn new(
        http: Client,
        ms_session: Arc<TokioMutex<Option<MicrosoftSession>>>,
        db: MigrationDb,
        cancel_token: tokio_util::sync::CancellationToken,
        app_handle: Option<tauri::AppHandle>,
    ) -> Self {
        Self {
            http,
            ms_session,
            db,
            base_url: "https://graph.microsoft.com".to_string(),
            cancel_token,
            app_handle,
        }
    }

    pub fn new_with_base_url(
        http: Client,
        ms_session: Arc<TokioMutex<Option<MicrosoftSession>>>,
        db: MigrationDb,
        base_url: String,
    ) -> Self {
        Self {
            http,
            ms_session,
            db,
            base_url,
            cancel_token: tokio_util::sync::CancellationToken::new(),
            app_handle: None,
        }
    }

    #[cfg(test)]
    pub fn new_with_base_url_and_cancel(
        http: Client,
        ms_session: Arc<TokioMutex<Option<MicrosoftSession>>>,
        db: MigrationDb,
        base_url: String,
        cancel_token: tokio_util::sync::CancellationToken,
    ) -> Self {
        Self {
            http,
            ms_session,
            db,
            base_url,
            cancel_token,
            app_handle: None,
        }
    }
}

impl SourceDownloader for OneDriveDownloader {
    fn download_file(
        &self,
        item_id: i64,
        source_item_id: &str,
        dest_path: &Path,
    ) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>> {
        let http = self.http.clone();
        let ms_session = self.ms_session.clone();
        let db = self.db.clone();
        let source_item_id = source_item_id.to_string();
        let dest_path = dest_path.to_path_buf();
        let base_url = self.base_url.clone();
        let cancel = self.cancel_token.clone();
        let app_handle = self.app_handle.clone();

        Box::pin(async move {
            let remove_partial = || async {
                match tokio::fs::remove_file(&dest_path).await {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => log::warn!(
                        "Download: failed to remove partial artifact {:?}: {}",
                        dest_path,
                        error
                    ),
                }
            };

            let (job_id, item_name) = {
                let conn = db.lock().map_err(|error| error.to_string())?;
                let mut stmt = conn
                    .prepare("SELECT job_id, name FROM migration_items WHERE id = ? LIMIT 1")
                    .map_err(|error| error.to_string())?;
                stmt.bind((1, item_id)).map_err(|error| error.to_string())?;
                if let Ok(sqlite::State::Row) = stmt.next() {
                    (
                        stmt.read::<i64, _>(0).unwrap_or(0),
                        stmt.read::<String, _>(1).unwrap_or_default(),
                    )
                } else {
                    return Err("Download: migration item not found".to_string());
                }
            };

            // 1. Refresh token if expired
            let access_token = tokio::select! {
                _ = cancel.cancelled() => {
                    remove_partial().await;
                    return Err("Download: cancelled".to_string());
                }
                result = async {
                    let mut session_guard = ms_session.lock().await;
                    if let Some(ref mut session) = *session_guard {
                        if session.is_expired() {
                            microsoft::refresh_access_token(session)
                                .await
                                .map_err(|e| format!("Authentication: failed to refresh token: {}", e))?;
                        }
                        Ok(session.access_token.clone())
                    } else {
                        Err("Authentication: No active Microsoft session".to_string())
                    }
                } => result?,
            };

            let item_url = format!("{}/v1.0/me/drive/items/{}", base_url, source_item_id);
            let resp = tokio::select! {
                _ = cancel.cancelled() => {
                    remove_partial().await;
                    return Err("Download: cancelled".to_string());
                }
                result = http.get(&item_url).bearer_auth(&access_token).send() => {
                    result.map_err(|e| format!("TransientNetwork: {}", e))?
                }
            };

            let status = resp.status();
            if status == reqwest::StatusCode::UNAUTHORIZED {
                return Err("Authentication: Unauthorized".to_string());
            } else if status == reqwest::StatusCode::FORBIDDEN {
                return Err("PermissionDenied: Forbidden".to_string());
            } else if status == reqwest::StatusCode::NOT_FOUND {
                return Err("SourceNotFound: Item not found".to_string());
            } else if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                return Err("RateLimited: Too many requests".to_string());
            } else if !status.is_success() {
                return Err(format!("InvalidResponse: HTTP {}", status));
            }

            let json: serde_json::Value = tokio::select! {
                _ = cancel.cancelled() => {
                    remove_partial().await;
                    return Err("Download: cancelled".to_string());
                }
                result = resp.json() => {
                    result.map_err(|e| format!("InvalidResponse: {}", e))?
                }
            };

            let download_url = match json["@microsoft.graph.downloadUrl"].as_str() {
                Some(url) => url.to_string(),
                None => return Err("InvalidResponse: Missing download URL".to_string()),
            };

            // 3. Download the file streamingly
            let mut stream_resp = tokio::select! {
                _ = cancel.cancelled() => {
                    remove_partial().await;
                    return Err("Download: cancelled".to_string());
                }
                result = http.get(&download_url).send() => {
                    result.map_err(|e| format!("TransientNetwork: {}", e))?
                }
            };

            if !stream_resp.status().is_success() {
                return Err(format!("InvalidResponse: HTTP {}", stream_resp.status()));
            }

            let total_bytes = stream_resp.content_length().unwrap_or(0);
            let mut file = match tokio::fs::File::create(&dest_path).await {
                Ok(f) => f,
                Err(e) => return Err(format!("Local filesystem error: {}", e)),
            };

            let mut hasher = sha2::Sha256::new();
            let mut downloaded_bytes = 0u64;
            let started_at = std::time::Instant::now();
            let mut last_emit = std::time::Instant::now()
                .checked_sub(std::time::Duration::from_millis(250))
                .unwrap_or_else(std::time::Instant::now);

            loop {
                let chunk = tokio::select! {
                    _ = cancel.cancelled() => {
                        drop(file);
                        remove_partial().await;
                        return Err("Download: cancelled".to_string());
                    }
                    result = stream_resp.chunk() => {
                        result.map_err(|e| format!("TransientNetwork: chunk error: {}", e))?
                    }
                };
                let Some(chunk) = chunk else { break };
                tokio::select! {
                    _ = cancel.cancelled() => {
                        drop(file);
                        remove_partial().await;
                        return Err("Download: cancelled".to_string());
                    }
                    result = file.write_all(&chunk) => {
                        result.map_err(|e| format!("Local filesystem error: {}", e))?;
                    }
                }
                sha2::Digest::update(&mut hasher, &chunk);
                downloaded_bytes += chunk.len() as u64;

                if last_emit.elapsed() >= std::time::Duration::from_millis(250) {
                    if let Some(ref app) = app_handle {
                        let elapsed = started_at.elapsed().as_secs_f64().max(0.001);
                        emit_item_progress(
                            app,
                            ItemProgressPayload {
                                job_id,
                                item_id,
                                item_name: item_name.clone(),
                                phase: "downloading".to_string(),
                                percent: if total_bytes > 0 {
                                    downloaded_bytes as f64 * 100.0 / total_bytes as f64
                                } else {
                                    0.0
                                },
                                bytes_done: downloaded_bytes,
                                bytes_total: total_bytes,
                                speed_bytes_per_sec: downloaded_bytes as f64 / elapsed,
                                timestamp: now_millis(),
                            },
                        );
                    }
                    last_emit = std::time::Instant::now();
                }
            }

            tokio::select! {
                _ = cancel.cancelled() => {
                    drop(file);
                    remove_partial().await;
                    return Err("Download: cancelled".to_string());
                }
                result = file.flush() => {
                    result.map_err(|e| format!("Local filesystem error: {}", e))?;
                }
            }

            if let Some(ref app) = app_handle {
                let elapsed = started_at.elapsed().as_secs_f64().max(0.001);
                emit_item_progress(
                    app,
                    ItemProgressPayload {
                        job_id,
                        item_id,
                        item_name,
                        phase: "downloading".to_string(),
                        percent: 100.0,
                        bytes_done: downloaded_bytes,
                        bytes_total: total_bytes.max(downloaded_bytes),
                        speed_bytes_per_sec: downloaded_bytes as f64 / elapsed,
                        timestamp: now_millis(),
                    },
                );
            }

            let hex_hash = format!("{:x}", hasher.finalize());
            Ok(hex_hash)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::db::open_migration_db_at_path;
    use crate::migration::models::MsAccountInfo;
    use std::path::PathBuf;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("temp_onedrive_tests_{}", rand::random::<u64>()));
            std::fs::create_dir_all(&path).unwrap();
            Self { path }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    #[tokio::test]
    async fn test_onedrive_download_success() {
        let mock_server = MockServer::start().await;
        let tmp = TempDir::new();
        let db_path = tmp.path.join("test_od.db");
        let db = open_migration_db_at_path(db_path).unwrap();

        // Seed item
        {
            let conn = db.lock().unwrap();
            conn.execute("INSERT INTO migration_jobs (id, source_folder_id, source_folder_path, telegram_destination_id, telegram_destination_name, local_backup_dir, workspace_dir, state, started_at, created_at, updated_at) VALUES (1, 'src', 'path', 'tg', 'tg', 'loc', 'ws', 'running', 0, 0, 0);").unwrap();
            conn.execute("INSERT INTO migration_items (id, job_id, folder_id, name, path, source_item_id, size, item_category, pipeline_stage, created_at, updated_at)
                          VALUES (1, 1, 'f', 'test.txt', 'test.txt', 'item123', 100, 'file', 'discovered', 0, 0);").unwrap();
        }

        let mock_metadata = serde_json::json!({
            "id": "item123",
            "name": "test.txt",
            "file": {
                "hashes": {
                    "quickXorHash": "fake_quickxor",
                    "sha1Hash": "fake_sha1"
                }
            },
            "@microsoft.graph.downloadUrl": format!("{}/download", mock_server.uri())
        });

        Mock::given(method("GET"))
            .and(path("/v1.0/me/drive/items/item123"))
            .and(header("Authorization", "Bearer fake_token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_metadata))
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/download"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(b"hello world", "text/plain"))
            .mount(&mock_server)
            .await;

        let session = MicrosoftSession {
            client_id: "client123".to_string(),
            access_token: "fake_token".to_string(),
            refresh_token: "ref_token".to_string(),
            expires_at: chrono::Utc::now().timestamp() + 3600,
            tenant: "common".to_string(),
            redirect_uri: "http://redirect".to_string(),
            account_info: MsAccountInfo {
                account_name: "Test".to_string(),
                account_email: "test@mail.com".to_string(),
            },
        };

        let downloader = OneDriveDownloader::new_with_base_url(
            Client::new(),
            Arc::new(TokioMutex::new(Some(session))),
            db.clone(),
            mock_server.uri(),
        );

        let dest = tmp.path.join("download.txt");
        let hash = downloader.download_file(1, "item123", &dest).await.unwrap();

        assert_eq!(
            hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        ); // SHA-256 of "hello world"
        assert!(dest.exists());
        let content = std::fs::read_to_string(&dest).unwrap();
        assert_eq!(content, "hello world");
    }

    #[tokio::test]
    async fn test_onedrive_download_errors() {
        let mock_server = MockServer::start().await;
        let tmp = TempDir::new();
        let db_path = tmp.path.join("test_od_err.db");
        let db = open_migration_db_at_path(db_path).unwrap();
        {
            let conn = db.lock().unwrap();
            conn.execute("INSERT INTO migration_jobs (id, source_folder_id, source_folder_path, telegram_destination_id, telegram_destination_name, local_backup_dir, workspace_dir, state, started_at, created_at, updated_at) VALUES (1, 'src', 'path', 'tg', 'tg', 'loc', 'ws', 'running', 0, 0, 0);").unwrap();
            conn.execute("INSERT INTO migration_items (id, job_id, folder_id, name, path, source_item_id, size, item_category, pipeline_stage, created_at, updated_at) VALUES (1, 1, 'f', 'test.txt', 'test.txt', 'missing', 100, 'file', 'queued_download', 0, 0);").unwrap();
        }

        let session = MicrosoftSession {
            client_id: "client123".to_string(),
            access_token: "fake_token".to_string(),
            refresh_token: "ref_token".to_string(),
            expires_at: chrono::Utc::now().timestamp() + 3600,
            tenant: "common".to_string(),
            redirect_uri: "http://redirect".to_string(),
            account_info: MsAccountInfo {
                account_name: "Test".to_string(),
                account_email: "test@mail.com".to_string(),
            },
        };

        let downloader = OneDriveDownloader::new_with_base_url(
            Client::new(),
            Arc::new(TokioMutex::new(Some(session))),
            db.clone(),
            mock_server.uri(),
        );

        // 404 test
        Mock::given(method("GET"))
            .and(path("/v1.0/me/drive/items/missing"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&mock_server)
            .await;

        let dest = tmp.path.join("missing.txt");
        let err = downloader
            .download_file(1, "missing", &dest)
            .await
            .unwrap_err();
        assert!(err.contains("SourceNotFound"));

        // 429 test
        Mock::given(method("GET"))
            .and(path("/v1.0/me/drive/items/throttled"))
            .respond_with(ResponseTemplate::new(429))
            .mount(&mock_server)
            .await;

        let err = downloader
            .download_file(1, "throttled", &dest)
            .await
            .unwrap_err();
        assert!(err.contains("RateLimited"));
    }

    #[tokio::test]
    async fn test_cancel_during_active_download_removes_partial_file() {
        let mock_server = MockServer::start().await;
        let tmp = TempDir::new();
        let db = open_migration_db_at_path(tmp.path.join("test_od_cancel.db")).unwrap();
        {
            let conn = db.lock().unwrap();
            conn.execute("INSERT INTO migration_jobs (id, source_folder_id, source_folder_path, telegram_destination_id, telegram_destination_name, local_backup_dir, workspace_dir, state, started_at, created_at, updated_at) VALUES (1, 'src', 'path', 'tg', 'tg', 'loc', 'ws', 'running', 0, 0, 0);").unwrap();
            conn.execute("INSERT INTO migration_items (id, job_id, folder_id, name, path, source_item_id, size, item_category, pipeline_stage, created_at, updated_at) VALUES (1, 1, 'f', 'slow.bin', 'slow.bin', 'slow', 100, 'file', 'downloading', 0, 0);").unwrap();
        }
        let session = MicrosoftSession {
            client_id: "client123".to_string(),
            access_token: "fake_token".to_string(),
            refresh_token: "ref_token".to_string(),
            expires_at: chrono::Utc::now().timestamp() + 3600,
            tenant: "common".to_string(),
            redirect_uri: "http://redirect".to_string(),
            account_info: MsAccountInfo {
                account_name: "Test".to_string(),
                account_email: "test@mail.com".to_string(),
            },
        };
        Mock::given(method("GET"))
            .and(path("/v1.0/me/drive/items/slow"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "@microsoft.graph.downloadUrl": format!("{}/slow-download", mock_server.uri())
            })))
            .mount(&mock_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/slow-download"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(std::time::Duration::from_secs(10))
                    .set_body_raw(vec![7u8; 1024], "application/octet-stream"),
            )
            .mount(&mock_server)
            .await;

        let cancel = tokio_util::sync::CancellationToken::new();
        let downloader = OneDriveDownloader::new_with_base_url_and_cancel(
            Client::new(),
            Arc::new(TokioMutex::new(Some(session))),
            db,
            mock_server.uri(),
            cancel.clone(),
        );
        let dest = tmp.path.join("slow.part");
        let download = downloader.download_file(1, "slow", &dest);
        let result = tokio::time::timeout(std::time::Duration::from_secs(2), async move {
            tokio::join!(download, async move {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                cancel.cancel();
            })
            .0
        })
        .await
        .expect("download request must stop promptly");
        assert!(result.unwrap_err().contains("cancelled"));
        assert!(!dest.exists());
    }
}
