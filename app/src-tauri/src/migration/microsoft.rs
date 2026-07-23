use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::migration::models::*;

pub const DEFAULT_MS_CLIENT_ID: &str = "78ce9682-700f-420c-b1af-194d911ab7d2";
pub const CALLBACK_PORT: u16 = 18420;
pub const DEFAULT_REDIRECT_URI: &str = "http://127.0.0.1:18420/callback";
pub const DRIVE_DELTA_URL: &str = "https://graph.microsoft.com/v1.0/me/drive/root/delta?$select=id,name,size,folder,file,eTag,lastModifiedDateTime,parentReference,root,deleted";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MicrosoftSession {
    #[serde(default = "default_client_id")]
    pub client_id: String,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
    pub tenant: String,
    pub redirect_uri: String,
    pub account_info: MsAccountInfo,
}

fn default_client_id() -> String {
    DEFAULT_MS_CLIENT_ID.to_string()
}

impl MicrosoftSession {
    pub fn is_expired(&self) -> bool {
        chrono::Utc::now().timestamp() >= self.expires_at - 60
    }
}

pub struct PkceChallenge {
    pub verifier: String,
    pub challenge: String,
    pub state: String,
}

pub fn generate_pkce() -> PkceChallenge {
    use rand::Rng;
    let mut rng = rand::rng();

    let verifier_bytes: Vec<u8> = (0..32).map(|_| rng.random()).collect();
    let verifier = URL_SAFE_NO_PAD.encode(&verifier_bytes);

    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let hash = hasher.finalize();
    let challenge = URL_SAFE_NO_PAD.encode(hash);

    let state_bytes: Vec<u8> = (0..16).map(|_| rng.random()).collect();
    let state = URL_SAFE_NO_PAD.encode(&state_bytes);

    PkceChallenge {
        verifier,
        challenge,
        state,
    }
}

pub fn build_auth_url(
    client_id: &str,
    tenant: &str,
    redirect_uri: &str,
    pkce: &PkceChallenge,
) -> String {
    let t = if tenant.trim().is_empty() {
        "common"
    } else {
        tenant.trim()
    };
    format!(
        "https://login.microsoftonline.com/{}/oauth2/v2.0/authorize?\
        client_id={}&\
        response_type=code&\
        redirect_uri={}&\
        response_mode=query&\
        scope=Files.Read%20offline_access%20user.read&\
        state={}&\
        code_challenge={}&\
        code_challenge_method=S256",
        t,
        urlencoding::encode(client_id),
        urlencoding::encode(redirect_uri),
        urlencoding::encode(&pkce.state),
        urlencoding::encode(&pkce.challenge)
    )
}

pub async fn start_oauth_flow(
    client_id: &str,
    tenant: &str,
    redirect_uri: &str,
    app_handle: &tauri::AppHandle,
) -> Result<MicrosoftSession, String> {
    use tauri_plugin_opener::OpenerExt;

    let r_uri = if redirect_uri.trim().is_empty() {
        DEFAULT_REDIRECT_URI
    } else {
        redirect_uri.trim()
    };

    let pkce = generate_pkce();
    let auth_url = build_auth_url(client_id, tenant, r_uri, &pkce);

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", CALLBACK_PORT))
        .await
        .map_err(|e| {
            format!(
                "Failed to bind OAuth callback listener on port {}: {}",
                CALLBACK_PORT, e
            )
        })?;

    app_handle
        .opener()
        .open_url(&auth_url, None::<&str>)
        .map_err(|e| format!("Failed to open browser: {}", e))?;

    let code = tokio::time::timeout(std::time::Duration::from_secs(120), async {
        let (mut socket, _) = listener.accept().await.map_err(|e| e.to_string())?;
        let mut buffer = [0u8; 2048];
        let n = socket.read(&mut buffer).await.map_err(|e| e.to_string())?;
        let req_str = String::from_utf8_lossy(&buffer[..n]);

        let first_line = req_str.lines().next().unwrap_or_default();
        let query_start = first_line.find('?').ok_or_else(|| "Invalid OAuth response".to_string())?;
        let query_end = first_line.find(" HTTP/").unwrap_or(first_line.len());
        let query_str = &first_line[query_start + 1..query_end];

        let mut code_val = None;
        let mut state_val = None;

        for pair in query_str.split('&') {
            let mut parts = pair.splitn(2, '=');
            if let (Some(k), Some(v)) = (parts.next(), parts.next()) {
                if k == "code" {
                    code_val = Some(urlencoding::decode(v).unwrap_or_default().to_string());
                } else if k == "state" {
                    state_val = Some(urlencoding::decode(v).unwrap_or_default().to_string());
                }
            }
        }

        let html = "<html><body style='font-family:sans-serif;text-align:center;padding-top:50px;'>\
                    <h2>Authentication Successful!</h2><p>You can close this tab and return to Telegram Drive.</p>\
                    </body></html>";
        let http_response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            html.len(),
            html
        );
        let _ = socket.write_all(http_response.as_bytes()).await;

        let state_received = state_val.ok_or_else(|| "State missing from OAuth callback".to_string())?;
        if state_received != pkce.state {
            return Err("OAuth state mismatch (CSRF warning)".to_string());
        }

        code_val.ok_or_else(|| "Authorization code missing from callback".to_string())
    })
    .await
    .map_err(|_| "OAuth authentication timed out (120 seconds)".to_string())??;

    exchange_code_for_tokens(client_id, tenant, r_uri, &code, &pkce.verifier).await
}

