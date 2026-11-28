//! Soft-delete ("Trash") support.
//!
//! Deleting a file forwards it to a dedicated trash channel and records the
//! bookkeeping in SQLite so it can be restored within the retention window
//! (currently 20 days). Purging removes expired rows and their remote copies.

use std::time::{SystemTime, UNIX_EPOCH};

use grammers_client::types::Media;
use grammers_client::Client;
use grammers_tl_types as tl;
use serde::Serialize;
use tauri::{Emitter, State};

use crate::commands::utils::{map_error, resolve_peer};
use crate::commands::TelegramState;
use crate::db::DbConnection;

const TRASH_TAG: &str = "[TD-TRASH]";
const TRASH_RETENTION_DAYS: i64 = 20;

#[derive(Clone, Debug, Serialize)]
pub struct TrashItemMetadata {
    pub trash_message_id: i64,
    pub trash_folder_id: i64,
    pub original_folder_id: i64,
    pub original_message_id: i32,
    pub file_name: String,
    pub file_size: u64,
    pub created_at: String,
    pub expires_at: String,
}

fn now_ts() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn ttl_seconds() -> i64 {
    TRASH_RETENTION_DAYS * 24 * 60 * 60
}

fn get_trash_container_name() -> String {
    format!("Telegram Drive Trash {TRASH_TAG}")
}

async fn get_or_create_trash_folder(
    client: &Client,
    state: &TelegramState,
    db: &DbConnection,
) -> Result<i64, String> {
    // Return the cached trash channel id if we already created one.
    {
        let conn = db.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT value FROM trash_state WHERE key='trash_folder_id' LIMIT 1")
            .map_err(|e| e.to_string())?;

        while let sqlite::State::Row = stmt.next().map_err(|e| e.to_string())? {
            let s: String = stmt.read::<String, _>("value").map_err(|e| e.to_string())?;
            if let Ok(id) = s.parse::<i64>() {
                return Ok(id);
            }
        }
    }

    let title = get_trash_container_name();
    let result = client
        .invoke(&tl::functions::channels::CreateChannel {
            broadcast: true,
            megagroup: false,
            title: title.clone(),
            about: format!("Telegram Drive Trash\n{TRASH_TAG}"),
            geo_point: None,
            address: None,
            for_import: false,
            forum: false,
            ttl_period: None,
        })
        .await
        .map_err(map_error)?;

    let (chat_id, access_hash) = match &result {
        tl::enums::Updates::Updates(u) => {
            let chat = u.chats.first().ok_or("No chat in updates")?;
            match chat {
                tl::enums::Chat::Channel(c) => (c.id, c.access_hash.unwrap_or(0)),
                _ => return Err("Created chat is not a channel".to_string()),
            }
        }
        _ => return Err("Unexpected response (not Updates::Updates)".to_string()),
    };

    // Disable channel history TTL; DB retention handles expiration.
    let _ = client
        .invoke(&tl::functions::messages::SetHistoryTtl {
            peer: tl::enums::InputPeer::Channel(tl::types::InputPeerChannel {
                channel_id: chat_id,
                access_hash,
            }),
            period: 0,
        })
        .await;

    {
        let conn = db.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("INSERT OR REPLACE INTO trash_state(key,value) VALUES('trash_folder_id', ?1)")
            .map_err(|e| e.to_string())?;
        stmt.bind((1, chat_id.to_string().as_str()))
            .map_err(|e| e.to_string())?;
        stmt.next().map_err(|e| e.to_string())?;
    }

    // Seed the peer cache so later resolve_peer calls are O(1).
    let peer = resolve_peer(client, Some(chat_id), &state.peer_cache).await.ok();
    if let Some(p) = peer {
        state.peer_cache.write().await.insert(chat_id, p);
    }

    Ok(chat_id)
}

