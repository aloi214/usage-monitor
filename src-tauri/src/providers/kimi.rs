//! Kimi Code — one card like OpenCode: Session + Weekly/Monthly from the
//! subscription, plus an API bar from the Moonshot pay-as-you-go wallet
//! when a key is saved in Settings.
//!
//! Quota comes from GET api.kimi.com/coding/v1/usages, authenticated with
//! the official Kimi Code CLI's OAuth login
//! (`%USERPROFILE%\.kimi-code\credentials\kimi-code.json`). Tokens refresh
//! against auth.kimi.com and are written back beside the CLI's file so the
//! CLI stays signed in — same write-back as Claude/Codex.
//!
//! Without a CLI login, a Kimi For Coding plan key pasted in Settings
//! (`%APPDATA%\Pane\kimi.json`) is sent as Bearer to the same endpoint —
//! what cc-switch and the plan's Anthropic-compatible endpoint accept
//! (issue #173). Login wins when both exist; the key is a fallback only.

use super::{http, stored_api_key, Metric, Snapshot};
use chrono::DateTime;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

const ID: &str = "kimi";
const NAME: &str = "Kimi Code";
/// Public OAuth client id the official CLI (and OpenUsage) uses.
const CLIENT_ID: &str = "17e5f671-d194-4dfb-9706-5516cb48c098";
const USAGES_URL: &str = "https://api.kimi.com/coding/v1/usages";
const ME_URL: &str = "https://api.kimi.com/coding/v1/me";
const TOKEN_URL: &str = "https://auth.kimi.com/api/oauth/token";
const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 86_400_000;
const MAX_CRED_BYTES: u64 = 64 * 1024;
const MAX_USAGES_BYTES: usize = 256 * 1024;
const MAX_ME_BYTES: usize = 64 * 1024;
const MAX_TOKEN_BYTES: usize = 16 * 1024;

pub async fn snapshot() -> Snapshot {
    match fetch().await {
        Ok(s) => s,
        Err(e) => Snapshot::error(ID, NAME, e),
    }
}

pub fn has_login() -> bool {
    cred_path().is_some()
}

/// Pasted plan key. Settings only — no env var, because the key normally
/// travels as `ANTHROPIC_AUTH_TOKEN` for a router, and reading that would
/// grab whatever vendor the router currently points at.
fn plan_key() -> Option<String> {
    stored_api_key("kimi", &[])
}

pub fn has_plan_key() -> bool {
    plan_key().is_some()
}

/// Anything that can produce the plan card: CLI login or pasted plan key.
/// Spend routing and the Moonshot fold key off this, not `has_login`.
pub fn has_credentials() -> bool {
    if !crate::access_runtime().snapshot().0.allows_account(ID) { return false; }
    has_login() || has_plan_key()
}

/// Official CLI home. `KIMI_CODE_HOME` is honored only when it is an
/// absolute path — a relative value would resolve against Pane's cwd and
/// could point the spend walk or credential write at the wrong tree.
pub fn code_home() -> PathBuf {
    if let Some(p) = env_code_home() {
        return p;
    }
    dirs::home_dir().unwrap_or_default().join(".kimi-code")
}

fn env_code_home() -> Option<PathBuf> {
    let raw = std::env::var("KIMI_CODE_HOME").ok()?;
    absolute_home(&raw)
}

fn absolute_home(raw: &str) -> Option<PathBuf> {
    let p = PathBuf::from(raw.trim());
    if p.as_os_str().is_empty() || !p.is_absolute() {
        return None;
    }
    #[cfg(windows)]
    {
        use std::path::Component;
        // `\Windows\...` is "absolute" on Windows but drive-relative.
        // Only honor a real drive/UNC prefix so a planted env can't aim
        // credential writes at the current-drive root.
        if !matches!(p.components().next(), Some(Component::Prefix(_))) {
            return None;
        }
    }
    Some(p)
}

fn cred_candidates(kimi_home: Option<PathBuf>, user_home: Option<PathBuf>) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(home) = kimi_home {
        candidates.push(home.join("credentials").join("kimi-code.json"));
    }
    if let Some(home) = user_home {
        candidates.push(home.join(".kimi-code").join("credentials").join("kimi-code.json"));
        // OpenUsage/Mac spelling — keep as a fallback.
        candidates.push(home.join(".kimi").join("credentials").join("kimi-code.json"));
    }
    candidates
}

