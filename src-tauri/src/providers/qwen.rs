//! Qwen Code — Alibaba Model Studio's Coding Plan used through the Qwen
//! Code CLI. The plan meters *requests* (not tokens) across three windows:
//! a rolling 5-hour session, a week (Monday 00:00 UTC+8), and a monthly
//! cycle on the subscription renewal date.
//!
//! Quota comes from the Model Studio console's own RPC — the exact call
//! the Coding Plan page makes (no public quota API exists; approach
//! borrowed from CodexBar's alibaba-coding-plan notes). The endpoint's
//! accepted auth is selected explicitly with its region in Settings. No
//! header or regional discovery probes run. Authorized local usage can
//! still provide a fallback when the selected console refuses the key.
//!
//! Key source: pasted in Settings, `BAILIAN_TOKEN_PLAN_API_KEY` (the env
//! var Qwen Code itself reads), or `DASHSCOPE_API_KEY`.

use super::{http, stored_api_key, Metric, Snapshot};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

const ID: &str = "qwen";
const NAME: &str = "Qwen Code";

const RPC_QUERY: &str = "data/api.json?action=zeldaEasy.broadscope-bailian.codingPlan.queryCodingPlanInstanceInfoV2&product=broadscope-bailian&api=queryCodingPlanInstanceInfoV2";
const HOUR_MS: i64 = 3_600_000;

fn find_api_key() -> Option<String> {
    stored_api_key(ID, &["BAILIAN_TOKEN_PLAN_API_KEY", "DASHSCOPE_API_KEY"])
}

pub async fn snapshot() -> Snapshot {
    match fetch().await {
        Ok(s) => s,
        Err(e) => Snapshot::error(ID, NAME, e),
    }
}

async fn fetch() -> Result<Snapshot, String> {
    crate::network_policy::current_selection(ID)?;
    let key = find_api_key();
    if let Some(key) = &key {
        if let Some(snap) = fetch_quota(key).await {
            return Ok(snap);
        }
    }
    if let Some(snap) = local_ledger() {
        return Ok(snap);
    }
    match key {
        Some(_) => Err("quota endpoint unreachable and no local Qwen Code usage found".into()),
        None => Ok(Snapshot::no_credentials(
            ID,
            NAME,
            "Set BAILIAN_TOKEN_PLAN_API_KEY (Qwen Code's own variable) or paste your sk-sp-… key in Settings.",
        )),
    }
}

// One current refusal context per account: changing credential or mode drops
// the previous context. Only its SHA-256 equality fingerprint is retained in
// memory, never the raw key. Root settings/key changes also explicitly reset.
#[derive(Default)]
struct QuotaCooldowns {
    sequence: u64,
    accounts: HashMap<String, QuotaCooldown>,
}
struct QuotaCooldown {
    mode: String,
    key_hash: [u8; 32],
    sequence: u64,
    blocked_until: i64,
}
struct CooldownTicket {
    account: String,
    sequence: u64,
}
impl QuotaCooldowns {
    fn begin(&mut self, account: &str, mode: &str, key: &str, now: i64) -> Option<CooldownTicket> {
        let key_hash: [u8; 32] = Sha256::digest(key.as_bytes()).into();
        if self.accounts.get(account).is_some_and(|entry| {
            entry.mode == mode && entry.key_hash == key_hash && entry.blocked_until > now
        }) {
            return None;
        }
        self.sequence = self.sequence.wrapping_add(1);
        self.accounts.insert(
            account.into(),
            QuotaCooldown {
                mode: mode.into(),
                key_hash,
                sequence: self.sequence,
                blocked_until: 0,
            },
        );
        Some(CooldownTicket {
            account: account.into(),
            sequence: self.sequence,
        })
    }
    fn refuse(&mut self, ticket: &CooldownTicket, until: i64) {
        if let Some(entry) = self.accounts.get_mut(&ticket.account) {
            if entry.sequence == ticket.sequence {
                entry.blocked_until = until;
            }
        }
    }
    fn reset(&mut self) {
        self.sequence = self.sequence.wrapping_add(1);
        self.accounts.clear();
    }
}
static QUOTA_COOLDOWNS: OnceLock<Arc<Mutex<QuotaCooldowns>>> = OnceLock::new();
#[cfg(test)]
tokio::task_local! { static TEST_QUOTA_COOLDOWNS: Arc<Mutex<QuotaCooldowns>>; }
fn quota_cooldowns() -> Arc<Mutex<QuotaCooldowns>> {
    #[cfg(test)]
    if let Ok(state) = TEST_QUOTA_COOLDOWNS.try_with(Clone::clone) {
        return state;
    }
    QUOTA_COOLDOWNS.get_or_init(Default::default).clone()
}

/// Call from the backend when saving/clearing a Qwen key, changing its network
/// mode, or resetting settings. A late pre-reset response cannot rearm it.
pub(crate) fn reset_quota_cooldown() {
    quota_cooldowns()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .reset();
}

