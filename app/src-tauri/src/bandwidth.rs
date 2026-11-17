use jiff::{civil::Date, Span};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::Manager;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BandwidthStats {
    /// Monday that starts the current quota window. Kept as `date` for
    /// backwards compatibility with existing bandwidth.json files.
    pub date: String,
    pub up_bytes: u64,
    pub down_bytes: u64,
    #[serde(default = "weekly_limit_bytes")]
    pub limit_bytes: u64,
    #[serde(default = "weekly_period_name")]
    pub period: String,
}

const WEEKLY_LIMIT_BYTES: u64 = 250 * 1024 * 1024 * 1024;

fn weekly_limit_bytes() -> u64 {
    WEEKLY_LIMIT_BYTES
}
fn weekly_period_name() -> String {
    "weekly".to_string()
}

fn week_start_for(date: Date) -> Date {
    date - Span::new().days(i64::from(date.weekday().to_monday_zero_offset()))
}

impl Default for BandwidthStats {
    fn default() -> Self {
        let week_start = week_start_for(crate::app_time::local_datetime().date());
        Self {
            date: week_start.to_string(),
            up_bytes: 0,
            down_bytes: 0,
            limit_bytes: WEEKLY_LIMIT_BYTES,
            period: weekly_period_name(),
        }
    }
}

