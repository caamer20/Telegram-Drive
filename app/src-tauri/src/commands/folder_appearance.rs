//! Per-folder accent colours.
//!
//! Folder colours are a local UI preference (they never touch Telegram). They
//! live in their own small table so the drive keeps working even if a colour
//! is set for a folder that has since been deleted remotely.

use std::collections::HashMap;

use tauri::State;

use crate::db::DbConnection;

/// Set (or clear, when `color` is `None`) the accent colour of a folder.
#[tauri::command]
pub async fn cmd_set_folder_color(
    folder_id: i64,
    color: Option<String>,
    db: State<'_, DbConnection>,
) -> Result<(), String> {
    let database = db.inner().clone();
    crate::db::with_connection(database, move |conn| {
        match color {
            Some(value) => {
                let mut stmt = conn
                    .prepare("INSERT INTO folder_appearance(folder_id, color) VALUES(?1, ?2) ON CONFLICT(folder_id) DO UPDATE SET color = ?2")
                    .map_err(|e| e.to_string())?;
                stmt.bind((1, folder_id)).map_err(|e| e.to_string())?;
                stmt.bind((2, value.as_str())).map_err(|e| e.to_string())?;
                stmt.next().map_err(|e| e.to_string())?;
            }
            None => {
                let mut stmt = conn
                    .prepare("DELETE FROM folder_appearance WHERE folder_id = ?1")
                    .map_err(|e| e.to_string())?;
                stmt.bind((1, folder_id)).map_err(|e| e.to_string())?;
                stmt.next().map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    })
    .await
}

/// Return every folder's saved colour as `{ folder_id: color }`.
#[tauri::command]
pub async fn cmd_get_folder_colors(
    db: State<'_, DbConnection>,
) -> Result<HashMap<i64, String>, String> {
    let database = db.inner().clone();
    crate::db::with_connection(database, move |conn| {
        let mut stmt = conn
            .prepare("SELECT folder_id, color FROM folder_appearance")
            .map_err(|e| e.to_string())?;
        let mut colors = HashMap::new();
        while let sqlite::State::Row = stmt.next().map_err(|e| e.to_string())? {
            let folder_id = stmt.read::<i64, _>("folder_id").map_err(|e| e.to_string())?;
            let color = stmt.read::<String, _>("color").map_err(|e| e.to_string())?;
            colors.insert(folder_id, color);
        }
        Ok(colors)
    })
    .await
}