fn cred_path() -> Option<PathBuf> {
    cred_candidates(env_code_home(), dirs::home_dir())
        .into_iter()
        .find(|p| is_regular_file(p))
}

async fn fetch() -> Result<Snapshot, String> {
    let path = cred_path();
    let key = plan_key();
    if path.is_none() && key.is_none() {
        return Ok(Snapshot::no_credentials(
            ID,
            NAME,
            "Kimi Code sign-in not found. Run `kimi login` in a terminal, or paste your Kimi For Coding key in Settings (gear icon).",
        ));
    }
    // The wallet has a separate grant. Enter its context BEFORE reading its
    // key, and inherit any frontend narrowing from this Kimi operation.
    let (usages, api) = tokio::join!(
        load_usages(path.as_deref(), key.as_deref()),
        async { crate::access_runtime().run_account("moonshot", &[], crate::guarded_moonshot_wallet()).await }
    );
    let (doc, plan) = usages?;
    let mut snap = parse_snapshot(&doc)?;
    snap.wallet_refresh_skipped = api.is_none();
    // The plan name moved to /me when usages dropped membership.level —
    // its answer wins over the doc's LEVEL_*/limit fallback.
    if let Some(name) = plan {
        snap.plan = Some(name);
    }
    if let Some(wallet) = api {
        if wallet.status == "error" {
            snap.warning = Some("Moonshot API wallet couldn't refresh — retrying next cycle".into());
            snap.wallet_history = Some(crate::snapshot::WalletHistory {
                source_account_id: "moonshot".into(),
                fetched_at: None,
                attempted_at: wallet.attempted_at,
                warning: snap.warning.clone(),
            });
        } else {
            snap.metrics.extend(wallet.metrics);
        }
    }
    Ok(snap)
}

enum UsagesError {
    Unauthorized,
    Other(String),
}

/// CLI login first (with the refresh-and-retry dance); the pasted plan key
/// only when there is no login or the login path failed. When both fail,
/// the login error is the one shown — `kimi login` is the actionable fix.
/// Returns the usages doc plus the /me plan name — the plan key path
/// never calls /me (a pasted "Kimi For Coding" key is not verified to
/// work there), so its plan stays `None` for the doc fallback.
async fn load_usages(
    cred: Option<&Path>,
    plan_key: Option<&str>,
) -> Result<(Value, Option<String>), String> {
    let login_err = match cred {
        Some(path) => match usages_via_login(path).await {
            Ok(pair) => return Ok(pair),
            Err(e) => Some(e),
        },
        None => None,
    };
    let Some(key) = plan_key else {
        return Err(login_err.unwrap_or_else(|| "no Kimi Code credentials".into()));
    };
    match fetch_usages(key).await {
        Ok(doc) => Ok((doc, None)),
        Err(UsagesError::Unauthorized) => Err(login_err.unwrap_or_else(|| {
            "Kimi For Coding key was rejected — check it in Settings (gear icon)".into()
        })),
        Err(UsagesError::Other(e)) => Err(login_err.unwrap_or(e)),
    }
}

async fn usages_via_login(path: &Path) -> Result<(Value, Option<String>), String> {
    let access = load_access(path, None).await?;
    let (usages, plan) = usages_and_plan(&access).await;
    match usages {
        Ok(doc) => Ok((doc, plan)),
        Err(UsagesError::Unauthorized) => {
            let access = load_access(
                path,
                Some(crate::credential_refresh::credential_version(
                    access.as_bytes(),
                )),
            )
            .await?;
            let (usages, plan) = usages_and_plan(&access).await;
            match usages {
                Ok(doc) => Ok((doc, plan)),
                Err(UsagesError::Unauthorized) => Err(
                    "Kimi Code sign-in was rotated — run `kimi login` in a terminal once and Pane recovers automatically"
                        .into(),
                ),
                Err(UsagesError::Other(e)) => Err(e),
            }
        }
        Err(UsagesError::Other(e)) => Err(e),
    }
}