pub async fn exchange_code_for_tokens(
    client_id: &str,
    tenant: &str,
    redirect_uri: &str,
    code: &str,
    code_verifier: &str,
) -> Result<MicrosoftSession, String> {
    let http = Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;

    let t = if tenant.trim().is_empty() {
        "common"
    } else {
        tenant.trim()
    };
    let token_endpoint = format!("https://login.microsoftonline.com/{}/oauth2/v2.0/token", t);

    let params = [
        ("client_id", client_id),
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("code_verifier", code_verifier),
    ];

    let resp = http
        .post(&token_endpoint)
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("Token request failed: {}", e))?;

    if !resp.status().is_success() {
        let err_text = resp.text().await.unwrap_or_default();
        return Err(format!("Microsoft token exchange error: {}", err_text));
    }

    let text = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read response body: {}", e))?;
    let token_json: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("Failed to parse token response JSON: {}", e))?;

    let access_token = token_json["access_token"]
        .as_str()
        .ok_or_else(|| "Missing access_token in response".to_string())?
        .to_string();

    let refresh_token = token_json["refresh_token"]
        .as_str()
        .ok_or_else(|| "Missing refresh_token in response".to_string())?
        .to_string();

    let expires_in = token_json["expires_in"].as_i64().unwrap_or(3600);
    let expires_at = chrono::Utc::now().timestamp() + expires_in;

    let account_info = get_user_profile(&http, &access_token).await?;

    Ok(MicrosoftSession {
        client_id: client_id.to_string(),
        access_token,
        refresh_token,
        expires_at,
        tenant: t.to_string(),
        redirect_uri: redirect_uri.to_string(),
        account_info,
    })
}

pub async fn refresh_access_token(session: &mut MicrosoftSession) -> Result<(), String> {
    let http = Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;

    let t = if session.tenant.trim().is_empty() {
        "common"
    } else {
        session.tenant.as_str()
    };
    let token_endpoint = format!("https://login.microsoftonline.com/{}/oauth2/v2.0/token", t);

    let params = [
        ("client_id", session.client_id.as_str()),
        ("grant_type", "refresh_token"),
        ("refresh_token", session.refresh_token.as_str()),
    ];

    let resp = http
        .post(&token_endpoint)
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("Refresh token request failed: {}", e))?;

    if !resp.status().is_success() {
        let err_text = resp.text().await.unwrap_or_default();
        return Err(format!("Failed to refresh Microsoft token: {}", err_text));
    }

    let text = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read refresh body: {}", e))?;
    let token_json: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("Failed to parse refresh JSON: {}", e))?;

    session.access_token = token_json["access_token"]
        .as_str()
        .ok_or_else(|| "Missing access_token in refresh response".to_string())?
        .to_string();

    if let Some(new_rt) = token_json["refresh_token"].as_str() {
        session.refresh_token = new_rt.to_string();
    }

    let expires_in = token_json["expires_in"].as_i64().unwrap_or(3600);
    session.expires_at = chrono::Utc::now().timestamp() + expires_in;

    Ok(())
}

pub async fn get_user_profile(http: &Client, access_token: &str) -> Result<MsAccountInfo, String> {
    let json =
        send_graph_request(http, "https://graph.microsoft.com/v1.0/me", access_token).await?;

    let account_name = json["displayName"]
        .as_str()
        .or_else(|| json["userPrincipalName"].as_str())
        .unwrap_or("Microsoft User")
        .to_string();

    let account_email = json["mail"]
        .as_str()
        .or_else(|| json["userPrincipalName"].as_str())
        .unwrap_or("")
        .to_string();

    Ok(MsAccountInfo {
        account_name,
        account_email,
    })
}

