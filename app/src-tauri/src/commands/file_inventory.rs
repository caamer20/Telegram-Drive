use crate::db::{self, DbConnection};
use crate::models::FileMetadata;
use tauri::Manager;

pub fn folder_key(folder_id: Option<i64>) -> String {
    folder_id
        .map(|id| id.to_string())
        .unwrap_or_else(|| "home".to_string())
}

async fn scoped_connection<T, F>(
    database: DbConnection,
    publication: Option<crate::file_inventory::Publication>,
    operation: F,
) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&sqlite::Connection) -> Result<T, String> + Send + 'static,
{
    if let Some(publication) = publication {
        tokio::task::spawn_blocking(move || {
            publication.persist(|| {
                let connection = database.lock().map_err(|_| "Database lock poisoned")?;
                operation(&connection)
            })
        })
        .await
        .map_err(|error| error.to_string())?
    } else {
        db::with_connection(database, operation).await
    }
}

pub async fn upsert_inventory_chunk(
    database: DbConnection,
    folder_key: String,
    scan_id: String,
    files: Vec<FileMetadata>,
    publication: Option<crate::file_inventory::Publication>,
) -> Result<(), String> {
    if files.is_empty() {
        return Ok(());
    }
    let updated_at = crate::app_time::unix_seconds();
    scoped_connection(database, publication, move |connection| {
        connection
            .execute("BEGIN IMMEDIATE")
            .map_err(|error| error.to_string())?;
        let result = (|| {
            for mut file in files {
                if file.encryption_state != "plain" {
                    file.name = "Protected file".into();
                    file.mime_type = Some("application/octet-stream".into());
                    file.file_ext = None;
                    file.encryption_state = "encrypted_locked".into();
                }
                let mut statement = connection
                    .prepare(
                        "INSERT INTO file_inventory (
                            folder_key, folder_id, message_id, file_name, file_size,
                            mime_type, file_ext, created_at, icon_type, encryption_state,
                            last_seen_scan, updated_at
                         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                         ON CONFLICT(folder_key, message_id) DO UPDATE SET
                            folder_id = excluded.folder_id,
                            file_name = excluded.file_name,
                            file_size = excluded.file_size,
                            mime_type = excluded.mime_type,
                            file_ext = excluded.file_ext,
                            created_at = excluded.created_at,
                            icon_type = excluded.icon_type,
                            encryption_state = excluded.encryption_state,
                            last_seen_scan = excluded.last_seen_scan,
                            updated_at = excluded.updated_at",
                    )
                    .map_err(|error| error.to_string())?;
                statement
                    .bind((1, folder_key.as_str()))
                    .map_err(|error| error.to_string())?;
                statement
                    .bind((2, file.folder_id))
                    .map_err(|error| error.to_string())?;
                statement
                    .bind((3, file.id))
                    .map_err(|error| error.to_string())?;
                statement
                    .bind((4, file.name.as_str()))
                    .map_err(|error| error.to_string())?;
                statement
                    .bind((5, i64::try_from(file.size).unwrap_or(i64::MAX)))
                    .map_err(|error| error.to_string())?;
                statement
                    .bind((6, file.mime_type.as_deref()))
                    .map_err(|error| error.to_string())?;
                statement
                    .bind((7, file.file_ext.as_deref()))
                    .map_err(|error| error.to_string())?;
                statement
                    .bind((8, file.created_at.as_str()))
                    .map_err(|error| error.to_string())?;
                statement
                    .bind((9, file.icon_type.as_str()))
                    .map_err(|error| error.to_string())?;
                statement
                    .bind((10, file.encryption_state.as_str()))
                    .map_err(|error| error.to_string())?;
                statement
                    .bind((11, scan_id.as_str()))
                    .map_err(|error| error.to_string())?;
                statement
                    .bind((12, updated_at))
                    .map_err(|error| error.to_string())?;
                statement.next().map_err(|error| error.to_string())?;
            }
            Ok::<(), String>(())
        })();
        match result {
            Ok(()) => connection
                .execute("COMMIT")
                .map_err(|error| error.to_string()),
            Err(error) => {
                let _ = connection.execute("ROLLBACK");
                Err(error)
            }
        }
    })
    .await
}