/// One usages attempt with the /me plan lookup alongside it: /me starts
/// together with usages and gets at most 1.5 s after usages returns;
/// the doc fallback covers the rest.
async fn usages_and_plan(access: &str) -> (Result<Value, UsagesError>, Option<String>) {
    let permit = crate::access_policy::current_operation();
    let mut me = tokio::spawn({
        let a = access.to_string();
        async move {
            match permit { Some(p) => p.run(fetch_plan_name(&a)).await.flatten(), None => None }
        }
    });
    let usages = fetch_usages(access).await;
    let plan = match tokio::time::timeout(Duration::from_millis(1500), &mut me).await {
        Ok(Ok(p)) => p,
        Ok(Err(_)) => None,
        Err(_) => {
            me.abort();
            None
        }
    };
    (usages, plan)
}

async fn fetch_usages(access: &str) -> Result<Value, UsagesError> {
    let resp = http(ID)
        .get(USAGES_URL)
        .bearer_auth(access)
        .header("Accept", "application/json")
        .timeout(Duration::from_secs(8))
        .send()
        .await
        .map_err(|e| UsagesError::Other(format!("usage request: {e}")))?;
    let status = resp.status();
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err(UsagesError::Unauthorized);
    }
    if !status.is_success() {
        return Err(UsagesError::Other(format!("usage endpoint: HTTP {status}")));
    }
    super::json_body(resp, MAX_USAGES_BYTES, "usage")
        .await
        .map_err(UsagesError::Other)
}

/// The plan display name now lives on /me (`user_level_name` — the
/// pricing-page name verbatim, e.g. "Allegro") since usages dropped
/// `user.membership.level`. Missing/blank → None, not an error.
fn plan_from_me(doc: &Value) -> Option<String> {
    doc.get("user_level_name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Best-effort sibling call on the OAuth login token only — spawned
/// alongside usages with a short grace after it lands; a dead /me must
/// never block the card, and the usages doc stays the plan fallback.
/// Same bearer token, same host.
async fn fetch_plan_name(access: &str) -> Option<String> {
    let resp = match http(ID)
        .get(ME_URL)
        .bearer_auth(access)
        .header("Accept", "application/json")
        .timeout(Duration::from_secs(4))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[pane] kimi plan: {e}");
            return None;
        }
    };
    if !resp.status().is_success() {
        eprintln!("[pane] kimi plan: HTTP {}", resp.status());
        return None;
    }
    match super::json_body(resp, MAX_ME_BYTES, "plan").await {
        Ok(doc) => plan_from_me(&doc),
        Err(e) => {
            eprintln!("[pane] kimi plan: {e}");
            None
        }
    }
}

/// Forced refresh carries the exact rejected access generation, so a queued
/// caller reuses a newer pair instead of rotating it a second time.
async fn load_access(
    path: &Path,
    rejected: Option<crate::credential_refresh::CredentialVersion>,
) -> Result<String, String> {
    use crate::credential_refresh::{refresh_file, Flavor, Reason, RemoteError};
    let result = refresh_file(
        path,
        Flavor::Kimi,
        rejected.map(Reason::Rejected).unwrap_or(Reason::Expiry),
        crate::access_policy::check_current_operation,
        |_| Ok(()),
        |input| async move {
            let form = [
                ("grant_type", "refresh_token"),
                ("refresh_token", input.refresh.as_str()),
                ("client_id", CLIENT_ID),
            ];
            let resp = http(ID)
                .post(TOKEN_URL)
                .header("Accept", "application/json")
                .form(&form)
                .timeout(Duration::from_secs(8))
                .send()
                .await
                .map_err(|_| RemoteError::Unavailable)?;
            if !resp.status().is_success() {
                return Err(RemoteError::Rejected);
            }
            super::json_body(resp, MAX_TOKEN_BYTES, "token refresh")
                .await
                .map_err(|_| RemoteError::SucceededInvalid)
        },
    )
    .await?;
    result
        .doc
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "Kimi access token is missing".into())
}

fn is_regular_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .ok()
        .is_some_and(|m| m.is_file() && !m.file_type().is_symlink())
}




