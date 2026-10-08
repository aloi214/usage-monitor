use super::{http, stored_api_key, Metric, Snapshot};
use serde_json::Value;

const ID: &str = "zai";
const NAME: &str = "Z.ai";
const MAX_QUOTA_BYTES: usize = 64 * 1024;

fn find_key() -> Option<String> {
    if let Some(key) = stored_api_key("zai", &["ZAI_API_KEY", "GLM_API_KEY"]) {
        return Some(key);
    }
    // The Z.ai CLI's own key file.
    let path = dirs::home_dir()?.join(".config").join("zai").join("key.json");
    let raw = std::fs::read_to_string(path).ok()?;
    let doc: Value = serde_json::from_str(&raw).ok()?;
    doc.get("apiKey")
        .or_else(|| doc.get("api_key"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

pub async fn snapshot() -> Snapshot {
    match fetch().await {
        Ok(s) => s,
        Err(e) => Snapshot::error(ID, NAME, e),
    }
}

async fn fetch() -> Result<Snapshot, String> {
    let base = crate::network_policy::selected_origin(ID)?;
    let Some(key) = find_key() else {
        return Ok(Snapshot::no_credentials(
            ID,
            NAME,
            "Paste a Z.ai API key in Settings (gear icon).",
        ));
    };
    fetch_at(base, &key).await.map_err(|failure| match failure {
        SiteFailure::WrongSite(e) | SiteFailure::RateLimited(e) | SiteFailure::SiteError(e) => e,
    })
}

enum SiteFailure {
    WrongSite(String),
    RateLimited(String),
    SiteError(String),
}

async fn fetch_at(base: &str, key: &str) -> Result<Snapshot, SiteFailure> {
    let quota_req = http(ID)
        .get(format!("{base}/api/monitor/usage/quota/limit"))
        .bearer_auth(key)
        .send();
    let plan_req = http(ID)
        .get(format!("{base}/api/biz/subscription/list"))
        .bearer_auth(key)
        .send();
    let (quota_resp, plan_resp) = tokio::join!(quota_req, plan_req);

    // A failure belongs to the selected region; never probe a sibling.
    let quota_resp =
        quota_resp.map_err(|e| SiteFailure::WrongSite(format!("quota request: {e}")))?;
    if quota_resp.status().as_u16() == 401 {
        return Err(SiteFailure::WrongSite(
            "API key was rejected — check it in Settings".into(),
        ));
    }
    if quota_resp.status().as_u16() == 429 {
        return Err(SiteFailure::RateLimited(format!(
            "quota endpoint: HTTP {}",
            quota_resp.status()
        )));
    }
    if !quota_resp.status().is_success() {
        return Err(SiteFailure::SiteError(format!(
            "quota endpoint: HTTP {}",
            quota_resp.status()
        )));
    }
    let quota: Value = super::json_body(quota_resp, MAX_QUOTA_BYTES, "quota")
        .await
        .map_err(SiteFailure::SiteError)?;

    let mut metrics = Vec::new();
    collect_quota_metrics(quota.get("data").unwrap_or(&quota), &mut metrics);
    if metrics.is_empty() {
        return Err(SiteFailure::SiteError(
            "unexpected quota response shape (endpoint is undocumented)".into(),
        ));
    }

    let mut plan = None;
    if let Ok(resp) = plan_resp {
        if resp.status().is_success() {
            if let Ok(doc) = super::json_body(resp, MAX_QUOTA_BYTES, "plan").await {
                plan = find_plan_name(doc.get("data").unwrap_or(&doc));
            }
        }
    }

    Ok(Snapshot::ok(ID, NAME, plan, metrics))
}

/// The quota endpoint is undocumented, so we parse tolerantly: any object
/// carrying a usage/limit pair (or a percentage) becomes a meter.
fn collect_quota_metrics(node: &Value, metrics: &mut Vec<Metric>) {
    collect_quota_metrics_inner(node, metrics);
    // Preserve the two recognized token windows even when unknown quotas arrive first.
    metrics.sort_by_key(|metric| match (metric.label.as_str(), metric.period_ms) {
        ("5 Hours", Some(18_000_000)) => 0,
        ("Weekly", Some(604_800_000)) => 1,
        _ => 2,
    });
    metrics.truncate(5);
}

fn collect_quota_metrics_inner(node: &Value, metrics: &mut Vec<Metric>) {
    match node {
        Value::Array(items) => {
            for item in items {
                collect_quota_metrics_inner(item, metrics);
            }
        }
        Value::Object(map) => {
            // TIME_LIMIT is the separate web-search quota: currentValue is used,
            // usage is capacity. Keep its existing reset handling unchanged.
            let type_name = ["type", "name"]
                .iter()
                .find_map(|k| map.get(*k).and_then(Value::as_str));
            if type_name == Some("TIME_LIMIT") {
                let used = map.get("currentValue").and_then(Value::as_f64).unwrap_or(0.0).max(0.0);
                let cap = map.get("usage").and_then(Value::as_f64).unwrap_or(0.0).max(0.0);
                if cap > 0.0 {
                    let resets_at = map
                        .get("nextResetTime")
                        .and_then(Value::as_i64)
                        .filter(|ms| *ms > 0);
                    metrics.push(
                        Metric::progress(
                            "Web Searches",
                            (used / cap * 100.0).clamp(0.0, 100.0),
                            Some(format!("{used:.0} of {cap:.0} searches")),
                        )
                        .with_reset(resets_at, Some(30 * 86_400_000)),
                    );
                }
                return;
            }

            if type_name == Some("TOKENS_LIMIT") {
                // Observed provider schema: unit=3 means hours; unit=6 means weeks.
                // Do not infer a window from array position or the quota type alone.
                let (label, period_ms) = match (
                    map.get("unit").and_then(Value::as_i64),
                    map.get("number").and_then(Value::as_i64),
                ) {
                    (Some(3), Some(5)) => ("5 Hours".to_string(), Some(18_000_000)),
                    (Some(6), Some(1)) => ("Weekly".to_string(), Some(604_800_000)),
                    _ => {
                        let cycle: Vec<String> = ["unit", "number"]
                            .iter()
                            .filter_map(|key| map.get(*key).map(|value| format!("{key}={value}")))
                            .collect();
                        let label = if cycle.is_empty() {
                            "TOKENS_LIMIT".to_string()
                        } else {
                            format!("TOKENS_LIMIT ({})", cycle.join(", "))
                        };
                        (label, None)
                    }
                };
                let percent = ["percentage", "percent", "usagePercent"]
                    .iter()
                    .find_map(|key| map.get(*key).and_then(Value::as_f64));
                let metric = if let Some(percent) = percent {
                    Some(Metric::progress(&label, percent, None))
                } else {
                    // For token quotas, usage is capacity and currentValue is used.
                    // Missing absolute values must never become a fabricated zero.
                    map.get("currentValue")
                        .and_then(Value::as_f64)
                        .zip(
                            map.get("usage")
                                .and_then(Value::as_f64)
                                .filter(|cap| *cap > 0.0),
                        )
                        .map(|(used, cap)| {
                            Metric::progress(
                                &label,
                                used / cap * 100.0,
                                Some(format!("{used:.0} of {cap:.0}")),
                            )
                        })
                };
                if let Some(metric) = metric {
                    let resets_at = map
                        .get("nextResetTime")
                        .and_then(Value::as_i64)
                        .filter(|ms| *ms > 0);
                    metrics.push(metric.with_reset(resets_at, period_ms));
                }
                return;
            }

            let label = ["type", "name", "unit", "quotaType"]
                .iter()
                .find_map(|k| map.get(*k).and_then(Value::as_str))
                .map(nice_label)
                .unwrap_or_else(|| "Quota".to_string());

            let used = ["usage", "used", "currentValue", "current"]
                .iter()
                .find_map(|k| map.get(*k).and_then(Value::as_f64));
            let limit = ["limit", "total", "maxValue", "max"]
                .iter()
                .find_map(|k| map.get(*k).and_then(Value::as_f64));
            let percent = ["percentage", "percent", "usagePercent"]
                .iter()
                .find_map(|k| map.get(*k).and_then(Value::as_f64));

            if let Some(p) = percent {
                metrics.push(Metric::progress(&label, p, None));
            } else if let (Some(u), Some(l)) = (used, limit) {
                if l > 0.0 {
                    metrics.push(Metric::progress(
                        &label,
                        u / l * 100.0,
                        Some(format!("{u:.0} of {l:.0}")),
                    ));
                }
            } else {
                for value in map.values() {
                    collect_quota_metrics_inner(value, metrics);
                }
            }
        }
        _ => {}
    }
}

fn nice_label(raw: &str) -> String {
    let lower = raw.to_lowercase();
    if lower.contains("5h") || lower.contains("five") || lower.contains("session") {
        "Session".to_string()
    } else if lower.contains("7d") || lower.contains("week") {
        "Weekly".to_string()
    } else if lower.contains("search") {
        "Web searches".to_string()
    } else {
        raw.to_string()
    }
}

fn find_plan_name(node: &Value) -> Option<String> {
    match node {
        Value::Array(items) => items.iter().find_map(find_plan_name),
        Value::Object(map) => ["productName", "planName", "plan", "name"]
            .iter()
            .find_map(|k| map.get(*k).and_then(Value::as_str))
            .map(str::to_string)
            .or_else(|| map.values().find_map(find_plan_name)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(value: Value) -> Vec<Metric> {
        let mut metrics = Vec::new();
        collect_quota_metrics(&value, &mut metrics);
        metrics
    }

    #[test]
    fn domestic_token_windows_keep_distinct_percentages_and_resets() {
        // Synthetic values; schema verified against CodexBar's BigModel CN fixture.
        let rows = json!([
            {"type":"TOKENS_LIMIT", "unit":3, "number":5, "percentage":8,
             "nextResetTime":1783049703178_i64},
            {"type":"TOKENS_LIMIT", "unit":6, "number":1, "percentage":7,
             "nextResetTime":1783496744998_i64}
        ]);
        for input in [rows.clone(), json!([rows[1], rows[0]])] {
            let metrics = parse(json!({"limits":input}));
            assert_eq!(metrics.len(), 2);
            let session = metrics
                .iter()
                .find(|m| m.label == "5 Hours")
                .expect("5-hour row");
            let weekly = metrics
                .iter()
                .find(|m| m.label == "Weekly")
                .expect("weekly row");
            assert_eq!(session.used_percent, Some(8.0));
            assert_eq!(weekly.used_percent, Some(7.0));
            assert_eq!(session.resets_at, Some(1783049703178));
            assert_eq!(weekly.resets_at, Some(1783496744998));
            assert_eq!(session.period_ms, Some(18_000_000));
            assert_eq!(weekly.period_ms, Some(604_800_000));
            assert!(metrics.iter().all(|m| m.detail.is_none()));
        }
    }

    #[test]
    fn single_token_window_does_not_invent_another_window() {
        for (unit, number, label) in [(3, 5, "5 Hours"), (6, 1, "Weekly")] {
            let metrics = parse(json!({"type":"TOKENS_LIMIT", "unit":unit,
                "number":number, "percentage":0}));
            assert_eq!(metrics.len(), 1);
            assert_eq!(metrics[0].label, label);
            assert_eq!(metrics[0].used_percent, Some(0.0));
            assert_eq!(metrics[0].resets_at, None);
        }
        assert!(parse(json!({"type":"TOKENS_LIMIT", "unit":3, "number":5})).is_empty());
    }

    #[test]
    fn token_absolute_usage_uses_current_value_against_usage_capacity() {
        let metrics = parse(json!({"type":"TOKENS_LIMIT", "unit":3, "number":5,
            "usage":100, "currentValue":20, "remaining":80}));
        assert_eq!(metrics.len(), 1);
        assert_eq!(metrics[0].used_percent, Some(20.0));
        assert_eq!(metrics[0].detail.as_deref(), Some("20 of 100"));
    }

    #[test]
    fn token_percentage_zero_is_not_replaced_by_absolute_or_missing_values() {
        let metrics = parse(json!({"type":"TOKENS_LIMIT", "unit":3, "number":5,
            "usage":100, "currentValue":20, "percentage":0}));
        assert_eq!(metrics[0].used_percent, Some(0.0));
        assert!(metrics[0].detail.is_none());
    }

    #[test]
    fn unknown_token_windows_preserve_raw_cycle_without_assumed_period() {
        let metrics = parse(json!([
            {"type":"TOKENS_LIMIT", "unit":9, "number":2, "percentage":10,
             "nextResetTime":1783049703178_i64},
            {"type":"TOKENS_LIMIT", "unit":9, "number":3, "percentage":20},
            {"type":"TOKENS_LIMIT", "unit":3, "number":2, "percentage":30},
            {"type":"TOKENS_LIMIT", "percentage":40}
        ]));
        assert_eq!(metrics.len(), 4);
        assert_eq!(metrics[0].label, "TOKENS_LIMIT (unit=9, number=2)");
        assert_eq!(metrics[1].label, "TOKENS_LIMIT (unit=9, number=3)");
        assert_eq!(metrics[2].label, "TOKENS_LIMIT (unit=3, number=2)");
        assert_eq!(metrics[3].label, "TOKENS_LIMIT");
        assert!(metrics.iter().all(|m| m.period_ms.is_none()));
        assert_eq!(metrics[0].resets_at, Some(1783049703178));
    }

    #[test]
    fn recognized_windows_survive_five_metric_cap_after_unknown_rows() {
        let mut rows: Vec<Value> = (0..6)
            .map(|n| {
                json!({"type":"TOKENS_LIMIT",
            "unit":9, "number":n, "percentage":n})
            })
            .collect();
        rows.push(json!({"type":"TOKENS_LIMIT", "unit":6, "number":1, "percentage":7}));
        rows.push(json!({"type":"TOKENS_LIMIT", "unit":3, "number":5, "percentage":8}));
        let metrics = parse(json!(rows));
        assert_eq!(metrics.len(), 5);
        assert_eq!(metrics[0].label, "5 Hours");
        assert_eq!(metrics[1].label, "Weekly");
    }

    #[test]
    fn web_search_quota_stays_separate_from_token_windows() {
        let metrics = parse(json!([
            {"type":"TIME_LIMIT", "unit":5, "number":1, "usage":1000,
             "currentValue":147, "percentage":14, "nextResetTime":1784706344993_i64},
            {"type":"TOKENS_LIMIT", "unit":3, "number":5, "percentage":8}
        ]));
        let search = metrics.iter().find(|m| m.label == "Web Searches").unwrap();
        assert!((search.used_percent.unwrap() - 14.7).abs() < 1e-9);
        assert_eq!(search.resets_at, Some(1784706344993));
        assert_eq!(search.detail.as_deref(), Some("147 of 1000 searches"));
    }

    #[test]
    fn invalid_reset_stays_unknown_and_generic_quota_still_parses() {
        let metrics = parse(json!([
            {"type":"TOKENS_LIMIT", "unit":3, "number":5, "percentage":12,
             "nextResetTime":-1},
            {"name":"other", "usage":10, "limit":50}
        ]));
        assert_eq!(metrics[0].resets_at, None);
        assert_eq!(metrics[1].label, "other");
        assert_eq!(metrics[1].used_percent, Some(20.0));
    }
}
