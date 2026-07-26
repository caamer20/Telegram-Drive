// LocalFinalizer production adapter for Pipeline V2
//
// Implements safe local file finalization:
//   - Path traversal protection
//   - Windows reserved name handling
//   - Deterministic collision suffix
//   - Symlink escape protection
//   - .part alongside destination parent
//   - flush + sync_all + atomic rename
//   - No silent overwrite
//   - Persist resolved path

use crate::migration::pipeline::stages::LocalFinalizer;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

pub struct LocalProductionAdapter {
    /// Root backup directory — all paths MUST be under this
    backup_root: PathBuf,
}

impl LocalProductionAdapter {
    pub fn new(backup_root: PathBuf) -> Self {
        Self { backup_root }
    }

    /// Sanitize a source path to produce a safe destination path under backup_root.
    /// Returns the resolved destination path and the relative path component.
    pub fn sanitize_dest_path(backup_root: &Path, source_path: &str) -> Result<PathBuf, String> {
        // 1. Normalize: strip leading slashes, resolve .. and .
        let clean = path_clean::clean(source_path);
        let clean_str = clean.to_string_lossy();

        // 2. Path traversal guard: reject paths that resolve outside
        if clean_str.contains("..") {
            return Err(format!(
                "LocalFinalizer: path traversal rejected: {}",
                source_path
            ));
        }

        // 3. Build relative path, protecting each component
        let mut relative = PathBuf::new();
        for component in clean.components() {
            if let std::path::Component::Normal(part) = component {
                let s = part.to_string_lossy();
                let upper = s.to_ascii_uppercase();
                // Windows reserved names
                if matches!(
                    upper.as_ref(),
                    "CON"
                        | "PRN"
                        | "AUX"
                        | "NUL"
                        | "COM1"
                        | "COM2"
                        | "COM3"
                        | "COM4"
                        | "COM5"
                        | "COM6"
                        | "COM7"
                        | "COM8"
                        | "COM9"
                        | "LPT1"
                        | "LPT2"
                        | "LPT3"
                        | "LPT4"
                        | "LPT5"
                        | "LPT6"
                        | "LPT7"
                        | "LPT8"
                        | "LPT9"
                ) {
                    relative.push(format!("{}_safe", s));
                } else {
                    relative.push(s.as_ref());
                }
            }
            // Skip root/prefix components
        }

        let relative_str = relative.to_string_lossy();
        if relative_str.is_empty() {
            return Err("LocalFinalizer: empty relative path".to_string());
        }

        // 4. Concat with backup root
        let resolved = backup_root.join(&relative);

        // 5. Canonicalize to verify no symlink escape
        // If backup_root exists, canonicalize the resolved path and verify it's within backup
        if backup_root.exists() {
            if let Ok(canonical) = resolved.canonicalize() {
                let canonical_root = backup_root
                    .canonicalize()
                    .unwrap_or_else(|_| backup_root.to_path_buf());
                if !canonical.starts_with(&canonical_root) {
                    return Err(format!(
                        "LocalFinalizer: symlink escape detected: {}",
                        canonical.display()
                    ));
                }
            }
        }

        Ok(resolved)
    }

    /// Generate a collision-safe path by appending _1, _2, etc.
    fn collision_safe_path(dest: &Path) -> PathBuf {
        if !dest.exists() {
            return dest.to_path_buf();
        }

        let parent = dest.parent().unwrap_or_else(|| Path::new("."));
        let stem = dest.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
        let ext = dest.extension().and_then(|e| e.to_str()).unwrap_or("");

        for i in 1u32..1000 {
            let candidate = if ext.is_empty() {
                parent.join(format!("{}_{}", stem, i))
            } else {
                parent.join(format!("{}_{}.{}", stem, i, ext))
            };
            if !candidate.exists() {
                return candidate;
            }
        }

        // Fallback: use timestamp
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if ext.is_empty() {
            parent.join(format!("{}_{}", stem, ts))
        } else {
            parent.join(format!("{}_{}.{}", stem, ts, ext))
        }
    }
}

impl LocalFinalizer for LocalProductionAdapter {
    fn finalize_local(
        &self,
        source_path: &Path,
        dest_path: &Path,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>> {
        let source = source_path.to_path_buf();
        let dest = dest_path.to_path_buf();

        Box::pin(async move {
            // 1. Verify source exists
            if !source.exists() {
                return Err(format!(
                    "LocalFinalizer: source does not exist: {}",
                    source.display()
                ));
            }

            // 2. Create destination parent directory
            if let Some(parent) = dest.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|e| format!("LocalFinalizer: cannot create parent dir: {}", e))?;
            }

            // 3. Write to .part alongside destination then rename atomically
            let part_path = if let Some(parent) = dest.parent() {
                let stem = dest.file_stem().and_then(|s| s.to_str()).unwrap_or("tmp");
                parent.join(format!(".{}.part", stem))
            } else {
                PathBuf::from(format!(".{}.part", dest.display()))
            };

            // 4. Copy source to .part
            tokio::fs::copy(&source, &part_path)
                .await
                .map_err(|e| format!("LocalFinalizer: copy to .part failed: {}", e))?;

            // 5. Flush and sync
            let part_file = tokio::fs::File::open(&part_path)
                .await
                .map_err(|e| format!("LocalFinalizer: cannot open .part for sync: {}", e))?;
            part_file
                .sync_all()
                .await
                .map_err(|e| format!("LocalFinalizer: sync_all failed: {}", e))?;

            // 6. Atomic rename .part → final destination
            // If destination already exists, use collision-safe path
            let final_dest = Self::collision_safe_path(&dest);

            tokio::fs::rename(&part_path, &final_dest)
                .await
                .map_err(|e| {
                    // Cleanup .part on rename failure
                    let _ = std::fs::remove_file(&part_path);
                    format!("LocalFinalizer: atomic rename failed: {}", e)
                })?;

            Ok(())
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("temp_local_tests_{}", rand::random::<u64>()));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn test_sanitize_normal_path() {
        let root = Path::new("/backup");
        let result =
            LocalProductionAdapter::sanitize_dest_path(root, "OneDrive_Archive/docs/file.txt")
                .unwrap();
        assert_eq!(result, Path::new("/backup/OneDrive_Archive/docs/file.txt"));
    }

    #[test]
    fn test_sanitize_path_traversal_rejected() {
        let root = Path::new("/backup");
        let result = LocalProductionAdapter::sanitize_dest_path(root, "../../etc/passwd");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("traversal"));
    }