fn parse_snapshot(doc: &Value) -> Result<Snapshot, String> {
    const SESSION_MS: i64 = 5 * HOUR_MS;
    const WEEK_MS: i64 = 7 * DAY_MS;
    let mut metrics = Vec::new();
    let mut weekly_limit = None;

    // Ratio pools supersede the legacy counts. Presence matters: a malformed
    // new pool must not silently resurrect a stale legacy reading.
    let usages = doc.get("usages");
    if let Some(session) = usages.and_then(|u| u.get("limit_5h")) {
        if let Some(m) = ratio_progress("Session", session, Some(SESSION_MS)) {
            metrics.push(m);
        }
    } else {
        // Explicit legacy window metadata is still safe when the new session
        // pool is absent. Never infer another duration from an unknown pool.
        for entry in doc.get("limits").and_then(Value::as_array).unwrap_or(&vec![]) {
            if !is_session_window(entry) {
                continue;
            }
            let node = entry.get("detail").unwrap_or(entry);
            if let Some(m) = progress_from("Session", node, SESSION_MS) {
                metrics.push(m);
                break;
            }
        }
    }

    if let Some(usages) = usages {
        for (key, label, period) in [
            ("limit_7d", "Weekly", Some(WEEK_MS)),
            ("limit_month_total", "Monthly", None),
        ] {
            if let Some(m) = usages.get(key).and_then(|node| ratio_progress(label, node, period)) {
                metrics.push(m);
            }
        }
        // limit_month_code is a consumption component of the shared monthly
        // total, not a separate allowance. A monthly reset alone doesn't give
        // a period start; leave duration unknown rather than assume 30 days.
    } else if let Some(usage) = doc.get("usage") {
        // Only the old schema identifies this otherwise unlabelled pool as
        // weekly. On new responses it must not create a spurious Weekly row
        // (or infer an old plan tier from an unrelated count).
        weekly_limit = json_f64(usage.get("limit"));
        if let Some(m) = progress_from("Weekly", usage, WEEK_MS) {
            metrics.push(m);
        }
    }

    if metrics.is_empty() {
        return Err("usage response had no recognizable limit windows".into());
    }
    Ok(Snapshot::ok(ID, NAME, plan_from_doc(doc, weekly_limit), metrics))
}

/// Official ratio schema: finite numbers or numeric strings in [0, 1].
/// Reject invalid readings rather than turn them into an unused/full quota.
fn ratio_progress(label: &str, node: &Value, period_ms: Option<i64>) -> Option<Metric> {
    let ratio = json_f64(node.get("used_ratio"))?;
    if !ratio.is_finite() || !(0.0..=1.0).contains(&ratio) {
        return None;
    }
    let resets_at = node.get("reset_time").and_then(Value::as_str).and_then(parse_iso_ms);
    Some(Metric::progress(label, ratio * 100.0, None).with_reset(resets_at, period_ms))
}

fn progress_from(label: &str, node: &Value, period_ms: i64) -> Option<Metric> {
    let used = used_percent(node)?;
    let resets_at = parse_reset(node.get("resetTime").or_else(|| node.get("reset_time")));
    Some(Metric::progress(label, used, None).with_reset(resets_at, Some(period_ms)))
}

