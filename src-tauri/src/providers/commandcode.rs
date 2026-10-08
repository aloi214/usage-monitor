//! Experimental, manually queried read-only CommandCode billing adapter.
use super::{Metric, Snapshot};
use serde_json::Value;
const ID: &str = "commandcode";
const NAME: &str = "CommandCode";

/// Caller holds the existing key-mutation/publication guard for this whole
/// transaction. Invalidate persisted history BEFORE changing the local key so
/// an unsuccessful cleanup cannot leave a new key next to old account data.
pub(crate) fn save_key_in(
    dir: &std::path::Path,
    key: &str,
    clear: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let path = dir.join("commandcode.json");
    clear().map_err(|error| {
        format!("CommandCode key unchanged; clearing cached data failed: {error}")
    })?;
    if key.is_empty() {
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("remove CommandCode key file: {error}")),
        }
    } else {
        crate::private_file::atomic_write(&path, &serde_json::json!({"apiKey":key}).to_string())
            .map_err(|error| format!("write CommandCode key file: {error}"))
    }
}

/// Only called by an explicitly requested, authorized refresh. Automatic cache
/// painting deliberately never consults the saved key's identity.
pub fn default_identity() -> Option<String> {
    super::stored_api_key(ID, &[]).map(|key| super::key_fingerprint(&key))
}

const MAX_RESPONSE_BYTES: usize = 64 * 1024;