/// Drop one row after a verified remote delete so drive metrics change at once.
pub async fn remove_entry(
    database: DbConnection,
    folder: Option<i64>,
    message_id: i64,
) -> Result<(), String> {
    let key = folder_key(folder);
    crate::db::with_connection(database, move |conn| {
        let mut stmt = conn
            .prepare("DELETE FROM file_inventory WHERE folder_key = ? AND message_id = ?")
            .map_err(|e| e.to_string())?;
        stmt.bind((1, key.as_str())).map_err(|e| e.to_string())?;
        stmt.bind((2, message_id)).map_err(|e| e.to_string())?;
        stmt.next().map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}


/// Re-add a row when a file comes back from the trash, so the folder's size and
/// the drive totals include it again immediately.
pub async fn restore_entry(
    database: DbConnection,
    folder: Option<i64>,
    message_id: i64,
    file_name: String,
    file_size: i64,
) -> Result<(), String> {
    let key = folder_key(folder);
    let updated_at = crate::app_time::unix_seconds();
    crate::db::with_connection(database, move |conn| {
        let mut stmt = conn
            .prepare(
                "INSERT INTO file_inventory (
                    folder_key, folder_id, message_id, file_name, file_size, mime_type,
                    file_ext, created_at, icon_type, encryption_state, last_seen_scan, updated_at
                 ) VALUES (?, ?, ?, ?, ?, NULL, NULL, '', 'file', 'plain', '', ?)
                 ON CONFLICT(folder_key, message_id) DO UPDATE SET
                    file_name = excluded.file_name,
                    file_size = excluded.file_size,
                    updated_at = excluded.updated_at",
            )
            .map_err(|e| e.to_string())?;
        stmt.bind((1, key.as_str())).map_err(|e| e.to_string())?;
        stmt.bind::<(usize, Option<i64>)>((2, folder)).map_err(|e| e.to_string())?;
        stmt.bind((3, message_id)).map_err(|e| e.to_string())?;
        stmt.bind((4, file_name.as_str())).map_err(|e| e.to_string())?;
        stmt.bind((5, file_size)).map_err(|e| e.to_string())?;
        stmt.bind((6, updated_at)).map_err(|e| e.to_string())?;
        stmt.next().map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

/// Copy a row's metadata into another folder under a new message id.
pub async fn copy_entry(
    database: DbConnection,
    source: Option<i64>,
    message_id: i64,
    target: Option<i64>,
    new_message_id: i64,
    remove_source: bool,
) -> Result<(), String> {
    let source_key = folder_key(source);
    let target_key = folder_key(target);
    let updated_at = crate::app_time::unix_seconds();
    crate::db::with_connection(database, move |conn| {
        conn.execute("BEGIN IMMEDIATE").map_err(|e| e.to_string())?;
        let result = (|| {
            let mut insert = conn
                .prepare(
                    "INSERT INTO file_inventory (
                        folder_key, folder_id, message_id, file_name, file_size, mime_type,
                        file_ext, created_at, icon_type, encryption_state, last_seen_scan, updated_at
                     )
                     SELECT ?, ?, ?, file_name, file_size, mime_type, file_ext, created_at,
                            icon_type, encryption_state, last_seen_scan, ?
                     FROM file_inventory WHERE folder_key = ? AND message_id = ?
                     ON CONFLICT(folder_key, message_id) DO UPDATE SET
                        file_name = excluded.file_name,
                        file_size = excluded.file_size,
                        mime_type = excluded.mime_type,
                        file_ext = excluded.file_ext,
                        created_at = excluded.created_at,
                        icon_type = excluded.icon_type,
                        encryption_state = excluded.encryption_state,
                        updated_at = excluded.updated_at",
                )
                .map_err(|e| e.to_string())?;
            insert.bind((1, target_key.as_str())).map_err(|e| e.to_string())?;
            insert
                .bind::<(usize, Option<i64>)>((2, target))
                .map_err(|e| e.to_string())?;
            insert.bind((3, new_message_id)).map_err(|e| e.to_string())?;
            insert.bind((4, updated_at)).map_err(|e| e.to_string())?;
            insert.bind((5, source_key.as_str())).map_err(|e| e.to_string())?;
            insert.bind((6, message_id)).map_err(|e| e.to_string())?;
            insert.next().map_err(|e| e.to_string())?;

            if remove_source {
                let mut delete = conn
                    .prepare("DELETE FROM file_inventory WHERE folder_key = ? AND message_id = ?")
                    .map_err(|e| e.to_string())?;
                delete.bind((1, source_key.as_str())).map_err(|e| e.to_string())?;
                delete.bind((2, message_id)).map_err(|e| e.to_string())?;
                delete.next().map_err(|e| e.to_string())?;
            }
            Ok::<(), String>(())
        })();
        match result {
            Ok(()) => conn
                .execute("COMMIT")
                .map_err(|e| e.to_string())
                .map(|_| ()),
            Err(error) => {
                let _ = conn.execute("ROLLBACK");
                Err(error)
            }
        }
    })
    .await
}

pub async fn complete_inventory_scan(
    database: DbConnection,
    folder_key: String,
    scan_id: String,
    publication: Option<crate::file_inventory::Publication>,
) -> Result<(), String> {
    let completed_at = crate::app_time::unix_seconds();
    scoped_connection(database, publication, move |connection| {
        connection
            .execute("BEGIN IMMEDIATE")
            .map_err(|error| error.to_string())?;
        let result = (|| {
            let mut delete = connection
                .prepare(
                    "DELETE FROM file_inventory
                     WHERE folder_key = ? AND last_seen_scan <> ?",
                )
                .map_err(|error| error.to_string())?;
            delete
                .bind((1, folder_key.as_str()))
                .map_err(|error| error.to_string())?;
            delete
                .bind((2, scan_id.as_str()))
                .map_err(|error| error.to_string())?;
            delete.next().map_err(|error| error.to_string())?;

            let mut count_statement = connection
                .prepare("SELECT COUNT(*) FROM file_inventory WHERE folder_key = ?")
                .map_err(|error| error.to_string())?;
            count_statement
                .bind((1, folder_key.as_str()))
                .map_err(|error| error.to_string())?;
            let file_count = if count_statement.next().map_err(|error| error.to_string())?
                == sqlite::State::Row
            {
                count_statement.read::<i64, _>(0).unwrap_or(0)
            } else {
                0
            };

            let mut state = connection
                .prepare(
                    "INSERT INTO file_inventory_state (folder_key, completed_at, file_count)
                     VALUES (?, ?, ?)
                     ON CONFLICT(folder_key) DO UPDATE SET
                        completed_at = excluded.completed_at,
                        file_count = excluded.file_count",
                )
                .map_err(|error| error.to_string())?;
            state
                .bind((1, folder_key.as_str()))
                .map_err(|error| error.to_string())?;
            state
                .bind((2, completed_at))
                .map_err(|error| error.to_string())?;
            state
                .bind((3, file_count))
                .map_err(|error| error.to_string())?;
            state.next().map_err(|error| error.to_string())?;
            Ok::<(), String>(())
        })();
        match result {
            Ok(()) => connection
                .execute("COMMIT")
                .map_err(|error| error.to_string()),
            Err(error) => {
                let _ = connection.execute("ROLLBACK");
                Err(error)
            }
        }
    })
    .await
}

#[tauri::command]
pub async fn cmd_get_cached_files(
    folder_id: Option<i64>,
    owner_id: Option<String>,
    app: tauri::AppHandle,
) -> Result<Vec<FileMetadata>, String> {
    let root = app.path().app_data_dir().map_err(|e| e.to_string())?;
    tokio::task::spawn_blocking(move || {
        let account = crate::workspace::AccountGuard::open(&root, owner_id.as_deref())?;
        let files =
            crate::workspace::store::Store::open(&root, account.owner)?.folder_files(folder_id)?;
        account.validate()?;
        Ok(files)
    })
    .await
    .map_err(|e| e.to_string())?
}
