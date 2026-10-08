//! Antigravity uses an explicitly selected access mode: Google's Cloud
//! Code API, or listeners owned by its locally discovered language-server
//! process. Local failure never switches to cloud credentials automatically.

use std::time::Duration;

use serde_json::{json, Value};

use super::{Metric, Snapshot};

const ID: &str = "antigravity";
const NAME: &str = "Antigravity";

const LS_SERVICE: &str = "exa.language_server_pb.LanguageServerService";
const CLOUD_BASES: [&str; 2] = [
    "https://daily-cloudcode-pa.googleapis.com",
    "https://cloudcode-pa.googleapis.com",
];
// Installed-app OAuth client — intentionally public, same values the IDE ships.
const GOOGLE_CLIENT_ID: &str =
    "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com";
const GOOGLE_CLIENT_SECRET: &str = "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";

pub async fn snapshot() -> Snapshot {
    let selection = match crate::network_policy::current_selection(ID) {
        Ok(selection) => selection,
        Err(error) => return Snapshot::error(ID, NAME, error),
    };
    if selection == "local_process" {
        let permit = crate::access_policy::current_operation();
        let servers = tauri::async_runtime::spawn_blocking(move || discover_language_servers(permit))
            .await.unwrap_or_default();
        for server in &servers {
            if let Some(snap) = try_language_server(server).await { return snap; }
        }
        return Snapshot::error(ID, NAME, "Selected local Antigravity process is unavailable. Start Antigravity or explicitly select Cloud mode in Settings.".into());
    }

    match try_cloud().await {
        CloudResult::Ok(snap) => snap,
        CloudResult::RefreshError(error) => Snapshot::error(ID, NAME, error),
        CloudResult::AuthExpired => Snapshot::error(
            ID,
            NAME,
            "Antigravity sign-in expired. Open Antigravity to refresh it.".into(),
        ),
        CloudResult::Unavailable => Snapshot::error(
            ID,
            NAME,
            "Antigravity usage is temporarily unavailable. Try again shortly.".into(),
        ),
        CloudResult::NoCredentials => {
            if installed() {
                Snapshot::no_credentials(ID, NAME, "Start Antigravity once and try again.")
            } else {
                Snapshot::no_credentials(ID, NAME, "Antigravity not found on this PC.")
            }
        }
    }
}

fn installed() -> bool {
    let mut candidates = Vec::new();
    if let Ok(appdata) = std::env::var("APPDATA") {
        candidates.push(std::path::PathBuf::from(appdata).join("Antigravity"));
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        candidates.push(std::path::PathBuf::from(local).join("Programs").join("Antigravity"));
    }
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join(".antigravity"));
    }
    candidates.iter().any(|p| p.exists())
}

// ---------------------------------------------------------------------------
// Language-server discovery
// ---------------------------------------------------------------------------

struct LanguageServer {
    ports: Vec<u16>,
    extension_port: Option<u16>,
    csrf_token: String,
}

/// Runs a console command without flashing a window.
fn run_hidden(program: &str, args: &[&str]) -> Option<String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = std::process::Command::new(program)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `--flag value` or `--flag=value` from a whitespace-tokenized command line.
fn flag_value(tokens: &[&str], flag: &str) -> Option<String> {
    for (i, t) in tokens.iter().enumerate() {
        if let Some(v) = t.strip_prefix(&format!("{flag}=")) {
            return Some(v.trim_matches('"').to_string());
        }
        if *t == flag {
            return tokens.get(i + 1).map(|v| v.trim_matches('"').to_string());
        }
    }
    None
}

