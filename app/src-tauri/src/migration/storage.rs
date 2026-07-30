use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const MIGRATION_RUNTIME_DIR: &str = ".telegram-drive-migration";
pub const MIGRATION_WORKSPACE_DIR: &str = "workspace";

/// Resolve the workspace for a new migration under the user-selected backup root.
/// The canonical backup root is used so a missing external volume cannot silently
/// fall back to a similarly named directory on the internal disk.
pub fn prepare_external_workspace(local_backup_dir: &Path) -> Result<PathBuf, String> {
    let backup_root = canonical_backup_root(local_backup_dir)?;
    let runtime_root = backup_root.join(MIGRATION_RUNTIME_DIR);
    let workspace = runtime_root.join(MIGRATION_WORKSPACE_DIR);
    fs::create_dir_all(&workspace).map_err(|error| {
        format!("Cannot create migration workspace on selected storage: {error}")
    })?;
    validate_workspace_available(&workspace, &backup_root)?;
    Ok(workspace)
}

/// Validate a persisted workspace before resume/retry without creating a missing
/// path. This is fail-closed: an unavailable external disk pauses migration
/// instead of recreating the workspace on the Mac's internal disk.
pub fn validate_persisted_workspace(
    workspace_dir: &Path,
    local_backup_dir: &Path,
) -> Result<PathBuf, String> {
    let backup_root = canonical_backup_root(local_backup_dir)?;
    if !workspace_dir.is_dir() {
        return Err(format!(
            "Migration storage is unavailable; reconnect the selected external drive: {}",
            workspace_dir.display()
        ));
    }
    let workspace = workspace_dir.canonicalize().map_err(|error| {
        format!(
            "Cannot resolve persisted migration workspace {}: {error}",
            workspace_dir.display()
        )
    })?;
    validate_workspace_available(&workspace, &backup_root)?;
    Ok(workspace)
}

fn canonical_backup_root(local_backup_dir: &Path) -> Result<PathBuf, String> {
    if !local_backup_dir.is_dir() {
        return Err(format!(
            "Selected migration storage is unavailable: {}",
            local_backup_dir.display()
        ));
    }
    local_backup_dir.canonicalize().map_err(|error| {
        format!(
            "Cannot resolve selected migration storage {}: {error}",
            local_backup_dir.display()
        )
    })
}

fn validate_workspace_available(workspace: &Path, backup_root: &Path) -> Result<(), String> {
    if !workspace.starts_with(backup_root) {
        return Err(format!(
            "Migration workspace must stay under selected storage: {}",
            backup_root.display()
        ));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let backup_device = fs::metadata(backup_root)
            .map_err(|error| format!("Cannot inspect selected migration storage: {error}"))?
            .dev();
        let workspace_device = fs::metadata(workspace)
            .map_err(|error| format!("Cannot inspect migration workspace: {error}"))?
            .dev();
        if backup_device != workspace_device {
            return Err(
                "Migration workspace and selected storage are on different filesystems".into(),
            );
        }
    }

    let probe = workspace.join(format!(
        ".storage-probe-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    let probe_result = (|| -> Result<(), String> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe)
            .map_err(|error| format!("Migration storage is not writable: {error}"))?;
        file.write_all(b"telegram-drive-migration-storage-probe")
            .map_err(|error| format!("Cannot write migration storage probe: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("Cannot sync migration storage probe: {error}"))?;
        Ok(())
    })();
    let _ = fs::remove_file(&probe);
    probe_result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "migration-storage-{label}-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ))
    }

    #[test]
    fn new_workspace_is_created_under_selected_storage() {
        let root = temp_root("new");
        fs::create_dir_all(&root).unwrap();
        let workspace = prepare_external_workspace(&root).unwrap();
        assert_eq!(
            workspace,
            root.canonicalize()
                .unwrap()
                .join(MIGRATION_RUNTIME_DIR)
                .join(MIGRATION_WORKSPACE_DIR)
        );
        assert!(workspace.is_dir());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn resume_does_not_recreate_missing_workspace() {
        let root = temp_root("missing");
        fs::create_dir_all(&root).unwrap();
        let missing = root
            .join(MIGRATION_RUNTIME_DIR)
            .join(MIGRATION_WORKSPACE_DIR);
        let error = validate_persisted_workspace(&missing, &root).unwrap_err();
        assert!(error.contains("reconnect"));
        assert!(!missing.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn persisted_workspace_must_be_under_selected_storage() {
        let root = temp_root("root");
        let elsewhere = temp_root("elsewhere");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&elsewhere).unwrap();
        let error = validate_persisted_workspace(&elsewhere, &root).unwrap_err();
        assert!(error.contains("must stay under"));
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(elsewhere).unwrap();
    }
}
