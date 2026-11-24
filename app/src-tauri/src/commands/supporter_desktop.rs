use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use keyring::Entry;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::OnceLock,
};
use tauri::{AppHandle, Manager};

const KEYRING_SERVICE: &str = "com.cameronamer.telegramdrive.supporter";
const DEVICE_KEY_ACCOUNT: &str = "device-signing-key-v1";
const PENDING_RECEIPT_ACCOUNT: &str = "pending-recovery-receipt-v1";
const RECOVERY_CODE_ACCOUNT: &str = "recovery-code-v1";
const CHECKOUT_SECRET_ACCOUNT: &str = "checkout-claim-secret-v1";
const TERMS_VERSION: &str = "2026-08-11";
static OPERATIONS: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

async fn supporter_operation() -> tokio::sync::MutexGuard<'static, ()> {
    OPERATIONS
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await
}

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|_| "Supporter verification is temporarily unavailable".into())
}

fn service_url() -> Option<&'static str> {
    option_env!("TELEGRAM_DRIVE_SUPPORTER_SERVICE_URL")
        .map(str::trim)
        .filter(|value| value.starts_with("https://") && !value.ends_with('/'))
}

fn configured_public_key() -> Option<&'static str> {
    option_env!("TELEGRAM_DRIVE_SUPPORTER_PUBLIC_KEY")
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct SupporterLocalState {
    device_public_key: Option<String>,
    entitlement_token: Option<String>,
    checkout_claim_id: Option<String>,
    checkout_expires_at: Option<i64>,
    #[serde(default)]
    revoked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EntitlementClaims {
    iss: String,
    aud: String,
    entitlement_id: String,
    device_key_hash: String,
    terms_version: String,
    issued_at: i64,
    expires_at: i64,
    offline_until: i64,
}

#[derive(Debug, Deserialize)]
struct EntitlementHeader {
    alg: String,
    typ: String,
    kid: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntitlementAccess {
    Active,
    OfflineGrace,
    Expired,
}

#[derive(Debug, Serialize)]
pub struct SupporterStatus {
    state: &'static str,
    ad_free: bool,
    message: String,
    terms_version: &'static str,
    terms_url: Option<String>,
    expires_at: Option<i64>,
    offline_until: Option<i64>,
    recovery_code_saved: bool,
    checkout_pending: bool,
}

#[derive(Debug, Deserialize)]
struct CheckoutResponse {
    claim_id: String,
    claim_secret: String,
    approval_url: String,
    expires_at: i64,
}

#[derive(Debug, Serialize)]
pub struct CheckoutStarted {
    approval_url: String,
    expires_at: i64,
}

#[derive(Debug, Deserialize)]
struct CheckoutStatusResponse {
    status: String,
    entitlement_token: Option<String>,
    recovery_code: Option<String>,
    #[serde(default)]
    claim_id: Option<String>,
    #[serde(default)]
    unpaid_final: bool,
    #[serde(default)]
    approval_url: Option<String>,
    #[serde(default)]
    expires_at: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct CheckoutPollResult {
    status: String,
    recovery_code: Option<String>,
    message: String,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    entitlement_token: String,
}

#[derive(Debug, Deserialize)]
struct ChallengeResponse {
    challenge_id: String,
    nonce: String,
}

#[derive(Debug, Deserialize)]
struct ServiceErrorEnvelope {
    error: Option<ServiceError>,
}

#[derive(Debug, Deserialize)]
struct ServiceError {
    code: String,
    message: String,
}

fn state_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|dir| dir.join("supporter-entitlement-v1.json"))
        .map_err(|error| format!("Unable to locate app data: {error}"))
}

fn read_state_file(path: &Path) -> Result<SupporterLocalState, String> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| "Stored supporter activation could not be read. It has been preserved; do not pay again.".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(SupporterLocalState::default()),
        Err(_) => Err("Stored supporter activation is temporarily unavailable. It has been preserved; do not pay again.".into()),
    }
}

fn load_state(app: &AppHandle) -> Result<SupporterLocalState, String> {
    read_state_file(&state_path(app)?)
}

fn save_state(app: &AppHandle, state: &SupporterLocalState) -> Result<(), String> {
    save_state_at(&state_path(app)?, state)
}

fn save_state_at(path: &Path, state: &SupporterLocalState) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Unable to create app data directory: {error}"))?;
    }
    let bytes = serde_json::to_vec_pretty(state)
        .map_err(|error| format!("Unable to encode supporter state: {error}"))?;
    persist_state_file(path, &bytes)
}