fn discover_language_servers(permit: Option<crate::access_policy::AccessPermit>) -> Vec<LanguageServer> {
    let Some(permit) = permit else { return Vec::new(); };
    if permit.check().is_err() { return Vec::new(); }
    let script = "Get-CimInstance Win32_Process | Where-Object { $_.Name -match '^(language_server|agy)' } | Select-Object ProcessId, Name, CommandLine | ConvertTo-Json -Compress";
    let raw = match run_hidden("powershell", &["-NoProfile", "-NonInteractive", "-Command", script])
    {
        Some(r) if !r.trim().is_empty() => r,
        _ => return Vec::new(),
    };
    let parsed: Value = match serde_json::from_str(raw.trim()) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let procs: Vec<Value> = match parsed {
        Value::Array(a) => a,
        obj @ Value::Object(_) => vec![obj],
        _ => Vec::new(),
    };

    // netstat once, shared across processes: pid -> listening loopback ports.
    if permit.check().is_err() { return Vec::new(); }
    let netstat = run_hidden("netstat", &["-ano", "-p", "TCP"]).unwrap_or_default();

    let mut found = Vec::new();
    for p in procs {
        if permit.check().is_err() { return Vec::new(); }
        let cmdline = p.get("CommandLine").and_then(Value::as_str).unwrap_or("");
        let pid = p.get("ProcessId").and_then(Value::as_u64).unwrap_or(0) as u32;
        if cmdline.is_empty() || pid == 0 {
            continue;
        }
        let tokens: Vec<&str> = cmdline.split_whitespace().collect();

        // Only Antigravity's own language server — Windsurf ships the same
        // binary with a different --ide_name.
        let ide_name = flag_value(&tokens, "--ide_name")
            .or_else(|| flag_value(&tokens, "--override_ide_name"))
            .unwrap_or_default()
            .to_lowercase();
        let app_data = flag_value(&tokens, "--app_data_dir").unwrap_or_default().to_lowercase();
        let is_antigravity = ide_name == "antigravity"
            || ide_name == "antigravity-ide"
            || app_data.contains("antigravity");
        if !is_antigravity {
            continue;
        }

        let csrf_token = flag_value(&tokens, "--csrf_token").unwrap_or_default();
        let extension_port = flag_value(&tokens, "--extension_server_port")
            .and_then(|v| v.parse::<u16>().ok());

        let ports = crate::network_policy::antigravity_process_ports(&netstat, pid);
        // Do not treat a command-line port as proof that the listener belongs
        // to this process. Extension ports qualify only when ownership matches.
        let extension_port = extension_port.filter(|port| ports.contains(port));

        if csrf_token.is_empty() || (ports.is_empty() && extension_port.is_none()) {
            continue;
        }
        found.push(LanguageServer { ports, extension_port, csrf_token });
    }
    found
}

// ---------------------------------------------------------------------------
// Language-server RPC
// ---------------------------------------------------------------------------

async fn ls_call(scheme: &str, port: u16, csrf: &str, method: &str) -> Option<Value> {
    let url = format!("{scheme}://127.0.0.1:{port}/{LS_SERVICE}/{method}");
    let body = json!({
        "metadata": {
            "ideName": "antigravity",
            "extensionName": "antigravity",
            "ideVersion": "unknown",
            "locale": "en",
        }
    });
    let origin = format!("{scheme}://127.0.0.1:{port}");
    let resp = crate::network_transport::CheckedClient::antigravity_local(&origin).ok()?
        .post(&url)
        .timeout(Duration::from_secs(10))
        .header("Content-Type", "application/json")
        .header("Connect-Protocol-Version", "1")
        .header("x-codeium-csrf-token", csrf)
        .json(&body)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.json::<Value>().await.ok()
}

