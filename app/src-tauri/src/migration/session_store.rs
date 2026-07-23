use std::path::PathBuf;
use tauri::{AppHandle, Manager};

use crate::migration::microsoft::MicrosoftSession;

const SESSION_FILE_NAME: &str = "microsoft-session.json";

fn session_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join(SESSION_FILE_NAME))
}

pub fn load(app: &AppHandle) -> Result<Option<MicrosoftSession>, String> {
    load_from_path(&session_path(app)?)
}

fn load_from_path(path: &std::path::Path) -> Result<Option<MicrosoftSession>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(path).map_err(|e| format!("Không thể đọc Microsoft session: {e}"))?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|e| format!("Microsoft session trên disk không hợp lệ: {e}"))
}

pub fn save(app: &AppHandle, session: &MicrosoftSession) -> Result<(), String> {
    save_to_path(&session_path(app)?, session)
}

fn save_to_path(path: &std::path::Path, session: &MicrosoftSession) -> Result<(), String> {
    let temp_path = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec(session).map_err(|e| e.to_string())?;

    std::fs::write(&temp_path, bytes)
        .map_err(|e| format!("Không thể ghi Microsoft session: {e}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temp_path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("Không thể giới hạn quyền Microsoft session: {e}"))?;
    }

    replace_file(&temp_path, path)
}

#[cfg(windows)]
fn replace_file(source: &std::path::Path, destination: &std::path::Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(
            existing_file_name: *const u16,
            new_file_name: *const u16,
            flags: u32,
        ) -> i32;
    }

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    let source_wide: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination_wide: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let result = unsafe {
        MoveFileExW(
            source_wide.as_ptr(),
            destination_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(format!(
            "Không thể thay Microsoft session atomically: {}",
            std::io::Error::last_os_error()
        ))
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_file(source: &std::path::Path, destination: &std::path::Path) -> Result<(), String> {
    std::fs::rename(source, destination)
        .map_err(|e| format!("Không thể hoàn tất Microsoft session: {e}"))
}

pub fn delete(app: &AppHandle) -> Result<(), String> {
    delete_path(&session_path(app)?)
}

fn delete_path(path: &std::path::Path) -> Result<(), String> {
    if path.exists() {
        std::fs::remove_file(path).map_err(|e| format!("Không thể xóa Microsoft session: {e}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::models::MsAccountInfo;

    #[test]
    fn session_round_trip_and_disconnect_deletion() {
        let path = std::env::temp_dir().join(format!(
            "telegram-drive-session-{}-{}.json",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let session = MicrosoftSession {
            client_id: "client".into(),
            access_token: "access-secret".into(),
            refresh_token: "refresh-secret".into(),
            expires_at: 123,
            tenant: "common".into(),
            redirect_uri: "http://localhost".into(),
            account_info: MsAccountInfo {
                account_name: "Test".into(),
                account_email: "test@example.com".into(),
            },
        };

        save_to_path(&path, &session).unwrap();
        let restored = load_from_path(&path).unwrap().unwrap();
        assert_eq!(restored.client_id, session.client_id);
        assert_eq!(restored.refresh_token, session.refresh_token);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }

        delete_path(&path).unwrap();
        assert!(!path.exists());
        assert!(load_from_path(&path).unwrap().is_none());
    }
}
