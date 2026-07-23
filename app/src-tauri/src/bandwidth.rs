use chrono::Local;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::Manager;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BandwidthStats {
    pub date: String,
    pub up_bytes: u64,
    pub down_bytes: u64,
}

impl Default for BandwidthStats {
    fn default() -> Self {
        Self {
            date: Local::now().format("%Y-%m-%d").to_string(),
            up_bytes: 0,
            down_bytes: 0,
        }
    }
}

pub struct BandwidthManager {
    pub file_path: PathBuf,
    pub stats: Mutex<BandwidthStats>,
    pub limit: u64, // Daily limit in bytes
}

impl BandwidthManager {
    pub fn new(app_handle: &tauri::AppHandle) -> Self {
        // Resolve app data directory
        let app_data_dir = app_handle
            .path()
            .app_data_dir()
            .unwrap_or_else(|_| PathBuf::from("data"));

        if !app_data_dir.exists() {
            let _ = std::fs::create_dir_all(&app_data_dir);
        }
        let file_path = app_data_dir.join("bandwidth.json");

        let stats = if file_path.exists() {
            let content = fs::read_to_string(&file_path).unwrap_or_default();
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            BandwidthStats::default()
        };

        Self {
            file_path,
            stats: Mutex::new(stats),
            limit: 250 * 1024 * 1024 * 1024, // 250 GB
        }
    }

    pub fn check_and_reset(&self) {
        let today = Local::now().format("%Y-%m-%d").to_string();
        let mut stats = self.stats.lock().unwrap();
        if stats.date != today {
            println!(
                "[Bandwidth] New day detected. Resetting stats. Old date: {}, New date: {}",
                stats.date, today
            );
            stats.date = today;
            stats.up_bytes = 0;
            stats.down_bytes = 0;
            // Save immediately using the existing locked stats reference
            self.save_locked(&stats);
        }
    }

    pub fn can_transfer(&self, bytes: u64) -> Result<(), String> {
        self.check_and_reset();
        let stats = self.stats.lock().unwrap();
        let total = stats.up_bytes + stats.down_bytes + bytes;
        if total > self.limit {
            return Err(format!(
                "Daily bandwidth limit ({}) exceeded! Used: {}",
                self.format_bytes(self.limit),
                self.format_bytes(total)
            ));
        }
        Ok(())
    }

    /// Atomically check the limit AND reserve bandwidth for an upload.
    /// Call release_up() if the transfer fails to avoid permanently consuming quota.
    pub fn try_reserve_up(&self, bytes: u64) -> Result<(), String> {
        self.check_and_reset();
        let mut stats = self.stats.lock().unwrap();
        let total = stats.up_bytes + stats.down_bytes + bytes;
        if total > self.limit {
            return Err(format!(
                "Daily bandwidth limit ({}) exceeded! Used: {}",
                self.format_bytes(self.limit),
                self.format_bytes(total)
            ));
        }
        stats.up_bytes += bytes;
        self.save_locked(&stats);
        Ok(())
    }

    /// Atomically check the limit AND reserve bandwidth for a download.
    /// Call release_down() if the transfer fails to avoid permanently consuming quota.
    pub fn try_reserve_down(&self, bytes: u64) -> Result<(), String> {
        self.check_and_reset();
        let mut stats = self.stats.lock().unwrap();
        let total = stats.up_bytes + stats.down_bytes + bytes;
        if total > self.limit {
            return Err(format!(
                "Daily bandwidth limit ({}) exceeded! Used: {}",
                self.format_bytes(self.limit),
                self.format_bytes(total)
            ));
        }
        stats.down_bytes += bytes;
        self.save_locked(&stats);
        Ok(())
    }

    /// Release reserved upload bandwidth after a failed transfer.
    pub fn release_up(&self, bytes: u64) {
        let mut stats = self.stats.lock().unwrap();
        stats.up_bytes = stats.up_bytes.saturating_sub(bytes);
        self.save_locked(&stats);
    }

    /// Release reserved download bandwidth after a failed transfer.
    pub fn release_down(&self, bytes: u64) {
        let mut stats = self.stats.lock().unwrap();
        stats.down_bytes = stats.down_bytes.saturating_sub(bytes);
        self.save_locked(&stats);
    }

    pub fn add_up(&self, bytes: u64) {
        self.check_and_reset();
        let mut stats = self.stats.lock().unwrap();
        stats.up_bytes += bytes;
        self.save_locked(&stats);
    }

    pub fn add_down(&self, bytes: u64) {
        self.check_and_reset();
        let mut stats = self.stats.lock().unwrap();
        stats.down_bytes += bytes;
        self.save_locked(&stats);
    }

    fn save_locked(&self, stats: &BandwidthStats) {
        if let Ok(json) = serde_json::to_string(stats) {
            let _ = fs::write(&self.file_path, json);
        }
    }

    pub fn get_stats(&self) -> BandwidthStats {
        self.check_and_reset();
        self.stats.lock().unwrap().clone()
    }

    fn format_bytes(&self, bytes: u64) -> String {
        const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
        let mut v = bytes as f64;
        let mut i = 0;
        while v >= 1024.0 && i < UNITS.len() - 1 {
            v /= 1024.0;
            i += 1;
        }
        format!("{:.2} {}", v, UNITS[i])
    }
}