fn persist_state_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temporary = path.with_extension("json.tmp");
    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .map_err(|error| format!("Unable to save supporter state: {error}"))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| format!("Unable to save supporter state: {error}"))?;
    drop(file);
    replace_state_file(&temporary, path)
        .map_err(|error| format!("Unable to commit supporter state: {error}"))?;
    #[cfg(unix)]
    if let Some(parent) = path.parent() {
        std::fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| format!("Unable to durably commit supporter state: {error}"))?;
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn replace_state_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::fs::rename(source, destination)
}

#[cfg(target_os = "windows")]
fn replace_state_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    let source = windows_extended_path(source);
    let destination = windows_extended_path(destination);
    let result = unsafe {
        windows_sys::Win32::Storage::FileSystem::MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            windows_sys::Win32::Storage::FileSystem::MOVEFILE_REPLACE_EXISTING
                | windows_sys::Win32::Storage::FileSystem::MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(target_os = "windows")]
fn windows_extended_path(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    let path: Vec<u16> = path.as_os_str().encode_wide().collect();
    const SLASH: u16 = b'\\' as u16;
    const QUESTION: u16 = b'?' as u16;
    let mut extended = if path.starts_with(&[SLASH, SLASH, QUESTION, SLASH]) {
        path
    } else if path.starts_with(&[SLASH, SLASH]) {
        "\\\\?\\UNC\\"
            .encode_utf16()
            .chain(path.into_iter().skip(2))
            .collect()
    } else {
        "\\\\?\\".encode_utf16().chain(path).collect()
    };
    extended.push(0);
    extended
}

fn keyring_entry(account: &str) -> Result<Entry, String> {
    Entry::new(KEYRING_SERVICE, account)
        .map_err(|error| format!("Secure credential storage is unavailable: {error}"))
}

fn load_signing_key() -> Result<Option<SigningKey>, String> {
    match keyring_entry(DEVICE_KEY_ACCOUNT)?.get_password() {
        Ok(encoded) => {
            let bytes = URL_SAFE_NO_PAD
                .decode(encoded)
                .map_err(|_| "Stored supporter device key is invalid".to_string())?;
            let key_bytes: [u8; 32] = bytes
                .try_into()
                .map_err(|_| "Stored supporter device key has the wrong length".to_string())?;
            Ok(Some(SigningKey::from_bytes(&key_bytes)))
        }
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(format!(
            "Unable to read secure supporter credential: {error}"
        )),
    }
}

fn signing_key() -> Result<SigningKey, String> {
    if let Some(key) = load_signing_key()? {
        return Ok(key);
    }
    let mut bytes = [0_u8; 32];
    getrandom::getrandom(&mut bytes)
        .map_err(|error| format!("Unable to create a secure device key: {error}"))?;
    let key = SigningKey::from_bytes(&bytes);
    keyring_entry(DEVICE_KEY_ACCOUNT)?
        .set_password(&URL_SAFE_NO_PAD.encode(key.to_bytes()))
        .map_err(|error| {
            format!("Unable to save the device key in secure credential storage: {error}")
        })?;
    Ok(key)
}

fn read_secure_code(account: &str) -> Result<Option<String>, String> {
    match keyring_entry(account)?.get_password() {
        Ok(code) => Ok(Some(code)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(_) => Err("Secure supporter recovery storage is temporarily unavailable. Do not start another payment.".into()),
    }
}

fn write_secure_code(account: &str, code: &str) -> Result<(), String> {
    keyring_entry(account)?
        .set_password(code)
        .map_err(|error| format!("Unable to save the recovery code securely: {error}"))
}

fn save_checkout_secret(secret: &str) -> Result<(), String> {
    keyring_entry(CHECKOUT_SECRET_ACCOUNT)?
        .set_password(secret)
        .map_err(|error| format!("Unable to save the checkout verification credential: {error}"))
}

fn load_checkout_secret() -> Result<String, String> {
    keyring_entry(CHECKOUT_SECRET_ACCOUNT)?
        .get_password()
        .map_err(|error| {
            format!("The secure checkout verification credential is unavailable: {error}")
        })
}

fn clear_secure_credential(account: &str) -> Result<(), String> {
    match keyring_entry(account)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(format!(
            "Unable to clear the secure supporter credential: {error}"
        )),
    }
}

fn recovery_code_present() -> Result<bool, String> {
    Ok(read_secure_code(RECOVERY_CODE_ACCOUNT)?.is_some())
}

fn save_recovery_code(code: &str) -> Result<(), String> {
    write_secure_code(RECOVERY_CODE_ACCOUNT, code)
}

fn save_checkout_receipt(
    code: &str,
    write: impl Fn(&str, &str) -> Result<(), String>,
) -> Result<bool, String> {
    write(RECOVERY_CODE_ACCOUNT, code)?;
    // A locked reminder slot cannot withhold a verified license. Retain the
    // existing claim for delivery retry until both credentials are durable.
    Ok(write(PENDING_RECEIPT_ACCOUNT, code).is_ok())
}

