use crate::db::DbConnection;
use crate::models::{FolderGroup, FolderMetadata};
use std::path::PathBuf;
use tauri::{Emitter, Manager, State};

/// Application data root and the signed-in account, when there is one.
pub(crate) fn layout_scope(app: &tauri::AppHandle) -> Option<(PathBuf, Option<i64>)> {
    let root = app.path().app_data_dir().ok()?;
    let owner = crate::workspace::current_owner(&root).ok();
    Some((root, owner))
}

/// Bring the signed-in account's folders and groups into the shared tables.
/// Returns whether the tables may be read for this session: after another
/// account's sign-out, or while nobody is signed in, they are not.
pub(crate) fn prepare_layout(
    conn: &sqlite::Connection,
    scope: Option<&(PathBuf, Option<i64>)>,
) -> Result<bool, String> {
    let Some((root, owner)) = scope else {
        return Ok(true);
    };
    if let Some(owner) = owner {
        crate::folder_layout::activate(conn, root, *owner)?;
    }
    Ok(crate::folder_layout::readable_by(root, *owner))
}

#[tauri::command]
pub async fn cmd_get_enriched_folders(
    app: tauri::AppHandle,
    db_pool: State<'_, DbConnection>,
) -> Result<Vec<FolderMetadata>, String> {
    let database = db_pool.inner().clone();
    let scope = layout_scope(&app);
    crate::db::with_connection(database, move |conn| {
        if !prepare_layout(conn, scope.as_ref())? {
            return Ok(Vec::new());
        }
        enriched_folders(conn)
    })
    .await
}

pub(crate) fn enriched_folders(conn: &sqlite::Connection) -> Result<Vec<FolderMetadata>, String> {
    {
        let query = "
        SELECT fm.channel_id, fm.name, fm.username, fm.is_public, fm.display_order, fm.group_id, fm.parent_id 
        FROM folder_metadata fm
        LEFT JOIN groups g ON fm.group_id = g.id
        WHERE fm.channel_id NOT IN (SELECT folder_id FROM folder_trash)
        ORDER BY 
            CASE WHEN fm.group_id IS NULL THEN 1 ELSE 0 END, 
            g.display_order ASC, 
            fm.display_order ASC
    ";

        let mut stmt = conn.prepare(query).map_err(|e| e.to_string())?;
        let mut folders = Vec::new();

        while let sqlite::State::Row = stmt.next().map_err(|e| e.to_string())? {
            let channel_id = stmt
                .read::<i64, _>("channel_id")
                .map_err(|e| e.to_string())?;
            let name = stmt.read::<String, _>("name").map_err(|e| e.to_string())?;
            let username = stmt.read::<Option<String>, _>("username").ok().flatten();
            let is_public = stmt
                .read::<i64, _>("is_public")
                .map_err(|e| e.to_string())?
                != 0;
            let display_order = stmt
                .read::<i64, _>("display_order")
                .map_err(|e| e.to_string())? as i32;
            let group_id = stmt
                .read::<Option<i64>, _>("group_id")
                .ok()
                .flatten()
                .map(|id| id as i32);
            let parent_id = stmt.read::<Option<i64>, _>("parent_id").ok().flatten();

            folders.push(FolderMetadata {
                id: channel_id,
                parent_id,
                name,
                username,
                is_public,
                group_id,
                display_order,
            });
        }

        Ok(folders)
    }
}

