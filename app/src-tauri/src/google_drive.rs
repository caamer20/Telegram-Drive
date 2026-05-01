use crate::bandwidth::BandwidthManager;
use crate::TelegramState;
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tauri::{Emitter, State};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_util::io::StreamReader;

const GOOGLE_AUTH_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
const GOOGLE_DRIVE_FILES_ENDPOINT: &str = "https://www.googleapis.com/drive/v3/files";
const DRIVE_SCOPE: &str = "https://www.googleapis.com/auth/drive.readonly";
const GOOGLE_DOC_MIME: &str = "application/vnd.google-apps.document";
const GOOGLE_SHEET_MIME: &str = "application/vnd.google-apps.spreadsheet";
const GOOGLE_SLIDES_MIME: &str = "application/vnd.google-apps.presentation";
const GOOGLE_DRAWING_MIME: &str = "application/vnd.google-apps.drawing";
const REDIRECT_URI: &str = "http://localhost:53682/oauth/google/callback";

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GdTokenResponse {
    pub access_token: String,
    pub expires_in: i64,
    pub refresh_token: Option<String>,
    pub scope: String,
    pub token_type: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GdFileItem {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    pub size: Option<String>,
    pub modified_time: Option<String>,
    pub export_links: Option<HashMap<String, String>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GdListResponse {
    pub files: Vec<GdFileItem>,
    pub next_page_token: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GdImportItem {
    pub id: String,
    pub name: String,
    pub size: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GdAuthUrlResponse {
    pub auth_url: String,
    pub redirect_uri: String,
    pub scope: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GdOAuthCodeResponse {
    pub code: String,
    pub state: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
struct ProgressPayload {
    id: String,
    percent: u8,
}

#[derive(Debug, Serialize, Clone)]
struct MigrationPayload {
    id: String,
    filename: String,
    status: String,
    percent: u8,
    error: Option<String>,
}

use crate::commands::fs::upload_stream_to_folder;

#[tauri::command]
pub async fn cmd_gd_auth_url(
    client_id: String,
    state: Option<String>,
) -> Result<GdAuthUrlResponse, String> {
    if client_id.trim().is_empty() {
        return Err("Google Client ID empty".to_string());
    }

    let mut url = reqwest::Url::parse(GOOGLE_AUTH_ENDPOINT).map_err(|e| e.to_string())?;

    let oauth_state = state.unwrap_or_else(|| {
        format!(
            "td-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0)
        )
    });

    {
        let mut qp = url.query_pairs_mut();
        qp.append_pair("client_id", client_id.trim());
        qp.append_pair("redirect_uri", REDIRECT_URI);
        qp.append_pair("response_type", "code");
        qp.append_pair("scope", DRIVE_SCOPE);
        qp.append_pair("access_type", "offline");
        qp.append_pair("prompt", "select_account");
        qp.append_pair("state", &oauth_state);
    }

    Ok(GdAuthUrlResponse {
        auth_url: url.to_string(),
        redirect_uri: REDIRECT_URI.to_string(),
        scope: DRIVE_SCOPE.to_string(),
    })
}

#[tauri::command]
pub async fn cmd_gd_exchange_token(
    client_id: String,
    client_secret: String,
    code: String,
) -> Result<GdTokenResponse, String> {
    if client_id.trim().is_empty() || client_secret.trim().is_empty() {
        return Err("Google Client ID/Secret empty".to_string());
    }
    if code.trim().is_empty() {
        return Err("OAuth code empty".to_string());
    }

    let http = Client::new();

    let mut form = HashMap::new();
    form.insert("client_id", client_id.trim().to_string());
    form.insert("client_secret", client_secret.trim().to_string());
    form.insert("code", code.trim().to_string());
    form.insert("grant_type", "authorization_code".to_string());
    form.insert("redirect_uri", REDIRECT_URI.to_string());

    let resp = http
        .post(GOOGLE_TOKEN_ENDPOINT)
        .form(&form)
        .send()
        .await
        .map_err(|e| format!("Google token request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!(
            "Google token exchange failed [{}]: {}",
            status, body
        ));
    }

    let token = resp
        .json::<GdTokenResponse>()
        .await
        .map_err(|e| format!("Invalid token response: {}", e))?;

    Ok(token)
}

#[tauri::command]
pub async fn cmd_gd_list_files(
    access_token: String,
    folder_id: Option<String>,
    page_token: Option<String>,
    page_size: Option<u32>,
) -> Result<GdListResponse, String> {
    if access_token.trim().is_empty() {
        return Err("Google access token empty".to_string());
    }

    let target_folder = folder_id.unwrap_or_else(|| "root".to_string());
    let mut q = format!(
        "'{}' in parents and trashed = false",
        target_folder.replace('"', "")
    );
    let _corpora = "user";

    // If reading the root of "Shared with me", we need a different query.
    if target_folder == "sharedWithMe" {
        q = "sharedWithMe = true and trashed = false".to_string();
    }

    let http = Client::new();
    let mut req = http
        .get(GOOGLE_DRIVE_FILES_ENDPOINT)
        .bearer_auth(access_token.trim())
        .query(&[
            ("q", q.as_str()),
            (
                "fields",
                "nextPageToken,files(id,name,mimeType,size,modifiedTime,exportLinks)",
            ),
            ("orderBy", "folder,name"),
            ("supportsAllDrives", "true"),
            ("includeItemsFromAllDrives", "true"),
            ("corpora", "allDrives"),
        ]);

    let size = page_size.unwrap_or(100).clamp(1, 1000);
    req = req.query(&[("pageSize", size)]);

    if let Some(token) = page_token {
        if !token.trim().is_empty() {
            req = req.query(&[("pageToken", token.trim())]);
        }
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("Google list request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Google list failed [{}]: {}", status, body));
    }

    let data = resp
        .json::<GdListResponse>()
        .await
        .map_err(|e| format!("Invalid list response: {}", e))?;

    Ok(data)
}

#[tauri::command]
pub async fn cmd_gd_import_files(
    access_token: String,
    items: Vec<GdImportItem>,
    folder_id: Option<i64>,
    app_handle: tauri::AppHandle,
    state: State<'_, TelegramState>,
    bw_state: State<'_, BandwidthManager>,
) -> Result<Vec<String>, String> {
    if access_token.trim().is_empty() {
        return Err("Google access token empty".to_string());
    }
    if items.is_empty() {
        return Ok(vec![]);
    }

    let client_opt = { state.client.lock().await.clone() };
    if client_opt.is_none() {
        return Err("Telegram client not connected".to_string());
    }
    let tg_client = client_opt.unwrap();

    let http = Client::new();
    let mut results = Vec::new();

    for item in items {
        let transfer_id = format!("gd-{}", item.id);

        let _ = app_handle.emit(
            "migration-progress",
            MigrationPayload {
                id: transfer_id.clone(),
                filename: item.name.clone(),
                status: "starting".to_string(),
                percent: 0,
                error: None,
            },
        );

        let import_res: Result<(), String> = async {
            let file_meta_url = format!("{}/{}", GOOGLE_DRIVE_FILES_ENDPOINT, item.id);

            let meta_resp = http
                .get(&file_meta_url)
                .bearer_auth(access_token.trim())
                .query(&[("fields", "id,name,size,mimeType,exportLinks")])
                .send()
                .await
                .map_err(|e| format!("Meta request failed: {}", e))?;

            if !meta_resp.status().is_success() {
                let status = meta_resp.status();
                let body = meta_resp.text().await.unwrap_or_default();
                return Err(format!("Meta request failed [{}]: {}", status, body));
            }

            let meta = meta_resp
                .json::<GdFileItem>()
                .await
                .map_err(|e| format!("Invalid meta response: {}", e))?;

            if meta.mime_type == "application/vnd.google-apps.folder" {
                return Err("Folder import not supported yet. Select files only.".to_string());
            }
            let export_target = google_export_target(&meta.mime_type);
            let size = if export_target.is_some() {
                0
            } else {
                meta.size
                    .as_deref()
                    .or(item.size.map(|s| s.to_string()).as_deref())
                    .ok_or_else(|| "Google file size missing".to_string())?
                    .parse::<u64>()
                    .map_err(|e| format!("Invalid Google file size: {}", e))?
            };

            if size > 0 {
                bw_state.can_transfer(size)?;
            }

            let _ = app_handle.emit(
                "migration-progress",
                MigrationPayload {
                    id: transfer_id.clone(),
                    filename: meta.name.clone(),
                    status: "downloading".to_string(),
                    percent: 10,
                    error: None,
                },
            );

            let (download_url, upload_name) = if let Some((mime, extension)) = export_target {
                (
                    format!(
                        "{}/{}/export?mimeType={}",
                        GOOGLE_DRIVE_FILES_ENDPOINT,
                        item.id,
                        urlencoding::encode(mime)
                    ),
                    ensure_extension(&meta.name, extension),
                )
            } else {
                (
                    format!("{}?alt=media&supportsAllDrives=true", file_meta_url),
                    meta.name.clone(),
                )
            };

            let media_resp = http
                .get(&download_url)
                .bearer_auth(access_token.trim())
                .send()
                .await
                .map_err(|e| format!("Drive download failed: {}", e))?;

            if !media_resp.status().is_success() {
                let status = media_resp.status();
                let body = media_resp.text().await.unwrap_or_default();
                return Err(format!("Drive download failed [{}]: {}", status, body));
            }

            let content_size = media_resp.content_length().unwrap_or(size);
            if content_size > 0 {
                bw_state.can_transfer(content_size)?;
            }

            let stream = media_resp
                .bytes_stream()
                .map(|chunk| chunk.map_err(std::io::Error::other));
            let mut reader = StreamReader::new(stream);

            let _ = app_handle.emit(
                "migration-progress",
                MigrationPayload {
                    id: transfer_id.clone(),
                    filename: meta.name.clone(),
                    status: "uploading".to_string(),
                    percent: 30,
                    error: None,
                },
            );
            let _ = app_handle.emit(
                "upload-progress",
                ProgressPayload {
                    id: transfer_id.clone(),
                    percent: 30,
                },
            );

            upload_stream_to_folder(
                &tg_client,
                &mut reader,
                content_size as usize,
                upload_name.clone(),
                folder_id,
            )
            .await?;

            bw_state.add_up(content_size);

            let _ = app_handle.emit(
                "migration-progress",
                MigrationPayload {
                    id: transfer_id.clone(),
                    filename: meta.name.clone(),
                    status: "done".to_string(),
                    percent: 100,
                    error: None,
                },
            );
            let _ = app_handle.emit(
                "upload-progress",
                ProgressPayload {
                    id: transfer_id.clone(),
                    percent: 100,
                },
            );

            Ok(())
        }
        .await;

        match import_res {
            Ok(()) => {
                results.push(format!("{}:ok", item.name));
            }
            Err(e) => {
                let _ = app_handle.emit(
                    "migration-progress",
                    MigrationPayload {
                        id: transfer_id,
                        filename: item.name.clone(),
                        status: "error".to_string(),
                        percent: 0,
                        error: Some(e.clone()),
                    },
                );
                results.push(format!("{}:error:{}", item.name, e));
            }
        }
    }

    Ok(results)
}

pub fn sanitize_filename(name: &str) -> String {
    let trimmed = name.trim();
    let fallback = "file";
    let base = if trimmed.is_empty() {
        fallback
    } else {
        trimmed
    };
    base.replace(['/', '\\'], "_")
}

pub async fn fallback_import_via_tempfile(
    tg_client: &grammers_client::Client,
    access_token: &str,
    file_id: &str,
    filename: &str,
    size: u64,
    folder_id: Option<i64>,
) -> Result<(), String> {
    let http = Client::new();
    let download_url = format!("{}/{}", GOOGLE_DRIVE_FILES_ENDPOINT, file_id);

    let mut temp = tempfile::Builder::new()
        .prefix("td-gd-")
        .suffix(&format!("-{}", sanitize_filename(filename)))
        .tempfile()
        .map_err(|e| format!("Create tempfile failed: {}", e))?;

    let mut resp = http
        .get(&download_url)
        .bearer_auth(access_token)
        .query(&[("alt", "media")])
        .send()
        .await
        .map_err(|e| format!("Drive download failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Drive download failed [{}]: {}", status, body));
    }

    while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
        use std::io::Write;
        temp.write_all(&chunk).map_err(|e| e.to_string())?;
    }

    let path = temp.path().to_path_buf();
    let mut file = tokio::fs::File::open(&path)
        .await
        .map_err(|e| format!("Open tempfile failed: {}", e))?;

    upload_stream_to_folder(
        tg_client,
        &mut file,
        size as usize,
        sanitize_filename(filename),
        folder_id,
    )
    .await
}

fn google_export_target(mime: &str) -> Option<(&'static str, &'static str)> {
    match mime {
        GOOGLE_DOC_MIME => Some((
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "docx",
        )),
        GOOGLE_SHEET_MIME => Some((
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "xlsx",
        )),
        GOOGLE_SLIDES_MIME => Some((
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            "pptx",
        )),
        GOOGLE_DRAWING_MIME => Some(("image/png", "png")),
        _ => None,
    }
}

fn ensure_extension(name: &str, ext: &str) -> String {
    if name
        .to_lowercase()
        .ends_with(&format!(".{}", ext.to_lowercase()))
    {
        name.to_string()
    } else {
        format!("{}.{}", name, ext)
    }
}

#[tauri::command]
pub async fn cmd_gd_wait_for_auth_code() -> Result<GdOAuthCodeResponse, String> {
    let listener = TcpListener::bind("127.0.0.1:53682")
        .await
        .map_err(|e| format!("Bind error: {}", e))?;

    // Timeout so we don't hang forever if user closes browser
    let accept_future = listener.accept();
    let accept_res = tokio::time::timeout(std::time::Duration::from_secs(120), accept_future).await;

    let (mut stream, _) = match accept_res {
        Ok(Ok(res)) => res,
        Ok(Err(e)) => return Err(format!("Accept error: {}", e)),
        Err(_) => return Err("Timeout waiting for Google auth callback".to_string()),
    };

    let mut buf = [0; 4096];
    let n = stream
        .read(&mut buf)
        .await
        .map_err(|e| format!("Read error: {}", e))?;
    let req = String::from_utf8_lossy(&buf[..n]);

    let mut code = String::new();
    let mut state = None;

    if let Some(line) = req.lines().next() {
        if let Some(path) = line.split_whitespace().nth(1) {
            if let Some(query) = path.split('?').nth(1) {
                for pair in query.split('&') {
                    let mut parts = pair.split('=');
                    if let (Some(k), Some(v)) = (parts.next(), parts.next()) {
                        if k == "code" {
                            code = v.to_string();
                        } else if k == "state" {
                            state = Some(v.to_string());
                        }
                    }
                }
            }
        }
    }

    let response = if code.is_empty() {
        concat!(
            "HTTP/1.1 400 Bad Request\r\n",
            "Content-Type: text/html\r\n",
            "Connection: close\r\n\r\n",
            "<!DOCTYPE html><html><head><title>Auth Failed</title>",
            "<style>",
            "body{background:#0f172a;color:#e2e8f0;font-family:system-ui,-apple-system,sans-serif;display:flex;align-items:center;justify-content:center;height:100vh;margin:0;}",
            "div{background:#1e293b;padding:2rem;border-radius:12px;text-align:center;box-shadow:0 10px 15px -3px rgb(0 0 0/0.1),0 4px 6px -4px rgb(0 0 0/0.1);border:1px solid #334155;}",
            "h2{color:#f87171;margin-top:0;} p{color:#94a3b8;}",
            "</style></head><body>",
            "<div><h2>Authentication Failed</h2><p>No authorization code found.</p><p>You can close this window.</p></div>",
            "</body></html>"
        )
    } else {
        concat!(
            "HTTP/1.1 200 OK\r\n",
            "Content-Type: text/html\r\n",
            "Connection: close\r\n\r\n",
            "<!DOCTYPE html><html><head><title>Auth Success</title>",
            "<style>",
            "body{background:#0f172a;color:#e2e8f0;font-family:system-ui,-apple-system,sans-serif;display:flex;align-items:center;justify-content:center;height:100vh;margin:0;}",
            "div{background:#1e293b;padding:2rem;border-radius:12px;text-align:center;box-shadow:0 10px 15px -3px rgb(0 0 0/0.1),0 4px 6px -4px rgb(0 0 0/0.1);border:1px solid #334155;}",
            "h2{color:#38bdf8;margin-top:0;} p{color:#94a3b8;}",
            "</style></head><body>",
            "<div><h2>Authentication Successful!</h2><p>You can return to Telegram Drive.</p><p style='font-size:0.875rem;margin-top:1.5rem;color:#64748b;'>Closing window automatically in 3 seconds...</p></div>",
            "<script>setTimeout(()=>window.close(), 3000);</script>",
            "</body></html>"
        )
    };

    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.flush().await;
    // VERY IMPORTANT: Shutdown the stream so the browser knows the response is fully sent
    let _ = stream.shutdown().await;

    if code.is_empty() {
        return Err("No code found in redirect URL".to_string());
    }

    Ok(GdOAuthCodeResponse { code, state })
}

#[tauri::command]
pub async fn cmd_gd_refresh_token(
    client_id: String,
    client_secret: String,
    refresh_token: String,
) -> Result<GdTokenResponse, String> {
    if client_id.trim().is_empty() || client_secret.trim().is_empty() {
        return Err("Google Client ID/Secret empty".to_string());
    }
    if refresh_token.trim().is_empty() {
        return Err("Refresh token empty".to_string());
    }

    let http = Client::new();

    let mut form = HashMap::new();
    form.insert("client_id", client_id.trim().to_string());
    form.insert("client_secret", client_secret.trim().to_string());
    form.insert("refresh_token", refresh_token.trim().to_string());
    form.insert("grant_type", "refresh_token".to_string());

    let resp = http
        .post(GOOGLE_TOKEN_ENDPOINT)
        .form(&form)
        .send()
        .await
        .map_err(|e| format!("Google token refresh failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!(
            "Google token refresh failed [{}]: {}",
            status, body
        ));
    }

    let token = resp
        .json::<GdTokenResponse>()
        .await
        .map_err(|e| format!("Invalid token refresh response: {}", e))?;

    Ok(token)
}