#[derive(Serialize)]
pub struct PendingSupporterReceipt {
    id: String,
    code: String,
}

fn pending_receipt(
    read: impl FnOnce(&str) -> Result<Option<String>, String>,
) -> Result<Option<PendingSupporterReceipt>, String> {
    Ok(
        read(PENDING_RECEIPT_ACCOUNT)?.map(|code| PendingSupporterReceipt {
            id: sha256_base64url(&code),
            code,
        }),
    )
}

fn acknowledge_receipt(
    state: &SupporterLocalState,
    receipt_id: &str,
    read: impl FnOnce(&str) -> Result<Option<String>, String>,
    clear: impl FnOnce(&str) -> Result<(), String>,
) -> Result<(), String> {
    let Some(pending) = pending_receipt(read)? else {
        return Ok(());
    };
    // The code is already safe to *read* after its secure write. Wait for
    // the entitlement commit before clearing it: redelivery of the same
    // unfinished claim would otherwise create a new reminder after confirmation.
    if state.entitlement_token.is_none() || state.checkout_claim_id.is_some() {
        return Err("The verified checkout is still being saved. Retry payment verification before confirming the receipt.".into());
    }
    if pending.id != receipt_id {
        return Err(
            "The pending receipt changed. Reopen the license settings before confirming.".into(),
        );
    }
    clear(PENDING_RECEIPT_ACCOUNT)
}

#[tauri::command]
pub async fn cmd_pending_supporter_receipt() -> Result<Option<PendingSupporterReceipt>, String> {
    let _operation = supporter_operation().await;
    pending_receipt(read_secure_code)
}

#[tauri::command]
pub async fn cmd_acknowledge_supporter_receipt(
    app: AppHandle,
    receipt_id: String,
) -> Result<(), String> {
    let _operation = supporter_operation().await;
    acknowledge_receipt(
        &load_state(&app)?,
        &receipt_id,
        read_secure_code,
        clear_secure_credential,
    )
}

fn public_key(key: &SigningKey) -> String {
    URL_SAFE_NO_PAD.encode(key.verifying_key().to_bytes())
}

fn sha256_base64url(value: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(value.as_bytes()))
}

fn parse_and_verify_token(
    token: &str,
    expected_device_public_key: &str,
) -> Result<EntitlementClaims, String> {
    let configured_key =
        configured_public_key().ok_or("Supporter verification is not configured in this build")?;
    parse_and_verify_token_with_key(token, expected_device_public_key, configured_key)
}

fn parse_and_verify_token_with_key(
    token: &str,
    expected_device_public_key: &str,
    configured_key: &str,
) -> Result<EntitlementClaims, String> {
    let mut parts = token.split('.');
    let header = parts.next().ok_or("Supporter token is malformed")?;
    let payload = parts.next().ok_or("Supporter token is malformed")?;
    let encoded_signature = parts.next().ok_or("Supporter token is malformed")?;
    if parts.next().is_some() {
        return Err("Supporter token is malformed".to_string());
    }

    let key_bytes: [u8; 32] = URL_SAFE_NO_PAD
        .decode(configured_key)
        .map_err(|_| "Configured supporter public key is invalid".to_string())?
        .try_into()
        .map_err(|_| "Configured supporter public key has the wrong length".to_string())?;
    let signature_bytes: [u8; 64] = URL_SAFE_NO_PAD
        .decode(encoded_signature)
        .map_err(|_| "Supporter token signature is invalid".to_string())?
        .try_into()
        .map_err(|_| "Supporter token signature has the wrong length".to_string())?;
    VerifyingKey::from_bytes(&key_bytes)
        .map_err(|_| "Configured supporter public key is invalid".to_string())?
        .verify(
            format!("{header}.{payload}").as_bytes(),
            &Signature::from_bytes(&signature_bytes),
        )
        .map_err(|_| "Supporter token signature could not be verified".to_string())?;

    let entitlement_header: EntitlementHeader = serde_json::from_slice(
        &URL_SAFE_NO_PAD
            .decode(header)
            .map_err(|_| "Supporter token header is invalid".to_string())?,
    )
    .map_err(|_| "Supporter token header is invalid".to_string())?;
    if entitlement_header.alg != "EdDSA"
        || entitlement_header.typ != "TD-SUPPORTER"
        || entitlement_header.kid != "v1"
    {
        return Err("Supporter token header is not supported".to_string());
    }

    let claims: EntitlementClaims = serde_json::from_slice(
        &URL_SAFE_NO_PAD
            .decode(payload)
            .map_err(|_| "Supporter token payload is invalid".to_string())?,
    )
    .map_err(|_| "Supporter token claims are invalid".to_string())?;
    if claims.iss != "telegram-drive-supporter" || claims.aud != "telegram-drive-desktop" {
        return Err("Supporter token was issued for a different application".to_string());
    }
    if claims.device_key_hash != sha256_base64url(expected_device_public_key) {
        return Err("Supporter token belongs to a different device".to_string());
    }
    if claims.entitlement_id.is_empty()
        || claims.terms_version.is_empty()
        || claims.issued_at < 0
        || claims.issued_at > claims.expires_at
        || claims.expires_at > claims.offline_until
    {
        return Err("Supporter token validity claims are invalid".to_string());
    }
    Ok(claims)
}