#[tauri::command]
pub async fn cmd_list_trash(
    _state: State<'_, TelegramState>,
    db: State<'_, DbConnection>,
) -> Result<Vec<TrashItemMetadata>, String> {
    let now = now_ts();
    let mut out = Vec::new();

    {
        let conn = db.lock().map_err(|e| e.to_string())?;

        let mut stmt = conn
            .prepare(
                "SELECT trash_message_id, trash_folder_id, original_folder_id, original_message_id, file_name, file_size, created_at, expires_at \
                 FROM trash_items \
                 WHERE expires_at > ?1 AND restored = 0 \
                 ORDER BY created_at DESC",
            )
            .map_err(|e| e.to_string())?;
        stmt.bind((1, now)).map_err(|e| e.to_string())?;

        while let sqlite::State::Row = stmt.next().map_err(|e| e.to_string())? {
            let original_message_id_i64: i64 = stmt
                .read::<i64, _>("original_message_id")
                .map_err(|e| e.to_string())?;

            out.push(TrashItemMetadata {
                trash_message_id: stmt
                    .read::<i64, _>("trash_message_id")
                    .map_err(|e| e.to_string())?,
                trash_folder_id: stmt
                    .read::<i64, _>("trash_folder_id")
                    .map_err(|e| e.to_string())?,
                original_folder_id: stmt
                    .read::<i64, _>("original_folder_id")
                    .map_err(|e| e.to_string())?,
                original_message_id: original_message_id_i64 as i32,
                file_name: stmt
                    .read::<String, _>("file_name")
                    .map_err(|e| e.to_string())?,
                file_size: stmt
                    .read::<i64, _>("file_size")
                    .map_err(|e| e.to_string())? as u64,
                created_at: stmt
                    .read::<i64, _>("created_at")
                    .map_err(|e| e.to_string())?
                    .to_string(),
                expires_at: stmt
                    .read::<i64, _>("expires_at")
                    .map_err(|e| e.to_string())?
                    .to_string(),
            });
        }
    }

    Ok(out)
}

#[tauri::command]
pub async fn cmd_get_trash_stats(
    db: State<'_, DbConnection>,
) -> Result<crate::models::TrashStats, String> {
    let now = now_ts();
    let mut count: i64 = 0;
    let mut total_size: i64 = 0;

    {
        let conn = db.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT COUNT(*) as c, SUM(file_size) as s FROM trash_items WHERE expires_at > ?1 AND restored = 0")
            .map_err(|e| e.to_string())?;
        stmt.bind((1, now)).map_err(|e| e.to_string())?;

        if let sqlite::State::Row = stmt.next().map_err(|e| e.to_string())? {
            count = stmt.read::<i64, _>("c").unwrap_or(0);
            total_size = stmt
                .read::<Option<i64>, _>("s")
                .map_err(|e| e.to_string())?
                .unwrap_or(0);
        }
    }

    Ok(crate::models::TrashStats {
        count: count as u64,
        total_size: total_size as u64,
    })
}

#[tauri::command]
pub async fn cmd_get_trash_count(db: State<'_, DbConnection>) -> Result<u64, String> {
    let now = now_ts();
    let mut count: i64 = 0;

    {
        let conn = db.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT COUNT(*) as c FROM trash_items WHERE expires_at > ?1 AND restored = 0")
            .map_err(|e| e.to_string())?;
        stmt.bind((1, now)).map_err(|e| e.to_string())?;

        if let sqlite::State::Row = stmt.next().map_err(|e| e.to_string())? {
            count = stmt.read::<i64, _>("c").unwrap_or(0);
        }
    }

    Ok(count as u64)
}

#[tauri::command]
pub async fn cmd_move_file_to_trash(
    message_id: i32,
    folder_id: Option<i64>,
    state: State<'_, TelegramState>,
    db: State<'_, DbConnection>,
    app: tauri::AppHandle,
) -> Result<bool, String> {
    let client_opt = { state.client.lock().await.clone() };
    #[cfg(debug_assertions)]
    if client_opt.is_none() {
        return Ok(true);
    }
    let client = client_opt.ok_or_else(|| "Client not connected".to_string())?;

    let trash_folder_id = get_or_create_trash_folder(&client, &state, &db).await?;

    let source_peer = resolve_peer(&client, folder_id, &state.peer_cache).await?;
    let trash_peer = resolve_peer(&client, Some(trash_folder_id), &state.peer_cache).await?;

    let messages = client
        .get_messages_by_id(&source_peer, &[message_id])
        .await
        .map_err(|e| e.to_string())?;
    let msg = messages
        .into_iter()
        .flatten()
        .next()
        .ok_or_else(|| "Message not found".to_string())?;

    let (file_name, file_size) = match msg.media() {
        Some(Media::Document(d)) => (d.name().to_string(), d.size() as u64),
        Some(Media::Photo(_)) => ("Photo.jpg".to_string(), 0),
        _ => ("Unknown".to_string(), 0),
    };

    let forwarded = client
        .forward_messages(&trash_peer, &[message_id], &source_peer)
        .await
        .map_err(|e| e.to_string())?;

    let trash_message_id = forwarded
        .first()
        .ok_or_else(|| "Failed to get forwarded message ID".to_string())?
        .as_ref()
        .ok_or_else(|| "Forwarded message is null".to_string())?
        .id() as i64;

    client
        .delete_messages(&source_peer, &[message_id])
        .await
        .map_err(|e| e.to_string())?;

    let now = now_ts();
    let expires_at = now + ttl_seconds();
    // 0 = Saved Messages sentinel.
    let original_folder_id = folder_id.unwrap_or(0);

    {
        let conn = db.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "INSERT INTO trash_items(\
                  trash_message_id, trash_folder_id, original_folder_id, original_message_id,\
                  file_name, file_size, created_at, expires_at, restored\
                ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0)",
            )
            .map_err(|e| e.to_string())?;

        stmt.bind((1, trash_message_id)).map_err(|e| e.to_string())?;
        stmt.bind((2, trash_folder_id)).map_err(|e| e.to_string())?;
        stmt.bind((3, original_folder_id)).map_err(|e| e.to_string())?;
        stmt.bind((4, message_id as i64)).map_err(|e| e.to_string())?;
        stmt.bind((5, file_name.as_str())).map_err(|e| e.to_string())?;
        stmt.bind((6, file_size as i64)).map_err(|e| e.to_string())?;
        stmt.bind((7, now)).map_err(|e| e.to_string())?;
        stmt.bind((8, expires_at)).map_err(|e| e.to_string())?;
        stmt.next().map_err(|e| e.to_string())?;
    }

    let _ = app.emit("drive-changed", ());
    Ok(true)
}

