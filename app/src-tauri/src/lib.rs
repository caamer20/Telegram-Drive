pub mod models;

pub mod commands;

use tauri::Manager;
use tokio::sync::Mutex;
use commands::TelegramState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let _handle = app.handle().clone(); // Prefix with _ if meant to be kept or used later, or remove. I'll remove it? No, keeping with _ is safer if future needs it. Actually I'll remove it.
            
            app.manage(TelegramState {
                client: Mutex::new(None),
                login_token: Mutex::new(None),
                password_token: Mutex::new(None),
            });
            
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::cmd_auth_request_code,
            commands::cmd_auth_sign_in,
            commands::cmd_auth_check_password,
            commands::cmd_get_files,
            commands::cmd_upload_file,
            commands::cmd_connect,
            commands::cmd_log,
            commands::cmd_delete_file,
            commands::cmd_download_file,
            commands::cmd_move_file,
            commands::cmd_create_folder,
            commands::cmd_delete_folder,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
