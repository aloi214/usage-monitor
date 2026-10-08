//! Portable provider snapshot model.
use serde::{Deserialize, Serialize};

/// One row inside a provider card, e.g. "Session ▓▓▓░░ 43% left · Resets in 2h".
/// `resets_at` (epoch ms) + `period_ms` are the structured facts the pace
/// engine needs; the UI formats countdowns and projections from them.
#[derive(Serialize, Deserialize, Clone)]
pub struct Metric {
    pub label: String,
    pub kind: String, // "progress" | "text" | "action" | "resets"
    pub used_percent: Option<f64>,
    pub detail: Option<String>,
    pub value: Option<String>,
    pub resets_at: Option<i64>,
    pub period_ms: Option<i64>,
    /// True when `resets_at` is an expiry — the row's value is lost at
    /// that moment, not renewed. Skipped in JSON when false so older
    /// caches and snapshots decode unchanged.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub expires: bool,
}

/// One banked rate-limit reset credit. `id` is present when Pane can redeem
/// it (Codex); Grok's are read-only.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ResetCredit {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Epoch ms; None when the API gave no expiry.
    pub expires_at: Option<i64>,
}

impl Metric {
    pub fn progress(label: &str, used_percent: f64, detail: Option<String>) -> Self {
        Self {
            label: label.into(),
            kind: "progress".into(),
            used_percent: Some(used_percent),
            detail,
            value: None,
            resets_at: None,
            period_ms: None,
            expires: false,
        }
    }

    #[allow(dead_code)]
    pub fn text(label: &str, value: String) -> Self {
        Self {
            label: label.into(),
            kind: "text".into(),
            used_percent: None,
            detail: None,
            value: Some(value),
            resets_at: None,
            period_ms: None,
            expires: false,
        }
    }

    /// "Rate Limit Resets": one row for all banked credits — the count in
    /// `value`, the per-credit list (soonest first) JSON-encoded in `detail`,
    /// the soonest expiry in `resets_at`. `credits` None = the count came from
    /// a source without per-credit expiries.
    pub fn resets(count: usize, credits: Option<Vec<ResetCredit>>) -> Self {
        let mut credits = credits;
        if let Some(c) = credits.as_mut() {
            c.sort_by_key(|credit| credit.expires_at.unwrap_or(i64::MAX));
        }
        Self {
            label: "Rate Limit Resets".into(),
            kind: "resets".into(),
            used_percent: None,
            detail: credits.as_ref().and_then(|c| serde_json::to_string(c).ok()),
            value: Some(count.to_string()),
            resets_at: credits
                .as_ref()
                .and_then(|c| c.iter().filter_map(|credit| credit.expires_at).min()),
            period_ms: None,
            expires: false,
        }
    }

    pub fn with_reset(mut self, resets_at: Option<i64>, period_ms: Option<i64>) -> Self {
        self.resets_at = resets_at;
        self.period_ms = period_ms;
        self
    }

    /// Marks `resets_at` as an expiry: the row's remaining value is
    /// lost at that moment rather than renewed, so the frontend counts
    /// down and no reset machinery may treat it as a rollover.
    pub fn with_expiry(mut self, expires_at: Option<i64>) -> Self {
        self.resets_at = expires_at;
        self.period_ms = None;
        self.expires = true;
        self
    }

    /// True for an expiring row whose deadline has passed — the value
    /// is gone even if the last API reading still shows a balance.
    pub fn expired_at(&self, now_ms: i64) -> bool {
        self.expires && self.resets_at.is_some_and(|r| r <= now_ms)
    }
}

/// Provenance of Moonshot rows retained during a Kimi plan-only refresh.
/// Missing clocks remain unknown; a new plan response cannot freshen a wallet.
#[derive(Serialize, Deserialize, Clone)]
pub struct WalletHistory {
    pub source_account_id: String,
    pub fetched_at: Option<i64>,
    pub attempted_at: Option<i64>,
    pub warning: Option<String>,
}

/// Everything one provider reports back after a refresh. `stale` marks a
/// snapshot that is historical: a saved manual result, or the last good fetch
/// after a failed attempt (`warning` carries that error). `fetched_at`
/// is when this data was last successfully fetched (epoch ms) — it rides
/// along so a restored snapshot can't pose as a fresh success downstream
/// (local HTTP API). `None` means unknown (old caches, before first
/// success). `attempt_failed` is set on every restore so the API can
/// report staleness during the UI's 3-minute grace window.
#[derive(Serialize, Deserialize, Clone)]
pub struct Snapshot {
    pub id: String,
    pub name: String,
    pub plan: Option<String>,
    pub status: String, // "ok" | "no_credentials" | "error" | "manual"
    pub error: Option<String>,
    pub metrics: Vec<Metric>,
    pub stale: bool,
    pub warning: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fetched_at: Option<i64>,
    /// Real manual query attempt time. Separate from last successful fetch so
    /// replaying a failed first query cannot acquire a new publication clock.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempted_at: Option<i64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub attempt_failed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dashboard_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wallet_history: Option<WalletHistory>,
    /// Ephemeral composite-result metadata. Never persisted or sent over IPC:
    /// a nested Moonshot permit was skipped, so merge saved wallet history.
    #[serde(skip)]
    pub wallet_refresh_skipped: bool,
}

impl Snapshot {
    pub fn has_saved_wallet_metric(&self, label: &str) -> bool {
        self.id == "kimi"
            && self.wallet_history.is_some()
            && matches!(
                label,
                "API" | "Credits used" | "Balance" | "Vouchers" | "Cash"
            )
    }

    pub fn ok(id: &str, name: &str, plan: Option<String>, metrics: Vec<Metric>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            plan,
            status: "ok".into(),
            error: None,
            metrics,
            stale: false,
            warning: None,
            fetched_at: None,
            attempted_at: None,
            attempt_failed: false,
            dashboard_url: None,
            wallet_history: None,
            wallet_refresh_skipped: false,
        }
    }

    pub fn no_credentials(id: &str, name: &str, hint: &str) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            plan: None,
            status: "no_credentials".into(),
            error: Some(hint.into()),
            metrics: vec![],
            stale: false,
            warning: None,
            fetched_at: None,
            attempted_at: None,
            attempt_failed: false,
            dashboard_url: None,
            wallet_history: None,
            wallet_refresh_skipped: false,
        }
    }

    pub fn error(id: &str, name: &str, message: String) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            plan: None,
            status: "error".into(),
            error: Some(message),
            metrics: vec![],
            stale: false,
            warning: None,
            fetched_at: None,
            attempted_at: None,
            attempt_failed: false,
            dashboard_url: None,
            wallet_history: None,
            wallet_refresh_skipped: false,
        }
    }
}
