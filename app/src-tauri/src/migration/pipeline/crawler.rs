use crate::migration::db::MigrationDb;
use crate::migration::microsoft::parse_graph_item;
use crate::migration::microsoft::MicrosoftSession;
use crate::migration::models::FolderQueueItem;
use crate::migration::pipeline::config::PipelineConfig;
use crate::migration::pipeline::stages::PipelineItem;
use chrono::Utc;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::Mutex as TokioMutex;
use tokio_util::sync::CancellationToken;

pub struct StreamingCrawler {
    pub db: MigrationDb,
    pub job_id: i64,
    pub ms_session: Arc<TokioMutex<Option<MicrosoftSession>>>,
    pub cancel_token: CancellationToken,
    pub config: PipelineConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GraphErrorKind {
    Fatal,
    Transient,
}

#[derive(Debug)]
struct GraphRequestError {
    kind: GraphErrorKind,
    message: String,
}

async fn request_graph_page(
    http: &reqwest::Client,
    url: &str,
    access_token: &str,
) -> Result<serde_json::Value, GraphRequestError> {
    let response = http
        .get(url)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|error| GraphRequestError {
            kind: GraphErrorKind::Transient,
            message: format!("Graph API request failed: {}", error),
        })?;
    let status = response.status();
    let body = response.text().await.map_err(|error| GraphRequestError {
        kind: GraphErrorKind::Transient,
        message: format!("Failed to read Graph API body: {}", error),
    })?;
    if status.is_success() {
        return serde_json::from_str(&body).map_err(|error| GraphRequestError {
            kind: GraphErrorKind::Transient,
            message: format!("Failed to parse Graph API JSON: {}", error),
        });
    }
    let kind = if status == reqwest::StatusCode::REQUEST_TIMEOUT
        || status == reqwest::StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
    {
        GraphErrorKind::Transient
    } else {
        GraphErrorKind::Fatal
    };
    Err(GraphRequestError {
        kind,
        message: format!("Graph API {}: {}", status, body),
    })
}

impl StreamingCrawler {
    pub async fn run(self: Arc<Self>, tx: mpsc::Sender<PipelineItem>) -> Result<(), String> {
        loop {
            if self.cancel_token.is_cancelled() {
                break;
            }

            // 1. Lấy một folder pending hoặc fetching từ database
            let folder_queue_item = self.get_next_folder()?;

            match folder_queue_item {
                Some(folder) => {
                    if let Err(e) = self.process_folder(folder, &tx).await {
                        if self.cancel_token.is_cancelled() {
                            break;
                        }
                        log::error!("Crawler failed to process folder: {}", e);
                        return Err(e);
                    }
                }
                None => {
                    // Nếu không còn folder nào pending, kiểm tra xem còn active file processing không.
                    // Nếu crawler đã cạn folder, nó có thể ngủ dài hơn chờ pipeline finish hoặc kết thúc.
                    log::info!("Crawler: No more folders to process.");
                    // We just break out of crawler loop. The channel `tx` is dropped, which cascades EOF to pipeline.
                    break;
                }
            }
        }

        log::debug!("Crawler loop exited for job {}", self.job_id);
        Ok(())
    }

    fn get_next_folder(&self) -> Result<Option<FolderQueueItem>, String> {
        let conn = self.db.lock().map_err(|e| e.to_string())?;

        let mut stmt = conn.prepare(
            "SELECT id, job_id, folder_id, parent_id, folder_path, state, next_page_link, has_more, discovered_files_count, discovered_folders_count, completed_files_count, last_error, created_at, updated_at
             FROM folder_queue
             WHERE job_id = ? AND state IN ('pending', 'fetching') AND has_more = 1
             ORDER BY id ASC LIMIT 1;"
        ).map_err(|e| e.to_string())?;

        stmt.bind((1, self.job_id)).map_err(|e| e.to_string())?;

        if let Ok(sqlite::State::Row) = stmt.next() {
            Ok(Some(FolderQueueItem {
                id: stmt.read(0).unwrap_or(0),
                job_id: stmt.read(1).unwrap_or(0),
                folder_id: stmt.read(2).unwrap_or_default(),
                parent_id: stmt.read(3).unwrap_or(None),
                folder_path: stmt.read(4).unwrap_or_default(),
                state: stmt.read(5).unwrap_or_default(),
                next_page_link: stmt.read(6).unwrap_or(None),
                has_more: stmt.read(7).unwrap_or(0) == 1,
                discovered_files_count: stmt.read(8).unwrap_or(0),
                discovered_folders_count: stmt.read(9).unwrap_or(0),
                completed_files_count: stmt.read(10).unwrap_or(0),
                last_error: stmt.read(11).unwrap_or(None),
                created_at: stmt.read(12).unwrap_or(0),
                updated_at: stmt.read(13).unwrap_or(0),
            }))
        } else {
            Ok(None)
        }
    }

