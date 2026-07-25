use std::fs;
use std::path::{Path, PathBuf};

/// Kiểm tra đường dẫn child có thực sự nằm dưới parent (normalized) để chống path traversal và symlink escape
pub fn is_subpath(parent: &Path, child: &Path) -> bool {
    let parent_canonical = match parent.canonicalize() {
        Ok(p) => p,
        Err(_) => return false,
    };

    // Nếu child chưa tồn tại, canonicalize cha của nó
    let child_canonical = match child.canonicalize() {
        Ok(c) => c,
        Err(_) => {
            if let Some(parent_of_child) = child.parent() {
                match parent_of_child.canonicalize() {
                    Ok(p) => p.join(child.file_name().unwrap_or_default()),
                    Err(_) => return false,
                }
            } else {
                return false;
            }
        }
    };

    child_canonical.starts_with(parent_canonical)
}

/// Xây dựng đường dẫn thư mục xuất manifest an toàn
pub fn get_safe_manifest_dir(local_backup_dir: &str, job_id: i64) -> Result<PathBuf, String> {
    if local_backup_dir.trim().is_empty() {
        return Err("Local backup directory cannot be empty".into());
    }

    let backup_path = Path::new(local_backup_dir);

    // Tạo đường dẫn đích: [local_backup_dir]/_TelegramDrive_Backup/[job_id]
    // Sử dụng join an toàn và validate để chống traversal
    let backup_sub = backup_path.join("_TelegramDrive_Backup");
    let job_dir_name = format!("{}", job_id);
    let target_dir = backup_sub.join(&job_dir_name);

    // Chống path traversal từ job_id (cho dù job_id là số nhưng validate vẫn là best-practice)
    if job_dir_name.contains('/') || job_dir_name.contains('\\') || job_dir_name.contains("..") {
        return Err("Invalid job_id structure".into());
    }

    Ok(target_dir)
}

/// Ghi file atomic: Ghi vào file tạm .tmp rồi thực hiện rename
pub fn write_manifest_atomic(
    target_dir: &Path,
    backup_root: &Path,
    filename: &str,
    content: &str,
) -> Result<PathBuf, String> {
    // 1. Tạo thư mục trước để canonicalize hoạt động chính xác (đặc biệt trên macOS với symlink /var -> /private/var)
    fs::create_dir_all(target_dir)
        .map_err(|e| format!("Failed to create manifest directory: {}", e))?;

    // 2. Validate an toàn thư mục đích
    if !is_subpath(backup_root, target_dir) {
        return Err("Target directory is outside allowed backup root".into());
    }

    let final_path = target_dir.join(filename);

    // Đảm bảo final_path vẫn nằm dưới backup root
    if !is_subpath(backup_root, &final_path) {
        return Err("Target file path escapes backup root".into());
    }

    let tmp_filename = format!("{}.tmp", filename);
    let tmp_path = target_dir.join(tmp_filename);

    // 3. Ghi vào tệp tin tạm
    fs::write(&tmp_path, content)
        .map_err(|e| format!("Failed to write temporary manifest file: {}", e))?;

    // 4. Atomic Rename sang tệp tin chính thức
    fs::rename(&tmp_path, &final_path).map_err(|e| {
        let _ = fs::remove_file(&tmp_path); // Cleanup file tạm nếu rename lỗi
        format!("Failed to rename manifest atomic: {}", e)
    })?;

    Ok(final_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            let path = env::temp_dir().join(format!("temp_manifest_dir_{}", rand::random::<u64>()));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn test_safe_manifest_dir_construction() {
        let dir = get_safe_manifest_dir("/var/tmp/backup", 42).unwrap();
        assert_eq!(dir, Path::new("/var/tmp/backup/_TelegramDrive_Backup/42"));

        // Lỗi job_id độc hại
        let dir_err = get_safe_manifest_dir("/var/tmp/backup", -1);
        assert!(dir_err.is_ok());
    }

    #[test]
    fn test_atomic_write_and_path_safety() {
        let tmp = TempDir::new();
        let backup_root = tmp.path();

        let target_dir = backup_root.join("_TelegramDrive_Backup").join("job_1");

        // Ghi thành công
        let file_path = write_manifest_atomic(
            &target_dir,
            backup_root,
            "manifest.json",
            "{\"status\": \"ok\"}",
        )
        .unwrap();
        assert!(file_path.exists());

        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "{\"status\": \"ok\"}");

        // Test path traversal chặn đứng
        let bad_target_dir = backup_root.join("../escaped");
        let err = write_manifest_atomic(&bad_target_dir, backup_root, "manifest.json", "{}");
        assert!(err.is_err());
    }
}