/// Native journeys check a token issued by the locally run supporter Worker
/// against that Worker's own test key. The key compiled into release builds
/// is not involved and nothing here changes how it is used; the verification
/// and validity-window code is the code every build runs.
#[cfg(feature = "native-e2e")]
pub(crate) fn verify_issued_token(
    token: &str,
    device_public_key: &str,
    service_public_key: &str,
    now: i64,
) -> Result<serde_json::Value, String> {
    let claims = parse_and_verify_token_with_key(token, device_public_key, service_public_key)?;
    let access = match entitlement_access_at(&claims, now) {
        EntitlementAccess::Active => "active",
        EntitlementAccess::OfflineGrace => "offline_grace",
        EntitlementAccess::Expired => "expired",
    };
    Ok(serde_json::json!({
        "access": access,
        "entitlementId": claims.entitlement_id,
        "termsVersion": claims.terms_version,
        "appTermsVersion": TERMS_VERSION,
        "expiresAt": claims.expires_at,
        "offlineUntil": claims.offline_until,
    }))
}

fn entitlement_access_at(claims: &EntitlementClaims, now: i64) -> EntitlementAccess {
    if now <= claims.expires_at {
        EntitlementAccess::Active
    } else if now <= claims.offline_until {
        EntitlementAccess::OfflineGrace
    } else {
        EntitlementAccess::Expired
    }
}

fn unix_time() -> i64 {
    crate::app_time::unix_seconds()
}

fn checkout_is_pending(state: &SupporterLocalState) -> bool {
    state.checkout_claim_id.is_some()
}

fn ensure_new_checkout_allowed(
    state: &SupporterLocalState,
    recovery_saved: bool,
) -> Result<(), String> {
    if state.entitlement_token.is_some()
        || state.revoked
        || checkout_is_pending(state)
        || recovery_saved
    {
        return Err("An existing purchase or payment verification is stored on this device. Refresh or restore it; do not pay again.".into());
    }
    Ok(())
}

fn valid_approval_url(value: &str) -> bool {
    reqwest::Url::parse(value).is_ok_and(|url| {
        url.scheme() == "https"
            && url.username().is_empty()
            && url.password().is_none()
            && url
                .host_str()
                .is_some_and(|host| host == "paypal.com" || host.ends_with(".paypal.com"))
    })
}

fn checkout_url(base: &str, claim_id: &str, action: &str) -> Result<reqwest::Url, String> {
    let mut url = reqwest::Url::parse(base).map_err(|_| "Invalid supporter service URL")?;
    url.path_segments_mut()
        .map_err(|_| "Invalid supporter service URL")?
        .pop_if_empty()
        .extend(["v1", "checkout", claim_id, action]);
    Ok(url)
}

async fn fetch_checkout(
    client: &reqwest::Client,
    base: &str,
    claim_id: &str,
    secret: &str,
) -> Result<CheckoutStatusResponse, String> {
    let response = client.get(checkout_url(base, claim_id, "status")?).bearer_auth(secret).send().await
        .map_err(|_| "Payment verification is temporarily unavailable. The existing payment has been kept; do not pay again.")?;
    if !response.status().is_success() {
        return Err(response_error(response).await);
    }
    response.json().await.map_err(|_| "The payment response could not be verified. The existing payment has been kept; do not pay again.".into())
}

/// The secret is only disposable after both durable stores have succeeded.
/// Missing old delivery material may restore a valid license, but retains its
/// only remaining claim instead of pretending its recovery code was saved.
fn commit_completed_checkout(
    previous: &SupporterLocalState,
    checkout: &CheckoutStatusResponse,
    verify: impl Fn(&str, &str) -> Result<String, String>,
    save_receipt: impl FnOnce(&str) -> Result<bool, String>,
    persist: impl FnOnce(&SupporterLocalState) -> Result<(), String>,
) -> Result<(SupporterLocalState, bool), String> {
    if checkout.status != "completed" {
        return Err("Payment has not completed".into());
    }
    let token = checkout
        .entitlement_token
        .as_deref()
        .ok_or("Completed checkout did not include an entitlement")?;
    let device_key = previous
        .device_public_key
        .as_deref()
        .ok_or("The local device identity is missing")?;
    let completed_entitlement = verify(token, device_key)?;
    if let Some(existing) = previous.entitlement_token.as_deref() {
        if verify(existing, device_key)? != completed_entitlement {
            return Err("The pending payment belongs to a different purchase. Your restored license and the pending payment have both been preserved; do not pay again.".into());
        }
    }
    let receipt_saved = if let Some(code) = checkout.recovery_code.as_deref() {
        if code.trim().is_empty() {
            return Err("Completed checkout returned an empty recovery code".into());
        }
        save_receipt(code)?
    } else {
        // A saved code can belong to an earlier recovery attempt whose local
        // state commit failed. Presence alone never proves this receipt is saved.
        false
    };
    let mut next = previous.clone();
    next.entitlement_token = Some(token.to_owned());
    next.revoked = false;
    if receipt_saved {
        next.checkout_claim_id = None;
        next.checkout_expires_at = None;
    }
    persist(&next)?;
    Ok((next, receipt_saved))
}

