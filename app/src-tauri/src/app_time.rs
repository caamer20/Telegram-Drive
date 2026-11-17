//! Date/time compatibility at the application's storage and HTTP boundaries.
#[cfg(feature = "native-e2e")]
use jiff::fmt::temporal::DateTimePrinter;
#[cfg(any(windows, feature = "native-e2e"))]
use jiff::tz::Offset;
#[cfg(not(windows))]
use jiff::tz::TimeZone;
use jiff::{
    civil::{Date, DateTime},
    Timestamp,
};
use std::borrow::Cow;
#[cfg(not(windows))]
use std::{
    sync::Mutex,
    time::{Duration, SystemTime},
};

const UNIX_EPOCH: DateTime = DateTime::constant(1970, 1, 1, 0, 0, 0, 0);

pub fn unix_seconds() -> i64 {
    // Stored epochs floor fractional instants, including those before 1970.
    Timestamp::now().as_nanosecond().div_euclid(1_000_000_000) as i64
}

pub fn unix_millis() -> i64 {
    Timestamp::now().as_nanosecond().div_euclid(1_000_000) as i64
}

pub fn unix_nanos() -> i64 {
    i64::try_from(Timestamp::now().as_nanosecond()).unwrap_or(0)
}

pub fn utc_log_stamp() -> String {
    Timestamp::now().strftime("%Y-%m-%dT%H:%M:%SZ").to_string()
}

pub fn utc_compact_stamp() -> String {
    Timestamp::now().strftime("%Y%m%dT%H%M%SZ").to_string()
}

pub fn utc_millis_stamp() -> String {
    Timestamp::now().strftime("%Y%m%dT%H%M%S%3fZ").to_string()
}

#[cfg(feature = "native-e2e")]
pub fn now_rfc3339() -> String {
    let timestamp = Timestamp::now();
    let fraction = timestamp.subsec_nanosecond();
    let precision = if fraction == 0 {
        0
    } else if fraction % 1_000_000 == 0 {
        3
    } else if fraction % 1_000 == 0 {
        6
    } else {
        9
    };
    // Existing persisted timestamps use 0/3/6/9 fraction digits and +00:00.
    DateTimePrinter::new()
        .precision(Some(precision))
        .timestamp_with_offset_to_string(&timestamp, Offset::UTC)
}

/// Parse the existing RFC 3339 input contract without accepting the broader
/// Temporal syntax (annotations, offset seconds, or dates without a time).
pub fn parse_rfc3339_seconds(value: &str) -> Option<i64> {
    let bytes = value.as_bytes();
    if bytes.len() < 20
        || bytes.get(4) != Some(&b'-')
        || bytes.get(7) != Some(&b'-')
        || !matches!(bytes.get(10), Some(b'T' | b't' | b' '))
        || bytes.get(13) != Some(&b':')
        || bytes.get(16) != Some(&b':')
    {
        return None;
    }
    let year = i16::from(two_digits(bytes, 0)?) * 100 + i16::from(two_digits(bytes, 2)?);
    let second = two_digits(bytes, 17)?;
    if second > 60 {
        return None;
    }
    let datetime = DateTime::new(
        year,
        two_digits(bytes, 5)? as i8,
        two_digits(bytes, 8)? as i8,
        two_digits(bytes, 11)? as i8,
        two_digits(bytes, 14)? as i8,
        // Unix seconds omit leap seconds; the existing parser retains :59.
        second.min(59) as i8,
        0,
    )
    .ok()?;
    let offset_start = if bytes.get(19) == Some(&b'.') {
        let digits = bytes[20..]
            .iter()
            .take_while(|digit| digit.is_ascii_digit())
            .count();
        if digits == 0 {
            return None;
        }
        // Fractions of any length are accepted. Dropping the fraction gives
        // the stored floor second even for an instant before the Unix epoch.
        20 + digits
    } else {
        19
    };
    let offset = &value[offset_start..];
    let offset_seconds = if matches!(offset, "Z" | "z") {
        0
    } else {
        let (sign, offset) = if let Some(offset) = offset.strip_prefix('+') {
            (1, offset)
        } else if let Some(offset) = offset
            .strip_prefix('-')
            .or_else(|| offset.strip_prefix('\u{2212}'))
        {
            (-1, offset)
        } else {
            return None;
        };
        let offset = offset.as_bytes();
        if offset.len() != 5 || offset[2] != b':' {
            return None;
        }
        let hours = two_digits(offset, 0)?;
        let minutes = two_digits(offset, 3)?;
        if hours > 23 || minutes > 59 {
            return None;
        }
        sign * (i64::from(hours) * 3_600 + i64::from(minutes) * 60)
    };
    // Civil arithmetic also preserves valid edge-year RFC 3339 inputs whose
    // offset would put the instant outside Jiff Timestamp's narrower range.
    Some(civil_utc_seconds(datetime) - offset_seconds)
}