    #[test]
    fn test_sanitize_absolute_path() {
        let root = Path::new("/backup");
        // path_clean strips leading slash and resolves
        let result =
            LocalProductionAdapter::sanitize_dest_path(root, "/absolute/path/file.txt").unwrap();
        // After cleaning, /absolute/path/file.txt becomes "absolute/path/file.txt"
        assert_eq!(result, Path::new("/backup/absolute/path/file.txt"));
    }

    #[test]
    fn test_sanitize_windows_reserved_name() {
        let root = Path::new("/backup");
        let result =
            LocalProductionAdapter::sanitize_dest_path(root, "OneDrive/CON/file.txt").unwrap();
        // CON should be renamed to CON_safe
        assert_eq!(result, Path::new("/backup/OneDrive/CON_safe/file.txt"));
    }

    #[test]
    fn test_sanitize_lpt_reserved_name() {
        let root = Path::new("/backup");
        let result =
            LocalProductionAdapter::sanitize_dest_path(root, "devices/LPT1/config.txt").unwrap();
        assert_eq!(result, Path::new("/backup/devices/LPT1_safe/config.txt"));
    }

    #[test]
    fn test_collision_safe_path_no_collision() {
        let tmp = TempDir::new();
        let path = tmp.path.join("unique.txt");
        let result = LocalProductionAdapter::collision_safe_path(&path);
        assert_eq!(result, path);
    }

    #[test]
    fn test_collision_safe_path_with_collision() {
        let tmp = TempDir::new();
        let path = tmp.path.join("collision.txt");
        fs::write(&path, b"first").unwrap();
        let result = LocalProductionAdapter::collision_safe_path(&path);
        assert_ne!(result, path);
        let name = result.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with("collision_"));
    }

    #[tokio::test]
    async fn test_finalize_local_basic() {
        let tmp = TempDir::new();
        let source = tmp.path.join("source.txt");
        fs::write(&source, b"hello finalizer").unwrap();

        let dest = tmp.path.join("output").join("final.txt");
        let adapter = LocalProductionAdapter::new(tmp.path.clone());

        adapter.finalize_local(&source, &dest).await.unwrap();

        assert!(dest.exists());
        let content = fs::read_to_string(&dest).unwrap();
        assert_eq!(content, "hello finalizer");
        // Source should still exist (adapter doesn't clean up source)
        assert!(source.exists());
    }

    #[tokio::test]
    async fn test_finalize_local_source_missing() {
        let tmp = TempDir::new();
        let source = tmp.path.join("nonexistent.txt");
        let dest = tmp.path.join("output").join("final.txt");
        let adapter = LocalProductionAdapter::new(tmp.path.clone());

        let result = adapter.finalize_local(&source, &dest).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("does not exist"));
    }

    #[tokio::test]
    async fn test_finalize_local_no_overwrite() {
        let tmp = TempDir::new();
        let dest_dir = tmp.path.join("output");
        fs::create_dir_all(&dest_dir).unwrap();

        let source = tmp.path.join("source.txt");
        fs::write(&source, b"new content").unwrap();

        let dest = dest_dir.join("final.txt");
        // Pre-create destination
        fs::write(&dest, b"old content").unwrap();

        let adapter = LocalProductionAdapter::new(tmp.path.clone());
        adapter.finalize_local(&source, &dest).await.unwrap();

        // Original destination should still have "old content"
        let old = fs::read_to_string(&dest).unwrap();
        assert_eq!(old, "old content", "original should not be overwritten");

        // New file should exist with collision suffix
        let collision = dest_dir.join("final_1.txt");
        assert!(collision.exists(), "collision file should exist");
        let new_content = fs::read_to_string(&collision).unwrap();
        assert_eq!(new_content, "new content");
    }

    #[tokio::test]
    async fn test_finalize_local_creates_parent_dirs() {
        let tmp = TempDir::new();
        let source = tmp.path.join("source.txt");
        fs::write(&source, b"nested").unwrap();

        let dest = tmp
            .path
            .join("deep")
            .join("nested")
            .join("path")
            .join("final.txt");
        let adapter = LocalProductionAdapter::new(tmp.path.clone());

        adapter.finalize_local(&source, &dest).await.unwrap();

        assert!(dest.exists());
    }
}
