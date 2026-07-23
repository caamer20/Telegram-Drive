use crate::commands::utils::resolve_peer;
use crate::commands::TelegramState;
use crate::db::DbConnection;
use actix_web::{cookie::Cookie, get, post, web, HttpRequest, HttpResponse, Responder};
use grammers_client::types::Media;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::sync::Arc;

#[derive(Clone)]
struct SharedLinkRow {
    _id: String,
    folder_id: Option<i64>,
    message_id: i32,
    file_name: String,
    _file_size: i64,
    password_hash: Option<String>,
    _password_salt: Option<String>,
    expires_at: Option<i64>,
    revoked: bool,
}

#[derive(Deserialize)]
struct VerifyForm {
    password: String,
}

/// Verify a password against a bcrypt hash.
fn verify_password(password: &str, hash: &str) -> bool {
    bcrypt::verify(password, hash).unwrap_or(false)
}

fn generate_cookie_val(token: &str, password_hash: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hasher.update(password_hash.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn get_share_by_token(db: &DbConnection, token: &str) -> Result<Option<SharedLinkRow>, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT id, folder_id, message_id, file_name, file_size, password_hash, password_salt, expires_at, revoked 
             FROM shared_links WHERE id = ?"
        )
        .map_err(|e| e.to_string())?;

    stmt.bind((1, token)).map_err(|e| e.to_string())?;

    if let sqlite::State::Row = stmt.next().map_err(|e| e.to_string())? {
        let id = stmt.read::<String, _>("id").map_err(|e| e.to_string())?;
        let folder_id = stmt.read::<Option<i64>, _>("folder_id").ok().flatten();
        let message_id = stmt
            .read::<i64, _>("message_id")
            .map_err(|e| e.to_string())? as i32;
        let file_name = stmt
            .read::<String, _>("file_name")
            .map_err(|e| e.to_string())?;
        let file_size = stmt
            .read::<i64, _>("file_size")
            .map_err(|e| e.to_string())?;
        let password_hash = stmt
            .read::<Option<String>, _>("password_hash")
            .ok()
            .flatten();
        let _password_salt = stmt
            .read::<Option<String>, _>("password_salt")
            .ok()
            .flatten();
        let expires_at = stmt.read::<Option<i64>, _>("expires_at").ok().flatten();
        let revoked = stmt.read::<i64, _>("revoked").map_err(|e| e.to_string())? != 0;

        Ok(Some(SharedLinkRow {
            _id: id,
            folder_id,
            message_id,
            file_name,
            _file_size: file_size,
            password_hash,
            _password_salt,
            expires_at,
            revoked,
        }))
    } else {
        Ok(None)
    }
}

/// Renders the password entry form for protected share links.
///
/// NOTE: This HTML contains an inline `<style>` block which requires
/// `style-src 'unsafe-inline'` in the Tauri CSP (tauri.conf.json).
/// This is acceptable because the page is served only over the local
/// Actix streaming server (127.0.0.1/0.0.0.0:14201), not the public internet,
/// so the XSS attack surface is minimal.
fn render_password_form(file_name: &str, token: &str, error: Option<&str>) -> HttpResponse {
    let error_html = match error {
        Some(err) => format!("<div class=\"error\">{}</div>", err),
        None => "".to_string(),
    };

    let html = format!(
        r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <title>Password Protected File - Telegram Drive</title>
    <style>
        body {{
            background-color: #182533;
            color: #ffffff;
            font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
            display: flex;
            align-items: center;
            justify-content: center;
            height: 100vh;
            margin: 0;
        }}
        .container {{
            background: #202b36;
            padding: 2rem;
            border-radius: 12px;
            box-shadow: 0 8px 24px rgba(0, 0, 0, 0.2);
            border: 1px solid #2f3e4e;
            width: 100%;
            max-width: 400px;
            text-align: center;
        }}
        h2 {{
            margin-top: 0;
            color: #40a7e3;
        }}
        p {{
            font-size: 14px;
            color: #7f91a4;
            margin-bottom: 20px;
        }}
        input[type="password"] {{
            width: 100%;
            padding: 12px;
            border-radius: 6px;
            border: 1px solid #2f3e4e;
            background: #182533;
            color: white;
            box-sizing: border-box;
            margin-bottom: 15px;
            font-size: 16px;
        }}
        input[type="password"]:focus {{
            outline: none;
            border-color: #40a7e3;
        }}
        button {{
            width: 100%;
            padding: 12px;
            border-radius: 6px;
            border: none;
            background: #40a7e3;
            color: white;
            font-weight: bold;
            cursor: pointer;
            font-size: 16px;
            transition: background 0.2s;
        }}
        button:hover {{
            background: #3598d1;
        }}
        .error {{
            color: #ff5e5e;
            font-size: 14px;
            margin-bottom: 15px;
        }}
    </style>
</head>
<body>
    <div class="container">
        <h2>Enter Password</h2>
        <p>This share link is password-protected.<br>File: <strong>{}</strong></p>
        {}
        <form method="POST" action="/d/{}/verify">
            <input type="password" name="password" placeholder="Password" autofocus required>
            <button type="submit">Verify & Download</button>
        </form>
    </div>
</body>
</html>"#,
        file_name, error_html, token
    );

    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(html)
}