/// Helper function to parse retryAfterSeconds from JSON body or headers
fn parse_retry_after_seconds(text: &str, retry_header: Option<u64>) -> u64 {
    if let Some(h) = retry_header {
        return h;
    }
    // Search JSON tree or raw string for "retryAfterSeconds": 57
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(text) {
        if let Some(n) = v["error"]["retryAfterSeconds"]
            .as_u64()
            .or_else(|| v["error"]["retryAfterSeconds"].as_i64().map(|i| i as u64))
        {
            return n;
        }
        if let Some(n) = v["error"]["innerError"]["retryAfterSeconds"]
            .as_u64()
            .or_else(|| {
                v["error"]["innerError"]["retryAfterSeconds"]
                    .as_i64()
                    .map(|i| i as u64)
            })
        {
            return n;
        }
        if let Some(n) = v["error"]["innerError"]["innerError"]["retryAfterSeconds"]
            .as_u64()
            .or_else(|| {
                v["error"]["innerError"]["innerError"]["retryAfterSeconds"]
                    .as_i64()
                    .map(|i| i as u64)
            })
        {
            return n;
        }
    }
    if let Some(idx) = text.find("\"retryAfterSeconds\"") {
        let rest = &text[idx + "\"retryAfterSeconds\"".len()..];
        let digits: String = rest
            .chars()
            .filter(|c| c.is_ascii_digit())
            .take(5)
            .collect();
        if let Ok(n) = digits.parse::<u64>() {
            return n;
        }
    }
    5
}

/// Helper function to execute a Graph API request with automatic 429 rate-limiting backoff & retry
pub async fn send_graph_request(
    http: &Client,
    url: &str,
    access_token: &str,
) -> Result<serde_json::Value, String> {
    let mut attempts = 0;
    const MAX_ATTEMPTS: u32 = 10;

    loop {
        attempts += 1;
        let resp = http
            .get(url)
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|e| format!("Graph API request failed: {}", e))?;

        let status = resp.status();
        let retry_header = resp
            .headers()
            .get("Retry-After")
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok());

        let text = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read Graph API body: {}", e))?;

        if status.is_success() {
            let json: serde_json::Value = serde_json::from_str(&text)
                .map_err(|e| format!("Failed to parse Graph API JSON: {}", e))?;
            return Ok(json);
        }

        let is_throttled = status == reqwest::StatusCode::TOO_MANY_REQUESTS
            || text.contains("activityLimitReached")
            || text.contains("throttledRequest")
            || text.contains("quota");

        if is_throttled && attempts <= MAX_ATTEMPTS {
            let wait_secs = retry_header.unwrap_or_else(|| {
                let response_hint = parse_retry_after_seconds(&text, None);
                let exponential_backoff = 2u64.pow(attempts.min(6));
                response_hint.max(exponential_backoff)
            });

            log::warn!(
                "Microsoft Graph API throttled. Waiting {} seconds before retrying (attempt {}/{})...",
                wait_secs,
                attempts,
                MAX_ATTEMPTS
            );
            tokio::time::sleep(std::time::Duration::from_secs(wait_secs)).await;
            continue;
        }

        return Err(format!("Graph API error: {}", text));
    }
}

fn parse_graph_item(val: &serde_json::Value, parent_path: &str) -> OneDriveItem {
    let id = val["id"].as_str().unwrap_or_default().to_string();
    let name = val["name"].as_str().unwrap_or_default().to_string();
    let size = val["size"].as_i64().unwrap_or(0);
    let is_folder = val.get("folder").is_some();
    let item_type = if is_folder { "folder" } else { "file" };

    let child_count = val["folder"]["childCount"].as_i64();
    let etag = val["eTag"].as_str().map(|s| s.to_string());
    let last_modified = val["lastModifiedDateTime"].as_str().map(|s| s.to_string());

    let quickxor_hash = val["file"]["hashes"]["quickXorHash"]
        .as_str()
        .map(|s| s.to_string());
    let sha1_hash = val["file"]["hashes"]["sha1Hash"]
        .as_str()
        .map(|s| s.to_string());

    let rel_path = if parent_path.is_empty() {
        name.clone()
    } else {
        format!("{}/{}", parent_path, name)
    };

    OneDriveItem {
        id,
        name,
        item_type: item_type.to_string(),
        size,
        path: Some(rel_path),
        child_count,
        etag,
        quickxor_hash,
        sha1_hash,
        last_modified,
    }
}