fn two_digits(bytes: &[u8], index: usize) -> Option<u8> {
    let first = *bytes.get(index)?;
    let second = *bytes.get(index + 1)?;
    (first.is_ascii_digit() && second.is_ascii_digit()).then(|| (first - b'0') * 10 + second - b'0')
}

pub fn parse_civil_datetime(value: &str) -> Result<DateTime, jiff::Error> {
    DateTime::strptime("%Y-%m-%d %H:%M:%S", legacy_datetime_input(value)?.as_ref())
}

#[cfg(feature = "native-e2e")]
pub fn parse_civil_minute(value: &str) -> Result<DateTime, jiff::Error> {
    DateTime::strptime("%Y-%m-%d %H:%M", legacy_datetime_input(value)?.as_ref())
}

pub fn parse_date(value: &str) -> Result<Date, jiff::Error> {
    Date::strptime("%Y-%m-%d", legacy_datetime_input(value)?.as_ref())
}

// The legacy strptime inputs accepted Unicode whitespace and redundant zeros
// in explicitly signed years. Normalize those spellings for Jiff without
// changing the strict RFC 3339 parser or allocating for ordinary saved dates.
fn legacy_datetime_input(value: &str) -> Result<Cow<'_, str>, jiff::Error> {
    let value = value.trim_start();
    let mut normalized = Cow::Borrowed(value);
    if matches!(value.as_bytes().first(), Some(b'+' | b'-')) {
        let digits = value.as_bytes()[1..]
            .iter()
            .take_while(|digit| digit.is_ascii_digit())
            .count();
        if digits == 0 {
            // Jiff otherwise permits whitespace between the sign and year;
            // the existing input contract requires a digit after the sign.
            return Err(jiff::Error::from_args(format_args!(
                "A signed year must start with a digit immediately after its sign"
            )));
        }
        if digits > 4 {
            let year = value[1..1 + digits].trim_start_matches('0');
            if year.len() <= 4 {
                let year = if year.is_empty() { "0" } else { year };
                normalized =
                    Cow::Owned(format!("{}{year:0>4}{}", &value[..1], &value[1 + digits..]));
            }
        }
    }
    if normalized
        .chars()
        .any(|character| !character.is_ascii() && character.is_whitespace())
    {
        normalized = Cow::Owned(
            normalized
                .chars()
                .map(|character| {
                    if character.is_whitespace() {
                        ' '
                    } else {
                        character
                    }
                })
                .collect(),
        );
    }
    Ok(normalized)
}

pub fn civil_utc_seconds(value: DateTime) -> i64 {
    value
        .duration_since(UNIX_EPOCH)
        .as_nanos()
        .div_euclid(1_000_000_000) as i64
}

pub fn local_datetime() -> DateTime {
    let timestamp = Timestamp::now();
    #[cfg(windows)]
    {
        windows_local_datetime(timestamp).expect("operating system local time is valid")
    }
    #[cfg(not(windows))]
    {
        timestamp.to_zoned(system_timezone()).datetime()
    }
}

#[cfg(feature = "native-e2e")]
pub fn local_datetime_at(seconds: i64) -> Result<DateTime, jiff::Error> {
    let timestamp = Timestamp::from_second(seconds)?;
    #[cfg(windows)]
    {
        windows_local_datetime(timestamp)
    }
    #[cfg(not(windows))]
    {
        Ok(timestamp.to_zoned(system_timezone()).datetime())
    }
}

#[cfg(windows)]
fn windows_local_datetime(timestamp: Timestamp) -> Result<DateTime, jiff::Error> {
    use windows_sys::Win32::{
        Foundation::SYSTEMTIME, System::Time::SystemTimeToTzSpecificLocalTime,
    };
    let utc = Offset::UTC.to_datetime(timestamp);
    if utc.year() < 1601 {
        return Err(jiff::Error::from_args(format_args!(
            "Windows local time conversion requires a year of at least 1601"
        )));
    }
    // Match the existing Windows behavior: get the OS rules for the UTC year
    // on each conversion, including custom/disabled DST settings. Looking up
    // an IANA name would lose those settings and delay OS-zone changes.
    let zone = windows_timezone_for_year(utc.year() as u16)?;
    let universal = SYSTEMTIME {
        wYear: utc.year() as u16,
        wMonth: utc.month() as u16,
        wDayOfWeek: 0,
        wDay: utc.day() as u16,
        wHour: utc.hour() as u16,
        wMinute: utc.minute() as u16,
        wSecond: utc.second() as u16,
        wMilliseconds: (utc.subsec_nanosecond() / 1_000_000) as u16,
    };
    let mut local = universal;
    // SAFETY: all pointers refer to initialized structures of the API's
    // required types. The output is read only after a successful conversion.
    if unsafe { SystemTimeToTzSpecificLocalTime(&zone, &universal, &mut local) } == 0 {
        return Err(jiff::Error::from_args(format_args!(
            "Unable to convert operating system local time: {}",
            std::io::Error::last_os_error()
        )));
    }
    DateTime::new(
        local.wYear as i16,
        local.wMonth as i8,
        local.wDay as i8,
        local.wHour as i8,
        local.wMinute as i8,
        local.wSecond as i8,
        // Timezone offsets are whole seconds; retain the original fraction
        // instead of reducing it to the Windows API's millisecond precision.
        utc.subsec_nanosecond(),
    )
}

