//! Application logging: the developer console plus a small, redacted log file.
//!
//! Console output keeps `env_logger` semantics (`RUST_LOG`). The file exists
//! so a problem can be diagnosed after the fact. It records warnings and
//! errors only, unless `TELEGRAM_DRIVE_LOG_LEVEL` asks for more, and every
//! line is redacted before it reaches the disk.
use log::{Level, LevelFilter, Log, Metadata, Record};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

pub const LOG_FILE_NAME: &str = "telegram-drive.log";
pub const PREVIOUS_LOG_FILE_NAME: &str = "telegram-drive.previous.log";
/// One rotation is kept, so logs never use more than twice this on disk.
const MAX_LOG_BYTES: u64 = 1024 * 1024;
const MAX_LINE_BYTES: usize = 4 * 1024;

struct LogFile {
    file: File,
    directory: PathBuf,
    written: u64,
}

struct AppLogger {
    console: env_logger::Logger,
    file_level: LevelFilter,
    file: Mutex<Option<LogFile>>,
}

static LOGGER: OnceLock<AppLogger> = OnceLock::new();

fn file_level_from_environment() -> LevelFilter {
    match std::env::var("TELEGRAM_DRIVE_LOG_LEVEL")
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "error" => LevelFilter::Error,
        "info" => LevelFilter::Info,
        "debug" => LevelFilter::Debug,
        // File names and folder names appear at lower levels throughout the
        // application, so the default records problems only.
        _ => LevelFilter::Warn,
    }
}

/// Install the logger. Safe to call once at process start; later calls are
/// ignored. The file is attached separately once its directory is known.
pub fn init() {
    let logger = LOGGER.get_or_init(|| AppLogger {
        console: env_logger::Builder::from_default_env().build(),
        file_level: file_level_from_environment(),
        file: Mutex::new(None),
    });
    if log::set_logger(logger).is_ok() {
        log::set_max_level(logger.console.filter().max(logger.file_level));
    }
}

fn open_private(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

fn open_log_file(directory: &Path) -> std::io::Result<LogFile> {
    std::fs::create_dir_all(directory)?;
    let path = directory.join(LOG_FILE_NAME);
    let existing = std::fs::metadata(&path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let written = if existing >= MAX_LOG_BYTES {
        let _ = std::fs::rename(&path, directory.join(PREVIOUS_LOG_FILE_NAME));
        0
    } else {
        existing
    };
    Ok(LogFile {
        file: open_private(&path)?,
        directory: directory.to_path_buf(),
        written,
    })
}

/// Start writing the log file in `directory`. Returns the file's path.
pub fn attach_file(directory: &Path) -> Result<PathBuf, String> {
    let logger = LOGGER.get().ok_or("Logging has not been initialised")?;
    let log_file = open_log_file(directory).map_err(|error| error.to_string())?;
    *logger
        .file
        .lock()
        .map_err(|_| "Log file lock poisoned".to_string())? = Some(log_file);
    Ok(directory.join(LOG_FILE_NAME))
}

/// Path of the attached log file, if any.
pub fn file_path() -> Option<PathBuf> {
    let logger = LOGGER.get()?;
    let file = logger.file.lock().ok()?;
    file.as_ref()
        .map(|log_file| log_file.directory.join(LOG_FILE_NAME))
}

fn is_token_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '=')
}

/// Query-style keys whose values are credentials wherever they appear.
const SECRET_KEYS: [&str; 9] = [
    "token=",
    "credential=",
    "api_hash=",
    "apihash=",
    "password=",
    "passphrase=",
    "secret=",
    "key=",
    "signature=",
];