#[tauri::command]
pub async fn cmd_update_folder_order(
    channel_id: i64,
    new_order: i32,
    db_pool: State<'_, DbConnection>,
) -> Result<(), String> {
    let database = db_pool.inner().clone();
    crate::db::with_connection(database, move |conn| {
        let mut stmt = conn
            .prepare("UPDATE folder_metadata SET display_order = ? WHERE channel_id = ?")
            .map_err(|e| e.to_string())?;
        stmt.bind((1, new_order as i64))
            .map_err(|e| e.to_string())?;
        stmt.bind((2, channel_id)).map_err(|e| e.to_string())?;
        stmt.next().map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn cmd_create_group(
    name: String,
    color_hex: String,
    db_pool: State<'_, DbConnection>,
) -> Result<i32, String> {
    let database = db_pool.inner().clone();
    crate::db::with_connection(database, move |conn| create_group(conn, &name, &color_hex)).await
}

pub(crate) fn create_group(
    conn: &sqlite::Connection,
    name: &str,
    color_hex: &str,
) -> Result<i32, String> {
    {
        // Determine max display order to append new group
        let mut max_stmt = conn
            .prepare("SELECT MAX(display_order) FROM groups")
            .map_err(|e| e.to_string())?;
        let mut display_order = 0;
        if let sqlite::State::Row = max_stmt.next().map_err(|e| e.to_string())? {
            display_order = max_stmt
                .read::<Option<i64>, _>(0)
                .ok()
                .flatten()
                .unwrap_or(0)
                + 1;
        }

        let mut stmt = conn
            .prepare("INSERT INTO groups (name, color_hex, display_order) VALUES (?, ?, ?)")
            .map_err(|e| e.to_string())?;
        stmt.bind((1, name)).map_err(|e| e.to_string())?;
        stmt.bind((2, color_hex)).map_err(|e| e.to_string())?;
        stmt.bind((3, display_order)).map_err(|e| e.to_string())?;
        stmt.next().map_err(|e| e.to_string())?;

        // Get last insert rowid
        let mut rowid_stmt = conn
            .prepare("SELECT last_insert_rowid()")
            .map_err(|e| e.to_string())?;
        let mut last_id = 0;
        if let sqlite::State::Row = rowid_stmt.next().map_err(|e| e.to_string())? {
            last_id = rowid_stmt.read::<i64, _>(0).map_err(|e| e.to_string())? as i32;
        }

        Ok(last_id)
    }
}

#[tauri::command]
pub async fn cmd_update_group(
    group_id: i32,
    name: String,
    color_hex: String,
    db_pool: State<'_, DbConnection>,
) -> Result<(), String> {
    let database = db_pool.inner().clone();
    crate::db::with_connection(database, move |conn| {
        let mut stmt = conn
            .prepare("UPDATE groups SET name = ?, color_hex = ? WHERE id = ?")
            .map_err(|e| e.to_string())?;
        stmt.bind((1, name.as_str())).map_err(|e| e.to_string())?;
        stmt.bind((2, color_hex.as_str()))
            .map_err(|e| e.to_string())?;
        stmt.bind((3, group_id as i64)).map_err(|e| e.to_string())?;
        stmt.next().map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn cmd_delete_group(
    group_id: i32,
    db_pool: State<'_, DbConnection>,
) -> Result<(), String> {
    let database = db_pool.inner().clone();
    crate::db::with_connection(database, move |conn| {
        // Delete group
        let mut stmt = conn
            .prepare("DELETE FROM groups WHERE id = ?")
            .map_err(|e| e.to_string())?;
        stmt.bind((1, group_id as i64)).map_err(|e| e.to_string())?;
        stmt.next().map_err(|e| e.to_string())?;

        // Foreign key with SET NULL will automatically set group_id to NULL in folder_metadata,
        // but we can also set it explicitly just in case.
        let mut update_stmt = conn
            .prepare("UPDATE folder_metadata SET group_id = NULL WHERE group_id = ?")
            .map_err(|e| e.to_string())?;
        update_stmt
            .bind((1, group_id as i64))
            .map_err(|e| e.to_string())?;
        update_stmt.next().map_err(|e| e.to_string())?;

        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn cmd_assign_folder_to_group(
    channel_id: i64,
    group_id: Option<i32>,
    db_pool: State<'_, DbConnection>,
) -> Result<(), String> {
    let database = db_pool.inner().clone();
    crate::db::with_connection(database, move |conn| {
        assign_folder_to_group(conn, channel_id, group_id)
    })
    .await
}

pub(crate) fn assign_folder_to_group(
    conn: &sqlite::Connection,
    channel_id: i64,
    group_id: Option<i32>,
) -> Result<(), String> {    {
        let mut stmt = conn
            .prepare("UPDATE folder_metadata SET group_id = ? WHERE channel_id = ?")
            .map_err(|e| e.to_string())?;
        match group_id {
            Some(gid) => stmt.bind((1, gid as i64)).map_err(|e| e.to_string())?,
            None => stmt
                .bind::<(usize, Option<i64>)>((1, None))
                .map_err(|e| e.to_string())?,
        };
        stmt.bind((2, channel_id)).map_err(|e| e.to_string())?;
        stmt.next().map_err(|e| e.to_string())?;
        Ok(())
    }
}


/// A folder that was deleted into the recycle bin. The Telegram channel is left
/// untouched, so restoring it costs nothing and never hits Telegram's limits.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TrashedFolder {
    pub folder_id: i64,
    pub name: String,
    pub parent_id: Option<i64>,
    pub deleted_at: i64,
}

fn collect_descendants(conn: &sqlite::Connection, folder_id: i64) -> Result<Vec<i64>, String> {
    let mut stmt = conn
        .prepare(
            "WITH RECURSIVE sub(id) AS (                SELECT channel_id FROM folder_metadata WHERE parent_id = ?1                 UNION ALL                 SELECT fm.channel_id FROM folder_metadata fm JOIN sub ON fm.parent_id = sub.id            ) SELECT id FROM sub",
        )
        .map_err(|e| e.to_string())?;
    stmt.bind((1, folder_id)).map_err(|e| e.to_string())?;
    let mut ids = Vec::new();
    while let sqlite::State::Row = stmt.next().map_err(|e| e.to_string())? {
        ids.push(stmt.read::<i64, _>(0).map_err(|e| e.to_string())?);
    }
    Ok(ids)
}

/// Move a folder (and its nested folders) into the recycle bin without touching
/// Telegram. Nothing is lost and no rate limit applies.
#[tauri::command]
pub async fn cmd_trash_folder(
    folder_id: i64,
    db_pool: State<'_, DbConnection>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let database = db_pool.inner().clone();
    crate::db::with_connection(database, move |conn| {
        let mut ids = collect_descendants(conn, folder_id)?;
        ids.push(folder_id);
        let deleted_at = crate::app_time::unix_seconds();
        conn.execute("BEGIN IMMEDIATE").map_err(|e| e.to_string())?;
        let result = (|| {
            for id in &ids {
                let mut info = conn
                    .prepare("SELECT name, parent_id FROM folder_metadata WHERE channel_id = ?")
                    .map_err(|e| e.to_string())?;
                info.bind((1, *id)).map_err(|e| e.to_string())?;
                let mut name = String::new();
                let mut parent_id: Option<i64> = None;
                if sqlite::State::Row == info.next().map_err(|e| e.to_string())? {
                    name = info.read::<String, _>(0).unwrap_or_default();
                    parent_id = info.read::<Option<i64>, _>(1).ok().flatten();
                }
                drop(info);
                let mut insert = conn
                    .prepare("INSERT INTO folder_trash(folder_id, name, parent_id, deleted_at) VALUES(?, ?, ?, ?) ON CONFLICT(folder_id) DO UPDATE SET deleted_at = excluded.deleted_at")
                    .map_err(|e| e.to_string())?;
                insert.bind((1, *id)).map_err(|e| e.to_string())?;
                insert.bind((2, name.as_str())).map_err(|e| e.to_string())?;
                insert
                    .bind::<(usize, Option<i64>)>((3, parent_id))
                    .map_err(|e| e.to_string())?;
                insert.bind((4, deleted_at)).map_err(|e| e.to_string())?;
                insert.next().map_err(|e| e.to_string())?;
            }
            Ok::<(), String>(())
        })();
        match result {
            Ok(()) => conn.execute("COMMIT").map_err(|e| e.to_string()).map(|_| ()),
            Err(error) => {
                let _ = conn.execute("ROLLBACK");
                Err(error)
            }
        }
    })
    .await?;
    let _ = app.emit("drive-changed", ());
    Ok(())
}

#[tauri::command]
pub async fn cmd_list_trashed_folders(
    db_pool: State<'_, DbConnection>,
) -> Result<Vec<TrashedFolder>, String> {
    let database = db_pool.inner().clone();
    crate::db::with_connection(database, move |conn| {
        let mut stmt = conn
            .prepare("SELECT folder_id, name, parent_id, deleted_at FROM folder_trash ORDER BY deleted_at DESC")
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        while let sqlite::State::Row = stmt.next().map_err(|e| e.to_string())? {
            out.push(TrashedFolder {
                folder_id: stmt.read::<i64, _>(0).map_err(|e| e.to_string())?,
                name: stmt.read::<String, _>(1).map_err(|e| e.to_string())?,
                parent_id: stmt.read::<Option<i64>, _>(2).ok().flatten(),
                deleted_at: stmt.read::<i64, _>(3).unwrap_or(0),
            });
        }
        Ok(out)
    })
    .await
}

/// Put a folder (and the folders deleted with it) back in the list.
#[tauri::command]
pub async fn cmd_restore_trashed_folder(
    folder_id: i64,
    db_pool: State<'_, DbConnection>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let database = db_pool.inner().clone();
    crate::db::with_connection(database, move |conn| {
        let mut ids = collect_descendants(conn, folder_id)?;
        ids.push(folder_id);
        for id in ids {
            let mut stmt = conn
                .prepare("DELETE FROM folder_trash WHERE folder_id = ?")
                .map_err(|e| e.to_string())?;
            stmt.bind((1, id)).map_err(|e| e.to_string())?;
            stmt.next().map_err(|e| e.to_string())?;
        }
        Ok(())
    })
    .await?;
    let _ = app.emit("drive-changed", ());
    Ok(())
}

/// Drop the recycle-bin rows only; the channel stays on Telegram.
#[tauri::command]
pub async fn cmd_forget_trashed_folder(
    folder_id: i64,
    db_pool: State<'_, DbConnection>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let database = db_pool.inner().clone();
    crate::db::with_connection(database, move |conn| {
        let mut stmt = conn
            .prepare("DELETE FROM folder_trash WHERE folder_id = ?")
            .map_err(|e| e.to_string())?;
        stmt.bind((1, folder_id)).map_err(|e| e.to_string())?;
        stmt.next().map_err(|e| e.to_string())?;
        Ok(())
    })
    .await?;
    let _ = app.emit("drive-changed", ());
    Ok(())
}

#[tauri::command]
pub async fn cmd_move_folder(
    folder_id: i64,
    new_parent_id: Option<i64>,
    db_pool: State<'_, DbConnection>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    if new_parent_id == Some(folder_id) {
        return Err("A folder cannot be moved inside itself.".to_string());
    }
    let database = db_pool.inner().clone();
    crate::db::with_connection(database, move |conn| {
        // Walk up from the requested parent; if we meet the folder being moved
        // the move would create a cycle.
        let mut cursor = new_parent_id;
        let mut hops = 0;
        while let Some(parent) = cursor {
            if parent == folder_id {
                return Err("Cannot move a folder inside its own subfolder.".to_string());
            }
            if hops >= 64 {
                break;
            }
            let mut lookup = conn
                .prepare("SELECT parent_id FROM folder_metadata WHERE channel_id = ?")
                .map_err(|e| e.to_string())?;
            lookup.bind((1, parent)).map_err(|e| e.to_string())?;
            if !matches!(
                lookup.next().map_err(|e| e.to_string())?,
                sqlite::State::Row
            ) {
                break;
            }
            cursor = lookup.read::<Option<i64>, _>(0).ok().flatten();
            hops += 1;
        }

        let mut stmt = conn
            .prepare("UPDATE folder_metadata SET parent_id = ? WHERE channel_id = ?")
            .map_err(|e| e.to_string())?;
        stmt.bind::<(usize, Option<i64>)>((1, new_parent_id))
            .map_err(|e| e.to_string())?;
        stmt.bind((2, folder_id)).map_err(|e| e.to_string())?;
        stmt.next().map_err(|e| e.to_string())?;
        Ok(())
    })
    .await?;
    let _ = app.emit("drive-changed", ());
    Ok(())
}

#[tauri::command]
pub async fn cmd_update_group_order(
    group_id: i32,
    new_order: i32,
    db_pool: State<'_, DbConnection>,
) -> Result<(), String> {
    let database = db_pool.inner().clone();
    crate::db::with_connection(database, move |conn| {
        let mut stmt = conn
            .prepare("UPDATE groups SET display_order = ? WHERE id = ?")
            .map_err(|e| e.to_string())?;
        stmt.bind((1, new_order as i64))
            .map_err(|e| e.to_string())?;
        stmt.bind((2, group_id as i64)).map_err(|e| e.to_string())?;
        stmt.next().map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn cmd_get_groups(
    app: tauri::AppHandle,
    db_pool: State<'_, DbConnection>,
) -> Result<Vec<FolderGroup>, String> {
    let database = db_pool.inner().clone();
    let scope = layout_scope(&app);
    crate::db::with_connection(database, move |conn| {
        if !prepare_layout(conn, scope.as_ref())? {
            return Ok(Vec::new());
        }
        list_groups(conn)
    })
    .await
}

pub(crate) fn list_groups(conn: &sqlite::Connection) -> Result<Vec<FolderGroup>, String> {
    {
        let mut stmt = conn
            .prepare(
                "SELECT id, name, color_hex, display_order FROM groups ORDER BY display_order ASC",
            )
            .map_err(|e| e.to_string())?;

        let mut groups = Vec::new();
        while let sqlite::State::Row = stmt.next().map_err(|e| e.to_string())? {
            let id = stmt.read::<i64, _>("id").map_err(|e| e.to_string())? as i32;
            let name = stmt.read::<String, _>("name").map_err(|e| e.to_string())?;
            let color_hex = stmt
                .read::<String, _>("color_hex")
                .map_err(|e| e.to_string())?;
            let display_order = stmt
                .read::<i64, _>("display_order")
                .map_err(|e| e.to_string())? as i32;

            groups.push(FolderGroup {
                id,
                name,
                color_hex,
                display_order,
            });
        }

        Ok(groups)
    }
}

pub fn get_enriched_folders_internal(
    conn: &sqlite::Connection,
    raw_folders: Vec<FolderMetadata>,
) -> Result<Vec<FolderMetadata>, String> {
    // 1. Fetch local folder metadata (group_id, display_order)
    let mut stmt = conn
        .prepare("SELECT channel_id, display_order, group_id, parent_id FROM folder_metadata")
        .map_err(|e| e.to_string())?;

    let mut local_map = std::collections::HashMap::new();
    while let sqlite::State::Row = stmt.next().map_err(|e| e.to_string())? {
        let channel_id = stmt
            .read::<i64, _>("channel_id")
            .map_err(|e| e.to_string())?;
        let display_order = stmt
            .read::<i64, _>("display_order")
            .map_err(|e| e.to_string())? as i32;
        let group_id = stmt
            .read::<Option<i64>, _>("group_id")
            .ok()
            .flatten()
            .map(|id| id as i32);
        // The scanned list knows nothing about nesting, so the stored parent is
        // reused. Without it every nested folder would jump back to Home.
        let parent_id = stmt.read::<Option<i64>, _>("parent_id").ok().flatten();
        local_map.insert(channel_id, (display_order, group_id, parent_id));
    }

    // 2. Perform merge & upsert
    let mut enriched = Vec::new();
    let mut max_order = local_map.values().map(|(o, _, _)| *o).max().unwrap_or(0);

    for mut folder in raw_folders {
        if let Some(&(order, group_id, parent_id)) = local_map.get(&folder.id) {
            folder.display_order = order;
            folder.group_id = group_id;
            folder.parent_id = parent_id;

            // Update other properties (name, username, is_public) in DB to keep cached info updated
            let mut update_stmt = conn
                .prepare("UPDATE folder_metadata SET name = ?, username = ?, is_public = ? WHERE channel_id = ?")
                .map_err(|e| e.to_string())?;
            update_stmt
                .bind((1, folder.name.as_str()))
                .map_err(|e| e.to_string())?;
            update_stmt
                .bind((2, folder.username.as_deref()))
                .map_err(|e| e.to_string())?;
            update_stmt
                .bind((3, if folder.is_public { 1 } else { 0 }))
                .map_err(|e| e.to_string())?;
            update_stmt
                .bind((4, folder.id))
                .map_err(|e| e.to_string())?;
            update_stmt.next().map_err(|e| e.to_string())?;
        } else {
            max_order += 1;
            folder.display_order = max_order;
            folder.group_id = None;

            let mut insert_stmt = conn
                .prepare("INSERT INTO folder_metadata (channel_id, name, username, is_public, display_order, group_id) VALUES (?, ?, ?, ?, ?, NULL)")
                .map_err(|e| e.to_string())?;
            insert_stmt
                .bind((1, folder.id))
                .map_err(|e| e.to_string())?;
            insert_stmt
                .bind((2, folder.name.as_str()))
                .map_err(|e| e.to_string())?;
            insert_stmt
                .bind((3, folder.username.as_deref()))
                .map_err(|e| e.to_string())?;
            insert_stmt
                .bind((4, if folder.is_public { 1 } else { 0 }))
                .map_err(|e| e.to_string())?;
            insert_stmt
                .bind((5, max_order as i64))
                .map_err(|e| e.to_string())?;
            insert_stmt.next().map_err(|e| e.to_string())?;
        }
        enriched.push(folder);
    }

    // 3. Prune folders that are no longer on Telegram
    let enriched_ids: std::collections::HashSet<i64> = enriched.iter().map(|f| f.id).collect();
    for id in local_map.keys() {
        if !enriched_ids.contains(id) {
            let mut delete_stmt = conn
                .prepare("DELETE FROM folder_metadata WHERE channel_id = ?")
                .map_err(|e| e.to_string())?;
            delete_stmt.bind((1, *id)).map_err(|e| e.to_string())?;
            delete_stmt.next().map_err(|e| e.to_string())?;
        }
    }

    // 4. Sort enriched folders by group display_order then folder display_order
    let mut group_order_map = std::collections::HashMap::new();
    let mut group_stmt = conn
        .prepare("SELECT id, display_order FROM groups")
        .map_err(|e| e.to_string())?;
    while let sqlite::State::Row = group_stmt.next().map_err(|e| e.to_string())? {
        let id = group_stmt.read::<i64, _>("id").map_err(|e| e.to_string())? as i32;
        let order = group_stmt
            .read::<i64, _>("display_order")
            .map_err(|e| e.to_string())? as i32;
        group_order_map.insert(id, order);
    }

    enriched.sort_by(|a, b| {
        let a_group_order = a
            .group_id
            .and_then(|id| group_order_map.get(&id).cloned())
            .unwrap_or(i32::MAX);
        let b_group_order = b
            .group_id
            .and_then(|id| group_order_map.get(&id).cloned())
            .unwrap_or(i32::MAX);

        match a_group_order.cmp(&b_group_order) {
            std::cmp::Ordering::Equal => match a.group_id.cmp(&b.group_id) {
                std::cmp::Ordering::Equal => a.display_order.cmp(&b.display_order),
                other => other,
            },
            other => other,
        }
    });

    Ok(enriched)
}