#[get("/d/{token}")]
async fn get_shared_file(
    req: HttpRequest,
    path: web::Path<String>,
    db_conn: web::Data<DbConnection>,
    tg_state: web::Data<Arc<TelegramState>>,
) -> impl Responder {
    let token = path.into_inner();

    let row = match get_share_by_token(&db_conn, &token) {
        Ok(Some(r)) => r,
        Ok(None) => return HttpResponse::NotFound().body("Shared link not found"),
        Err(e) => {
            log::error!("DB error resolving token {}: {}", token, e);
            return HttpResponse::InternalServerError().body("Internal server error");
        }
    };

    // Check validation (revocation and expiration)
    if row.revoked {
        return HttpResponse::NotFound().body("This shared link has been revoked");
    }

    if let Some(expiry) = row.expires_at {
        let now = chrono::Utc::now().timestamp();
        if expiry < now {
            return HttpResponse::Gone().body("This shared link has expired");
        }
    }

    // Check password protection
    if let Some(hash) = &row.password_hash {
        let mut authenticated = false;
        if let Some(cookie) = req.cookie(&format!("share_auth_{}", token)) {
            let expected = generate_cookie_val(&token, hash);
            if cookie.value() == expected {
                authenticated = true;
            }
        }

        if !authenticated {
            return render_password_form(&row.file_name, &token, None);
        }
    }

    // Retrieve and stream the file from Telegram
    let client_opt = { tg_state.client.lock().await.clone() };
    let client = match client_opt {
        Some(c) => c,
        None => return HttpResponse::ServiceUnavailable().body("Telegram client is not connected"),
    };

    let peer = match resolve_peer(&client, row.folder_id, &tg_state.peer_cache).await {
        Ok(p) => p,
        Err(e) => {
            log::error!("Failed to resolve peer for share: {}", e);
            return HttpResponse::InternalServerError().body("Failed to locate folder");
        }
    };

    match client.get_messages_by_id(peer, &[row.message_id]).await {
        Ok(messages) => {
            if let Some(Some(msg)) = messages.first() {
                if let Some(media) = msg.media() {
                    let mime = match &media {
                        Media::Document(d) => d
                            .mime_type()
                            .unwrap_or("application/octet-stream")
                            .to_string(),
                        _ => "application/octet-stream".to_string(),
                    };
                    let filename = &row.file_name;

                    return crate::server::build_media_response(
                        &client,
                        &media,
                        &req,
                        &mime,
                        Some(filename),
                        crate::server::StreamingExtras {
                            extra_headers: vec![],
                            log_label: "Share download",
                        },
                    );
                }
            }
            HttpResponse::NotFound().body("Message or media not found in Telegram")
        }
        Err(e) => {
            log::error!("Failed to fetch shared message {}: {}", row.message_id, e);
            HttpResponse::InternalServerError().body(format!("Failed to retrieve file: {}", e))
        }
    }
}

#[post("/d/{token}/verify")]
async fn verify_shared_file_password(
    path: web::Path<String>,
    form: web::Form<VerifyForm>,
    db_conn: web::Data<DbConnection>,
) -> impl Responder {
    let token = path.into_inner();

    let row = match get_share_by_token(&db_conn, &token) {
        Ok(Some(r)) => r,
        Ok(None) => return HttpResponse::NotFound().body("Shared link not found"),
        Err(e) => {
            log::error!("DB error resolving token {}: {}", token, e);
            return HttpResponse::InternalServerError().body("Internal server error");
        }
    };

    if row.revoked {
        return HttpResponse::NotFound().body("This shared link has been revoked");
    }

    let hash = match &row.password_hash {
        Some(h) => h,
        None => return HttpResponse::BadRequest().body("No password required for this link"),
    };

    if verify_password(&form.password, hash) {
        // Set session cookie (30 min).
        // NOTE: The streaming share server binds to 0.0.0.0 over plain HTTP (not HTTPS),
        // so the cookie cannot use `.secure(true)` without becoming unusable.
        // The cookie is protected by `.http_only(true)` and `.same_site(Strict)`
        // to mitigate XSS and CSRF within the constraints of a local-network HTTP service.
        let val = generate_cookie_val(&token, hash);
        let cookie = Cookie::build(format!("share_auth_{}", token), val)
            .path(format!("/d/{}", token))
            .http_only(true)
            .same_site(actix_web::cookie::SameSite::Strict)
            .max_age(actix_web::cookie::time::Duration::minutes(30))
            .finish();

        HttpResponse::Found()
            .insert_header(("Location", format!("/d/{}", token)))
            .cookie(cookie)
            .finish()
    } else {
        render_password_form(
            &row.file_name,
            &token,
            Some("Incorrect password. Please try again."),
        )
    }
}

pub fn configure_share_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(get_shared_file)
        .service(verify_shared_file_password);
}