/// Successful declared payload is durable. Admission holds remain in memory,
/// so killing an unfinished transfer cannot charge it again on restart.
pub struct BandwidthManager {
    pub file_path: PathBuf,
    accounting: Mutex<Accounting>,
    limit: std::sync::atomic::AtomicU64,
    #[cfg(feature = "native-e2e")]
    date: Mutex<Option<Date>>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum ReservationDirection {
    Upload,
    Download,
}
struct Hold {
    bytes: u64,
    direction: ReservationDirection,
}
struct Accounting {
    stats: BandwidthStats,
    holds: std::collections::HashMap<u64, Hold>,
    next_id: u64,
    persistence_error: Option<String>,
    load_error: Option<String>,
}
pub struct BandwidthReservation {
    manager: std::sync::Arc<BandwidthManager>,
    id: u64,
    completed: bool,
}
impl BandwidthReservation {
    pub fn resize(&mut self, bytes: u64) -> Result<(), String> {
        self.manager.resize(self.id, bytes)
    }
    pub fn upload(manager: std::sync::Arc<BandwidthManager>, bytes: u64) -> Result<Self, String> {
        let id = manager.reserve(ReservationDirection::Upload, bytes)?;
        Ok(Self {
            manager,
            id,
            completed: false,
        })
    }
    pub fn download(manager: std::sync::Arc<BandwidthManager>, bytes: u64) -> Result<Self, String> {
        let id = manager.reserve(ReservationDirection::Download, bytes)?;
        Ok(Self {
            manager,
            id,
            completed: false,
        })
    }
    pub fn commit(&mut self) {
        if self.completed {
            return;
        }
        self.completed = true;
        if let Err(error) = self.manager.commit(self.id) {
            // Content already published must not be reported as an upload failure
            // that invites a duplicate publication. Keep the charged bytes in RAM;
            // new admission retries persistence and fails closed while it cannot save.
            log::error!("Bandwidth accounting could not be persisted: {error}");
        }
    }
}
impl Drop for BandwidthReservation {
    fn drop(&mut self) {
        if !self.completed {
            self.manager.release(self.id);
        }
    }
}
impl BandwidthManager {
    pub fn new(app: &tauri::AppHandle) -> Self {
        let root = app
            .path()
            .app_data_dir()
            .unwrap_or_else(|_| PathBuf::from("data"));
        Self::at(&root)
    }
    pub fn at(root: &Path) -> Self {
        let _ = fs::create_dir_all(root);
        let file_path = root.join("bandwidth.json");
        let (stats, load_error) = match Self::load(&file_path, true) {
            Ok(stats) => (stats, None),
            Err(error) => (BandwidthStats::default(), Some(error)),
        };
        let limit = stats.limit_bytes;
        Self {
            file_path,
            limit: std::sync::atomic::AtomicU64::new(limit),
            #[cfg(feature = "native-e2e")]
            date: Mutex::new(None),
            accounting: Mutex::new(Accounting {
                stats,
                holds: std::collections::HashMap::new(),
                next_id: 0,
                persistence_error: None,
                load_error,
            }),
        }
    }
    fn load(path: &Path, initial: bool) -> Result<BandwidthStats, String> {
        let value = match fs::read_to_string(path) {
            Ok(value) => value,
            Err(error) if initial && error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(BandwidthStats::default());
            }
            Err(error) => return Err(format!("Unable to read bandwidth accounting: {error}")),
        };
        let stats: BandwidthStats = serde_json::from_str(&value)
            .map_err(|error| format!("Invalid bandwidth accounting: {error}"))?;
        crate::app_time::parse_date(&stats.date)
            .map_err(|error| format!("Invalid bandwidth accounting date: {error}"))?;
        if stats.limit_bytes == 0 || stats.up_bytes.checked_add(stats.down_bytes).is_none() {
            return Err("Invalid bandwidth accounting quota or usage".into());
        }
        Ok(stats)
    }
    fn prepare(&self, state: &mut Accounting) -> Result<(), String> {
        if state.load_error.is_some() {
            // Preserve invalid existing data until it is repaired; never turn an
            // unreadable accounting file into a fresh quota window.
            match Self::load(&self.file_path, false) {
                Ok(stats) => {
                    self.limit
                        .store(stats.limit_bytes, std::sync::atomic::Ordering::SeqCst);
                    state.stats = stats;
                    state.load_error = None;
                }
                Err(error) => {
                    state.load_error = Some(error.clone());
                    return Err(error);
                }
            }
        }
        let today = self.today();
        let monday = week_start_for(today);
        let canonical = monday.to_string();
        let previous = state.stats.clone();
        let date = crate::app_time::parse_date(&state.stats.date)
            .map_err(|error| format!("Invalid bandwidth accounting date: {error}"))?;
        if week_start_for(date) > monday {
            return Err("Bandwidth clock moved backward; retaining the later quota window".into());
        }
        if date < monday {
            state.stats.up_bytes = 0;
            state.stats.down_bytes = 0;
        }
        state.stats.date = canonical;
        state.stats.period = weekly_period_name();
        state.stats.limit_bytes = self.limit.load(std::sync::atomic::Ordering::SeqCst);
        if previous.date != state.stats.date
            || previous.period != state.stats.period
            || previous.limit_bytes != state.stats.limit_bytes
        {
            self.persist(state)?;
        }
        if state.persistence_error.is_some() {
            self.persist(state)?;
        }
        Ok(())
    }
    fn persist(&self, state: &mut Accounting) -> Result<(), String> {
        match persist_stats_atomically(&self.file_path, &state.stats) {
            Ok(()) => {
                state.persistence_error = None;
                Ok(())
            }
            Err(error) => {
                state.persistence_error = Some(error.clone());
                Err(error)
            }
        }
    }
    fn used(state: &Accounting) -> Result<u64, String> {
        state
            .holds
            .values()
            .try_fold(
                state
                    .stats
                    .up_bytes
                    .checked_add(state.stats.down_bytes)
                    .ok_or("Bandwidth accounting overflowed")?,
                |total, hold| {
                    total
                        .checked_add(hold.bytes)
                        .ok_or("Bandwidth accounting overflowed")
                },
            )
            .map_err(str::to_string)
    }
    fn reserve(&self, direction: ReservationDirection, bytes: u64) -> Result<u64, String> {
        let mut state = self
            .accounting
            .lock()
            .map_err(|_| "Bandwidth accounting unavailable")?;
        self.prepare(&mut state)?;
        let total = Self::used(&state)?
            .checked_add(bytes)
            .ok_or("Bandwidth accounting overflowed")?;
        if total > state.stats.limit_bytes {
            return Err(format!(
                "Weekly bandwidth limit ({}) exceeded! Used: {}",
                Self::format_bytes(state.stats.limit_bytes),
                Self::format_bytes(total)
            ));
        }
        if state.holds.len() >= 512 {
            return Err("Too many bandwidth reservations".into());
        }
        let id = state
            .next_id
            .checked_add(1)
            .ok_or("Bandwidth reservation identifiers exhausted")?;
        // Verify durable accounting is writable before any content is transferred.
        self.persist(&mut state)?;
        state.next_id = id;
        state.holds.insert(id, Hold { bytes, direction });
        Ok(id)
    }
    fn release(&self, id: u64) {
        let mut state = self.accounting.lock().unwrap_or_else(|e| e.into_inner());
        // Identity, not byte subtraction, prevents a late old-week cancellation
        // from refunding committed bytes or another transfer's current-week hold.
        state.holds.remove(&id);
    }
    fn resize(&self, id: u64, bytes: u64) -> Result<(), String> {
        let mut state = self
            .accounting
            .lock()
            .map_err(|_| "Bandwidth accounting unavailable")?;
        self.prepare(&mut state)?;
        let old = state
            .holds
            .get(&id)
            .ok_or("Bandwidth reservation expired")?
            .bytes;
        let total = Self::used(&state)?
            .checked_sub(old)
            .and_then(|used| used.checked_add(bytes))
            .ok_or("Bandwidth accounting overflowed")?;
        if bytes > old && total > state.stats.limit_bytes {
            return Err("Weekly bandwidth limit exceeded".into());
        }
        self.persist(&mut state)?;
        state
            .holds
            .get_mut(&id)
            .ok_or("Bandwidth reservation expired")?
            .bytes = bytes;
        Ok(())
    }
    pub fn set_limit(&self, bytes: u64) -> Result<BandwidthStats, String> {
        if bytes == 0 {
            return Err("Weekly quota must be positive".into());
        }
        let mut state = self
            .accounting
            .lock()
            .map_err(|_| "Bandwidth accounting unavailable")?;
        self.prepare(&mut state)?;
        let previous = state.stats.clone();
        state.stats.limit_bytes = bytes;
        if let Err(error) = self.persist(&mut state) {
            state.stats = previous;
            return Err(error);
        }
        self.limit.store(bytes, std::sync::atomic::Ordering::SeqCst);
        Ok(Self::display_stats(&state))
    }
    fn today(&self) -> Date {
        #[cfg(feature = "native-e2e")]
        if let Some(date) = *self.date.lock().unwrap_or_else(|e| e.into_inner()) {
            return date;
        }
        crate::app_time::local_datetime().date()
    }
    #[cfg(feature = "native-e2e")]
    pub(crate) fn set_date(&self, date: Date) {
        *self.date.lock().unwrap_or_else(|e| e.into_inner()) = Some(date);
    }
    fn commit(&self, id: u64) -> Result<(), String> {
        let mut state = self
            .accounting
            .lock()
            .map_err(|_| "Bandwidth accounting unavailable")?;
        // Retain a conservative charge even if period rollover cannot be saved.
        let _ = self.prepare(&mut state);
        let hold = state
            .holds
            .remove(&id)
            .ok_or("Bandwidth reservation expired")?;
        let used = match hold.direction {
            ReservationDirection::Upload => &mut state.stats.up_bytes,
            ReservationDirection::Download => &mut state.stats.down_bytes,
        };
        *used = used
            .checked_add(hold.bytes)
            .ok_or("Bandwidth accounting overflowed")?;
        self.persist(&mut state)
    }
    pub fn check_and_reset(&self) {
        let mut state = self.accounting.lock().unwrap_or_else(|e| e.into_inner());
        if let Err(error) = self.prepare(&mut state) {
            log::error!("Unable to persist the bandwidth period: {error}");
        }
    }
    pub fn checked_stats(&self) -> Result<BandwidthStats, String> {
        let mut state = self
            .accounting
            .lock()
            .map_err(|_| "Bandwidth accounting unavailable")?;
        self.prepare(&mut state)?;
        Ok(Self::display_stats(&state))
    }
    pub fn get_stats(&self) -> BandwidthStats {
        let mut state = self.accounting.lock().unwrap_or_else(|e| e.into_inner());
        if let Err(error) = self.prepare(&mut state) {
            log::error!("Unable to persist bandwidth metadata: {error}");
        }
        Self::display_stats(&state)
    }
    fn display_stats(state: &Accounting) -> BandwidthStats {
        let mut stats = state.stats.clone();
        for hold in state.holds.values() {
            match hold.direction {
                ReservationDirection::Upload => {
                    stats.up_bytes = stats.up_bytes.saturating_add(hold.bytes)
                }
                ReservationDirection::Download => {
                    stats.down_bytes = stats.down_bytes.saturating_add(hold.bytes)
                }
            }
        }
        stats
    }
    fn format_bytes(bytes: u64) -> String {
        const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
        let mut value = bytes as f64;
        let mut index = 0;
        while value >= 1024.0 && index < UNITS.len() - 1 {
            value /= 1024.0;
            index += 1;
        }
        format!("{:.2} {}", value, UNITS[index])
    }
}

fn persist_stats_atomically(path: &Path, stats: &BandwidthStats) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Bandwidth data path has no parent".to_string())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let temporary = parent.join(format!(".bandwidth.{}.tmp", uuid::Uuid::new_v4()));
    let bytes = serde_json::to_vec_pretty(stats).map_err(|error| error.to_string())?;
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temporary)
            .map_err(|error| error.to_string())?;
        file.write_all(&bytes).map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        drop(file);
        atomic_replace(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(not(target_os = "windows"))]
fn atomic_replace(source: &Path, destination: &Path) -> Result<(), String> {
    fs::rename(source, destination).map_err(|error| error.to_string())
}

#[cfg(target_os = "windows")]
fn atomic_replace(source: &Path, destination: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::ReplaceFileW;
    if !destination.exists() {
        return fs::rename(source, destination).map_err(|error| error.to_string());
    }
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let source: Vec<u16> = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let replaced = unsafe {
        ReplaceFileW(
            destination.as_ptr(),
            source.as_ptr(),
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if replaced == 0 {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(())
    }
}
