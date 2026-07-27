use serde::Serialize;
use tauri::Emitter;

#[derive(Debug, Clone, Serialize)]
pub struct ItemProgressPayload {
    pub job_id: i64,
    pub item_id: i64,
    pub item_name: String,
    pub phase: String,
    pub percent: f64,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub speed_bytes_per_sec: f64,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ItemCompletePayload {
    pub job_id: i64,
    pub item_id: i64,
    pub item_name: String,
    pub phase: String,
    pub status: String,
    pub error_type: Option<String>,
    pub error_message: Option<String>,
    pub timestamp: i64,
}

pub fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

pub fn emit_item_progress(app: &tauri::AppHandle, payload: ItemProgressPayload) {
    if let Err(error) = app.emit("migration:item-progress", payload) {
        log::warn!("Failed to emit migration:item-progress: {}", error);
    }
}

pub fn emit_item_complete(app: &tauri::AppHandle, payload: ItemCompletePayload) {
    if let Err(error) = app.emit("migration:item-complete", payload) {
        log::warn!("Failed to emit migration:item-complete: {}", error);
    }
}