pub async fn list_children(
    http: &Client,
    access_token: &str,
    parent_id: Option<&str>,
) -> Result<Vec<OneDriveItem>, String> {
    let mut items = Vec::new();

    let mut next_url = match parent_id {
        Some(id) if !id.is_empty() && id != "root" => {
            format!(
                "https://graph.microsoft.com/v1.0/me/drive/items/{}/children?$top=200",
                id
            )
        }
        _ => "https://graph.microsoft.com/v1.0/me/drive/root/children?$top=200".to_string(),
    };

    loop {
        let json = send_graph_request(http, &next_url, access_token).await?;

        if let Some(arr) = json["value"].as_array() {
            for val in arr {
                items.push(parse_graph_item(val, ""));
            }
        }

        if let Some(next) = json["@odata.nextLink"].as_str() {
            next_url = next.to_string();
        } else {
            break;
        }
    }

    Ok(items)
}

pub async fn scan_folder_recursive(
    http: &Client,
    access_token: &str,
    folder_id: &str,
    current_rel_path: &str,
) -> Result<Vec<OneDriveItem>, String> {
    let mut result = Vec::new();
    let mut queue = vec![(folder_id.to_string(), current_rel_path.to_string())];

    while let Some((curr_id, curr_path)) = queue.pop() {
        let children = list_children(http, access_token, Some(&curr_id)).await?;

        for mut item in children {
            let item_rel_path = if curr_path.is_empty() {
                item.name.clone()
            } else {
                format!("{}/{}", curr_path, item.name)
            };

            item.path = Some(item_rel_path.clone());

            if item.item_type == "folder" {
                queue.push((item.id.clone(), item_rel_path));
            }
            result.push(item);
        }
    }

    Ok(result)
}

#[derive(Clone)]
struct DeltaDriveItem {
    item: OneDriveItem,
    parent_id: Option<String>,
    is_root: bool,
}

pub struct DeltaScanResume {
    pub next_url: Option<String>,
    pub pages_scanned: usize,
    pub elapsed_ms: u64,
    pub entries: Vec<serde_json::Value>,
}

pub enum DeltaScanOutcome {
    Completed(Vec<OneDriveItem>),
    Stopped(ScanProgressPayload),
}

fn apply_delta_values(
    entries: &mut std::collections::HashMap<String, DeltaDriveItem>,
    values: &[serde_json::Value],
) {
    for value in values {
        let id = value["id"].as_str().unwrap_or_default();
        if id.is_empty() {
            continue;
        }
        if value.get("deleted").is_some() {
            entries.remove(id);
            continue;
        }
        entries.insert(
            id.to_string(),
            DeltaDriveItem {
                item: parse_graph_item(value, ""),
                parent_id: value["parentReference"]["id"]
                    .as_str()
                    .map(ToString::to_string),
                is_root: value.get("root").is_some(),
            },
        );
    }
}