pub async fn snapshot() -> Snapshot {
    match fetch().await {
        Ok(snapshot) => snapshot,
        Err(error) => Snapshot::error(ID, NAME, error),
    }
}
async fn fetch() -> Result<Snapshot, String> {
    // Check the exact provider/operation grant before any credential lookup.
    let base = crate::network_policy::selected_origin(ID)?;
    let Some(key) = super::stored_api_key(ID, &[]) else {
        return Ok(Snapshot::no_credentials(
            ID,
            NAME,
            "Paste a CommandCode API key in Settings, then refresh manually.",
        ));
    };
    let credits = billing(base, "credits", &key).await?;
    // Validate required quota before a second request, including HTTP-200 errors.
    parse_snapshot(&credits, None, None)?;
    crate::access_policy::check_current_operation()?;
    if super::stored_api_key(ID, &[]).as_deref() != Some(key.as_str()) {
        return Err("CommandCode key changed during refresh; refresh manually again".into());
    }
    let subscription = billing(base, "subscriptions", &key).await;
    crate::access_policy::check_current_operation()?;
    if super::stored_api_key(ID, &[]).as_deref() != Some(key.as_str()) {
        return Err("CommandCode key changed during refresh; refresh manually again".into());
    }
    match subscription {
        Ok(subscription) => parse_snapshot(&credits, Some(&subscription), None),
        Err(error) => parse_snapshot(&credits, None, Some(&error)),
    }
}
async fn billing(base: &str, operation: &str, key: &str) -> Result<Value, String> {
    let response = super::http(ID)
        .get(format!("{base}/alpha/billing/{operation}"))
        .bearer_auth(key)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|error| format!("CommandCode {operation}: {error}"))?;
    if !response.status().is_success() {
        let retry = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| {
                v.parse::<u64>().ok().or_else(|| {
                    chrono::DateTime::parse_from_rfc2822(v).ok().map(|date| {
                        (date.timestamp() - chrono::Utc::now().timestamp()).max(0) as u64
                    })
                })
            })
            .map(|seconds| format!(" retry_after_s={}", seconds.min(3600)))
            .unwrap_or_default();
        return Err(format!(
            "CommandCode {operation}: HTTP {}{retry}",
            response.status().as_u16()
        ));
    }
    // Never surface bodies or parser excerpts that may contain account/key data.
    super::json_body(response, MAX_RESPONSE_BYTES, "CommandCode billing")
        .await
        .map_err(|_| format!("CommandCode {operation}: invalid or oversized JSON response"))
}
fn rejected(doc: &Value) -> bool {
    !doc.is_object()
        || doc.get("success") == Some(&Value::Bool(false))
        || doc.get("error").is_some_and(|v| !v.is_null())
}
fn number(node: Option<&Value>) -> Option<f64> {
    // Numeric strings are accepted only as finite, bounded credit quantities.
    let n = match node? {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) => s.parse().ok()?,
        _ => return None,
    };
    (n.is_finite() && (0.0..=1e12).contains(&n)).then_some(n)
}
fn timestamp(node: Option<&Value>) -> Option<i64> {
    let value = node?;
    let n = value
        .as_i64()
        .or_else(|| value.as_str()?.parse::<i64>().ok());
    let ms = if let Some(n) = n {
        if n < 100_000_000_000 {
            n.checked_mul(1000)?
        } else {
            n
        }
    } else {
        chrono::DateTime::parse_from_rfc3339(value.as_str()?)
            .ok()?
            .timestamp_millis()
    };
    // Reject implausible units/overflow instead of manufacturing a reset.
    (946_684_800_000..=4_102_444_800_000)
        .contains(&ms)
        .then_some(ms)
}
fn parse_snapshot(
    doc: &Value,
    subscription: Option<&Value>,
    subscription_error: Option<&str>,
) -> Result<Snapshot, String> {
    if rejected(doc) {
        return Err("CommandCode credits: unsuccessful or invalid response".into());
    }
    let data = doc.get("data").filter(|v| v.is_object()).unwrap_or(doc);
    if rejected(data) {
        return Err("CommandCode credits: unsuccessful or invalid response".into());
    }
    let credits = data.get("credits");
    let windows = data.get("windowLimits");
    let mut metrics = Vec::new();
    let mut warnings = Vec::new();
    let mut valid_quota = false;
    for (key, label) in [
        ("monthlyCredits", "Monthly credits remaining"),
        ("purchasedCredits", "Purchased credits"),
        ("freeCredits", "Free credits"),
    ] {
        if let Some(n) = number(credits.and_then(|v| v.get(key))) {
            metrics.push(Metric::text(label, format!("{n} credits")));
            valid_quota = true;
        } else if key == "monthlyCredits" || credits.is_some_and(|v| v.get(key).is_some()) {
            warnings.push(format!("{label} unavailable"));
        }
    }
    for (key, label, duration) in [
        ("fiveHour", "5-hour", 18_000_000),
        ("weekly", "Weekly", 604_800_000),
    ] {
        let window = windows.and_then(|v| v.get(key));
        let used = number(window.and_then(|v| v.get("used")));
        let cap = number(window.and_then(|v| v.get("cap"))).filter(|v| *v > 0.0);
        let reset = timestamp(window.and_then(|v| v.get("resetAt")));
        if let (Some(used), Some(cap)) = (used, cap) {
            let exceeded = window
                .and_then(|v| v.get("exceeded"))
                .and_then(Value::as_bool)
                == Some(true)
                || used >= cap;
            let detail = format!(
                "{used} of {cap} credits used{}",
                if exceeded { " · Limit reached" } else { "" }
            );
            metrics.push(
                Metric::progress(label, (used / cap * 100.0).clamp(0.0, 100.0), Some(detail))
                    .with_reset(reset, Some(duration)),
            );
            valid_quota = true;
            if reset.is_none() {
                warnings.push(format!("{label} reset time unknown"));
            }
        } else {
            metrics.push(Metric::text(label, "Unknown".into()));
            warnings.push(format!("{label} usage unavailable"));
        }
    }
    if !valid_quota {
        return Err(
            "CommandCode credits: unknown quota response shape (experimental endpoint)".into(),
        );
    }
    let metadata = subscription
        .filter(|doc| !rejected(doc))
        .and_then(|doc| doc.get("data").or_else(|| doc.get("subscription")))
        .filter(|v| v.is_object() && !rejected(v));
    if let Some(error) = subscription_error {
        warnings.push(error.to_string());
    } else if metadata.is_none() {
        warnings.push("Subscription metadata unavailable".into());
    }
    let plan_id = metadata
        .and_then(|v| v.get("planId"))
        .and_then(Value::as_str)
        .or_else(|| {
            credits
                .and_then(|v| v.get("planId"))
                .and_then(Value::as_str)
        });
    let plan = match plan_id {
        Some("individual-goat") => "GOAT · Experimental",
        _ => "Experimental",
    };
    if let Some(end) = timestamp(metadata.and_then(|v| v.get("currentPeriodEnd"))) {
        // A billing period is metadata, not evidence of a new monthly balance.
        let value = chrono::DateTime::from_timestamp_millis(end)
            .unwrap()
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        metrics.push(Metric::text("Subscription period ends", value));
    } else if metadata.is_some() {
        warnings.push("Subscription period end unknown".into());
    }
    let mut snapshot = Snapshot::ok(ID, NAME, Some(plan.into()), metrics);
    if !warnings.is_empty() {
        snapshot.warning = Some(warnings.join("; "));
    }
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn credits() -> Value {
        json!({"credits":{"monthlyCredits":52.5,"purchasedCredits":0,"freeCredits":1.25,"planId":"individual-goat"},"windowLimits":{"fiveHour":{"used":3.5,"cap":14,"resetAt":1893474000000_i64},"weekly":{"used":7,"cap":35,"resetAt":"2030-01-08T00:00:00Z"}}})
    }
    fn subscription() -> Value {
        json!({"success":true,"data":{"planId":"individual-goat","status":"active","currentPeriodStart":1893456000_i64,"currentPeriodEnd":1896134400_i64}})
    }
    #[test]
    fn key_cleanup_failure_keeps_old_file_and_success_cleans_before_writing_or_clearing() {
        let dir = std::env::temp_dir().join(format!(
            "pane-commandcode-key-fixture-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("commandcode.json");
        std::fs::write(&path, r#"{"apiKey":"old-synthetic"}"#).unwrap();
        assert!(save_key_in(&dir, "new-synthetic", || Err(
            "synthetic cleanup failure".into()
        ))
        .is_err());
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("old-synthetic"));
        save_key_in(&dir, "new-synthetic", || {
            assert!(std::fs::read_to_string(&path)
                .unwrap()
                .contains("old-synthetic"));
            Ok(())
        })
        .unwrap();
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("new-synthetic"));
        assert!(save_key_in(&dir, "", || Err("synthetic cleanup failure".into())).is_err());
        assert!(path.exists());
        save_key_in(&dir, "", || Ok(())).unwrap();
        assert!(!path.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn real_credit_shape_uses_server_caps_and_credit_units() {
        let s = parse_snapshot(&credits(), Some(&subscription()), None).unwrap();
        assert_eq!(s.plan.as_deref(), Some("GOAT · Experimental"));
        let five = s.metrics.iter().find(|m| m.label == "5-hour").unwrap();
        assert_eq!(five.used_percent, Some(25.0));
        assert_eq!(five.resets_at, Some(1893474000000));
        assert_eq!(five.period_ms, Some(18_000_000));
        assert!(five.detail.as_ref().unwrap().contains("credits"));
        assert!(s
            .metrics
            .iter()
            .any(|m| m.label == "Monthly credits remaining"
                && m.value.as_deref() == Some("52.5 credits")));
        assert!(s
            .metrics
            .iter()
            .any(|m| m.label == "Purchased credits" && m.value.as_deref() == Some("0 credits")));
        assert!(!serde_json::to_string(&s).unwrap().contains('$'));
        assert!(s.warning.is_none());
    }
    #[test]
    fn partial_and_unknown_values_never_become_zero_or_fake_success() {
        for bad in [
            Value::Null,
            json!("bad"),
            json!(true),
            json!(-1),
            json!(1e100),
        ] {
            let d = json!({"credits":{"monthlyCredits":bad},"windowLimits":{"fiveHour":{"used":bad,"cap":14},"weekly":{"used":0,"cap":0}}});
            assert!(parse_snapshot(&d, None, None).is_err());
        }
        for doc in [
            json!({}),
            json!({"success":false,"credits":{"monthlyCredits":1}}),
            json!({"error":"synthetic-secret","credits":{"monthlyCredits":1}}),
        ] {
            let error = parse_snapshot(&doc, None, None).err().unwrap();
            assert!(!error.contains("synthetic-secret"));
        }
        let s = parse_snapshot(
            &json!({"credits":{"monthlyCredits":0}}),
            None,
            Some("subscription unavailable"),
        )
        .unwrap();
        assert!(s.warning.is_some());
        assert!(s
            .metrics
            .iter()
            .filter(|m| m.kind == "progress")
            .next()
            .is_none());
    }
    #[test]
    fn metadata_failure_keeps_credits_without_inventing_subscription_period() {
        let s = parse_snapshot(&credits(), None, Some("subscriptions: HTTP 503")).unwrap();
        assert!(s
            .metrics
            .iter()
            .any(|m| m.label == "Monthly credits remaining"));
        assert!(!s
            .metrics
            .iter()
            .any(|m| m.label == "Subscription period ends"));
        assert!(s.warning.as_ref().unwrap().contains("503"));
        let unknown = json!({"data":{"planId":"individual-goat-future","currentPeriodEnd":null}});
        let s = parse_snapshot(&credits(), Some(&unknown), None).unwrap();
        assert_ne!(s.plan.as_deref(), Some("GOAT · Experimental"));
        assert!(!s.metrics.iter().any(|m| m.label.contains("inferred")));
    }
    #[test]
    fn missing_malformed_and_elapsed_resets_are_never_inferred_or_rolled() {
        for bad in [
            Value::Null,
            json!(false),
            json!("not-a-date"),
            json!(-1),
            json!(1e100),
        ] {
            let mut d = credits();
            d["windowLimits"]["fiveHour"]["resetAt"] = bad;
            let s = parse_snapshot(&d, Some(&subscription()), None).unwrap();
            let m = s.metrics.iter().find(|m| m.label == "5-hour").unwrap();
            assert!(m.resets_at.is_none());
            assert!(s.warning.is_some());
        }
        let mut d = credits();
        d["windowLimits"]["fiveHour"] =
            json!({"used":18,"cap":14,"resetAt":946684800000_i64,"exceeded":true});
        let s = parse_snapshot(&d, Some(&subscription()), None).unwrap();
        let m = s.metrics.iter().find(|m| m.label == "5-hour").unwrap();
        assert_eq!(m.used_percent, Some(100.0));
        assert_eq!(m.resets_at, Some(946684800000));
        assert!(m.detail.as_ref().unwrap().contains("18"));
    }
    #[test]
    fn nested_envelope_and_subscription_failure_are_bounded() {
        let s = parse_snapshot(
            &json!({"success":true,"data":credits()}),
            Some(&json!({"success":false,"error":"private"})),
            None,
        )
        .unwrap();
        assert!(s.warning.is_some());
        assert!(!serde_json::to_string(&s).unwrap().contains("private"));
    }
}
