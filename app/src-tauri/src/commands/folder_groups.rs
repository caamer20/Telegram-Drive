use crate::db::DbConnection;
use crate::models::{FolderGroup, FolderMetadata};
use tauri::State;

#[tauri::command]
pub async fn cmd_get_enriched_folders(
    db_pool: State<'_, DbConnection>,
) -> Result<Vec<FolderMetadata>, String> {
    let conn = db_pool.lock().map_err(|e| e.to_string())?;

    let query = "
        SELECT fm.channel_id, fm.name, fm.username, fm.is_public, fm.display_order, fm.group_id 
        FROM folder_metadata fm
        LEFT JOIN groups g ON fm.group_id = g.id
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

        folders.push(FolderMetadata {
            id: channel_id,
            parent_id: None,
            name,
            username,
            is_public,
            group_id,
            display_order,
        });
    }

    Ok(folders)
}

#[tauri::command]
pub async fn cmd_update_folder_order(
    channel_id: i64,
    new_order: i32,
    db_pool: State<'_, DbConnection>,
) -> Result<(), String> {
    let conn = db_pool.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("UPDATE folder_metadata SET display_order = ? WHERE channel_id = ?")
        .map_err(|e| e.to_string())?;
    stmt.bind((1, new_order as i64))
        .map_err(|e| e.to_string())?;
    stmt.bind((2, channel_id)).map_err(|e| e.to_string())?;
    stmt.next().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn cmd_create_group(
    name: String,
    color_hex: String,
    db_pool: State<'_, DbConnection>,
) -> Result<i32, String> {
    let conn = db_pool.lock().map_err(|e| e.to_string())?;

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
    stmt.bind((1, name.as_str())).map_err(|e| e.to_string())?;
    stmt.bind((2, color_hex.as_str()))
        .map_err(|e| e.to_string())?;
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

#[tauri::command]
pub async fn cmd_update_group(
    group_id: i32,
    name: String,
    color_hex: String,
    db_pool: State<'_, DbConnection>,
) -> Result<(), String> {
    let conn = db_pool.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("UPDATE groups SET name = ?, color_hex = ? WHERE id = ?")
        .map_err(|e| e.to_string())?;
    stmt.bind((1, name.as_str())).map_err(|e| e.to_string())?;
    stmt.bind((2, color_hex.as_str()))
        .map_err(|e| e.to_string())?;
    stmt.bind((3, group_id as i64)).map_err(|e| e.to_string())?;
    stmt.next().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn cmd_delete_group(
    group_id: i32,
    db_pool: State<'_, DbConnection>,
) -> Result<(), String> {
    let conn = db_pool.lock().map_err(|e| e.to_string())?;

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
}

#[tauri::command]
pub async fn cmd_assign_folder_to_group(
    channel_id: i64,
    group_id: Option<i32>,
    db_pool: State<'_, DbConnection>,
) -> Result<(), String> {
    let conn = db_pool.lock().map_err(|e| e.to_string())?;

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

#[tauri::command]
pub async fn cmd_update_group_order(
    group_id: i32,
    new_order: i32,
    db_pool: State<'_, DbConnection>,
) -> Result<(), String> {
    let conn = db_pool.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("UPDATE groups SET display_order = ? WHERE id = ?")
        .map_err(|e| e.to_string())?;
    stmt.bind((1, new_order as i64))
        .map_err(|e| e.to_string())?;
    stmt.bind((2, group_id as i64)).map_err(|e| e.to_string())?;
    stmt.next().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn cmd_get_groups(db_pool: State<'_, DbConnection>) -> Result<Vec<FolderGroup>, String> {
    let conn = db_pool.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT id, name, color_hex, display_order FROM groups ORDER BY display_order ASC")
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

pub fn get_enriched_folders_internal(
    conn: &sqlite::Connection,
    raw_folders: Vec<FolderMetadata>,
) -> Result<Vec<FolderMetadata>, String> {
    // 1. Fetch local folder metadata (group_id, display_order)
    let mut stmt = conn
        .prepare("SELECT channel_id, display_order, group_id FROM folder_metadata")
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
        local_map.insert(channel_id, (display_order, group_id));
    }

    // 2. Perform merge & upsert
    let mut enriched = Vec::new();
    let mut max_order = local_map.values().map(|(o, _)| *o).max().unwrap_or(0);

    for mut folder in raw_folders {
        if let Some(&(order, group_id)) = local_map.get(&folder.id) {
            folder.display_order = order;
            folder.group_id = group_id;

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