fn confirmed_unpaid(state: &SupporterLocalState, response: &CheckoutStatusResponse) -> bool {
    response.unpaid_final
        && matches!(response.status.as_str(), "expired" | "cancelled" | "failed")
        && state.checkout_claim_id.is_some()
        && response.claim_id == state.checkout_claim_id
}

fn clear_pending_checkout(app: &AppHandle, state: &mut SupporterLocalState) -> Result<(), String> {
    state.checkout_claim_id = None;
    state.checkout_expires_at = None;
    save_state(app, state)?;
    let _ = clear_secure_credential(CHECKOUT_SECRET_ACCOUNT);
    Ok(())
}

async fn response_error(response: reqwest::Response) -> String {
    let status = response.status();
    match response.json::<ServiceErrorEnvelope>().await {
        Ok(body) => body
            .error
            .map(|error| format!("{}: {}", error.code, error.message))
            .unwrap_or_else(|| format!("Supporter service returned {status}")),
        Err(_) => format!("Supporter service returned {status}"),
    }
}

fn explicit_revocation(status: StatusCode, body: Option<&ServiceErrorEnvelope>) -> bool {
    status == StatusCode::FORBIDDEN
        && body
            .and_then(|body| body.error.as_ref())
            .is_some_and(|error| {
                matches!(
                    error.code.as_str(),
                    "ENTITLEMENT_NOT_ACTIVE" | "DEVICE_NOT_ACTIVE"
                )
            })
}

async fn refresh_error(
    app: &AppHandle,
    state: &mut SupporterLocalState,
    response: reqwest::Response,
) -> String {
    let status = response.status();
    let body = response.json::<ServiceErrorEnvelope>().await.ok();
    if explicit_revocation(status, body.as_ref()) {
        // Keep the signed purchase as recovery evidence. Only the service's
        // explicit entitlement/device decision can revoke cached access.
        state.revoked = true;
        if let Err(error) = save_state(app, state) {
            return error;
        }
        SPONSOR_OFFLINE_UNTIL.store(0, std::sync::atomic::Ordering::Release);
    }
    body.and_then(|body| body.error)
        .map(|error| format!("{}: {}", error.code, error.message))
        .unwrap_or_else(|| format!("Supporter service returned {status}"))
}

fn recovery_presence_for_status(
    state: &SupporterLocalState,
    presence: Result<bool, String>,
) -> Result<bool, String> {
    match presence {
        Ok(saved) => Ok(saved),
        Err(_) if state.entitlement_token.is_some() || state.revoked => Ok(false),
        Err(error) => Err(error),
    }
}

// Only signature-verified, device-bound local access may populate this cache.
// HTTP routes read RAM only; they never initiate verification or show an ad.
static SPONSOR_OFFLINE_UNTIL: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

pub(crate) fn sponsor_requests_suppressed() -> bool {
    let until = SPONSOR_OFFLINE_UNTIL.load(std::sync::atomic::Ordering::Acquire);
    until > 0 && unix_time() <= until
}

fn cache_verified_access(claims: &EntitlementClaims, now: i64) {
    SPONSOR_OFFLINE_UNTIL.store(
        if entitlement_access_at(claims, now) == EntitlementAccess::Expired {
            0
        } else {
            claims.offline_until
        },
        std::sync::atomic::Ordering::Release,
    );
}

#[cfg(feature = "native-e2e")]
pub(crate) async fn load_sponsor_fixture(
    token: &str,
    device: &str,
    key: &str,
    now: i64,
) -> Result<(), String> {
    let _operation = supporter_operation().await;
    let claims = parse_and_verify_token_with_key(token, device, key)?;
    cache_verified_access(&claims, now);
    Ok(())
}

