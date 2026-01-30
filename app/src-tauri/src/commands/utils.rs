use grammers_client::Client;
use grammers_client::types::Peer;
use tauri::State;
use crate::bandwidth::BandwidthManager;

pub async fn resolve_peer(client: &Client, folder_id: Option<i64>) -> Result<Peer, String> {
    if let Some(fid) = folder_id {
        let mut dialogs = client.iter_dialogs();
        while let Some(dialog) = dialogs.next().await.map_err(|e| e.to_string())? {
            // We use .raw.id() based on compiler suggestions that .id() might be missing on wrapper types in this version
            match &dialog.peer {
                Peer::Channel(c) => if c.raw.id == fid { return Ok(dialog.peer.clone()); },
                Peer::User(u) => if u.raw.id() == fid { return Ok(dialog.peer.clone()); },
                _ => {}
            }
        }
        Err(format!("Folder/Chat {} not found", fid))
    } else {
        match client.get_me().await {
            Ok(me) => Ok(Peer::User(me)),
            Err(e) => Err(e.to_string()),
        }
    }
}

#[tauri::command]
pub fn cmd_log(message: String) {
    log::info!("[FRONTEND] {}", message);
}

#[tauri::command]
pub fn cmd_get_bandwidth(bw_state: State<'_, BandwidthManager>) -> crate::bandwidth::BandwidthStats {
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