    fn mark_folder_failed(&self, folder_id: i64, error: &str) -> Result<(), String> {
        let conn = self.db.lock().map_err(|value| value.to_string())?;
        let mut stmt = conn
            .prepare(
                "UPDATE folder_queue SET state = 'failed', has_more = 0, last_error = ?, updated_at = ? WHERE id = ?",
            )
            .map_err(|value| value.to_string())?;
        stmt.bind((1, error)).map_err(|value| value.to_string())?;
        stmt.bind((2, Utc::now().timestamp()))
            .map_err(|value| value.to_string())?;
        stmt.bind((3, folder_id))
            .map_err(|value| value.to_string())?;
        stmt.next().map_err(|value| value.to_string())?;
        Ok(())
    }

    async fn fetch_graph_page(
        &self,
        folder: &FolderQueueItem,
        http: &reqwest::Client,
        request_url: &str,
        access_token: &str,
    ) -> Result<Option<serde_json::Value>, String> {
        let mut retries = 0u32;
        loop {
            let result = tokio::select! {
                result = request_graph_page(http, request_url, access_token) => result,
                _ = self.cancel_token.cancelled() => return Ok(None),
            };
            match result {
                Ok(json) => return Ok(Some(json)),
                Err(error) if error.kind == GraphErrorKind::Fatal => {
                    self.mark_folder_failed(folder.id, &error.message)?;
                    return Err(error.message);
                }
                Err(error) if retries < self.config.max_network_retries => {
                    let delay = self
                        .config
                        .retry_delay_secs
                        .get(retries as usize)
                        .copied()
                        .or_else(|| self.config.retry_delay_secs.last().copied())
                        .unwrap_or(0);
                    retries += 1;
                    log::warn!(
                        "Crawler transient Graph error for folder {} (retry {}/{} in {}s): {}",
                        folder.folder_id,
                        retries,
                        self.config.max_network_retries,
                        delay,
                        error.message
                    );
                    tokio::select! {
                        _ = self.cancel_token.cancelled() => return Ok(None),
                        _ = tokio::time::sleep(tokio::time::Duration::from_secs(delay)) => {}
                    }
                }
                Err(error) => {
                    let message = format!(
                        "Graph retries exhausted after {} retries: {}",
                        self.config.max_network_retries, error.message
                    );
                    self.mark_folder_failed(folder.id, &message)?;
                    if folder.parent_id.is_none() {
                        return Err(message);
                    }
                    log::error!(
                        "Crawler skipped failed child folder {}: {}",
                        folder.folder_id,
                        message
                    );
                    return Ok(None);
                }
            }
        }
    }

