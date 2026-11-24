use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri_plugin_store::StoreExt;

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CrashReportInput {
    event: String,
    app_version: String,
    source: String,
    error_type: String,
    frames: Vec<String>,
    platform: String,
    occurred_at: String,
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(address) => {
            !address.is_private()
                && !address.is_loopback()
                && !address.is_link_local()
                && !address.is_broadcast()
                && !address.is_unspecified()
                && !address.is_multicast()
        }
        IpAddr::V6(address) => {
            !address.is_loopback()
                && !address.is_unspecified()
                && !address.is_multicast()
                && !address.is_unique_local()
                && !address.is_unicast_link_local()
        }
    }
}

fn validate_report(report: &CrashReportInput) -> Result<(), String> {
    if report.event != "app_crash"
        || report.app_version.len() > 32
        || report.source.len() > 16
        || report.error_type.len() > 96
        || report.platform.len() > 96
        || report.occurred_at.len() > 64
        || report.frames.len() > 6
        || report.frames.iter().any(|frame| frame.len() > 160)
    {
        return Err("Crash report did not match the privacy-safe schema".to_string());
    }
    Ok(())
}

/// Sends the narrow, consent-gated crash envelope with the native HTTP client so
/// the WebView CSP never needs a broad external `connect-src` exception.
#[tauri::command]
pub async fn cmd_submit_crash_report(
    app: tauri::AppHandle,
    report: CrashReportInput,
    consent_id: Option<String>,
) -> Result<(), String> {
    submit_crash_report(
        &app,
        "settings.json".as_ref(),
        report,
        consent_id.as_deref(),
    )
    .await
}

pub(crate) async fn submit_crash_report(
    app: &tauri::AppHandle,
    settings_path: &std::path::Path,
    report: CrashReportInput,
    consent_id: Option<&str>,
) -> Result<(), String> {
    let require_consent = || {
        let settings = app
            .get_store(settings_path)
            .and_then(|store| store.get("settings"));
        let allowed = settings.is_some_and(|value| {
            value["crashReportingEnabled"] == true
                && value["crashReportingConsentSeen"] == true
                && value["crashReportingConsentId"] == serde_json::Value::from(consent_id)
        });
        allowed
            .then_some(())
            .ok_or_else(|| "Crash reporting consent is inactive".to_string())
    };
    require_consent()?;
    validate_report(&report)?;
    let endpoint = option_env!("TELEGRAM_DRIVE_CRASH_REPORT_ENDPOINT")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Crash reporting is not configured in this build".to_string())?;
    if endpoint.len() > 2_048 {
        return Err("Crash report endpoint is too long".to_string());
    }
    let url =
        reqwest::Url::parse(endpoint).map_err(|_| "Invalid crash report endpoint".to_string())?;
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return Err(
            "Crash reports require an HTTPS endpoint without embedded credentials".to_string(),
        );
    }
    let host = url
        .host_str()
        .ok_or_else(|| "Crash report endpoint has no host".to_string())?;
    let port = url.port_or_known_default().unwrap_or(443);
    let addresses: Vec<SocketAddr> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|error| error.to_string())?
        .filter(|address| is_public_ip(address.ip()))
        .collect();
    if addresses.is_empty() {
        return Err("Crash report endpoint did not resolve to a public address".to_string());
    }

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .resolve_to_addrs(host, &addresses)
        .build()
        .map_err(|error| error.to_string())?;
    let body = serde_json::to_vec(&report).map_err(|error| error.to_string())?;
    require_consent()?;
    let response = client
        .post(url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "Crash endpoint returned HTTP {}",
            response.status()
        ));
    }
    Ok(())
}
