use crate::bandwidth::BandwidthManager;
use grammers_client::types::Peer;
use grammers_client::Client;
use std::collections::HashMap;
use std::sync::Arc;
use tauri::State;
use tokio::sync::RwLock;

/// Resolve a folder_id to a Telegram Peer, using the cache for O(1) lookups.
///
/// - `folder_id == None` → returns the user's own peer (Saved Messages)
/// - Cache hit → returns immediately without any network call
/// - Cache miss → scans all dialogs, populates the cache, and returns
pub async fn resolve_peer(
    client: &Client,
    folder_id: Option<i64>,
    peer_cache: &Arc<RwLock<HashMap<i64, Peer>>>,
) -> Result<Peer, String> {
    if let Some(fid) = folder_id {
        // Fast path: check cache
        {
            let cache = peer_cache.read().await;
            if let Some(peer) = cache.get(&fid) {
                return Ok(peer.clone());
            }
        }

        // Slow path: scan dialogs and populate cache
        log::debug!("Peer cache miss for folder_id={}, scanning dialogs...", fid);
        let mut found: Option<Peer> = None;
        let mut dialogs = client.iter_dialogs();
        let mut discovered = HashMap::new();
        while let Some(dialog) = dialogs.next().await.map_err(|e| e.to_string())? {
            let peer_id = match &dialog.peer {
                Peer::Channel(c) => Some(c.raw.id),
                Peer::User(u) => Some(u.raw.id()),
                _ => None,
            };
            if let Some(id) = peer_id {
                discovered.insert(id, dialog.peer.clone());
                if id == fid {
                    found = Some(dialog.peer.clone());
                    // Don't break — keep scanning to warm the cache
                }
            }
        }

        {
            let mut cache = peer_cache.write().await;
            cache.extend(discovered);
        }

        found.ok_or_else(|| format!("Folder/Chat {} not found", fid))
    } else {
        match client.get_me().await {
            Ok(me) => Ok(Peer::User(me)),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// Clear the peer cache (called on logout)
pub async fn clear_peer_cache(peer_cache: &Arc<RwLock<HashMap<i64, Peer>>>) {
    peer_cache.write().await.clear();
}

#[tauri::command]
pub fn cmd_log(message: String) {
    log::info!("[FRONTEND] {}", message);
}

#[tauri::command]
pub fn cmd_get_bandwidth(
    bw_state: State<'_, Arc<BandwidthManager>>,
) -> crate::bandwidth::BandwidthStats {
    bw_state.get_stats()
}

pub fn map_error(e: impl std::fmt::Display) -> String {
    let err_str = e.to_string();
    if err_str.contains("FLOOD_WAIT") {
        // Expected format: ... (value: 1234)
        if let Some(start) = err_str.find("(value: ") {
            let rest = &err_str[start + 8..];
            if let Some(end) = rest.find(')') {
                if let Ok(seconds) = rest[..end].parse::<i64>() {
                    return format!("FLOOD_WAIT_{}", seconds);
                }
            }
        }
        // Fallback if parsing fails but we know it's a flood wait
        return "FLOOD_WAIT_60".to_string();
    }
    err_str
}