async fn fetch_quota(key: &str) -> Option<Snapshot> {
    let selection = crate::network_policy::current_selection(ID).ok()?;
    let console = crate::network_policy::selected_origin(ID).ok()?;
    let permit = crate::access_policy::current_operation()?;
    let now = chrono::Utc::now().timestamp();
    let cooldowns = quota_cooldowns();
    let ticket = cooldowns
        .lock()
        .ok()?
        .begin(permit.account_id(), &selection, key, now)?;
    let (header, value) = match selection.split_once(':')?.1 {
        "bearer" => ("authorization", format!("Bearer {key}")),
        "x_api_key" => ("x-api-key", key.to_string()),
        "dashscope_api_key" => ("x-dashscope-api-key", key.to_string()),
        _ => return None,
    };
    let resp = http(ID)
        .post(format!("{console}/{RPC_QUERY}"))
        .header(header, value)
        .header("accept", "application/json")
        .json(&serde_json::json!({}))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        if matches!(resp.status().as_u16(), 401 | 403) && permit.check().is_ok() {
            cooldowns.lock().ok()?.refuse(&ticket, now + 6 * 3600);
        }
        return None;
    }
    let doc = resp.json::<Value>().await.ok()?;
    if let Some(snap) = parse_quota(&doc) {
        return Some(snap);
    }
    if permit.check().is_ok() {
        cooldowns.lock().ok()?.refuse(&ticket, now + 6 * 3600);
    }
    None
}

/// Depth-first search for the object carrying the quota fields — the RPC
/// wraps its payload in envelope layers we'd rather not hardcode.
fn find_object_with<'a>(v: &'a Value, marker: &str) -> Option<&'a Value> {
    match v {
        Value::Object(m) => {
            if m.contains_key(marker) {
                return Some(v);
            }
            m.values().find_map(|v| find_object_with(v, marker))
        }
        Value::Array(a) => a.iter().find_map(|v| find_object_with(v, marker)),
        _ => None,
    }
}

/// Numbers may arrive as JSON numbers or quoted strings; take either.
fn num(v: &Value, key: &str) -> Option<f64> {
    let f = v.get(key)?;
    f.as_f64()
        .or_else(|| f.as_str().and_then(|s| s.trim().parse().ok()))
}

fn parse_quota(doc: &Value) -> Option<Snapshot> {
    let q = find_object_with(doc, "per5HourTotalQuota")?;
    let window = |label: &str, used_key: &str, total_key: &str, reset_key: &str, period: i64| {
        let total = num(q, total_key).filter(|t| *t > 0.0)?;
        let used = num(q, used_key).unwrap_or(0.0);
        let resets = num(q, reset_key).map(|ms| ms as i64).filter(|ms| *ms > 0);
        Some(
            Metric::progress(
                label,
                (used / total * 100.0).clamp(0.0, 100.0),
                Some(format!("{used:.0} of {total:.0} requests")),
            )
            .with_reset(resets, Some(period)),
        )
    };
    let metrics: Vec<Metric> = [
        window(
            "Session",
            "per5HourUsedQuota",
            "per5HourTotalQuota",
            "per5HourQuotaNextRefreshTime",
            5 * HOUR_MS,
        ),
        window(
            "Weekly",
            "perWeekUsedQuota",
            "perWeekTotalQuota",
            "perWeekQuotaNextRefreshTime",
            7 * 24 * HOUR_MS,
        ),
        window(
            "Monthly",
            "perBillMonthUsedQuota",
            "perBillMonthTotalQuota",
            "perBillMonthQuotaNextRefreshTime",
            30 * 24 * HOUR_MS,
        ),
    ]
    .into_iter()
    .flatten()
    .collect();
    if metrics.is_empty() {
        return None;
    }
    let plan = find_object_with(doc, "planName")
        .and_then(|o| o.get("planName").and_then(Value::as_str))
        .map(str::to_string)
        .or(Some("Coding Plan".into()));
    Some(Snapshot::ok(ID, NAME, plan, metrics))
}

/// Fallback card from the CLI's own per-request ledger: request and token
/// counts for today and the current month. No percentages — the plan's
/// limits aren't knowable locally.
fn local_ledger() -> Option<Snapshot> {
    // Local rows carry no account ownership proof and must not enter the
    // query-snapshot cache. Their request counts are in source:qwen instead.
    Some(Snapshot::error(ID, NAME,
        "Server quota is unavailable. Qwen request counts and tokens are shown separately when its directory is enabled under Local log sources.".into()))
}

#[cfg(test)]
pub(crate) async fn with_isolated_quota_cooldown<T>(
    future: impl std::future::Future<Output = T>,
) -> T {
    TEST_QUOTA_COOLDOWNS
        .scope(Arc::new(Mutex::new(QuotaCooldowns::default())), future)
        .await
}

#[cfg(test)]
pub(crate) async fn quota_with_synthetic_key(key: &str) -> Option<Snapshot> {
    if TEST_QUOTA_COOLDOWNS.try_with(|_| ()).is_ok() {
        fetch_quota(key).await
    } else {
        // The complete request operation, not just reset(), is isolated from
        // other test tasks. Explicit outer scopes can test repeated requests.
        with_isolated_quota_cooldown(fetch_quota(key)).await
    }
}