fn status_from_state(state: &SupporterLocalState) -> Result<SupporterStatus, String> {
    // A recovery-secret read is not needed to verify an existing signed token.
    // Still fail closed on empty state so a locked keychain cannot enable a
    // second checkout for a purchaser whose recovery code is stored there.
    let recovery_code_saved = recovery_presence_for_status(state, recovery_code_present())?;
    let terms_url = service_url().map(|url| format!("{url}/terms"));
    let unavailable = |message: String| {
        Ok(SupporterStatus {
            state: "unavailable",
            ad_free: false,
            message,
            terms_version: TERMS_VERSION,
            terms_url: terms_url.clone(),
            expires_at: None,
            offline_until: None,
            recovery_code_saved,
            checkout_pending: checkout_is_pending(state),
        })
    };
    if service_url().is_none() || configured_public_key().is_none() {
        return unavailable(
            "Verified supporter activation is not configured in this build.".to_string(),
        );
    }
    if state.revoked {
        SPONSOR_OFFLINE_UNTIL.store(0, std::sync::atomic::Ordering::Release);
        return Ok(SupporterStatus {
            state: "revoked",
            ad_free: false,
            message: "This supporter entitlement was revoked after a refund, reversal, dispute, or device deactivation.".to_string(),
            terms_version: TERMS_VERSION,
            terms_url,
            expires_at: None,
            offline_until: None,
            recovery_code_saved,
            checkout_pending: checkout_is_pending(state),
        });
    }
    let Some(token) = state.entitlement_token.as_deref() else {
        return Ok(SupporterStatus {
            state: "inactive",
            ad_free: false,
            message: "No verified supporter activation is stored on this device.".to_string(),
            terms_version: TERMS_VERSION,
            terms_url,
            expires_at: None,
            offline_until: None,
            recovery_code_saved,
            checkout_pending: checkout_is_pending(state),
        });
    };
    let Some(device_public_key) = state.device_public_key.as_deref() else {
        return unavailable("The local supporter device identity is missing.".to_string());
    };
    match parse_and_verify_token(token, device_public_key) {
        Ok(claims) => {
            let now = unix_time();
            cache_verified_access(&claims, now);
            let (status, ad_free, message) = match entitlement_access_at(&claims, now) {
                EntitlementAccess::Active => (
                    "active",
                    true,
                    "Verified ad-free supporter access is active.".to_string(),
                ),
                EntitlementAccess::OfflineGrace => ("needs_refresh", true, "Ad-free access is active during the offline grace period. Connect to refresh verification.".to_string()),
                EntitlementAccess::Expired => (
                    "expired",
                    false,
                    "Supporter verification expired. Connect to refresh or use your recovery code."
                        .to_string(),
                ),
            };
            Ok(SupporterStatus {
                state: status,
                ad_free,
                message,
                terms_version: TERMS_VERSION,
                terms_url,
                expires_at: Some(claims.expires_at),
                offline_until: Some(claims.offline_until),
                recovery_code_saved,
                checkout_pending: checkout_is_pending(state),
            })
        }
        Err(error) => unavailable(error),
    }
}

#[tauri::command]
pub async fn cmd_get_supporter_status(app: AppHandle) -> Result<SupporterStatus, String> {
    let _operation = supporter_operation().await;
    let state = load_state(&app)?;
    status_from_state(&state)
}