async fn wait_for_scan_stop(stop_requested: &std::sync::atomic::AtomicBool) {
    while !stop_requested.load(std::sync::atomic::Ordering::SeqCst) {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

fn resolve_delta_path(
    id: &str,
    entries: &std::collections::HashMap<String, DeltaDriveItem>,
    cache: &mut std::collections::HashMap<String, String>,
    visiting: &mut std::collections::HashSet<String>,
) -> String {
    if let Some(path) = cache.get(id) {
        return path.clone();
    }

    let Some(entry) = entries.get(id) else {
        return String::new();
    };
    if entry.is_root {
        return String::new();
    }
    if !visiting.insert(id.to_string()) {
        return entry.item.name.clone();
    }

    let name = entry.item.name.clone();
    let parent_id = entry.parent_id.clone();
    let parent_path = parent_id
        .as_deref()
        .map(|value| resolve_delta_path(value, entries, cache, visiting))
        .unwrap_or_default();
    visiting.remove(id);

    let path = if parent_path.is_empty() {
        name
    } else {
        format!("{parent_path}/{name}")
    };
    cache.insert(id.to_string(), path.clone());
    path
}

fn finalize_delta_entries(
    entries: std::collections::HashMap<String, DeltaDriveItem>,
) -> Vec<OneDriveItem> {
    use std::collections::{HashMap, HashSet};

    let mut path_cache = HashMap::new();
    let paths = entries
        .keys()
        .map(|id| {
            let path = resolve_delta_path(id, &entries, &mut path_cache, &mut HashSet::new());
            (id.clone(), path)
        })
        .collect::<HashMap<_, _>>();
    let mut items = entries
        .into_iter()
        .filter_map(|(id, mut entry)| {
            if entry.is_root {
                return None;
            }
            entry.item.path = paths.get(&id).cloned();
            Some(entry.item)
        })
        .collect::<Vec<_>>();
    items.sort_by(|left, right| {
        left.path
            .as_deref()
            .unwrap_or(&left.name)
            .cmp(right.path.as_deref().unwrap_or(&right.name))
            .then_with(|| left.id.cmp(&right.id))
    });
    items
}

pub fn build_delta_snapshot(values: &[serde_json::Value]) -> Vec<OneDriveItem> {
    let mut entries = std::collections::HashMap::new();
    apply_delta_values(&mut entries, values);
    finalize_delta_entries(entries)
}

pub async fn scan_drive_delta<F, C>(
    http: &Client,
    access_token: &str,
    resume: DeltaScanResume,
    stop_requested: &std::sync::atomic::AtomicBool,
    mut on_checkpoint: C,
    mut on_progress: F,
) -> Result<DeltaScanOutcome, String>
where
    F: FnMut(crate::migration::models::ScanProgressPayload),
    C: FnMut(
        &[serde_json::Value],
        Option<&str>,
        &crate::migration::models::ScanProgressPayload,
    ) -> Result<(), String>,
{
    use crate::migration::models::ScanProgressPayload;
    use std::collections::HashMap;
    use std::time::Instant;

    let started_at = Instant::now();
    let mut pages_scanned = resume.pages_scanned;
    let mut entries: HashMap<String, DeltaDriveItem> = HashMap::new();
    apply_delta_values(&mut entries, &resume.entries);
    let mut next_url = resume.next_url;

    let make_progress =
        |phase: &str, pages_scanned: usize, entries: &HashMap<String, DeltaDriveItem>| {
            ScanProgressPayload {
                phase: phase.into(),
                pages_scanned,
                discovered_files: entries
                    .values()
                    .filter(|entry| entry.item.item_type == "file")
                    .count(),
                discovered_folders: entries
                    .values()
                    .filter(|entry| entry.item.item_type == "folder" && !entry.is_root)
                    .count(),
                elapsed_ms: resume
                    .elapsed_ms
                    .saturating_add(started_at.elapsed().as_millis() as u64),
            }
        };

    on_progress(make_progress(
        if next_url.is_some() {
            "enumerating"
        } else {
            "building_snapshot"
        },
        pages_scanned,
        &entries,
    ));

    while let Some(page_url) = next_url.clone() {
        if stop_requested.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(DeltaScanOutcome::Stopped(make_progress(
                "stopped",
                pages_scanned,
                &entries,
            )));
        }
        let json = tokio::select! {
            result = send_graph_request(http, &page_url, access_token) => result?,
            _ = wait_for_scan_stop(stop_requested) => {
                return Ok(DeltaScanOutcome::Stopped(make_progress(
                    "stopped",
                    pages_scanned,
                    &entries,
                )));
            }
        };
        pages_scanned += 1;

        let page_values = json["value"].as_array().cloned().unwrap_or_default();
        apply_delta_values(&mut entries, &page_values);
        next_url = json["@odata.nextLink"].as_str().map(ToString::to_string);
        let progress = make_progress(
            if next_url.is_some() {
                "enumerating"
            } else {
                "building_snapshot"
            },
            pages_scanned,
            &entries,
        );
        on_checkpoint(&page_values, next_url.as_deref(), &progress)?;
        on_progress(progress);

        if stop_requested.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(DeltaScanOutcome::Stopped(make_progress(
                "stopped",
                pages_scanned,
                &entries,
            )));
        }
    }

    on_progress(make_progress("building_snapshot", pages_scanned, &entries));

    Ok(DeltaScanOutcome::Completed(finalize_delta_entries(entries)))
}