fn is_session_window(entry: &Value) -> bool {
    let duration = json_f64(entry.pointer("/window/duration"));
    let unit = entry
        .pointer("/window/timeUnit")
        .or_else(|| entry.pointer("/window/time_unit"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let unit = unit.to_ascii_uppercase();
    match (duration, unit.as_str()) {
        (Some(d), u) if (d - 300.0).abs() < 0.5 && u.contains("MINUTE") => true,
        (Some(d), u) if (d - 5.0).abs() < 0.5 && u.contains("HOUR") => true,
        _ => false,
    }
}

fn used_percent(node: &Value) -> Option<f64> {
    let limit = json_f64(node.get("limit"))?;
    if limit <= 0.0 {
        return None;
    }
    if let Some(used) = json_f64(node.get("used")) {
        return Some((used / limit * 100.0).clamp(0.0, 100.0));
    }
    let remaining = json_f64(node.get("remaining"))?;
    Some(((limit - remaining) / limit * 100.0).clamp(0.0, 100.0))
}

fn plan_from_doc(doc: &Value, weekly_limit: Option<f64>) -> Option<String> {
    if let Some(level) = doc
        .pointer("/user/membership/level")
        .and_then(Value::as_str)
        .and_then(map_membership_level)
    {
        return Some(level);
    }
    match weekly_limit.map(|n| n.round() as i64) {
        // Last resort when the API omits membership.level. These are the
        // old request-count caps; current responses usually send percent
        // (limit 100) plus a LEVEL_* code instead.
        Some(1024) => Some("Andante".into()),
        Some(2048) => Some("Moderato".into()),
        Some(7168) => Some("Allegretto".into()),
        _ => None,
    }
}

/// Display names match https://www.kimi.ai/membership/pricing
/// (Moderato / Allegretto / Allegro / Vivace). `LEVEL_*` codes shifted
/// when Allegro and Vivace launched (issue #156): INTERMEDIATE used to
/// print as Moderato. Older codes (Adagio / Andante) are mapped if the
/// API still sends them.
fn map_membership_level(raw: &str) -> Option<String> {
    let upper = raw.trim().to_ascii_uppercase();
    let key = upper.strip_prefix("LEVEL_").unwrap_or(&upper);
    Some(match key {
        "FREE" | "ADAGIO" | "BASIC" => "Adagio".into(),
        "ANDANTE" | "BEGINNER" | "INTRO" => "Andante".into(),
        "MODERATO" | "STANDARD" => "Moderato".into(),
        "INTERMEDIATE" | "ALLEGRETTO" | "ALLEGROTTO" | "PROFESSIONAL" | "PRO" => {
            "Allegretto".into()
        }
        "ADVANCED" | "ALLEGRO" => "Allegro".into(),
        "PREMIUM" | "VIVACE" => "Vivace".into(),
        "" => return None,
        other => title_case(other),
    })
}

fn title_case(s: &str) -> String {
    let mut out = String::new();
    let mut cap = true;
    for c in s.chars() {
        if c == '_' || c == '-' {
            out.push(' ');
            cap = true;
            continue;
        }
        if cap {
            out.extend(c.to_uppercase());
            cap = false;
        } else {
            out.extend(c.to_lowercase());
        }
    }
    out
}

fn json_f64(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn expires_at_ms(v: Option<&Value>) -> i64 {
    let Some(n) = json_f64(v) else { return 0 };
    if n.abs() >= 1e12 {
        n as i64
    } else {
        (n * 1000.0) as i64
    }
}

/// `resetTime` is ISO-8601; some responses carry extra fractional digits
/// that strict RFC3339 rejects, so trim the fraction to microseconds.
fn parse_reset(v: Option<&Value>) -> Option<i64> {
    match v? {
        Value::String(s) => parse_iso_ms(s),
        Value::Number(n) => {
            let n = n.as_f64()?;
            Some(if n.abs() < 1e10 { (n * 1000.0) as i64 } else { n as i64 })
        }
        _ => None,
    }
}

fn parse_iso_ms(s: &str) -> Option<i64> {
    let s = s.trim();
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.timestamp_millis());
    }
    let dot = s.find('.')?;
    let (head, rest) = s.split_at(dot);
    let rest = &rest[1..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    let tz: String = rest.chars().skip_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    let frac: String = digits.chars().chain(std::iter::repeat('0')).take(6).collect();
    DateTime::parse_from_rfc3339(&format!("{head}.{frac}{tz}"))
        .ok()
        .map(|dt| dt.timestamp_millis())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn labels(metrics: &[Metric]) -> Vec<&str> {
        metrics.iter().map(|m| m.label.as_str()).collect()
    }

    // Wire keys are documented by MoonshotAI/kimi-code's
    // packages/oauth/src/managed-usage.ts; these are synthetic fixtures.
    #[test]
    fn ratio_session_then_monthly_with_no_invented_month_duration() {
        let snap = parse_snapshot(&json!({"usages": {
            "limit_month_code": {"used_ratio": 0.25},
            "limit_month_total": {"used_ratio": 0.4, "reset_time": "2026-10-01T00:00:00Z"},
            "limit_5h": {"used_ratio": "0.0056", "reset_time": "2026-09-11T18:00:00Z"}
        }})).unwrap();
        assert_eq!(labels(&snap.metrics), ["Session", "Monthly"]);
        assert!((snap.metrics[0].used_percent.unwrap() - 0.56).abs() < 1e-9);
        assert_eq!(snap.metrics[0].period_ms, Some(5 * HOUR_MS));
        let monthly = &snap.metrics[1];
        assert_eq!(monthly.used_percent, Some(40.0));
        assert_eq!(monthly.resets_at, parse_iso_ms("2026-10-01T00:00:00Z"));
        assert_eq!(monthly.period_ms, None);
        assert_eq!(monthly.detail, None);
        assert_eq!(monthly.value, None);
    }

    #[test]
    fn ratio_pools_have_stable_order_and_keep_explicit_weekly() {
        let snap = parse_snapshot(&json!({"usages": {
            "limit_month_total": {"used_ratio": 0.4},
            "limit_7d": {"used_ratio": 0.2},
            "limit_5h": {"used_ratio": 0.3}
        }})).unwrap();
        assert_eq!(labels(&snap.metrics), ["Session", "Weekly", "Monthly"]);
        assert_eq!(snap.metrics[1].used_percent, Some(20.0));
        assert_eq!(snap.metrics[1].period_ms, Some(7 * DAY_MS));
    }

    #[test]
    fn ratio_legacy_plan_keeps_session_and_weekly_without_monthly() {
        let snap = parse_snapshot(&json!({"usages": {
            "limit_7d": {"used_ratio": "0.2", "reset_time": "2026-09-17T00:00:00Z"},
            "limit_5h": {"used_ratio": 0.3}
        }})).unwrap();
        assert_eq!(labels(&snap.metrics), ["Session", "Weekly"]);
        assert_eq!(snap.metrics[1].used_percent, Some(20.0));
        assert_eq!(snap.metrics[1].resets_at, parse_iso_ms("2026-09-17T00:00:00Z"));
    }

    #[test]
    fn ratio_format_wins_without_relabeling_legacy_usage_as_weekly() {
        let snap = parse_snapshot(&json!({
            "usages": {"limit_5h": {"used_ratio": 0.3}, "limit_month_total": {"used_ratio": 0.4}},
            "usage": {"limit": 2048, "used": 10},
            "limits": [{"window": {"duration": 300, "timeUnit": "TIME_UNIT_MINUTE"},
                        "detail": {"limit": 100, "used": 99}}]
        })).unwrap();
        assert_eq!(labels(&snap.metrics), ["Session", "Monthly"]);
        assert_eq!(snap.metrics[0].used_percent, Some(30.0));
        assert_eq!(snap.plan, None);
    }

    #[test]
    fn absent_ratio_session_can_use_explicit_legacy_session() {
        let snap = parse_snapshot(&json!({
            "usages": {"limit_month_total": {"used_ratio": 0.4}},
            "limits": [{"window": {"duration": 300, "timeUnit": "TIME_UNIT_MINUTE"},
                        "detail": {"limit": 100, "used": 15}}]
        })).unwrap();
        assert_eq!(labels(&snap.metrics), ["Session", "Monthly"]);
        assert_eq!(snap.metrics[0].used_percent, Some(15.0));
    }

    #[test]
    fn ratio_zero_and_one_are_valid_without_reset() {
        for (ratio, percent) in [(json!(0), 0.0), (json!("1"), 100.0)] {
            let snap = parse_snapshot(&json!({"usages": {"limit_month_total": {"used_ratio": ratio}}})).unwrap();
            assert_eq!(labels(&snap.metrics), ["Monthly"]);
            assert_eq!(snap.metrics[0].used_percent, Some(percent));
            assert_eq!(snap.metrics[0].resets_at, None);
            assert_eq!(snap.metrics[0].period_ms, None);
        }
    }

    #[test]
    fn malformed_ratios_are_not_zero_or_legacy_weekly() {
        for ratio in [json!(null), json!(true), json!(""), json!(" "), json!("NaN"),
                      json!("inf"), json!("40%"), json!(-0.1), json!(1.1), json!({})] {
            assert!(parse_snapshot(&json!({
                "usages": {"limit_month_total": {"used_ratio": ratio}},
                "usage": {"limit": 100, "used": 0}
            })).is_err(), "invalid ratio: {ratio}");
        }
    }

    #[test]
    fn unknown_and_component_pools_do_not_invent_a_quota() {
        for usages in [json!({}), json!(null), json!({"limit_month_code": {"used_ratio": 0.2}}),
                       json!({"limit_future": {"used_ratio": 0.1}}),
                       json!({"limit_month_total": {"reset_time": "2026-10-01T00:00:00Z"}})] {
            assert!(parse_snapshot(&json!({"usages": usages})).is_err());
        }
    }

    #[test]
    fn malformed_ratio_pool_does_not_hide_valid_sibling_or_fall_back() {
        let snap = parse_snapshot(&json!({
            "usages": {"limit_5h": {"used_ratio": "bad"}, "limit_month_total": {"used_ratio": 0.4}},
            "limits": [{"window": {"duration": 300, "timeUnit": "TIME_UNIT_MINUTE"},
                        "detail": {"limit": 100, "used": 15}}]
        })).unwrap();
        assert_eq!(labels(&snap.metrics), ["Monthly"]);
    }

    #[test]
    fn ratio_reset_requires_valid_rfc3339_and_preserves_offset() {
        for reset in [json!(null), json!(42), json!(""), json!("not a date")] {
            let snap = parse_snapshot(&json!({"usages": {"limit_month_total": {
                "used_ratio": 0.4, "reset_time": reset
            }}})).unwrap();
            assert_eq!(snap.metrics[0].resets_at, None);
        }
        let snap = parse_snapshot(&json!({"usages": {"limit_month_total": {
            "used_ratio": 0.4, "reset_time": "2026-10-01T08:00:00+08:00"
        }}})).unwrap();
        assert_eq!(snap.metrics[0].resets_at, parse_iso_ms("2026-10-01T00:00:00Z"));
    }

    #[test]
    fn session_then_weekly_like_claude() {
        let doc = json!({
            "usage": {
                "limit": "2048",
                "used": "214",
                "remaining": "1834",
                "resetTime": "2026-01-09T15:23:13.716839Z"
            },
            "limits": [{
                "window": {"duration": 300, "timeUnit": "TIME_UNIT_MINUTE"},
                "detail": {
                    "limit": "200",
                    "used": "139",
                    "remaining": "61",
                    "resetTime": "2026-01-06T13:33:02.717479433Z"
                }
            }],
            "user": {"membership": {"level": "LEVEL_INTERMEDIATE"}}
        });
        let snap = parse_snapshot(&doc).expect("parse");
        assert_eq!(snap.id, "kimi");
        assert_eq!(snap.name, "Kimi Code");
        assert_eq!(snap.plan.as_deref(), Some("Allegretto"));
        assert_eq!(labels(&snap.metrics), ["Session", "Weekly"]);
        let session = &snap.metrics[0];
        assert!((session.used_percent.unwrap() - 69.5).abs() < 0.01);
        assert_eq!(session.period_ms, Some(5 * HOUR_MS));
        assert!(session.resets_at.is_some());
        let weekly = &snap.metrics[1];
        assert!((weekly.used_percent.unwrap() - (214.0 / 2048.0 * 100.0)).abs() < 0.01);
        assert_eq!(weekly.period_ms, Some(7 * DAY_MS));
    }

    #[test]
    fn remaining_without_used_still_meters() {
        let doc = json!({
            "usage": {"limit": "100", "remaining": "74", "resetTime": "2026-02-11T17:32:50.757941Z"},
            "limits": [{
                "window": {"duration": 300, "timeUnit": "TIME_UNIT_MINUTE"},
                "detail": {"limit": "100", "remaining": "85", "resetTime": "2026-02-07T12:32:50.757941Z"}
            }]
        });
        let snap = parse_snapshot(&doc).expect("parse");
        assert_eq!(labels(&snap.metrics), ["Session", "Weekly"]);
        assert!((snap.metrics[0].used_percent.unwrap() - 15.0).abs() < 0.01);
        assert!((snap.metrics[1].used_percent.unwrap() - 26.0).abs() < 0.01);
        assert_eq!(snap.plan.as_deref(), None);
    }

    #[test]
    fn weekly_limit_names_the_tier() {
        let andante = json!({"usage": {"limit": 1024, "used": 10, "remaining": 1014}});
        assert_eq!(parse_snapshot(&andante).unwrap().plan.as_deref(), Some("Andante"));
        let mid = json!({"usage": {"limit": 2048, "used": 0, "remaining": 2048}});
        assert_eq!(parse_snapshot(&mid).unwrap().plan.as_deref(), Some("Moderato"));
        let high = json!({"usage": {"limit": "7168", "used": "0", "remaining": "7168"}});
        assert_eq!(parse_snapshot(&high).unwrap().plan.as_deref(), Some("Allegretto"));
    }

    #[test]
    fn me_user_level_name_becomes_the_plan() {
        assert_eq!(
            plan_from_me(&json!({"user_level": 27, "user_level_name": "Allegro"}))
                .as_deref(),
            Some("Allegro")
        );
        assert!(plan_from_me(&json!({"user_level_name": "  "})).is_none());
        assert!(plan_from_me(&json!({})).is_none());
    }

    #[test]
    fn membership_level_matches_kimi_page_names() {
        let cases = [
            ("LEVEL_FREE", "Adagio"),
            ("LEVEL_BASIC", "Adagio"),
            ("ADAGIO", "Adagio"),
            ("LEVEL_ANDANTE", "Andante"),
            ("LEVEL_STANDARD", "Moderato"),
            ("LEVEL_MODERATO", "Moderato"),
            ("LEVEL_INTERMEDIATE", "Allegretto"),
            ("ALLEGRETTO", "Allegretto"),
            ("ALLEGROTTO", "Allegretto"),
            ("LEVEL_ADVANCED", "Allegro"),
            ("LEVEL_ALLEGRO", "Allegro"),
            ("LEVEL_PREMIUM", "Vivace"),
            ("VIVACE", "Vivace"),
            ("PRO", "Allegretto"),
        ];
        for (raw, name) in cases {
            assert_eq!(map_membership_level(raw).as_deref(), Some(name), "{raw}");
        }
    }

    #[test]
    fn ignores_non_session_windows() {
        let doc = json!({
            "usage": {"limit": "100", "used": "10", "remaining": "90"},
            "limits": [{
                "window": {"duration": 10080, "timeUnit": "TIME_UNIT_MINUTE"},
                "detail": {"limit": "100", "used": "10", "remaining": "90"}
            }]
        });
        let snap = parse_snapshot(&doc).expect("weekly-only is ok");
        assert_eq!(labels(&snap.metrics), ["Weekly"]);
    }

    #[test]
    fn empty_body_is_an_error() {
        assert!(parse_snapshot(&json!({})).is_err());
    }

    #[test]
    fn iso_with_extra_fractional_digits_parses() {
        let ms = parse_iso_ms("2026-01-06T13:33:02.717479433Z").expect("ns iso");
        let expected = DateTime::parse_from_rfc3339("2026-01-06T13:33:02.717479Z")
            .unwrap()
            .timestamp_millis();
        assert_eq!(ms, expected);
    }

    #[test]
    fn expires_at_seconds_and_millis() {
        assert_eq!(expires_at_ms(Some(&json!(1_700_000_000))), 1_700_000_000_000);
        assert_eq!(expires_at_ms(Some(&json!(1_700_000_000_000i64))), 1_700_000_000_000);
        assert_eq!(expires_at_ms(Some(&json!("1700000000"))), 1_700_000_000_000);
    }

    #[test]
    fn relative_kimi_code_home_is_ignored() {
        assert!(absolute_home("relative/path").is_none());
        assert!(absolute_home("").is_none());
        assert!(absolute_home("  ").is_none());
        #[cfg(windows)]
        {
            assert!(absolute_home(r"\Windows\System32").is_none());
            assert!(absolute_home(r"C:\Users\jazii\.kimi-code").is_some());
        }
        #[cfg(not(windows))]
        {
            assert!(absolute_home("/home/me/.kimi-code").is_some());
        }
    }

    #[test]
    fn cred_candidates_prefer_kimi_home_then_dot_dirs() {
        let kimi = PathBuf::from(r"D:\custom-kimi");
        let user = PathBuf::from(r"C:\Users\me");
        let paths = cred_candidates(Some(kimi.clone()), Some(user.clone()));
        assert_eq!(
            paths,
            [
                kimi.join("credentials").join("kimi-code.json"),
                user.join(".kimi-code").join("credentials").join("kimi-code.json"),
                user.join(".kimi").join("credentials").join("kimi-code.json"),
            ]
        );
    }

    #[test]
    fn oversized_credentials_are_rejected() {
        let dir = std::env::temp_dir().join(format!("pane-kimi-cred-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("kimi-code.json");
        std::fs::write(&path, vec![b'x'; (MAX_CRED_BYTES as usize) + 1]).unwrap();
        let err = crate::providers::read_small_text(&path, MAX_CRED_BYTES, "credentials").unwrap_err();
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
        assert!(err.contains("large"), "{err}");
    }
}
