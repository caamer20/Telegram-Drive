use crate::commands::utils::resolve_peer;
use crate::commands::TelegramState;
use crate::db::DbConnection;
use crate::workspace::AccountGuard;

#[derive(Clone)]
pub struct ShareAccountRoot(pub std::path::PathBuf);
use actix_web::{
    cookie::Cookie, get, middleware::DefaultHeaders, post, web, HttpRequest, HttpResponse,
    Responder,
};
use grammers_client::types::Media;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, LazyLock, Mutex};

const SHARE_PASSWORD_ATTEMPT_LIMIT: usize = 5;
const SHARE_PASSWORD_WINDOW_SECONDS: i64 = 5 * 60;
const SHARE_PASSWORD_COOLDOWN_SECONDS: i64 = 30;
const MAX_TRACKED_SHARE_TOKENS: usize = 1_024;
const MAX_SHARE_PASSWORD_CHARS: usize = 128;
const MAX_SHARE_PASSWORD_BYTES: usize = 512;
const SHARE_VERIFY_FORM_LIMIT_BYTES: usize = 1_024;
const SHARE_SECURITY_HEADERS: &[(&str, &str)] = &[
    ("Content-Security-Policy", "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; base-uri 'none'; frame-ancestors 'none'"),
    ("X-Content-Type-Options", "nosniff"),
    ("X-Frame-Options", "DENY"),
    ("Referrer-Policy", "no-referrer"),
    ("Permissions-Policy", "camera=(), microphone=(), geolocation=(), payment=(), usb=()"),
    ("Cache-Control", "no-store"),
    ("Cross-Origin-Opener-Policy", "same-origin"),
];

#[derive(Default)]
struct PasswordAttemptState {
    attempts: VecDeque<i64>,
    blocked_until: i64,
    last_seen: i64,
}

#[derive(Default)]
struct PasswordAttemptLimiter {
    attempts_by_token: HashMap<[u8; 32], PasswordAttemptState>,
}

impl PasswordAttemptLimiter {
    fn begin_attempt(&mut self, token_key: [u8; 32], now: i64) -> Result<(), i64> {
        self.prune(now);

        if !self.attempts_by_token.contains_key(&token_key)
            && self.attempts_by_token.len() >= MAX_TRACKED_SHARE_TOKENS
        {
            if let Some(oldest_key) = self
                .attempts_by_token
                .iter()
                .min_by_key(|(_, state)| state.last_seen)
                .map(|(key, _)| *key)
            {
                self.attempts_by_token.remove(&oldest_key);
            }
        }

        let state = self.attempts_by_token.entry(token_key).or_default();
        state.last_seen = now;

        if state.blocked_until > now {
            return Err((state.blocked_until - now).max(1));
        }
        if state.blocked_until != 0 {
            state.blocked_until = 0;
            state.attempts.clear();
        }

        let cutoff = now.saturating_sub(SHARE_PASSWORD_WINDOW_SECONDS);
        while state
            .attempts
            .front()
            .is_some_and(|attempt| *attempt <= cutoff)
        {
            state.attempts.pop_front();
        }

        if state.attempts.len() >= SHARE_PASSWORD_ATTEMPT_LIMIT {
            state.attempts.clear();
            state.blocked_until = now.saturating_add(SHARE_PASSWORD_COOLDOWN_SECONDS);
            return Err(SHARE_PASSWORD_COOLDOWN_SECONDS);
        }

        // Reserve the attempt before running bcrypt so concurrent requests cannot
        // all bypass the limit while password verification is in progress.
        state.attempts.push_back(now);
        Ok(())
    }

    fn record_failure(&mut self, token_key: &[u8; 32], now: i64) {
        if let Some(state) = self.attempts_by_token.get_mut(token_key) {
            state.last_seen = now;
            if state.attempts.len() >= SHARE_PASSWORD_ATTEMPT_LIMIT {
                state.attempts.clear();
                state.blocked_until = now.saturating_add(SHARE_PASSWORD_COOLDOWN_SECONDS);
            }
        }
    }

    fn clear(&mut self, token_key: &[u8; 32]) {
        self.attempts_by_token.remove(token_key);
    }