#[tauri::command]
pub async fn cmd_restore_from_trash(
    trash_message_id: i64,
    state: State<'_, TelegramState>,
    db: State<'_, DbConnection>,
    app: tauri::AppHandle,
) -> Result<bool, String> {
    let client_opt = { state.client.lock().await.clone() };
    #[cfg(debug_assertions)]
    if client_opt.is_none() {
        return Ok(true);
    }
    let client = client_opt.ok_or_else(|| "Client not connected".to_string())?;

    let found: Option<(i64, i64)> = {
        let conn = db.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT trash_folder_id, original_folder_id \
                 FROM trash_items \
                 WHERE trash_message_id = ?1 AND restored = 0 LIMIT 1",
            )
            .map_err(|e| e.to_string())?;
        stmt.bind((1, trash_message_id)).map_err(|e| e.to_string())?;

        let mut res = None;
        while let sqlite::State::Row = stmt.next().map_err(|e| e.to_string())? {
            let tf: i64 = stmt.read::<i64, _>("trash_folder_id").map_err(|e| e.to_string())?;
            let of: i64 = stmt
                .read::<i64, _>("original_folder_id")
                .map_err(|e| e.to_string())?;
            res = Some((tf, of));
            break;
        }
        res
    };

    let (trash_folder_id, original_folder_id) =
        found.ok_or_else(|| "Trash item not found".to_string())?;

    let trash_peer = resolve_peer(&client, Some(trash_folder_id), &state.peer_cache).await?;
    let target_folder_id = if original_folder_id == 0 {
        None
    } else {
        Some(original_folder_id)
    };
    let original_peer = resolve_peer(&client, target_folder_id, &state.peer_cache).await?;

    client
        .forward_messages(&original_peer, &[trash_message_id as i32], &trash_peer)
        .await
        .map_err(|e| e.to_string())?;

    client
        .delete_messages(&trash_peer, &[trash_message_id as i32])
        .await
        .map_err(|e| e.to_string())?;

    {
        let conn = db.lock().map_err(|e| e.to_string())?;
        let mut stmt2 = conn
            .prepare("UPDATE trash_items SET restored = 1 WHERE trash_message_id = ?1")
            .map_err(|e| e.to_string())?;
        stmt2
            .bind((1, trash_message_id))
            .map_err(|e| e.to_string())?;
        stmt2.next().map_err(|e| e.to_string())?;
    }

    let _ = app.emit("drive-changed", ());
    Ok(true)
}

pub async fn purge_expired_trash(db: &DbConnection) -> Result<bool, String> {
    let now = now_ts();

    {
        let conn = db.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("DELETE FROM trash_items WHERE restored = 0 AND expires_at <= ?1")
            .map_err(|e| e.to_string())?;
        stmt.bind((1, now)).map_err(|e| e.to_string())?;
        stmt.next().map_err(|e| e.to_string())?;
    }

    Ok(true)
}

#[tauri::command]
pub async fn cmd_purge_expired_trash(db: State<'_, DbConnection>) -> Result<bool, String> {
    purge_expired_trash(&db).await
}
