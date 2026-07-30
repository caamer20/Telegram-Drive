use std::fs;
use std::path::{Path, PathBuf};

pub const HARDCODED_PRIMARY_TEMP: &str = "/Volumes/DATASTORE/Temo";

/// Resolve the temporary directory for media processing, downloads, uploads, and archives.
///
/// Priority:
/// 1. `custom_dir` (e.g. `local_backup_dir`) if provided and accessible.
/// 2. `/Volumes/DATASTORE/Temo` if accessible (created automatically if possible).
/// 3. Fallback to system OS temp directory `std::env::temp_dir()`.
pub fn get_media_temp_dir(custom_dir: Option<&Path>) -> PathBuf {
    if let Some(dir) = custom_dir {
        if dir.exists() || fs::create_dir_all(dir).is_ok() {
            let media_tmp = dir.join(".media_tmp");
            if fs::create_dir_all(&media_tmp).is_ok() {
                return media_tmp;
            }
            return dir.to_path_buf();
        }
    }

    let hardcoded_path = Path::new(HARDCODED_PRIMARY_TEMP);
    if hardcoded_path.exists() || fs::create_dir_all(hardcoded_path).is_ok() {
        return hardcoded_path.to_path_buf();
    }

    std::env::temp_dir()
}

/// Helper to get a unique temporary file path within the resolved temp directory.
pub fn get_temp_file_path(custom_dir: Option<&Path>, prefix: &str, extension: &str) -> PathBuf {
    let base_dir = get_media_temp_dir(custom_dir);
    let ext = extension.trim_start_matches('.');
    let filename = if ext.is_empty() {
        format!(
            "{}_{}_{}",
            prefix,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
            rand::random::<u32>()
        )
    } else {
        format!(
            "{}_{}_{}.{}",
            prefix,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
            rand::random::<u32>(),
            ext
        )
    };
    base_dir.join(filename)
}