pub async fn download_item<F>(
    http: &Client,
    access_token: &str,
    item_id: &str,
    dest_part_path: &str,
    progress_cb: Option<F>,
) -> Result<String, String>
where
    F: Fn(u64, u64) + Send + Sync + 'static,
{
    let item_url = format!(
        "https://graph.microsoft.com/v1.0/me/drive/items/{}",
        item_id
    );
    let json = send_graph_request(http, &item_url, access_token).await?;

    let download_url = json["@microsoft.graph.downloadUrl"]
        .as_str()
        .ok_or_else(|| "Missing @microsoft.graph.downloadUrl in item response".to_string())?;

    let mut stream_resp = http
        .get(download_url)
        .send()
        .await
        .map_err(|e| format!("Download request error: {}", e))?;

    if !stream_resp.status().is_success() {
        return Err(format!(
            "Download stream HTTP error {}",
            stream_resp.status()
        ));
    }

    let total_bytes = stream_resp.content_length().unwrap_or(0);

    let mut file = tokio::fs::File::create(dest_part_path)
        .await
        .map_err(|e| format!("Failed to create part file {}: {}", dest_part_path, e))?;

    let mut hasher = Sha256::new();
    let mut downloaded_bytes = 0u64;

    while let Some(chunk) = stream_resp
        .chunk()
        .await
        .map_err(|e| format!("Stream chunk error: {}", e))?
    {
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("Failed to write chunk: {}", e))?;
        hasher.update(&chunk);
        downloaded_bytes += chunk.len() as u64;

        if let Some(ref cb) = progress_cb {
            cb(downloaded_bytes, total_bytes);
        }
    }

    file.flush()
        .await
        .map_err(|e| format!("Flush file error: {}", e))?;

    let hash_result = hasher.finalize();
    let hex_hash = format!("{:x}", hash_result);

    Ok(hex_hash)
}

pub async fn delete_onedrive_item(
    http: &Client,
    access_token: &str,
    item_id: &str,
) -> Result<(), String> {
    let url = format!(
        "https://graph.microsoft.com/v1.0/me/drive/items/{}",
        item_id
    );
    let mut attempts = 0;

    loop {
        attempts += 1;
        let resp = http
            .delete(&url)
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await
            .map_err(|e| format!("Delete Graph API request failed: {}", e))?;

        let status = resp.status();
        if status.is_success()
            || status == reqwest::StatusCode::NO_CONTENT
            || status == reqwest::StatusCode::NOT_FOUND
        {
            return Ok(());
        }

        let text = resp.text().await.unwrap_or_default();
        let is_throttled = status == reqwest::StatusCode::TOO_MANY_REQUESTS
            || text.contains("activityLimitReached")
            || text.contains("throttledRequest")
            || text.contains("quota");

        if is_throttled && attempts <= 5 {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            continue;
        }

        return Err(format!("Failed to delete item from OneDrive: {}", text));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    fn delta_entry(
        id: &str,
        name: &str,
        item_type: &str,
        parent_id: Option<&str>,
        is_root: bool,
    ) -> DeltaDriveItem {
        DeltaDriveItem {
            item: OneDriveItem {
                id: id.into(),
                name: name.into(),
                item_type: item_type.into(),
                size: 0,
                path: None,
                child_count: None,
                etag: None,
                quickxor_hash: None,
                sha1_hash: None,
                last_modified: None,
            },
            parent_id: parent_id.map(ToString::to_string),
            is_root,
        }
    }

    #[test]
    fn delta_paths_are_rebuilt_from_parent_ids() {
        let entries = HashMap::from([
            (
                "root".into(),
                delta_entry("root", "OneDrive", "folder", None, true),
            ),
            (
                "photos".into(),
                delta_entry("photos", "Photos", "folder", Some("root"), false),
            ),
            (
                "year".into(),
                delta_entry("year", "2026", "folder", Some("photos"), false),
            ),
            (
                "file".into(),
                delta_entry("file", "image.jpg", "file", Some("year"), false),
            ),
        ]);

        let path = resolve_delta_path("file", &entries, &mut HashMap::new(), &mut HashSet::new());

        assert_eq!(path, "Photos/2026/image.jpg");
    }

    #[test]
    fn persisted_delta_values_build_a_read_only_partial_snapshot() {
        let values = vec![
            serde_json::json!({
                "id": "root",
                "name": "OneDrive",
                "folder": { "childCount": 1 },
                "root": {}
            }),
            serde_json::json!({
                "id": "documents",
                "name": "Documents",
                "folder": { "childCount": 1 },
                "parentReference": { "id": "root" }
            }),
            serde_json::json!({
                "id": "file",
                "name": "checkpoint.txt",
                "size": 42,
                "file": {},
                "parentReference": { "id": "documents" }
            }),
        ];

        let snapshot = build_delta_snapshot(&values);

        assert_eq!(snapshot.len(), 2);
        assert_eq!(
            snapshot
                .iter()
                .find(|item| item.id == "file")
                .and_then(|item| item.path.as_deref()),
            Some("Documents/checkpoint.txt")
        );
    }
}