    async fn process_folder(
        &self,
        folder: FolderQueueItem,
        tx: &mpsc::Sender<PipelineItem>,
    ) -> Result<(), String> {
        // Cập nhật state sang 'fetching'
        {
            let conn = self.db.lock().map_err(|e| e.to_string())?;
            let mut upd = conn
                .prepare("UPDATE folder_queue SET state = 'fetching' WHERE id = ?;")
                .unwrap();
            upd.bind((1, folder.id)).unwrap();
            upd.next().unwrap();
        }

        let access_token_result: Result<String, String> = {
            let mut guard = self.ms_session.lock().await;
            if let Some(ref mut session) = *guard {
                if session.is_expired() {
                    if let Err(error) =
                        crate::migration::microsoft::refresh_access_token(session).await
                    {
                        Err(format!("Microsoft token refresh failed: {}", error))
                    } else {
                        Ok(session.access_token.clone())
                    }
                } else {
                    Ok(session.access_token.clone())
                }
            } else {
                Err("Microsoft account not connected".into())
            }
        };
        let access_token = match access_token_result {
            Ok(token) => token,
            Err(error) => {
                self.mark_folder_failed(folder.id, &error)?;
                return Err(error);
            }
        };

        let request_url = if let Some(link) = &folder.next_page_link {
            link.clone()
        } else if folder.folder_id == "root" {
            "https://graph.microsoft.com/v1.0/me/drive/root/children?$top=200".to_string()
        } else {
            format!(
                "https://graph.microsoft.com/v1.0/me/drive/items/{}/children?$top=200",
                folder.folder_id
            )
        };
        let http = reqwest::Client::new();

        let Some(json) = self
            .fetch_graph_page(&folder, &http, &request_url, &access_token)
            .await?
        else {
            return Ok(());
        };

        let mut files_to_insert = Vec::new();
        let mut folders_to_insert = Vec::new();

        if let Some(arr) = json["value"].as_array() {
            for val in arr {
                let parsed = parse_graph_item(val, &folder.folder_path);
                if parsed.item_type == "folder" {
                    folders_to_insert.push(parsed);
                } else {
                    files_to_insert.push(parsed);
                }
            }
        }

        let next_link = json["@odata.nextLink"].as_str().map(ToString::to_string);
        let has_more = next_link.is_some();
        let state = if has_more { "fetching" } else { "completed" };

        let now = Utc::now().timestamp();

        let mut db_pipeline_items = Vec::new();

        // Transaction insert
        {
            let conn = self.db.lock().map_err(|e| e.to_string())?;
            conn.execute("BEGIN TRANSACTION;")
                .map_err(|e| e.to_string())?;

            struct TransactionGuard<'a>(&'a sqlite::Connection, bool);
            impl<'a> Drop for TransactionGuard<'a> {
                fn drop(&mut self) {
                    if !self.1 {
                        let _ = self.0.execute("ROLLBACK;");
                    }
                }
            }
            let mut guard = TransactionGuard(&conn, false);

            let mut inserted_folder_count = 0i64;
            let mut inserted_file_count = 0i64;

            // 1. Insert child folders. RETURNING only yields rows inserted by this transaction.
            let mut stmt_folder = conn
                .prepare(
                    "INSERT INTO folder_queue (
                    job_id, folder_id, parent_id, folder_path, state, created_at, updated_at
                ) VALUES (?, ?, ?, ?, 'pending', ?, ?)
                  ON CONFLICT(job_id, folder_id) DO NOTHING
                  RETURNING id;",
                )
                .map_err(|e| e.to_string())?;

            for child_folder in &folders_to_insert {
                let child_path = if folder.folder_path.is_empty() || folder.folder_path == "/" {
                    child_folder.name.clone()
                } else {
                    format!("{}/{}", folder.folder_path, child_folder.name)
                };

                stmt_folder.bind((1, self.job_id)).unwrap();
                stmt_folder.bind((2, child_folder.id.as_str())).unwrap();
                stmt_folder.bind((3, folder.folder_id.as_str())).unwrap();
                stmt_folder.bind((4, child_path.as_str())).unwrap();
                stmt_folder.bind((5, now)).unwrap();
                stmt_folder.bind((6, now)).unwrap();
                if matches!(stmt_folder.next(), Ok(sqlite::State::Row)) {
                    inserted_folder_count += 1;
                }
                stmt_folder.reset().map_err(|error| error.to_string())?;
            }

