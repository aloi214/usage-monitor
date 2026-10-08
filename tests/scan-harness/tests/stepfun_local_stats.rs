use chrono::{Datelike, Local, TimeZone};
use pane_scan_tests::{providers, scan_policy::ScanPolicy, spend};
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs, path::PathBuf, sync::Mutex};

// Tests share only the harness's synthetic app configuration.
static CONFIG: Mutex<()> = Mutex::new(());
struct Fixture(PathBuf);
impl Fixture {
    fn new(name: &str, tier: Value) -> Self {
        let root = std::env::temp_dir().join(format!("pane-stepfun-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(
            providers::config_dir().join("config.json"),
            json!({"stepfunPlanCredits":tier}).to_string(),
        )
        .unwrap();
        Self(root)
    }
    fn policy(&self) -> ScanPolicy {
        ScanPolicy::new(BTreeMap::from([("stepcode".into(), vec![self.0.clone()])])).unwrap()
    }
    fn log(&self, model: &str, cost: f64) {
        fs::write(
            self.0.join("session.jsonl"),
            event("current", Local::now().to_rfc3339(), model, cost),
        )
        .unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
        let _ = fs::remove_file(providers::config_dir().join("config.json"));
    }
}
fn event(id: &str, timestamp: String, model: &str, cost: f64) -> String {
    json!({"type":"message","id":id,"timestamp":timestamp,"message":{"role":"assistant","provider":"step","model":model,"usage":{"input":100,"output":0,"totalTokens":100,"cost":{"total":cost}}}}).to_string() + "\n"
}
fn stepfun(rows: &[spend::ProviderSpend]) -> &spend::ProviderSpend {
    rows.iter()
        .find(|row| row.id == "stepfun")
        .expect("StepFun source slice")
}
fn stat<'a>(row: &'a spend::ProviderSpend, label: &str) -> &'a str {
    &row.local_stats
        .iter()
        .find(|stat| stat.label == label)
        .unwrap_or_else(|| panic!("missing local stat {label}"))
        .value
}

#[test]
fn configured_tier_estimate_uses_only_current_month_authorized_logs() {
    let _guard = CONFIG.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new("tier", json!(400));
    let now = Local::now();
    let last_month = now.date_naive().with_day(1).unwrap().pred_opt().unwrap();
    let last_month = Local
        .from_local_datetime(&last_month.and_hms_opt(12, 0, 0).unwrap())
        .unwrap();
    let rows = event("current", now.to_rfc3339(), "step-test-model", 2.0)
        + &event("past", last_month.to_rfc3339(), "step-test-model", 5.0)
        + &event(
            "future",
            (now + chrono::Duration::days(1)).to_rfc3339(),
            "step-test-model",
            9.0,
        );
    fs::write(f.0.join("session.jsonl"), rows).unwrap();
    let rows = spend::collect(&f.policy().with_revision(7), None);
    let row = stepfun(&rows);
    assert_eq!(row.sources, ["stepcode"]);
    assert_eq!(row.scan_revision, Some(7));
    assert_eq!(row.month_cost, 2.0);
    assert!(stat(row, "Plan Credits").contains("≈14M of 400M"));
    assert!(stat(row, "Plan Credits").contains("est. from logs"));
    assert!(stat(row, "Plan attribution").contains("Account unverified"));
    assert!(stat(row, "Plan attribution").contains("non-plan"));
    assert!(stat(row, "Configured plan tier").contains("Flash Mini"));
    assert!(stat(row, "Configured plan tier").contains("not verified"));
    let serialized = serde_json::to_value(row).unwrap();
    assert!(
        serialized.get("metrics").is_none(),
        "must stay out of account metrics"
    );
}

#[test]
fn missing_or_unknown_tier_keeps_an_unqualified_local_estimate() {
    let _guard = CONFIG.lock().unwrap_or_else(|e| e.into_inner());
    for tier in [Value::Null, json!(999), json!("400")] {
        let f = Fixture::new("no-tier", tier);
        f.log("step-test-model", 2.0);
        let rows = spend::collect(&f.policy(), None);
        let row = stepfun(&rows);
        assert!(stat(row, "Plan Credits").contains("≈14M Credits this month"));
        assert!(!stat(row, "Plan Credits").contains(" of "));
        assert!(!row
            .local_stats
            .iter()
            .any(|s| s.label == "Configured plan tier"));
        assert!(stat(row, "Plan attribution").contains("Account unverified"));
    }
}

#[test]
fn unpriced_logs_keep_tokens_without_zero_known_plan_usage() {
    let _guard = CONFIG.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new("unpriced", json!(400));
    f.log("step-synthetic-unpriced-model-does-not-exist", 0.0);
    let rows = spend::collect(&f.policy(), None);
    let row = stepfun(&rows);
    assert_eq!(row.last30.tokens, 100.0);
    assert_eq!(row.unpriced, 1);
    let credits = stat(row, "Plan Credits");
    assert!(credits.contains("Unavailable"));
    assert!(credits.contains("missing model prices"));
    assert!(!credits.contains("0M"));
}

#[test]
fn incomplete_scan_never_claims_known_plan_usage() {
    let _guard = CONFIG.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new("incomplete", json!(400));
    f.log("step-test-model", 2.0);
    let logs = f.0.join("stepcode");
    fs::create_dir_all(&logs).unwrap();
    fs::rename(f.0.join("session.jsonl"), logs.join("session.jsonl")).unwrap();
    let broken = f.0.join("hermes");
    fs::create_dir_all(&broken).unwrap();
    fs::write(broken.join("state.db"), "synthetic invalid SQLite database").unwrap();
    let p = ScanPolicy::new(BTreeMap::from([
        ("stepcode".into(), vec![logs]),
        ("hermes".into(), vec![broken]),
    ]))
    .unwrap();
    let rows = spend::collect(&p, None);
    assert!(
        !spend::window_totals("stepfun", 0, i64::MAX)
            .unwrap()
            .complete
    );
    let credits = stat(stepfun(&rows), "Plan Credits");
    assert!(credits.contains("Unavailable"));
    assert!(credits.contains("scan incomplete"));
    assert!(!credits.contains("14M"));
}

#[test]
fn no_records_or_no_source_grant_do_not_create_plan_usage() {
    let _guard = CONFIG.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new("empty", json!(400));
    assert!(spend::collect(&f.policy(), None).is_empty());
    f.log("step-test-model", 2.0);
    assert!(spend::collect(&ScanPolicy::default(), None).is_empty());
    // A different producer's rows must not manufacture a zero StepFun row,
    // even though routing carries conservative source provenance to it.
    fs::write(
        f.0.join("session.jsonl"),
        event("other", Local::now().to_rfc3339(), "claude-sonnet-4", 1.0)
            .replace("\"step\"", "\"anthropic\""),
    )
    .unwrap();
    assert!(!spend::collect(&f.policy(), None)
        .iter()
        .any(|row| row.id == "stepfun"));
}