    fn prune(&mut self, now: i64) {
        let cutoff = now.saturating_sub(SHARE_PASSWORD_WINDOW_SECONDS);
        self.attempts_by_token.retain(|_, state| {
            while state
                .attempts
                .front()
                .is_some_and(|attempt| *attempt <= cutoff)
            {
                state.attempts.pop_front();
            }
            state.blocked_until > now || !state.attempts.is_empty()
        });
    }
}

static PASSWORD_ATTEMPT_LIMITER: LazyLock<Mutex<PasswordAttemptLimiter>> =
    LazyLock::new(|| Mutex::new(PasswordAttemptLimiter::default()));

fn token_attempt_key(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

fn with_password_attempt_limiter<T>(operation: impl FnOnce(&mut PasswordAttemptLimiter) -> T) -> T {
    let mut limiter = PASSWORD_ATTEMPT_LIMITER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    operation(&mut limiter)
}

#[derive(Clone)]
struct SharedLinkRow {
    _id: String,
    owner_id: Option<i64>,
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
    #[serde(default)]
    lang: Option<String>,
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

async fn get_share_by_token(
    db: DbConnection,
    token: String,
) -> Result<Option<SharedLinkRow>, String> {
    crate::db::with_connection(db, move |conn| {
        let mut stmt = conn
        .prepare(
            "SELECT id, owner_id, folder_id, message_id, file_name, file_size, password_hash, password_salt, expires_at, revoked
             FROM shared_links WHERE id = ?"
        )
        .map_err(|e| e.to_string())?;

        stmt.bind((1, token.as_str())).map_err(|e| e.to_string())?;

    if let sqlite::State::Row = stmt.next().map_err(|e| e.to_string())? {
        let id = stmt.read::<String, _>("id").map_err(|e| e.to_string())?;
        let folder_id = stmt.read::<Option<i64>, _>("folder_id").ok().flatten();
        let message_id = stmt.read::<i64, _>("message_id").map_err(|e| e.to_string())? as i32;
        let file_name = stmt.read::<String, _>("file_name").map_err(|e| e.to_string())?;
        let file_size = stmt.read::<i64, _>("file_size").map_err(|e| e.to_string())?;
        let password_hash = stmt.read::<Option<String>, _>("password_hash").ok().flatten();
        let _password_salt = stmt.read::<Option<String>, _>("password_salt").ok().flatten();
        let expires_at = stmt.read::<Option<i64>, _>("expires_at").ok().flatten();
        let revoked = stmt.read::<i64, _>("revoked").map_err(|e| e.to_string())? != 0;

            Ok(Some(SharedLinkRow {
            _id: id,
            owner_id: stmt.read::<Option<i64>, _>("owner_id").map_err(|e| e.to_string())?,
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
    }).await
}

fn share_account(root: &ShareAccountRoot, row: &SharedLinkRow) -> Result<AccountGuard, String> {
    let owner = row
        .owner_id
        .filter(|owner| *owner > 0)
        .ok_or("This legacy link has no verified owner; recreate it from the original account")?;
    AccountGuard::open(&root.0, Some(&owner.to_string()))
}

/// Renders the password entry form for protected share links.
///
/// NOTE: This HTML contains an inline `<style>` block which requires
/// `style-src 'unsafe-inline'` in the Tauri CSP (tauri.conf.json).
/// This page is served only by the loopback-bound Actix streaming server,
/// not by a hosted internet service. Dynamic values are escaped before they
/// are interpolated into the document.
fn escape_html(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

fn query_language(req: &HttpRequest) -> Option<&'static str> {
    req.query_string().split('&').take(64).find_map(|field| {
        let (name, value) = field.split_once('=')?;
        let name = urlencoding::decode(name).ok()?;
        if name != "lang" {
            return None;
        }
        crate::native_localization::canonical_language(&urlencoding::decode(value).ok()?)
    })
}

fn language_quality(value: &str) -> Option<u16> {
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if !matches!(whole, "0" | "1")
        || fraction.len() > 3
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    if whole == "1" {
        return fraction.bytes().all(|byte| byte == b'0').then_some(1000);
    }
    Some(if fraction.is_empty() {
        0
    } else {
        fraction.parse::<u16>().ok()? * 10_u16.pow(3 - fraction.len() as u32)
    })
}

fn resolve_req_lang(
    req: &HttpRequest,
    form_language: Option<&str>,
) -> (&'static str, &'static str) {
    use crate::native_localization::{canonical_language, direction, LANGUAGES};
    if let Some(language) =
        query_language(req).or_else(|| form_language.and_then(canonical_language))
    {
        return (language, direction(language));
    }
    let accept = req
        .headers()
        .get("Accept-Language")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let ranges: Vec<_> = accept
        .split(',')
        .take(64)
        .enumerate()
        .filter_map(|(order, entry)| {
            let mut parts = entry.trim().split(';');
            let range = parts.next()?.trim().to_ascii_lowercase();
            if range != "*" && canonical_language(&range).is_none() {
                return None;
            }
            let quality = match parts.next() {
                None => 1000,
                Some(parameter) => {
                    let (name, value) = parameter.trim().split_once('=')?;
                    if !name.trim().eq_ignore_ascii_case("q") || parts.next().is_some() {
                        return None;
                    }
                    language_quality(value.trim())?
                }
            };
            Some((range, quality, order))
        })
        .collect();
    let mut best: Option<(&'static str, u16, usize)> = None;
    let mut excluded = Vec::new();
    for &language in LANGUAGES {
        let tag = language.to_ascii_lowercase();
        let matching = ranges
            .iter()
            .filter(|(range, _, _)| {
                range == "*"
                    || canonical_language(range) == Some(language)
                    || tag == *range
                    || tag
                        .strip_prefix(range)
                        .is_some_and(|rest| rest.starts_with('-'))
            })
            .max_by(|left, right| {
                let specificity = |range: &str| {
                    if range == "*" {
                        0
                    } else {
                        range.split('-').count()
                    }
                };
                specificity(&left.0)
                    .cmp(&specificity(&right.0))
                    // Regional aliases share one catalog. At equal specificity,
                    // retain the best accepted variant before breaking ties by order.
                    .then_with(|| left.1.cmp(&right.1))
                    .then_with(|| right.2.cmp(&left.2))
            });
        let Some((_, quality, order)) = matching else {
            continue;
        };
        if *quality == 0 {
            excluded.push(language);
            continue;
        }
        if best.is_none_or(|(_, previous_quality, previous_order)| {
            *quality > previous_quality || (*quality == previous_quality && *order < previous_order)
        }) {
            best = Some((language, *quality, *order));
        }
    }
    // Unsupported or wholly unacceptable headers may be disregarded, as HTTP
    // permits. Prefer a non-excluded default whenever one remains available.
    let language = best.map(|(language, _, _)| language).unwrap_or_else(|| {
        LANGUAGES
            .iter()
            .copied()
            .find(|language| !excluded.contains(language))
            .unwrap_or("en")
    });
    (language, direction(language))
}

fn render_password_form(
    req: &HttpRequest,
    file_name: &str,
    token: &str,
    error: Option<&str>,
    form_language: Option<&str>,
) -> HttpResponse {
    let (lang, dir) = resolve_req_lang(req, form_language);
    let safe_file_name = escape_html(file_name);
    use crate::native_localization::text;
    let title_text = escape_html(text(lang, "share_page.title"));
    let heading_text = escape_html(text(lang, "share_page.heading"));
    let desc_text = escape_html(text(lang, "share_page.description"));
    let file_label = escape_html(text(lang, "files.file_name"));
    let password_placeholder = escape_html(text(lang, "common.password"));
    let btn_text = escape_html(text(lang, "share_page.verify"));
    let incorrect_password = text(lang, "share_page.incorrect");
    let form_action = escape_html(&format!(
        "/d/{}/verify?lang={lang}",
        urlencoding::encode(token)
    ));
    let error_html = match error {
        Some(_) => format!(
            "<div class=\"error\">{}</div>",
            escape_html(incorrect_password)
        ),
        None => "".to_string(),
    };

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="{}" dir="{}">
<head>
    <meta charset="utf-8">
    <title>{} - Telegram Drive</title>
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
        <h2>{}</h2>
        <p>{}<br>{}: <strong><bdi dir="auto">{}</bdi></strong></p>
        {}
        <form method="POST" action="{}">
            <input type="hidden" name="lang" value="{}">
            <input type="password" name="password" placeholder="{}" autofocus required>
            <button type="submit">{}</button>
        </form>
    </div>
</body>
</html>"#,
        escape_html(lang),
        escape_html(dir),
        title_text,
        heading_text,
        desc_text,
        file_label,
        safe_file_name,
        error_html,
        form_action,
        escape_html(lang),
        password_placeholder,
        btn_text
    );

    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(html)
}

#[get("/{token}")]
async fn get_shared_file(
    req: HttpRequest,
    path: web::Path<String>,
    db_conn: web::Data<DbConnection>,
    account_root: web::Data<ShareAccountRoot>,
    tg_state: web::Data<Arc<TelegramState>>,
) -> impl Responder {
    let token = path.into_inner();

    let row = match get_share_by_token(db_conn.get_ref().clone(), token.clone()).await {
        Ok(Some(r)) => r,
        Ok(None) => return HttpResponse::NotFound().body("Shared link not found"),
        Err(e) => {
            log::error!("Database error resolving share link: {}", e);
            return HttpResponse::InternalServerError().body("Internal server error");
        }
    };

    let account = match share_account(&account_root, &row) {
        Ok(account) => account,
        Err(_) => {
            return HttpResponse::NotFound()
                .body("Shared link is unavailable for the current account")
        }
    };

    // Check validation (revocation and expiration)
    if row.revoked {
        return HttpResponse::NotFound().body("This shared link has been revoked");
    }

    if let Some(expiry) = row.expires_at {
        let now = crate::app_time::unix_seconds();
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
            return render_password_form(&req, &row.file_name, &token, None, None);
        }
    }

    // Retrieve and stream the file from Telegram
    let client_opt = { tg_state.client.lock().await.clone() };
    let client = match client_opt {
        Some(c) => c,
        None => return HttpResponse::ServiceUnavailable().body("Telegram client is not connected"),
    };

    if account.validate().is_err()
        || !matches!(client.get_me().await, Ok(user) if user.bare_id() == account.owner)
    {
        return HttpResponse::NotFound().body("Shared link is unavailable for the current account");
    }
    let peer = match resolve_peer(&client, row.folder_id, &tg_state.peer_cache).await {
        Ok(p) => p,
        Err(e) => {
            log::error!("Failed to resolve peer for share: {}", e);
            return HttpResponse::InternalServerError().body("Failed to locate folder");
        }
    };

    if account.validate().is_err() {
        return HttpResponse::NotFound().body("Shared link is unavailable for the current account");
    }
    match client.get_messages_by_id(peer, &[row.message_id]).await {
        Ok(messages) => {
            if let Some(Some(msg)) = messages.first() {
                if let Some(media) = msg.media() {
                    match crate::commands::fs::resolve_remote_envelope(
                        &account,
                        &client,
                        row.folder_id,
                        row.message_id,
                        &media,
                        msg.text(),
                    )
                    .await
                    {
                        Ok(None) => {}
                        Ok(Some(_)) => {
                            return HttpResponse::Forbidden()
                                .body("Encrypted sharing is unavailable")
                        }
                        Err(_) => {
                            return HttpResponse::NotFound()
                                .body("Shared file could not be verified for the current account")
                        }
                    }
                    let mime = match &media {
                        Media::Document(d) => d
                            .mime_type()
                            .unwrap_or("application/octet-stream")
                            .to_string(),
                        _ => "application/octet-stream".to_string(),
                    };
                    let filename = &row.file_name;

                    return crate::server::build_media_response_guarded(
                        &client,
                        &media,
                        &req,
                        &mime,
                        Some(filename),
                        crate::server::StreamingExtras {
                            bandwidth: req
                                .app_data::<web::Data<Arc<crate::bandwidth::BandwidthManager>>>()
                                .expect("Shared bandwidth accounting")
                                .get_ref()
                                .clone(),
                            network: req
                                .app_data::<web::Data<Arc<crate::vpn_optimizer::NetworkConfig>>>()
                                .expect("Shared network state")
                                .get_ref()
                                .clone(),
                            extra_headers: vec![],
                            log_label: "Share download",
                        },
                        Some(account),
                    );
                }
            }
            HttpResponse::NotFound().body("Message or media not found in Telegram")
        }
        Err(e) => {
            log::error!("Failed to fetch shared media: {}", e);
            HttpResponse::InternalServerError().body("Unable to retrieve shared media")
        }
    }
}

#[post("/{token}/verify")]
async fn verify_shared_file_password(
    req: HttpRequest,
    path: web::Path<String>,
    form: web::Form<VerifyForm>,
    db_conn: web::Data<DbConnection>,
    account_root: web::Data<ShareAccountRoot>,
) -> impl Responder {
    let token = path.into_inner();

    let row = match get_share_by_token(db_conn.get_ref().clone(), token.clone()).await {
        Ok(Some(r)) => r,
        Ok(None) => return HttpResponse::NotFound().body("Shared link not found"),
        Err(e) => {
            log::error!("Database error resolving share link: {}", e);
            return HttpResponse::InternalServerError().body("Internal server error");
        }
    };

    let account = match share_account(&account_root, &row) {
        Ok(account) => account,
        Err(_) => {
            return HttpResponse::NotFound()
                .body("Shared link is unavailable for the current account")
        }
    };

    if row.revoked {
        return HttpResponse::NotFound().body("This shared link has been revoked");
    }

    if row
        .expires_at
        .is_some_and(|expiry| expiry < crate::app_time::unix_seconds())
    {
        return HttpResponse::Gone().body("This shared link has expired");
    }

    let hash = match &row.password_hash {
        Some(h) => h,
        None => return HttpResponse::BadRequest().body("No password required for this link"),
    };

    let token_key = token_attempt_key(&token);
    let now = crate::app_time::unix_seconds();
    if let Err(retry_after) =
        with_password_attempt_limiter(|limiter| limiter.begin_attempt(token_key, now))
    {
        return HttpResponse::TooManyRequests()
            .insert_header(("Retry-After", retry_after.to_string()))
            .body("Too many password attempts. Try again shortly.");
    }

    if form.password.chars().count() > MAX_SHARE_PASSWORD_CHARS
        || form.password.len() > MAX_SHARE_PASSWORD_BYTES
    {
        with_password_attempt_limiter(|limiter| limiter.record_failure(&token_key, now));
        return render_password_form(
            &req,
            &row.file_name,
            &token,
            Some("invalid"),
            form.lang.as_deref(),
        );
    }

    let password = form.password.clone();
    let hash_for_verify = hash.clone();
    let verified =
        tokio::task::spawn_blocking(move || verify_password(&password, &hash_for_verify))
            .await
            .unwrap_or(false);

    if account.validate().is_err() {
        return HttpResponse::NotFound().body("Shared link is unavailable for the current account");
    }
    if verified {
        with_password_attempt_limiter(|limiter| limiter.clear(&token_key));
        // Set session cookie (30 min).
        // NOTE: The streaming share server is loopback-only and uses plain HTTP,
        // so the cookie cannot use `.secure(true)` without becoming unusable locally.
        // The cookie is protected by `.http_only(true)` and `.same_site(Strict)`
        // to mitigate XSS and CSRF within the constraints of this local service.
        let val = generate_cookie_val(&token, hash);
        let cookie = Cookie::build(format!("share_auth_{}", token), val)
            .path(format!("/d/{}", token))
            .http_only(true)
            .same_site(actix_web::cookie::SameSite::Strict)
            .max_age(actix_web::cookie::time::Duration::minutes(30))
            .finish();

        let (language, _) = resolve_req_lang(&req, form.lang.as_deref());
        let explicit_language = query_language(&req).is_some()
            || form
                .lang
                .as_deref()
                .and_then(crate::native_localization::canonical_language)
                .is_some();
        let location = if language != "en" || explicit_language {
            format!("/d/{}?lang={language}", urlencoding::encode(&token))
        } else {
            format!("/d/{}", urlencoding::encode(&token))
        };
        HttpResponse::Found()
            .insert_header(("Location", location))
            .cookie(cookie)
            .finish()
    } else {
        with_password_attempt_limiter(|limiter| limiter.record_failure(&token_key, now));
        render_password_form(
            &req,
            &row.file_name,
            &token,
            Some("Incorrect password. Please try again."),
            form.lang.as_deref(),
        )
    }
}

pub fn configure_share_routes(cfg: &mut web::ServiceConfig) {
    let mut headers = DefaultHeaders::new();
    for (name, value) in SHARE_SECURITY_HEADERS {
        headers = headers.add((*name, *value));
    }
    cfg.service(
        web::scope("/d")
            .wrap(headers)
            .app_data(web::FormConfig::default().limit(SHARE_VERIFY_FORM_LIMIT_BYTES))
            .service(get_shared_file)
            .service(verify_shared_file_password),
    );
}