            // 2. Insert files and enqueue only rows inserted here.
            let mut stmt_file = conn.prepare(
                "INSERT INTO migration_items (
                    job_id, folder_id, source_item_id, name, path, size, item_category, pipeline_stage, created_at, updated_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?, 'queued_download', ?, ?)
                  ON CONFLICT(job_id, source_item_id) DO NOTHING
                  RETURNING id;"
            ).map_err(|e| e.to_string())?;

            for child_file in &files_to_insert {
                let child_path = if folder.folder_path.is_empty() || folder.folder_path == "/" {
                    child_file.name.clone()
                } else {
                    format!("{}/{}", folder.folder_path, child_file.name)
                };

                stmt_file.bind((1, self.job_id)).unwrap();
                stmt_file.bind((2, folder.folder_id.as_str())).unwrap();
                stmt_file.bind((3, child_file.id.as_str())).unwrap();
                stmt_file.bind((4, child_file.name.as_str())).unwrap();
                stmt_file.bind((5, child_path.as_str())).unwrap();
                stmt_file.bind((6, child_file.size)).unwrap();
                stmt_file.bind((7, child_file.item_type.as_str())).unwrap();
                stmt_file.bind((8, now)).unwrap();
                stmt_file.bind((9, now)).unwrap();

                if let Ok(sqlite::State::Row) = stmt_file.next() {
                    let item_id = stmt_file
                        .read::<i64, _>(0)
                        .map_err(|error| error.to_string())?;
                    inserted_file_count += 1;
                    db_pipeline_items.push(PipelineItem {
                        id: item_id,
                        job_id: self.job_id,
                        name: child_file.name.clone(),
                        source_path: child_path,
                        source_item_id: Some(child_file.id.clone()),
                        size_bytes: child_file.size,
                        source_etag: child_file.etag.clone(),
                        source_last_modified: child_file.last_modified.clone(),
                        source_fingerprint_type: None,
                        source_fingerprint_value: None,
                        state: "queued_download".to_string(),
                        original_sha256: None,
                        processed_sha256: None,
                        local_artifact_path: None,
                        processed_artifact_path: None,
                        telegram_random_id: None,
                        video_decision: None,
                        retry_count: 0,
                    });
                }
                stmt_file.reset().map_err(|error| error.to_string())?;
            }

            // 3. Cập nhật thống kê job
            let mut upd_job = conn
                .prepare(
                    "UPDATE migration_jobs 
                 SET discovered_folders = discovered_folders + ?, 
                     discovered_items = discovered_items + ?, 
                     waiting_items = waiting_items + ?
                 WHERE id = ?;",
                )
                .unwrap();
            upd_job.bind((1, inserted_folder_count)).unwrap();
            upd_job.bind((2, inserted_file_count)).unwrap();
            upd_job.bind((3, inserted_file_count)).unwrap();
            upd_job.bind((4, self.job_id)).unwrap();
            upd_job.next().unwrap();

            // 4. Update folder state
            let mut upd_folder = conn
                .prepare(
                    "UPDATE folder_queue 
                 SET state = ?, 
                     next_page_link = ?, 
                     has_more = ?, 
                     discovered_files_count = discovered_files_count + ?, 
                     discovered_folders_count = discovered_folders_count + ?,
                     last_error = NULL,
                     updated_at = ?
                 WHERE id = ?;",
                )
                .unwrap();
            upd_folder.bind((1, state)).unwrap();
            match next_link {
                Some(link) => upd_folder.bind((2, link.as_str())).unwrap(),
                None => upd_folder.bind((2, sqlite::Value::Null)).unwrap(),
            }
            upd_folder
                .bind((3, if has_more { 1i64 } else { 0i64 }))
                .unwrap();
            upd_folder.bind((4, inserted_file_count)).unwrap();
            upd_folder.bind((5, inserted_folder_count)).unwrap();
            upd_folder.bind((6, now)).unwrap();
            upd_folder.bind((7, folder.id)).unwrap();
            upd_folder.next().unwrap();

            conn.execute("COMMIT;").map_err(|e| e.to_string())?;
            guard.1 = true;
        }

        // 5. Nạp thẳng các file mới quét được vào bounded channel
        // Bounded channel tx sẽ backpressure nếu hệ thống tải quá nhiều tệp chưa xử lý kịp.
        for item in db_pipeline_items {
            if self.cancel_token.is_cancelled() {
                break;
            }
            // Gửi vào channel, block nếu channel đầy.
            // Điều này tạo ra backpressure tự nhiên lên Crawler.
            tokio::select! {
                _ = self.cancel_token.cancelled() => break,
                result = tx.send(item) => {
                    if result.is_err() {
                        log::error!("Crawler failed to send item to pipeline channel (channel closed)");
                        break;
                    }
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::db::open_migration_db_at_path;
    use crate::migration::models::MsAccountInfo;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn test_session() -> MicrosoftSession {
        MicrosoftSession {
            client_id: "client".to_string(),
            access_token: "token".to_string(),
            refresh_token: "refresh".to_string(),
            expires_at: chrono::Utc::now().timestamp() + 3600,
            tenant: "common".to_string(),
            redirect_uri: "http://localhost".to_string(),
            account_info: MsAccountInfo {
                account_name: "Test".to_string(),
                account_email: "test@example.com".to_string(),
            },
        }
    }

    fn crawler_fixture(
        name: &str,
        page_url: &str,
        session: Option<MicrosoftSession>,
        config: PipelineConfig,
    ) -> (StreamingCrawler, std::path::PathBuf) {
        let db_path = std::env::temp_dir().join(format!(
            "crawler-{}-{}-{}.db",
            name,
            std::process::id(),
            rand::random::<u64>()
        ));
        let db = open_migration_db_at_path(db_path.clone()).unwrap();
        {
            let conn = db.lock().unwrap();
            conn.execute("INSERT INTO migration_jobs (id, source_folder_id, source_folder_path, telegram_destination_name, local_backup_dir, workspace_dir, state, started_at, created_at, updated_at) VALUES (1, 'root', '/', 'Saved Messages', '/tmp', '/tmp', 'running', 0, 0, 0)").unwrap();
            let mut stmt = conn.prepare("INSERT INTO folder_queue (id, job_id, folder_id, folder_path, state, next_page_link, has_more, created_at, updated_at) VALUES (1, 1, 'root', '/', 'pending', ?, 1, 0, 0)").unwrap();
            stmt.bind((1, page_url)).unwrap();
            stmt.next().unwrap();
        }
        (
            StreamingCrawler {
                db,
                job_id: 1,
                ms_session: Arc::new(TokioMutex::new(session)),
                cancel_token: CancellationToken::new(),
                config,
            },
            db_path,
        )
    }

    #[tokio::test]
    async fn permanent_auth_error_is_not_retried_and_fails_root_folder() {
        let (crawler, db_path) = crawler_fixture(
            "auth-fatal",
            "http://127.0.0.1/unused",
            None,
            PipelineConfig::default(),
        );
        let (tx, _rx) = mpsc::channel(1);
        let folder = crawler.get_next_folder().unwrap().unwrap();
        let error = crawler.process_folder(folder, &tx).await.unwrap_err();
        assert!(error.contains("Microsoft account not connected"));
        let conn = crawler.db.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT state, last_error FROM folder_queue WHERE id = 1")
            .unwrap();
        stmt.next().unwrap();
        assert_eq!(stmt.read::<String, _>(0).unwrap(), "failed");
        assert!(stmt.read::<String, _>(1).unwrap().contains("not connected"));
        drop(stmt);
        drop(conn);
        drop(crawler);
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn transient_graph_errors_retry_only_to_configured_limit() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500).set_body_string("temporary"))
            .expect(4)
            .mount(&server)
            .await;
        let mut config = PipelineConfig::default();
        config.max_network_retries = 3;
        config.retry_delay_secs = vec![0, 0, 0];
        let (crawler, db_path) = crawler_fixture(
            "bounded-retry",
            &format!("{}/page", server.uri()),
            Some(test_session()),
            config,
        );
        let (tx, _rx) = mpsc::channel(1);
        let folder = crawler.get_next_folder().unwrap().unwrap();
        let error = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            crawler.process_folder(folder, &tx),
        )
        .await
        .expect("bounded retry timed out")
        .unwrap_err();
        assert!(error.contains("retries exhausted"));
        server.verify().await;
        drop(crawler);
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn duplicate_graph_page_enqueues_and_counts_each_row_once() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "value": [
                {"id":"file-1","name":"video.mp4","size":123,"file":{"hashes":{}}},
                {"id":"folder-1","name":"Child","size":0,"folder":{"childCount":0}}
            ]
        });
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(2)
            .mount(&server)
            .await;
        let (crawler, db_path) = crawler_fixture(
            "duplicate-page",
            &format!("{}/page", server.uri()),
            Some(test_session()),
            PipelineConfig::default(),
        );
        let (tx, mut rx) = mpsc::channel(4);
        let folder = crawler.get_next_folder().unwrap().unwrap();
        crawler.process_folder(folder.clone(), &tx).await.unwrap();
        crawler.process_folder(folder, &tx).await.unwrap();
        assert!(rx.try_recv().is_ok());
        assert!(rx.try_recv().is_err());
        let conn = crawler.db.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT discovered_items, discovered_folders, waiting_items FROM migration_jobs WHERE id = 1")
            .unwrap();
        stmt.next().unwrap();
        assert_eq!(stmt.read::<i64, _>(0).unwrap(), 1);
        assert_eq!(stmt.read::<i64, _>(1).unwrap(), 1);
        assert_eq!(stmt.read::<i64, _>(2).unwrap(), 1);
        drop(stmt);
        drop(conn);
        server.verify().await;
        drop(crawler);
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn crawler_does_not_enqueue_item_already_owned_by_recovery() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "value": [{"id":"file-1","name":"video.mp4","size":123,"file":{"hashes":{}}}]
        });
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
        let (crawler, db_path) = crawler_fixture(
            "recovery-owned",
            &format!("{}/page", server.uri()),
            Some(test_session()),
            PipelineConfig::default(),
        );
        {
            let conn = crawler.db.lock().unwrap();
            conn.execute("INSERT INTO migration_items (job_id, folder_id, source_item_id, name, path, size, item_category, pipeline_stage, created_at, updated_at) VALUES (1, 'root', 'file-1', 'video.mp4', 'video.mp4', 123, 'file', 'queued_processing', 0, 0)").unwrap();
        }
        let (tx, mut rx) = mpsc::channel(2);
        let folder = crawler.get_next_folder().unwrap().unwrap();
        crawler.process_folder(folder, &tx).await.unwrap();
        assert!(rx.try_recv().is_err());
        let conn = crawler.db.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT discovered_items, waiting_items FROM migration_jobs WHERE id = 1")
            .unwrap();
        stmt.next().unwrap();
        assert_eq!(stmt.read::<i64, _>(0).unwrap(), 0);
        assert_eq!(stmt.read::<i64, _>(1).unwrap(), 0);
        drop(stmt);
        drop(conn);
        drop(crawler);
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn cancellation_during_graph_request_is_not_fatal() {
        let db_path = std::env::temp_dir().join(format!(
            "crawler-cancel-{}-{}.db",
            std::process::id(),
            rand::random::<u64>()
        ));
        let db = open_migration_db_at_path(db_path.clone()).unwrap();
        {
            let conn = db.lock().unwrap();
            conn.execute("INSERT INTO migration_jobs (id, source_folder_id, source_folder_path, telegram_destination_name, local_backup_dir, workspace_dir, state, started_at, created_at, updated_at) VALUES (1, 'root', '/', 'Saved Messages', '/tmp', '/tmp', 'running', 0, 0, 0)").unwrap();
            conn.execute("INSERT INTO folder_queue (id, job_id, folder_id, folder_path, state, has_more, created_at, updated_at) VALUES (1, 1, 'root', '/', 'pending', 1, 0, 0)").unwrap();
        }
        let cancel = CancellationToken::new();
        cancel.cancel();
        let crawler = StreamingCrawler {
            db,
            job_id: 1,
            ms_session: Arc::new(TokioMutex::new(Some(MicrosoftSession {
                client_id: "client".to_string(),
                access_token: "token".to_string(),
                refresh_token: "refresh".to_string(),
                expires_at: chrono::Utc::now().timestamp() + 3600,
                tenant: "common".to_string(),
                redirect_uri: "http://localhost".to_string(),
                account_info: MsAccountInfo {
                    account_name: "Test".to_string(),
                    account_email: "test@example.com".to_string(),
                },
            }))),
            cancel_token: cancel,
            config: PipelineConfig::default(),
        };
        let (tx, _rx) = mpsc::channel(1);
        let folder = crawler.get_next_folder().unwrap().unwrap();
        let result = tokio::time::timeout(
            tokio::time::Duration::from_secs(2),
            crawler.process_folder(folder, &tx),
        )
        .await
        .expect("cancelled Graph request did not return");
        assert!(result.is_ok());
        let _ = std::fs::remove_file(db_path);
    }
}
