#[cfg(not(any(target_os = "android", target_os = "ios")))]
mod desktop {
    //! Opt-in packaged startup evidence. Production identity is unchanged unless
    //! an explicit private smoke marker and disposable-user authorization are present.
    use std::path::PathBuf;
    use tauri::Manager;
    struct Smoke {
        root: PathBuf,
        token: String,
        identifier: String,
    }
    fn requested() -> Result<Option<Smoke>, String> {
        let Some(root) = std::env::var_os("TELEGRAM_DRIVE_STARTUP_SMOKE_ROOT") else {
            return Ok(None);
        };
        if std::env::var("TELEGRAM_DRIVE_SMOKE_DISPOSABLE_USER").as_deref() != Ok("1") {
            return Err("Packaged smoke requires a disposable OS user/VM".into());
        }
        let token = std::env::var("TELEGRAM_DRIVE_STARTUP_SMOKE_TOKEN")
            .map_err(|_| "Missing smoke token")?;
        if token.len() != 32 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("Invalid smoke token".into());
        }
        let root = PathBuf::from(root);
        if !root.is_absolute()
            || !std::fs::symlink_metadata(&root)
                .map_err(|e| e.to_string())?
                .file_type()
                .is_dir()
            || std::fs::read_to_string(root.join(".packaged-startup-smoke"))
                .map_err(|e| e.to_string())?
                != format!("{token}\n")
        {
            return Err("Invalid private smoke directory".into());
        }
        Ok(Some(Smoke {
            root,
            identifier: format!(
                "com.cameronamer.telegramdrive.smoke.{}{token}",
                if token.as_bytes()[0].is_ascii_digit() {
                    "r"
                } else {
                    ""
                }
            ),
            token,
        }))
    }
    pub(crate) fn configure<R: tauri::Runtime>(
        context: &mut tauri::Context<R>,
    ) -> Result<(), String> {
        if let Some(smoke) = requested()? {
            // All Tauri path resolvers and plugins see the unique profile identity
            // before Builder::build initializes them. Stable keyring identifiers
            // stay unchanged; smoke must run in a disposable OS user/VM.
            context.config_mut().identifier = smoke.identifier;
        }
        Ok(())
    }
    pub(crate) fn validate_profile<R: tauri::Runtime>(
        app: &tauri::AppHandle<R>,
    ) -> Result<(), String> {
        let Some(smoke) = requested()? else {
            return Ok(());
        };
        if app.config().identifier != smoke.identifier {
            return Err("Smoke profile identity was not applied".into());
        }
        for (name, path) in [
            ("app_data_dir", app.path().app_data_dir()),
            ("app_cache_dir", app.path().app_cache_dir()),
            ("app_config_dir", app.path().app_config_dir()),
            ("app_local_data_dir", app.path().app_local_data_dir()),
        ] {
            let path = path.map_err(|e| format!("Smoke {name} resolver failed: {e}"))?;
            if path.file_name().and_then(|value| value.to_str()) != Some(smoke.identifier.as_str())
            {
                return Err("Resolved Tauri path is not isolated".into());
            }
        }
        Ok(())
    }
    pub(crate) fn mark_ready<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Result<(), String> {
        let Some(smoke) = requested()? else {
            return Ok(());
        };
        validate_profile(app)?;
        let record = serde_json::json!({
            "process_id": std::process::id(), "run_token": smoke.token, "profile_identifier": smoke.identifier,
            "version": app.package_info().version.to_string(), "database_ready": true,
            "app_data_ready": true, "streaming_runtime_ready": true,
            "bundle_type": tauri::utils::platform::bundle_type().map(|kind| kind.to_string()),
            "app_data_dir": app.path().app_data_dir().map_err(|e| e.to_string())?,
            "app_cache_dir": app.path().app_cache_dir().map_err(|e| e.to_string())?,
            "app_config_dir": app.path().app_config_dir().map_err(|e| e.to_string())?,
            "app_local_data_dir": app.path().app_local_data_dir().map_err(|e| e.to_string())?,
        });
        let partial = smoke.root.join("startup-ready.partial");
        std::fs::write(
            &partial,
            serde_json::to_vec(&record).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        crate::desktop_preferences::atomic_replace(&partial, &smoke.root.join("startup-ready.json"))
            .map_err(|e| e.to_string())
    }
}
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(crate) use desktop::{configure, mark_ready, validate_profile};
#[cfg(any(target_os = "android", target_os = "ios"))]
pub(crate) fn configure<R: tauri::Runtime>(_context: &mut tauri::Context<R>) -> Result<(), String> {
    Ok(())
}
#[cfg(any(target_os = "android", target_os = "ios"))]
pub(crate) fn validate_profile<R: tauri::Runtime>(
    _app: &tauri::AppHandle<R>,
) -> Result<(), String> {
    Ok(())
}
#[cfg(any(target_os = "android", target_os = "ios"))]
pub(crate) fn mark_ready<R: tauri::Runtime>(_app: &tauri::AppHandle<R>) -> Result<(), String> {
    Ok(())
}