#[cfg(windows)]
fn windows_timezone_for_year(
    year: u16,
) -> Result<windows_sys::Win32::System::Time::TIME_ZONE_INFORMATION, jiff::Error> {
    use windows_sys::Win32::System::Time::{GetTimeZoneInformationForYear, TIME_ZONE_INFORMATION};
    #[cfg(feature = "native-e2e")]
    if let Some(zone) = *WINDOWS_TIMEZONE_FIXTURE
        .lock()
        .unwrap_or_else(|error| error.into_inner())
    {
        return Ok(zone);
    }
    let mut zone = TIME_ZONE_INFORMATION::default();
    // SAFETY: the output structure is valid and initialized; a null dynamic
    // timezone argument selects the current operating system's configuration.
    if unsafe { GetTimeZoneInformationForYear(year, std::ptr::null(), &mut zone) } == 0 {
        return Err(jiff::Error::from_args(format_args!(
            "Unable to read operating system time zone: {}",
            std::io::Error::last_os_error()
        )));
    }
    Ok(zone)
}

#[cfg(all(windows, feature = "native-e2e"))]
static WINDOWS_TIMEZONE_FIXTURE: std::sync::Mutex<
    Option<windows_sys::Win32::System::Time::TIME_ZONE_INFORMATION>,
> = std::sync::Mutex::new(None);

#[cfg(all(windows, feature = "native-e2e"))]
pub fn set_windows_timezone_for_e2e(
    zone: Option<windows_sys::Win32::System::Time::TIME_ZONE_INFORMATION>,
) {
    *WINDOWS_TIMEZONE_FIXTURE
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = zone;
}

#[cfg(not(windows))]
struct SystemZoneCache {
    checked: SystemTime,
    zone: TimeZone,
}

#[cfg(not(windows))]
fn system_timezone() -> TimeZone {
    static CACHE: Mutex<Option<SystemZoneCache>> = Mutex::new(None);
    let now = SystemTime::now();
    let mut cache = CACHE.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(cached) = cache.as_ref() {
        if now
            .duration_since(cached.checked)
            .is_ok_and(|elapsed| elapsed < Duration::from_secs(1))
        {
            return cached.zone.clone();
        }
    }
    // Preserve the application's one-second OS-zone reselection. Jiff's
    // built-in system-zone cache lasts five minutes, including after a user
    // changes the operating system's time zone while the app is running.
    let zone = discover_system_timezone();
    *cache = Some(SystemZoneCache {
        checked: now,
        zone: zone.clone(),
    });
    zone
}

#[cfg(not(windows))]
fn discover_system_timezone() -> TimeZone {
    #[cfg(unix)]
    {
        let zone = match std::env::var("TZ") {
            Ok(value) => timezone_from_env(&value),
            Err(_) => timezone_from_file(std::path::Path::new("/etc/localtime")),
        };
        if let Some(zone) = zone {
            return zone;
        }
    }
    // Unix historically falls back to the OS setting when TZ is malformed.
    // UTC remains the final fallback when no system zone is available.
    iana_time_zone::get_timezone()
        .ok()
        .and_then(|name| TimeZone::get(&name).ok())
        .unwrap_or(TimeZone::UTC)
}

#[cfg(unix)]
fn timezone_from_env(value: &str) -> Option<TimeZone> {
    if value.is_empty() {
        return Some(TimeZone::UTC);
    }
    if value == "localtime" {
        return timezone_from_file(std::path::Path::new("/etc/localtime"));
    }
    let file_only = value.starts_with(':');
    let name = value.strip_prefix(':').unwrap_or(value);
    let path = std::path::Path::new(name);
    if path.is_absolute() {
        return timezone_from_file(path);
    }
    for root in [
        "/usr/share/zoneinfo",
        "/share/zoneinfo",
        "/etc/zoneinfo",
        "/usr/share/lib/zoneinfo",
    ] {
        if let Some(zone) = timezone_from_file(&std::path::Path::new(root).join(path)) {
            return Some(zone);
        }
    }
    TimeZone::get(name).ok().or_else(|| {
        if file_only {
            None
        } else {
            TimeZone::posix(value.trim_matches(|c: char| c.is_ascii_whitespace())).ok()
        }
    })
}

#[cfg(unix)]
fn timezone_from_file(path: &std::path::Path) -> Option<TimeZone> {
    let data = std::fs::read(path).ok()?;
    TimeZone::tzif("localtime", &data).ok()
}
