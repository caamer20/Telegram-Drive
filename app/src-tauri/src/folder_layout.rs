//! The folder list and its groups live in two shared tables, but they
//! describe one Telegram account. This module keeps a copy per account and
//! swaps the shared tables when the signed-in account changes, so one
//! account's groups, ordering and folder names are neither lost to another
//! account's folder scan nor shown in another account's session.
//!
//! A marker file names the account whose layout the shared tables hold. While
//! that account stays signed in the tables are authoritative and nothing is
//! copied. The per-account copy is written when the account is switched out.
use crate::workspace::store::Store;
use serde::{Deserialize, Serialize};
use sqlite::{Connection, State};
use std::path::Path;

const OWNER_MARKER: &str = "folder-layout.owner";
/// Marker value while the tables are being replaced or after sign-out: their
/// content belongs to nobody and must not be saved to an account.
const NO_OWNER: i64 = 0;
const RECORD_KIND: &str = "folder_layout";
const RECORD_ID: &str = "v1";

#[derive(Debug, Default, Serialize, Deserialize)]
struct SavedLayout {
    groups: Vec<SavedGroup>,
    folders: Vec<SavedFolder>,
}

#[derive(Debug, Serialize, Deserialize)]
struct SavedGroup {
    id: i64,
    name: String,
    color_hex: Option<String>,
    display_order: i64,
}

#[derive(Debug, Serialize, Deserialize)]
struct SavedFolder {
    channel_id: i64,
    name: String,
    username: Option<String>,
    is_public: bool,
    display_order: i64,
    group_id: Option<i64>,
    #[serde(default)]
    parent_id: Option<i64>,
}

