use crate::migration::db::MigrationDb;
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
}

impl OneDriveDownloader {
    pub fn new(
        http: Client,
        ms_session: Arc<TokioMutex<Option<MicrosoftSession>>>,
        db: MigrationDb,
    ) -> Self {
        Self {
            http,
            ms_session,
            db,
            base_url: "https://graph.microsoft.com".to_string(),
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
        let _db = self.db.clone();
        let source_item_id = source_item_id.to_string();
        let dest_path = dest_path.to_path_buf();
        let base_url = self.base_url.clone();

        Box::pin(async move {
            // 1. Refresh token if expired
            let access_token = {
                let mut session_guard = ms_session.lock().await;
                if let Some(ref mut session) = *session_guard {
                    if session.is_expired() {
                        if let Err(e) = microsoft::refresh_access_token(session).await {
                            return Err(format!("Authentication: failed to refresh token: {}", e));
                        }
                    }
                    session.access_token.clone()
                } else {
                    return Err("Authentication: No active Microsoft session".to_string());
                }
            };

            // 2. Fetch metadata from OneDrive
            let item_url = format!("{}/v1.0/me/drive/items/{}", base_url, source_item_id);
            let resp = match http.get(&item_url).bearer_auth(&access_token).send().await {
                Ok(r) => r,
                Err(e) => return Err(format!("TransientNetwork: {}", e)),
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

            let json: serde_json::Value = match resp.json().await {
                Ok(j) => j,
                Err(e) => return Err(format!("InvalidResponse: {}", e)),
            };

            let download_url = match json["@microsoft.graph.downloadUrl"].as_str() {
                Some(url) => url.to_string(),
                None => return Err("InvalidResponse: Missing download URL".to_string()),
            };

            // 3. Download the file streamingly
            let mut stream_resp = match http.get(&download_url).send().await {
                Ok(r) => r,
                Err(e) => return Err(format!("TransientNetwork: {}", e)),
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

            while let Some(chunk) = match stream_resp.chunk().await {
                Ok(c) => c,
                Err(e) => return Err(format!("TransientNetwork: chunk error: {}", e)),
            } {
                if let Err(e) = file.write_all(&chunk).await {
                    return Err(format!("Local filesystem error: {}", e));
                }
                sha2::Digest::update(&mut hasher, &chunk);
                downloaded_bytes += chunk.len() as u64;

                log::info!(
                    "Download progress for item {}: {}/{}",
                    item_id,
                    downloaded_bytes,
                    total_bytes
                );
            }

            if let Err(e) = file.flush().await {
                return Err(format!("Local filesystem error: {}", e));
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
    use std::fs;
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
}
