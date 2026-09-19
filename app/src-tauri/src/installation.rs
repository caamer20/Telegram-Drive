use serde::Serialize;
use std::ffi::OsStr;

pub const PACMAN_ENVIRONMENT_VALUE: &str = "pacman";
pub const MICROSOFT_STORE_PACKAGE_MANAGER: &str = "microsoft-store";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallationInfo {
    pub managed_by_package_manager: bool,
    pub package_manager: Option<String>,
}

fn installation_info(value: Option<&OsStr>, packaged_windows_app: bool) -> InstallationInfo {
    if packaged_windows_app {
        return InstallationInfo {
            managed_by_package_manager: true,
            package_manager: Some(MICROSOFT_STORE_PACKAGE_MANAGER.to_string()),
        };
    }
    let package_manager = value
        .and_then(OsStr::to_str)
        .map(str::trim)
        .filter(|value| value.eq_ignore_ascii_case(PACMAN_ENVIRONMENT_VALUE));

    InstallationInfo {
        managed_by_package_manager: package_manager.is_some(),
        package_manager: package_manager.map(|_| PACMAN_ENVIRONMENT_VALUE.to_string()),
    }
}

pub fn current_installation_info() -> InstallationInfo {
    installation_info(
        std::env::var_os("TELEGRAM_DRIVE_PACKAGE_MANAGER").as_deref(),
        is_packaged_windows_app(),
    )
}

pub fn is_packaged_windows_app() -> bool {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::Foundation::APPMODEL_ERROR_NO_PACKAGE;
        use windows_sys::Win32::Storage::Packaging::Appx::GetCurrentPackageFullName;

        let mut length = 0;
        // Querying the required length is sufficient to distinguish MSIX from
        // the standalone installer. On an unexpected API error, leave updates
        // to the Store instead of risking an external installer launch.
        let result = unsafe { GetCurrentPackageFullName(&mut length, std::ptr::null_mut()) };
        result != APPMODEL_ERROR_NO_PACKAGE
    }
    #[cfg(not(target_os = "windows"))]
    {
        false
    }
}

#[tauri::command]
pub fn cmd_open_microsoft_store_updates(app: tauri::AppHandle) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        use tauri_plugin_opener::OpenerExt;
        if !is_packaged_windows_app() {
            return Err("This installation is not managed by Microsoft Store".into());
        }
        app.opener()
            .open_url("ms-windows-store://downloadsandupdates", None::<&str>)
            .map_err(|error| format!("Could not open Microsoft Store: {error}"))
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = app;
        Err("Microsoft Store updates are only available on Windows".into())
    }
}

#[tauri::command]
pub fn cmd_get_installation_info() -> InstallationInfo {
    current_installation_info()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "windows")]
    #[test]
    fn unpackaged_windows_test_process_has_no_package_identity() {
        assert!(!is_packaged_windows_app());
    }

    #[test]
    fn recognizes_only_the_packaged_pacman_launcher() {
        assert_eq!(
            installation_info(Some(OsStr::new("pacman")), false),
            InstallationInfo {
                managed_by_package_manager: true,
                package_manager: Some("pacman".to_string()),
            }
        );
        assert_eq!(
            installation_info(Some(OsStr::new("PACMAN")), false),
            InstallationInfo {
                managed_by_package_manager: true,
                package_manager: Some("pacman".to_string()),
            }
        );
    }

    #[test]
    fn leaves_other_installations_self_managed() {
        for value in [None, Some(OsStr::new("")), Some(OsStr::new("unknown"))] {
            assert_eq!(
                installation_info(value, false),
                InstallationInfo {
                    managed_by_package_manager: false,
                    package_manager: None,
                }
            );
        }
    }

    #[test]
    fn packaged_windows_apps_always_use_store_updates() {
        for value in [
            None,
            Some(OsStr::new("pacman")),
            Some(OsStr::new("unknown")),
        ] {
            assert_eq!(
                installation_info(value, true),
                InstallationInfo {
                    managed_by_package_manager: true,
                    package_manager: Some(MICROSOFT_STORE_PACKAGE_MANAGER.to_string()),
                }
            );
        }
    }
}