/// Remove what must never be written to disk from a log line: credentials in
/// `key=value` form, phone numbers, long opaque tokens, and the name of the
/// user's home directory. Messages stay readable for diagnosis.
pub fn redact(message: &str) -> String {
    let mut text = message.replace(['\r', '\n'], " ");
    for variable in ["HOME", "USERPROFILE"] {
        if let Ok(home) = std::env::var(variable) {
            if home.len() > 3 {
                text = text.replace(&home, "~");
            }
        }
    }

    let lower = text.to_ascii_lowercase();
    let characters: Vec<char> = text.chars().collect();
    let lower_characters: Vec<char> = lower.chars().collect();
    let mut output = String::with_capacity(text.len());
    let mut index = 0;
    while index < characters.len() {
        // key=value credentials.
        let secret_key = SECRET_KEYS.iter().find(|key| {
            let key: Vec<char> = key.chars().collect();
            lower_characters[index..].starts_with(&key)
                && (index == 0 || !lower_characters[index - 1].is_ascii_alphanumeric())
        });
        if let Some(key) = secret_key {
            let key_length = key.chars().count();
            output.extend(&characters[index..index + key_length]);
            index += key_length;
            let start = index;
            while index < characters.len()
                && !characters[index].is_whitespace()
                && !matches!(characters[index], '&' | '"' | '\'' | ',' | ';' | ')')
            {
                index += 1;
            }
            if index > start {
                output.push_str("<redacted>");
            }
            continue;
        }

        // International phone numbers.
        if characters[index] == '+' {
            let digits = characters[index + 1..]
                .iter()
                .take_while(|character| character.is_ascii_digit())
                .count();
            if (8..=15).contains(&digits) {
                output.push_str("+<redacted>");
                index += 1 + digits;
                continue;
            }
        }

        // Long opaque runs: session tokens, hashes, keys, encoded bundles.
        if is_token_character(characters[index])
            && (index == 0 || !is_token_character(characters[index - 1]))
        {
            let run = characters[index..]
                .iter()
                .take_while(|character| is_token_character(**character))
                .count();
            let has_digit = characters[index..index + run]
                .iter()
                .any(|character| character.is_ascii_digit());
            if run >= 32 && has_digit {
                output.push_str("<redacted>");
                index += run;
                continue;
            }
        }

        output.push(characters[index]);
        index += 1;
    }

    if output.len() > MAX_LINE_BYTES {
        let mut end = MAX_LINE_BYTES;
        while !output.is_char_boundary(end) {
            end -= 1;
        }
        output.truncate(end);
        output.push('…');
    }
    output
}

impl AppLogger {
    fn write_to_file(&self, record: &Record) {
        let Ok(mut guard) = self.file.lock() else {
            return;
        };
        let Some(log_file) = guard.as_mut() else {
            return;
        };
        let line = format!(
            "{} {:<5} {}: {}\n",
            crate::app_time::utc_log_stamp(),
            record.level(),
            record.target(),
            redact(&record.args().to_string())
        );
        if log_file.written.saturating_add(line.len() as u64) > MAX_LOG_BYTES {
            // Rotate in place; a failure simply stops file logging.
            let directory = log_file.directory.clone();
            *guard = None;
            let _ = std::fs::rename(
                directory.join(LOG_FILE_NAME),
                directory.join(PREVIOUS_LOG_FILE_NAME),
            );
            match open_log_file(&directory) {
                Ok(reopened) => *guard = Some(reopened),
                Err(_) => return,
            }
        }
        if let Some(log_file) = guard.as_mut() {
            if log_file.file.write_all(line.as_bytes()).is_ok() {
                log_file.written += line.len() as u64;
                if record.level() <= Level::Error {
                    let _ = log_file.file.flush();
                }
            }
        }
    }
}

impl Log for AppLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        self.console.enabled(metadata) || metadata.level() <= self.file_level
    }

    fn log(&self, record: &Record) {
        if self.console.enabled(record.metadata()) {
            self.console.log(record);
        }
        if record.level() <= self.file_level {
            self.write_to_file(record);
        }
    }

    fn flush(&self) {
        self.console.flush();
        if let Ok(mut guard) = self.file.lock() {
            if let Some(log_file) = guard.as_mut() {
                let _ = log_file.file.flush();
            }
        }
    }
}