async fn try_language_server(server: &LanguageServer) -> Option<Snapshot> {
    let mut attempts: Vec<(String, u16)> = Vec::new();
    for scheme in ["https", "http"] {
        for port in &server.ports {
            attempts.push((scheme.to_string(), *port));
        }
    }
    if let Some(ext) = server.extension_port {
        attempts.push(("http".into(), ext));
    }

    for (scheme, port) in attempts {
        // Authoritative endpoint first: merged pools + weekly windows.
        if let Some(doc) =
            ls_call(&scheme, port, &server.csrf_token, "RetrieveUserQuotaSummary").await
        {
            let payload = doc.get("response").unwrap_or(&doc);
            let metrics = parse_quota_buckets(payload);
            if !metrics.is_empty() {
                let plan = ls_call(&scheme, port, &server.csrf_token, "GetUserStatus")
                    .await
                    .and_then(|d| extract_plan(&d));
                return Some(Snapshot::ok(ID, NAME, plan, metrics));
            }
        }
        // Legacy: per-model configs pooled into Session/Claude.
        if let Some(doc) = ls_call(&scheme, port, &server.csrf_token, "GetUserStatus").await {
            let metrics = parse_model_configs(&doc);
            if !metrics.is_empty() {
                let plan = extract_plan(&doc);
                return Some(Snapshot::ok(ID, NAME, plan, metrics));
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Cloud Code fallback (works while the IDE is closed)
// ---------------------------------------------------------------------------

enum CloudResult {
    Ok(Snapshot),
    RefreshError(String),
    AuthExpired,
    Unavailable,
    NoCredentials,
}

/// Read only the authorized Credential Manager source. Its complete blob
/// fingerprint binds the existing access-only cache to this exact login.
fn load_stored_token() -> Result<crate::credential_refresh::CachedSource, String> {
    use crate::credential_refresh::{credential_version, CachedSource};
    crate::access_policy::check_current_operation()?;
    let json_text =
        super::credential_string("gemini:antigravity").ok_or("Antigravity sign-in not found")?;
    if json_text.len() > 64 * 1024 {
        return Err("Antigravity credential source exceeds size limit".into());
    }
    let doc: Value = serde_json::from_str(json_text.trim())
        .map_err(|_| "Antigravity credential JSON is invalid")?;
    let token = doc.get("token").unwrap_or(&doc);
    let access = token
        .get("access_token")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let refresh = token
        .get("refresh_token")
        .and_then(Value::as_str)
        .map(str::to_string);
    let expires = token
        .get("expiry")
        .and_then(Value::as_str)
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.timestamp_millis())
        .unwrap_or(i64::MAX);
    Ok(CachedSource {
        version: credential_version(json_text.as_bytes()),
        access,
        refresh,
        expires,
    })
}
fn cached_token_path() -> std::path::PathBuf {
    super::config_dir().join("antigravity-token.json")
}
async fn refresh_google_token(
    refresh: String,
) -> Result<Value, crate::credential_refresh::RemoteError> {
    use crate::credential_refresh::RemoteError;
    let form = [
        ("client_id", GOOGLE_CLIENT_ID),
        ("client_secret", GOOGLE_CLIENT_SECRET),
        ("refresh_token", refresh.as_str()),
        ("grant_type", "refresh_token"),
    ];
    let resp = super::http(ID)
        .post("https://oauth2.googleapis.com/token")
        .form(&form)
        .send()
        .await
        .map_err(|_| RemoteError::Unavailable)?;
    if !resp.status().is_success() {
        return Err(RemoteError::Rejected);
    }
    super::json_body(resp, 16 * 1024, "Antigravity token refresh")
        .await
        .map_err(|_| RemoteError::SucceededInvalid)
}
async fn cloud_access(
    rejected: Option<crate::credential_refresh::CredentialVersion>,
) -> Result<crate::credential_refresh::CachedToken, String> {
    crate::access_policy::check_current_operation()?;
    let path = cached_token_path();
    let key = format!(
        "credential-manager:gemini:antigravity:{}",
        path.to_str()
            .ok_or("Antigravity cache path encoding is unsupported")?
    );
    crate::credential_refresh::cached_access(
        &key,
        &path,
        load_stored_token,
        rejected,
        crate::access_policy::check_current_operation,
        refresh_google_token,
    )
    .await
}

async fn cloud_call(access_token: &str, path: &str) -> Result<Option<Value>, bool> {
    // Ok(Some(doc)) = 2xx; Ok(None) = non-auth failure; Err(true) = 401/403.
    for base in CLOUD_BASES {
        let resp = super::http(ID)
            .post(format!("{base}{path}"))
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {access_token}"))
            .header("User-Agent", "antigravity")
            .json(&json!({}))
            .send()
            .await;
        match resp {
            Ok(r) if r.status().is_success() => return Ok(r.json::<Value>().await.ok()),
            Ok(r) if matches!(r.status().as_u16(), 401 | 403) => return Err(true),
            _ => continue,
        }
    }
    Ok(None)
}

async fn cloud_snapshot(access_token: &str) -> Result<Option<Snapshot>, bool> {
    let doc = cloud_call(access_token, "/v1internal:retrieveUserQuotaSummary").await?;
    if let Some(doc) = doc {
        let metrics = parse_quota_buckets(&doc);
        if !metrics.is_empty() {
            let plan = match cloud_call(access_token, "/v1internal:loadCodeAssist").await {
                Ok(Some(d)) => d
                    .get("paidTier")
                    .or_else(|| d.get("currentTier"))
                    .and_then(|t| t.get("name"))
                    .and_then(Value::as_str)
                    .map(format_plan),
                _ => None,
            };
            return Ok(Some(Snapshot::ok(ID, NAME, plan, metrics)));
        }
    }
    Ok(None)
}

async fn try_cloud() -> CloudResult {
    let token = match cloud_access(None).await {
        Ok(v) => v,
        Err(e) if e == "Antigravity sign-in not found" => return CloudResult::NoCredentials,
        Err(e) => return CloudResult::RefreshError(e),
    };
    match cloud_snapshot(&token.access).await {
        Ok(Some(snap)) => CloudResult::Ok(snap),
        Ok(None) => CloudResult::Unavailable,
        Err(_) => {
            let rejected = crate::credential_refresh::credential_version(token.access.as_bytes());
            let fresh = match cloud_access(Some(rejected)).await {
                Ok(v) => v,
                Err(e) => return CloudResult::RefreshError(e),
            };
            match cloud_snapshot(&fresh.access).await {
                Ok(Some(snap)) => CloudResult::Ok(snap),
                Ok(None) => CloudResult::Unavailable,
                Err(_) => CloudResult::AuthExpired,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Response parsing (shared by LS and Cloud Code shapes)
// ---------------------------------------------------------------------------

const FIVE_HOURS_MS: i64 = 5 * 60 * 60 * 1000;
const ONE_WEEK_MS: i64 = 7 * 24 * 60 * 60 * 1000;

fn bucket_label(bucket_id: &str) -> Option<(&'static str, i64, usize)> {
    // label, period, sort order — exact bucketId match like the Mac app.
    match bucket_id {
        "gemini-5h" => Some(("Session", FIVE_HOURS_MS, 0)),
        "gemini-weekly" => Some(("Weekly", ONE_WEEK_MS, 1)),
        "3p-5h" => Some(("Claude", FIVE_HOURS_MS, 2)),
        "3p-weekly" => Some(("Claude Weekly", ONE_WEEK_MS, 3)),
        _ => None,
    }
}

fn parse_iso_ms(v: Option<&Value>) -> Option<i64> {
    v.and_then(Value::as_str)
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.timestamp_millis())
}

fn parse_quota_buckets(doc: &Value) -> Vec<Metric> {
    let mut rows: Vec<(usize, Metric)> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    let groups = doc.get("groups").and_then(Value::as_array).cloned().unwrap_or_default();
    for group in groups {
        let buckets = group.get("buckets").and_then(Value::as_array).cloned().unwrap_or_default();
        for bucket in buckets {
            let Some(id) = bucket.get("bucketId").and_then(Value::as_str) else { continue };
            if seen.iter().any(|s| s == id) {
                continue;
            }
            let Some((label, period_ms, order)) = bucket_label(id) else { continue };
            let Some(remaining) = bucket.get("remainingFraction").and_then(Value::as_f64) else {
                continue;
            };
            if !remaining.is_finite() {
                continue;
            }
            seen.push(id.to_string());
            let used = ((1.0 - remaining) * 100.0).clamp(0.0, 100.0);
            let resets_at = parse_iso_ms(bucket.get("resetTime"));
            rows.push((
                order,
                Metric::progress(label, used, None).with_reset(resets_at, Some(period_ms)),
            ));
        }
    }
    rows.sort_by_key(|(order, _)| *order);
    rows.into_iter().map(|(_, m)| m).collect()
}

/// Legacy shape: per-model quotas pooled into "Session" (Gemini) and
/// "Claude" (everything else), keeping each pool's worst remaining fraction.
fn parse_model_configs(doc: &Value) -> Vec<Metric> {
    let configs = doc
        .pointer("/userStatus/cascadeModelConfigData/clientModelConfigs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut pools: [Option<(f64, Option<i64>)>; 2] = [None, None]; // [gemini, other]
    for cfg in configs {
        let label = cfg.get("label").and_then(Value::as_str).unwrap_or_default();
        let Some(remaining) = cfg.pointer("/quotaInfo/remainingFraction").and_then(Value::as_f64)
        else {
            continue;
        };
        let resets_at = parse_iso_ms(cfg.pointer("/quotaInfo/resetTime"));
        let idx = if label.to_lowercase().contains("gemini") { 0 } else { 1 };
        let worse = match pools[idx] {
            Some((existing, _)) => remaining < existing,
            None => true,
        };
        if worse {
            pools[idx] = Some((remaining, resets_at));
        }
    }
    let mut metrics = Vec::new();
    for (idx, label) in [(0, "Session"), (1, "Claude")] {
        if let Some((remaining, resets_at)) = pools[idx] {
            let used = ((1.0 - remaining) * 100.0).clamp(0.0, 100.0);
            metrics.push(
                Metric::progress(label, used, None).with_reset(resets_at, Some(FIVE_HOURS_MS)),
            );
        }
    }
    metrics
}

fn format_plan(raw: &str) -> String {
    let stripped = raw.trim().trim_start_matches("Google AI ").trim();
    for tier in ["Ultra", "Pro", "Free"] {
        if stripped.contains(tier) {
            return tier.to_string();
        }
    }
    stripped.to_string()
}

fn extract_plan(doc: &Value) -> Option<String> {
    let raw = doc
        .pointer("/userStatus/userTier/name")
        .or_else(|| doc.pointer("/userStatus/planStatus/planInfo/planName"))
        .and_then(Value::as_str)?;
    Some(format_plan(raw))
}

#[cfg(test)]
mod tests {
    /// Live probe against this machine's real credentials — run manually via
    /// `cargo test --lib antigravity -- --ignored --nocapture`. Prints field
    /// names and numbers only, never token values.
    #[test]
    #[ignore]
    fn live_probe() {
        let snap = tauri::async_runtime::block_on(super::snapshot());
        eprintln!(
            "antigravity: status={} plan={:?} error={:?} metrics={}",
            snap.status,
            snap.plan,
            snap.error,
            snap.metrics.len()
        );
        for m in &snap.metrics {
            eprintln!(
                "  {}: used={:?} resets_at={:?} period={:?}",
                m.label, m.used_percent, m.resets_at, m.period_ms
            );
        }
    }
}