#[cfg(feature = "native-e2e")]
pub(crate) async fn fixture_local_receipt(
    root: &Path,
    request: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    // Exercise real on-disk activation, the status HTTP request and receipt
    // handling. Only the native credential provider uses disposable files.
    let path = root.join("supporter-entitlement-v1.json");
    let state = read_state_file(&path)?;
    let credential = |account: &str| match account {
        PENDING_RECEIPT_ACCOUNT => Ok(root.join("fixture-pending-receipt")),
        RECOVERY_CODE_ACCOUNT => Ok(root.join("fixture-recovery-code")),
        _ => Err("Unexpected fixture credential".to_string()),
    };
    let read = |account: &str| match std::fs::read_to_string(credential(account)?) {
        Ok(code) => Ok(Some(code)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    };
    match request["action"].as_str() {
        Some("poll") => {
            let port = request["port"]
                .as_u64()
                .and_then(|n| u16::try_from(n).ok())
                .ok_or("Invalid fixture port")?;
            let claim = state
                .checkout_claim_id
                .as_deref()
                .ok_or("Missing fixture checkout")?;
            let checkout = fetch_checkout(
                &http_client()?,
                &format!("http://127.0.0.1:{port}"),
                claim,
                "fixture-secret",
            )
            .await?;
            let key = std::fs::read_to_string(root.join("fixture-verifier.pub"))
                .map_err(|e| e.to_string())?;
            let (_, delivered) = commit_completed_checkout(
                &state,
                &checkout,
                |token, device| {
                    parse_and_verify_token_with_key(token, device, &key)
                        .map(|claims| claims.entitlement_id)
                },
                |code| {
                    save_checkout_receipt(code, |account, value| {
                        std::fs::write(credential(account)?, value).map_err(|e| e.to_string())
                    })
                },
                |next| save_state_at(&path, next),
            )?;
            Ok(serde_json::json!(delivered))
        }
        Some("read") => serde_json::to_value(pending_receipt(read)?).map_err(|e| e.to_string()),
        Some("ack") => {
            acknowledge_receipt(
                &state,
                request["id"].as_str().ok_or("Missing receipt ID")?,
                read,
                |account| std::fs::remove_file(credential(account)?).map_err(|e| e.to_string()),
            )?;
            Ok(serde_json::Value::Null)
        }
        _ => Err("Unknown fixture receipt operation".into()),
    }
}

#[tauri::command]
pub async fn cmd_begin_supporter_checkout(
    app: AppHandle,
    accepted_terms_version: String,
) -> Result<CheckoutStarted, String> {
    let _operation = supporter_operation().await;
    let base_url = service_url().ok_or("Supporter activation is not configured in this build")?;
    if accepted_terms_version != TERMS_VERSION {
        return Err("Accept the current supporter terms before continuing".to_string());
    }
    let mut state = load_state(&app)?;
    let client = http_client()?;
    if let Some(claim_id) = state.checkout_claim_id.as_deref() {
        let checkout =
            fetch_checkout(&client, base_url, claim_id, &load_checkout_secret()?).await?;
        let approval_url = checkout.approval_url.filter(|url| valid_approval_url(url))
            .ok_or("The existing payment is still being checked. Check payment status; do not pay again.")?;
        return Ok(CheckoutStarted {
            approval_url,
            expires_at: checkout
                .expires_at
                .or(state.checkout_expires_at)
                .unwrap_or(0),
        });
    }
    ensure_new_checkout_allowed(&state, recovery_code_present()?)?;
    let key = signing_key()?;
    let device_public_key = public_key(&key);
    let response = client
        .post(format!("{base_url}/v1/checkout"))
        .json(&serde_json::json!({
            "device_public_key": device_public_key,
            "terms_version": TERMS_VERSION,
            "terms_accepted": true,
            "app_version": env!("CARGO_PKG_VERSION"),
            "platform": std::env::consts::OS,
        }))
        .send()
        .await
        .map_err(|error| format!("Unable to reach the supporter service: {error}"))?;
    if response.status() != StatusCode::CREATED {
        return Err(response_error(response).await);
    }
    let checkout = response
        .json::<CheckoutResponse>()
        .await
        .map_err(|error| format!("Supporter service returned an invalid checkout: {error}"))?;
    save_checkout_secret(&checkout.claim_secret)?;
    if !valid_approval_url(&checkout.approval_url) {
        return Err("Supporter service returned an invalid PayPal approval link".into());
    }
    state.device_public_key = Some(public_key(&key));
    state.checkout_claim_id = Some(checkout.claim_id);
    state.checkout_expires_at = Some(checkout.expires_at);
    save_state(&app, &state)?;
    Ok(CheckoutStarted {
        approval_url: checkout.approval_url,
        expires_at: checkout.expires_at,
    })
}

#[tauri::command]
pub async fn cmd_poll_supporter_checkout(app: AppHandle) -> Result<CheckoutPollResult, String> {
    let _operation = supporter_operation().await;
    let base_url = service_url().ok_or("Supporter activation is not configured in this build")?;
    let mut state = load_state(&app)?;
    let claim_id = state
        .checkout_claim_id
        .clone()
        .ok_or("No supporter checkout is waiting for verification")?;
    let claim_secret = load_checkout_secret()?;
    let client = http_client()?;
    let checkout = fetch_checkout(&client, base_url, &claim_id, &claim_secret).await?;
    if checkout.status == "completed" {
        let (_, receipt_saved) = commit_completed_checkout(
            &state,
            &checkout,
            |token, device| {
                parse_and_verify_token(token, device).map(|claims| claims.entitlement_id)
            },
            |code| save_checkout_receipt(code, write_secure_code),
            |next| save_state(&app, next),
        )?;
        if receipt_saved {
            // New acknowledgement is optional for older Worker compatibility.
            // Failure cannot roll back an already durable activation.
            let _ = client
                .post(checkout_url(base_url, &claim_id, "acknowledge")?)
                .timeout(std::time::Duration::from_secs(3))
                .bearer_auth(&claim_secret)
                .send()
                .await;
            let _ = clear_secure_credential(CHECKOUT_SECRET_ACCOUNT);
        }
        // Verification is local and uses the just-persisted activation. Failure
        // cannot undo a completed purchase or require a second payment.
        if let Ok(saved) = load_state(&app) {
            let _ = status_from_state(&saved);
        }
        return Ok(CheckoutPollResult {
            status: "completed".into(), recovery_code: checkout.recovery_code,
            message: if receipt_saved { "Payment verified. Ad-free supporter access is active." }
                else { "Ad-free access is active. Recovery information is still pending; keep this activation and do not pay again." }.into(),
        });
    }
    if confirmed_unpaid(&state, &checkout) {
        clear_pending_checkout(&app, &mut state)?;
        return Ok(CheckoutPollResult { status: "expired".into(), recovery_code: None,
            message: "PayPal confirmed that this order is closed without a payment. No supporter purchase was made.".into() });
    }
    Ok(CheckoutPollResult { status: "pending".into(), recovery_code: None,
        message: "The existing payment is still being verified. Retry verification or continue the same checkout; do not pay again.".into() })
}

#[tauri::command]
pub async fn cmd_activate_supporter(
    app: AppHandle,
    recovery_code: String,
    accepted_terms_version: String,
) -> Result<SupporterStatus, String> {
    let _operation = supporter_operation().await;
    let base_url = service_url().ok_or("Supporter activation is not configured in this build")?;
    if accepted_terms_version != TERMS_VERSION {
        return Err("Accept the current supporter terms before continuing".to_string());
    }
    let mut state = load_state(&app)?;
    let key = signing_key()?;
    let device_public_key = public_key(&key);
    let response = http_client()?
        .post(format!("{base_url}/v1/activate"))
        .json(&serde_json::json!({
            "recovery_code": recovery_code,
            "device_public_key": device_public_key,
            "terms_version": TERMS_VERSION,
            "terms_accepted": true,
        }))
        .send()
        .await
        .map_err(|error| format!("Unable to reach the supporter service: {error}"))?;
    if !response.status().is_success() {
        return Err(response_error(response).await);
    }
    let token = response
        .json::<TokenResponse>()
        .await
        .map_err(|error| format!("Supporter service returned an invalid activation: {error}"))?
        .entitlement_token;
    parse_and_verify_token(&token, &device_public_key)?;
    save_recovery_code(&recovery_code)?;
    state.device_public_key = Some(device_public_key);
    state.entitlement_token = Some(token);
    // Recovery remains available while an older payment is unresolved. Keep
    // that claim and secret until its own verified outcome is safely saved.
    state.revoked = false;
    save_state(&app, &state)?;
    if !checkout_is_pending(&state) {
        let _ = clear_secure_credential(CHECKOUT_SECRET_ACCOUNT);
    }
    status_from_state(&state)
}

#[tauri::command]
pub async fn cmd_refresh_supporter(app: AppHandle) -> Result<SupporterStatus, String> {
    let _operation = supporter_operation().await;
    let base_url = service_url().ok_or("Supporter activation is not configured in this build")?;
    let mut state = load_state(&app)?;
    let token = state
        .entitlement_token
        .clone()
        .ok_or("No supporter activation is stored on this device")?;
    let key = load_signing_key()?
        .ok_or("The secure supporter device key is missing; use your recovery code")?;
    let device_public_key = public_key(&key);
    let claims = parse_and_verify_token(&token, &device_public_key)?;
    let client = http_client()?;
    let challenge_response = client
        .post(format!("{base_url}/v1/challenge"))
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|error| format!("Unable to request supporter verification: {error}"))?;
    if !challenge_response.status().is_success() {
        return Err(refresh_error(&app, &mut state, challenge_response).await);
    }
    let challenge = challenge_response
        .json::<ChallengeResponse>()
        .await
        .map_err(|error| format!("Supporter service returned an invalid challenge: {error}"))?;
    let proof = format!(
        "telegram-drive-supporter-refresh:{}:{}",
        challenge.challenge_id, challenge.nonce
    );
    let signature = URL_SAFE_NO_PAD.encode(key.sign(proof.as_bytes()).to_bytes());
    let response = client
        .post(format!("{base_url}/v1/refresh"))
        .json(&serde_json::json!({
            "entitlement_token": token,
            "challenge_id": challenge.challenge_id,
            "nonce": challenge.nonce,
            "signature": signature,
        }))
        .send()
        .await
        .map_err(|error| format!("Unable to refresh supporter verification: {error}"))?;
    if !response.status().is_success() {
        return Err(refresh_error(&app, &mut state, response).await);
    }
    let refreshed_token = response
        .json::<TokenResponse>()
        .await
        .map_err(|error| format!("Supporter service returned an invalid entitlement: {error}"))?
        .entitlement_token;
    let refreshed_claims = parse_and_verify_token(&refreshed_token, &device_public_key)?;
    if refreshed_claims.entitlement_id != claims.entitlement_id {
        return Err(
            "Refreshed supporter entitlement did not match the stored purchase".to_string(),
        );
    }
    state.entitlement_token = Some(refreshed_token);
    save_state(&app, &state)?;
    status_from_state(&state)
}
