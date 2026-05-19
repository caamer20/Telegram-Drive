use tauri::State;

/// Holds the per-session streaming config (token + port)
#[derive(Clone)]
pub struct StreamConfig {
    pub token: String,
    pub port: u16,
}

/// Returned to the frontend so it can construct stream URLs dynamically
#[derive(serde::Serialize)]
pub struct StreamInfo {
    pub token: String,
    pub base_url: String,
}

/// Returns the streaming server's session token and base URL to the frontend.
/// The frontend must use the returned base_url to construct stream URLs,
/// never hardcoding the port.
#[tauri::command]
pub fn cmd_get_stream_info(config: State<'_, StreamConfig>) -> StreamInfo {
    StreamInfo {
        token: config.token.clone(),
        base_url: format!("http://localhost:{}", config.port),
    }
}
