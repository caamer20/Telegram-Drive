use super::{utils::resolve_peer, TelegramState};
use crate::db::DbConnection;
use crate::workspace::AccountGuard;
use rand::Rng;
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

#[derive(Debug, Serialize)]
pub struct ShareInfo {
    pub id: String,
    pub owner_id: String,
    pub file_name: String,
    pub file_size: i64,
    pub created_at: i64,
    pub expires_at: Option<i64>,
    pub has_password: bool,
    pub link: String,
}

fn generate_share_token() -> String {
    let mut rng = rand::rng();
    let bytes: Vec<u8> = (0..16).map(|_| rng.random()).collect();
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Hash a password using bcrypt (cost factor 12).
/// bcrypt embeds the salt in the output hash string, so no separate salt storage is needed.
fn hash_password(password: &str) -> Result<String, String> {
    bcrypt::non_truncating_hash(password, 12).map_err(|e| format!("Password hashing failed: {}", e))
}

const MIN_SHARE_PASSWORD_CHARS: usize = 4;
const MAX_SHARE_PASSWORD_CHARS: usize = 128;
const MAX_SHARE_PASSWORD_BYTES: usize = 72;

fn validate_share_password(password: Option<String>) -> Result<Option<String>, String> {
    let Some(password) = password else {
        return Ok(None);
    };
    if password.is_empty() {
        return Ok(None);
    }
    let chars = password.chars().count();
    if !(MIN_SHARE_PASSWORD_CHARS..=MAX_SHARE_PASSWORD_CHARS).contains(&chars)
        || password.len() > MAX_SHARE_PASSWORD_BYTES
    {
        return Err(format!(
            "Share passwords must contain between {MIN_SHARE_PASSWORD_CHARS} and {MAX_SHARE_PASSWORD_CHARS} characters"
        ));
    }
    Ok(Some(password))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri injects app/state; keep the existing IPC fields compatible.
pub async fn cmd_create_share(
    app: AppHandle,
    owner_id: Option<String>,
    folder_id: Option<i64>,
    message_id: i32,
    file_name: String,
    file_size: i64,
    password: Option<String>,
    expiry_hours: Option<i64>,
    db_pool: State<'_, DbConnection>,
    state: State<'_, TelegramState>,
) -> Result<ShareInfo, String> {
    let root = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    let account = AccountGuard::open(&root, owner_id.as_deref())?;
    let owner = account.owner;
    let client = state
        .client
        .lock()
        .await
        .clone()
        .ok_or("Telegram client is not connected")?;
    account.validate()?;
    if client
        .get_me()
        .await
        .map_err(|error| error.to_string())?
        .bare_id()
        != owner
    {
        return Err("ACCOUNT_CHANGED: Reopen sharing from the current account".into());
    }
    let peer = resolve_peer(&client, folder_id, &state.peer_cache).await?;
    account.validate()?;
    let messages = client
        .get_messages_by_id(peer, &[message_id])
        .await
        .map_err(|error| error.to_string())?;
    let message = messages
        .first()
        .and_then(Option::as_ref)
        .ok_or("Shared file was not found in the current account")?;
    let media = message.media().ok_or("Shared message has no file")?;
    if super::fs::resolve_remote_envelope(
        &account,
        &client,
        folder_id,
        message_id,
        &media,
        message.text(),
    )
    .await?
    .is_some()
    {
        return Err("[ENCRYPTED_SHARE_UNAVAILABLE] Encrypted sharing is disabled until a credential-safe sharing flow is available".into());
    }
    account.validate()?;
    let token = generate_share_token();
    let created_at = crate::app_time::unix_seconds();
    let expires_at = expiry_hours.map(|hours| created_at + hours * 3600);
    let password = validate_share_password(password)?;
    // Bcrypt is deliberately completed on a blocking worker before the SQLite
    // connection is acquired, so expensive password work cannot hold the DB lock.
    let password_hash = match password {
        Some(password) => Some(
            tokio::task::spawn_blocking(move || hash_password(&password))
                .await
                .map_err(|error| format!("Password hashing worker failed: {error}"))??,
        ),
        None => None,
    };
    let database = db_pool.inner().clone();
    let token_for_db = token.clone();
    let file_name_for_db = file_name.clone();
    let insert_account = account.clone();
    let has_password = crate::db::with_connection(database, move |conn| {
        insert_account.validate()?;
        let mut stmt = conn.prepare(
            "INSERT INTO shared_links (id, folder_id, message_id, file_name, file_size, password_hash, password_salt, expires_at, revoked, created_at, owner_id)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, 0, ?, ?)"
        ).map_err(|e| e.to_string())?;
        stmt.bind((1, token_for_db.as_str())).map_err(|e| e.to_string())?;
        stmt.bind((2, folder_id)).map_err(|e| e.to_string())?;
        stmt.bind((3, message_id as i64)).map_err(|e| e.to_string())?;
        stmt.bind((4, file_name_for_db.as_str())).map_err(|e| e.to_string())?;
        stmt.bind((5, file_size)).map_err(|e| e.to_string())?;
        stmt.bind((6, password_hash.as_deref())).map_err(|e| e.to_string())?;
        stmt.bind::<(usize, Option<&str>)>((7, None)).map_err(|e| e.to_string())?;
        stmt.bind((8, expires_at)).map_err(|e| e.to_string())?;
        stmt.bind((9, created_at)).map_err(|e| e.to_string())?;
        stmt.bind((10, owner)).map_err(|e| e.to_string())?;
        stmt.next().map_err(|e| e.to_string())?;
        Ok(password_hash.is_some())
    }).await?;

    account.validate()?;
    let link = format!("http://127.0.0.1:{}/d/{}", crate::stream_port(), token);

    Ok(ShareInfo {
        id: token,
        owner_id: owner.to_string(),
        file_name,
        file_size,
        created_at,
        expires_at,
        has_password,
        link,
    })
}

#[tauri::command]
pub async fn cmd_list_shares(
    app: AppHandle,
    owner_id: Option<String>,
    db_pool: State<'_, DbConnection>,
) -> Result<Vec<ShareInfo>, String> {
    let root = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    let account = AccountGuard::open(&root, owner_id.as_deref())?;
    let database = db_pool.inner().clone();
    let listed_account = account.clone();
    let shares = crate::db::with_connection(database, move |conn| {
        list_shares_for_account(conn, &listed_account)
    })
    .await?;
    account.validate()?;
    Ok(shares)
}

fn list_shares_for_account(
    conn: &sqlite::Connection,
    account: &AccountGuard,
) -> Result<Vec<ShareInfo>, String> {
    account.validate()?;
    let owner = account.owner;

    let mut stmt = conn
        .prepare(
            "SELECT id, folder_id, message_id, file_name, file_size, password_hash, expires_at, created_at 
             FROM shared_links WHERE revoked = 0 AND owner_id = ? ORDER BY created_at DESC"
        )
        .map_err(|e| e.to_string())?;

    stmt.bind((1, owner)).map_err(|e| e.to_string())?;
    let mut shares = Vec::new();
    while let sqlite::State::Row = stmt.next().map_err(|e| e.to_string())? {
        let id = stmt.read::<String, _>("id").map_err(|e| e.to_string())?;
        let has_password = stmt
            .read::<Option<String>, _>("password_hash")
            .ok()
            .flatten()
            .is_some();
        let expires_at = stmt.read::<Option<i64>, _>("expires_at").ok().flatten();
        let file_name = stmt
            .read::<String, _>("file_name")
            .map_err(|e| e.to_string())?;
        let file_size = stmt
            .read::<i64, _>("file_size")
            .map_err(|e| e.to_string())?;
        let created_at = stmt
            .read::<i64, _>("created_at")
            .map_err(|e| e.to_string())?;
        let link = format!("http://127.0.0.1:{}/d/{}", crate::stream_port(), id);

        shares.push(ShareInfo {
            id,
            owner_id: owner.to_string(),
            file_name,
            file_size,
            created_at,
            expires_at,
            has_password,
            link,
        });
    }

    Ok(shares)
}

#[tauri::command]
pub async fn cmd_revoke_share(
    app: AppHandle,
    owner_id: Option<String>,
    id: String,
    db_pool: State<'_, DbConnection>,
) -> Result<(), String> {
    let root = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    let account = AccountGuard::open(&root, owner_id.as_deref())?;
    let database = db_pool.inner().clone();
    crate::db::with_connection(database, move |conn| {
        revoke_share_for_account(conn, &account, &id)
    })
    .await
}

fn revoke_share_for_account(
    conn: &sqlite::Connection,
    account: &AccountGuard,
    id: &str,
) -> Result<(), String> {
    account.validate()?;
    let mut stmt = conn
        .prepare("UPDATE shared_links SET revoked = 1 WHERE id = ? AND owner_id = ?")
        .map_err(|e| e.to_string())?;
    stmt.bind((1, id)).map_err(|e| e.to_string())?;
    stmt.bind((2, account.owner)).map_err(|e| e.to_string())?;
    stmt.next().map_err(|e| e.to_string())?;
    Ok(())
}