fn text(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn read_marker(root: &Path) -> Option<i64> {
    std::fs::read_to_string(root.join(OWNER_MARKER))
        .ok()
        .and_then(|value| value.trim().parse().ok())
}

fn write_marker(root: &Path, owner: i64) -> Result<(), String> {
    let path = root.join(OWNER_MARKER);
    let staged = root.join(format!("{OWNER_MARKER}.tmp"));
    std::fs::write(&staged, owner.to_string()).map_err(text)?;
    std::fs::rename(&staged, &path).map_err(text)
}

fn read_tables(connection: &Connection) -> Result<SavedLayout, String> {
    let mut layout = SavedLayout::default();
    let mut groups = connection
        .prepare("SELECT id, name, color_hex, display_order FROM groups ORDER BY id")
        .map_err(text)?;
    while groups.next().map_err(text)? == State::Row {
        layout.groups.push(SavedGroup {
            id: groups.read(0).map_err(text)?,
            name: groups.read(1).map_err(text)?,
            color_hex: groups.read(2).map_err(text)?,
            display_order: groups.read(3).map_err(text)?,
        });
    }
    let mut folders = connection
        .prepare("SELECT channel_id, name, username, is_public, display_order, group_id, parent_id FROM folder_metadata ORDER BY channel_id")
        .map_err(text)?;
    while folders.next().map_err(text)? == State::Row {
        layout.folders.push(SavedFolder {
            channel_id: folders.read(0).map_err(text)?,
            name: folders.read(1).map_err(text)?,
            username: folders.read(2).map_err(text)?,
            is_public: folders.read::<i64, _>(3).map_err(text)? != 0,
            display_order: folders.read(4).map_err(text)?,
            group_id: folders.read(5).map_err(text)?,
            parent_id: folders.read(6).map_err(text)?,
        });
    }
    Ok(layout)
}

/// Replace both tables in one transaction so they never hold a mixture.
fn write_tables(connection: &Connection, layout: &SavedLayout) -> Result<(), String> {
    connection.execute("BEGIN IMMEDIATE").map_err(text)?;
    let result = (|| {
        connection
            .execute("DELETE FROM folder_metadata; DELETE FROM groups;")
            .map_err(text)?;
        for group in &layout.groups {
            let mut insert = connection
                .prepare(
                    "INSERT INTO groups (id, name, color_hex, display_order) VALUES (?, ?, ?, ?)",
                )
                .map_err(text)?;
            insert.bind((1, group.id)).map_err(text)?;
            insert.bind((2, group.name.as_str())).map_err(text)?;
            insert.bind((3, group.color_hex.as_deref())).map_err(text)?;
            insert.bind((4, group.display_order)).map_err(text)?;
            insert.next().map_err(text)?;
        }
        for folder in &layout.folders {
            let mut insert = connection
                .prepare("INSERT INTO folder_metadata (channel_id, name, username, is_public, display_order, group_id, parent_id) VALUES (?, ?, ?, ?, ?, ?, ?)")
                .map_err(text)?;
            insert.bind((1, folder.channel_id)).map_err(text)?;
            insert.bind((2, folder.name.as_str())).map_err(text)?;
            insert.bind((3, folder.username.as_deref())).map_err(text)?;
            insert
                .bind((4, i64::from(folder.is_public)))
                .map_err(text)?;
            insert.bind((5, folder.display_order)).map_err(text)?;
            insert.bind((6, folder.group_id)).map_err(text)?;
            insert.bind((7, folder.parent_id)).map_err(text)?;
            insert.next().map_err(text)?;
        }
        Ok::<(), String>(())
    })();
    match result {
        Ok(()) => connection.execute("COMMIT").map_err(text),
        Err(error) => {
            let _ = connection.execute("ROLLBACK");
            Err(error)
        }
    }
}

fn save_for(connection: &Connection, root: &Path, owner: i64) -> Result<(), String> {
    Store::open(root, owner)?.put_record(RECORD_KIND, RECORD_ID, &read_tables(connection)?)
}

/// Make the shared tables hold `owner`'s folders and groups.
///
/// Call before reading or changing them for a signed-in account. It does
/// nothing when that account's layout is already in place.
pub fn activate(connection: &Connection, root: &Path, owner: i64) -> Result<(), String> {
    match read_marker(root) {
        Some(current) if current == owner => Ok(()),
        // First run with per-account layouts: the tables hold the single
        // layout earlier releases kept, and it belongs to whoever is signed in.
        None => write_marker(root, owner),
        Some(previous) => {
            if previous != NO_OWNER {
                save_for(connection, root, previous)?;
            }
            // From here until the swap commits the tables belong to nobody,
            // so an interruption can never credit them to the wrong account.
            write_marker(root, NO_OWNER)?;
            let layout = Store::open(root, owner)?
                .record::<SavedLayout>(RECORD_KIND, RECORD_ID)?
                .unwrap_or_default();
            write_tables(connection, &layout)?;
            write_marker(root, owner)
        }
    }
}

/// Put the signed-out state in place: the active account's layout is saved to
/// its own store and the shared tables are emptied, so nothing of it can be
/// read by whoever signs in next or by a background service in between.
///
/// `signing_out` is the account being signed out when it is still known. It
/// claims the single layout an earlier release left without a marker.
pub fn deactivate(
    connection: &Connection,
    root: &Path,
    signing_out: Option<i64>,
) -> Result<(), String> {
    let owner = match read_marker(root) {
        Some(NO_OWNER) => None,
        Some(owner) => Some(owner),
        None => signing_out,
    };
    if let Some(owner) = owner {
        save_for(connection, root, owner)?;
    } else if read_marker(root).is_none() {
        // Unscoped legacy data and nobody to credit it to: leave it alone.
        return Ok(());
    }
    write_marker(root, NO_OWNER)?;
    write_tables(connection, &SavedLayout::default())
}

/// Whether the shared tables may be shown to the current session: either they
/// hold `owner`'s layout, or per-account scoping has never been set up.
pub fn readable_by(root: &Path, owner: Option<i64>) -> bool {
    match (read_marker(root), owner) {
        (None, _) => true,
        (Some(current), Some(owner)) => current == owner,
        (Some(_), None) => false,
    }
}
