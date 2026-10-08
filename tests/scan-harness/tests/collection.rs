use pane_scan_tests::{scan_policy::ScanPolicy, spend};
use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
struct Fixture(PathBuf);
impl Fixture {
    fn new(name: &str) -> Self {
        let p = std::env::temp_dir().join(format!("pane-collection-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn dir(&self, name: &str) -> PathBuf {
        let p = self.0.join(name);
        fs::create_dir_all(&p).unwrap();
        p
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn policy(entries: Vec<(&str, Vec<PathBuf>)>) -> ScanPolicy {
    ScanPolicy::new(entries.into_iter().map(|(s, p)| (s.into(), p)).collect()).unwrap()
}
fn claude(model: &str, tokens: f64) -> String {
    json!({"type":"assistant","timestamp":chrono::Utc::now().to_rfc3339(),"requestId":"request","message":{"id":"message","model":model,"usage":{"input_tokens":tokens,"output_tokens":0}}}).to_string()+"\n"
}
#[test]
fn disabled_sources_do_not_collect_existing_logs() {
    let f = Fixture::new("disabled");
    fs::write(f.0.join("session.jsonl"), claude("MiniMax-M3", 100.)).unwrap();
    assert!(spend::collect(&ScanPolicy::default(), None).is_empty());
}
#[test]
fn mixed_sources_keep_tokens_once() {
    let f = Fixture::new("mixed");
    let cc = f.dir("claude");
    let cx = f.dir("codex");
    let pi = f.dir("pi");
    fs::write(cc.join("session.jsonl"), claude("MiniMax-M3", 100.)).unwrap();
    fs::write(cc.join("auth.json"), "synthetic auth must be ignored").unwrap();
    let now = chrono::Utc::now().to_rfc3339();
    let codex=[json!({"type":"turn_context","timestamp":now,"payload":{"model":"kimi-oauth/k3"}}),json!({"type":"event_msg","timestamp":now,"payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":50,"output_tokens":0,"total_tokens":50},"total_token_usage":{"input_tokens":50,"output_tokens":0,"total_tokens":50}}}})].into_iter().map(|v|v.to_string()).collect::<Vec<_>>().join("\n");
    fs::write(cx.join("rollout-test.jsonl"), codex).unwrap();
    fs::write(pi.join("session.jsonl"),json!({"type":"message","id":"m","timestamp":now,"message":{"role":"assistant","provider":"step","model":"step-3.5-flash","usage":{"input":25,"output":0,"totalTokens":25,"cost":{"total":0.2}}}}).to_string()).unwrap();
    let p = policy(vec![
        ("claude", vec![cc.clone(), cc.join(".")]),
        ("codex", vec![cx]),
        ("pi", vec![pi]),
    ]);
    let a = spend::collect(&p, None);
    let b = spend::collect(&p, None);
    for rows in [a, b] {
        assert_eq!(rows.iter().map(|s| s.last30.tokens).sum::<f64>(), 175.);
        assert_eq!(
            rows.iter()
                .find(|s| s.id == "minimax")
                .unwrap()
                .last30
                .tokens,
            100.
        );
        assert_eq!(
            rows.iter()
                .find(|s| s.id == "moonshot")
                .unwrap()
                .last30
                .tokens,
            50.
        );
        assert_eq!(
            rows.iter()
                .find(|s| s.id == "stepfun")
                .unwrap()
                .last30
                .tokens,
            25.
        );
        assert!(rows
            .iter()
            .find(|s| s.id == "minimax")
            .unwrap()
            .sources
            .contains(&"claude".into()));
    }
}
#[test]
fn scan_result_revocation_invalidates_published_totals() {
    let f = Fixture::new("revocation");
    fs::write(f.0.join("session.jsonl"), claude("MiniMax-M3", 100.)).unwrap();
    let live = Arc::new(AtomicBool::new(true));
    let check = live.clone();
    let p = policy(vec![("claude", vec![f.0.clone()])])
        .with_check(move || check.load(Ordering::SeqCst));
    assert!(!spend::collect(&p, None).is_empty());
    assert!(spend::month_to_date_cost("minimax").is_some());
    live.store(false, Ordering::SeqCst);
    assert!(
        spend::month_to_date_cost("minimax").is_none(),
        "revoked cached totals must not feed a query card"
    );
    assert!(spend::window_totals("minimax", 0, i64::MAX).is_none());
}
#[test]
fn cursor_csv_needs_no_local_scan_root() {
    let p = ScanPolicy::default();
    let now = chrono::Utc::now().to_rfc3339();
    let csv = format!(
        "Date,Model,Cost,Input Tokens,Output Tokens,Total Tokens\n{now},gpt-5,1.0,10,5,15\n"
    );
    let rows = spend::collect(&p, Some(csv));
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, "cursor");
    assert!(rows[0].sources.is_empty());
}
#[test]
#[cfg(unix)]
fn collection_rejects_escaped_jsonl_and_auth_alias() {
    let f = Fixture::new("links");
    let logs = f.dir("logs");
    let outside = f.dir("outside");
    let secret = outside.join("session.jsonl");
    fs::write(&secret, claude("MiniMax-M3", 100.)).unwrap();
    std::os::unix::fs::symlink(&secret, logs.join("escape.jsonl")).unwrap();
    fs::write(logs.join("auth.json"), claude("MiniMax-M3", 200.)).unwrap();
    std::os::unix::fs::symlink(logs.join("auth.json"), logs.join("auth-alias.jsonl")).unwrap();
    let p = policy(vec![("claude", vec![logs])]);
    assert!(spend::collect(&p, None).is_empty());
}
#[test]
fn changed_source_reparses_instead_of_reusing_another_codec_cache() {
    let f = Fixture::new("codec");
    let now = chrono::Utc::now().to_rfc3339();
    let pi=json!({"type":"message","id":"m","timestamp":now,"message":{"role":"assistant","provider":"step","model":"step-3.5-flash","usage":{"input":25,"output":0,"totalTokens":25,"cost":{"total":0.2}}}}).to_string();
    fs::write(
        f.0.join("session.jsonl"),
        claude("MiniMax-M3", 100.) + &pi + "\n",
    )
    .unwrap();
    assert_eq!(
        spend::collect(&policy(vec![("claude", vec![f.0.clone()])]), None)
            .iter()
            .map(|s| s.last30.tokens)
            .sum::<f64>(),
        100.
    );
    assert_eq!(
        spend::collect(&policy(vec![("pi", vec![f.0.clone()])]), None)
            .iter()
            .map(|s| s.last30.tokens)
            .sum::<f64>(),
        25.
    );
}
#[test]
fn qwen_quota_fallback_needs_independent_scan_grant() {
    let f = Fixture::new("qwen");
    let now = chrono::Local::now();
    let file =
        f.0.join(format!("token-usage-{}.jsonl", now.format("%Y-%m")));
    fs::write(
        file,
        json!({"totalTokens":123,"localDate":now.format("%Y-%m-%d").to_string()}).to_string()
            + "\n",
    )
    .unwrap();
    assert!(spend::qwen_request_counts(&ScanPolicy::default()).is_none());
    assert_eq!(
        spend::qwen_request_counts(&policy(vec![("qwen", vec![f.0.clone()])])),
        Some((1, 1))
    );
}
#[test]
fn local_source_rows_keep_qwen_counts_and_hermes_details() {
    let f = Fixture::new("details");
    let q = f.dir("qwen");
    let h = f.dir("hermes");
    let now = chrono::Local::now();
    fs::write(q.join(format!("token-usage-{}.jsonl",now.format("%Y-%m"))),json!({"timestamp":now.to_rfc3339(),"model":"qwen3.8-max","inputTokens":12,"totalTokens":12,"localDate":now.format("%Y-%m-%d").to_string()}).to_string()+"\n").unwrap();
    let db = rusqlite::Connection::open(h.join("state.db")).unwrap();
    db.execute_batch("CREATE TABLE session_model_usage (last_seen REAL, model TEXT, billing_provider TEXT,input_tokens REAL,output_tokens REAL,reasoning_tokens REAL,cache_read_tokens REAL,cache_write_tokens REAL,actual_cost_usd REAL,estimated_cost_usd REAL);").unwrap();
    db.execute(
        "INSERT INTO session_model_usage VALUES (?1,'MiniMax-M3','minimax',10,5,0,0,0,1,0)",
        [now.timestamp()],
    )
    .unwrap();
    drop(db);
    let rows = spend::collect(&policy(vec![("qwen", vec![q]), ("hermes", vec![h])]), None);
    let qwen = rows.iter().find(|r| r.id == "source:qwen").unwrap();
    assert!(qwen
        .local_stats
        .iter()
        .any(|m| m.label == "Requests today" && m.value == "1"));
    let hermes = rows.iter().find(|r| r.id == "source:hermes").unwrap();
    assert!(hermes
        .local_stats
        .iter()
        .any(|m| m.label == "Recent models" && m.value.contains("MiniMax-M3")));
    assert!(hermes
        .local_stats
        .iter()
        .any(|m| m.label == "Sessions" && m.value == "1"));
    assert_eq!(rows.iter().map(|r| r.last30.tokens).sum::<f64>(), 27.);
}
#[test]
fn unknown_account_is_not_a_zero_usage_sample() {
    let f = Fixture::new("unknown-account");
    fs::write(f.0.join("session.jsonl"), claude("claude-sonnet-4", 100.)).unwrap();
    let rows = spend::collect(&policy(vec![("claude", vec![f.0.clone()])]), None);
    assert!(rows.iter().any(|r| r.id == "source:claude"));
    assert!(spend::window_totals("claude", 0, i64::MAX).is_none());
    assert!(spend::month_to_date_cost("claude").is_none());
}
fn hermes_db(root: &std::path::Path) {
    let db = rusqlite::Connection::open(root.join("state.db")).unwrap();
    db.execute_batch("CREATE TABLE session_model_usage (last_seen REAL,model TEXT,billing_provider TEXT,input_tokens REAL,output_tokens REAL,reasoning_tokens REAL,cache_read_tokens REAL,cache_write_tokens REAL,actual_cost_usd REAL,estimated_cost_usd REAL);").unwrap();
    db.execute(
        "INSERT INTO session_model_usage VALUES (?1,'MiniMax-M3','minimax',10,5,0,0,0,1,0)",
        [chrono::Utc::now().timestamp()],
    )
    .unwrap();
}
#[test]
fn hermes_adapter_rejects_another_tools_scope() {
    let f = Fixture::new("hermes-wrong-source");
    hermes_db(&f.0);
    let p = policy(vec![("claude", vec![f.0.clone()])]);
    let events = p.scope("claude", || {
        pane_scan_tests::hermes_adapter::collect_usage_events_in(&f.0.join("state.db"))
    });
    assert!(events.is_empty());
}
#[test]
fn hermes_regrant_cannot_republish_a_prior_grants_cached_events() {
    let f = Fixture::new("hermes-regrant");
    hermes_db(&f.0);
    let p = policy(vec![("hermes", vec![f.0.clone()])]).with_revision(10);
    let first = spend::collect(&p, None);
    assert_eq!(first.iter().map(|r| r.last30.tokens).sum::<f64>(), 15.);
    assert!(spend::collect(&ScanPolicy::default().with_revision(11), None).is_empty());
    fs::write(
        f.0.join("state.db"),
        "unreadable synthetic SQLite replacement",
    )
    .unwrap();
    let p = policy(vec![("hermes", vec![f.0.clone()])]).with_revision(12);
    assert!(
        spend::collect(&p, None).is_empty(),
        "new consent cannot use the prior grant's last-good events"
    );
}
#[test]
fn aborted_codec_scan_does_not_poison_a_regranted_source() {
    use std::sync::atomic::AtomicUsize;
    let f = Fixture::new("aborted-codec");
    let now = chrono::Utc::now().to_rfc3339();
    let pi=json!({"type":"message","id":"m","timestamp":now,"message":{"role":"assistant","provider":"step","model":"step-3.5-flash","usage":{"input":25,"output":0,"totalTokens":25,"cost":{"total":0.2}}}}).to_string();
    fs::write(
        f.0.join("session.jsonl"),
        claude("claude-sonnet-4", 100.) + &pi + "\n",
    )
    .unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let c = count.clone();
    let measure = policy(vec![("pi", vec![f.0.clone()])])
        .with_revision(1)
        .with_check(move || {
            c.fetch_add(1, Ordering::SeqCst);
            true
        });
    assert_eq!(
        spend::collect(&measure, None)
            .iter()
            .map(|r| r.last30.tokens)
            .sum::<f64>(),
        25.
    );
    let calls = count.load(Ordering::SeqCst);
    let good = policy(vec![("claude", vec![f.0.clone()])]).with_revision(2);
    assert_eq!(
        spend::collect(&good, None)
            .iter()
            .map(|r| r.last30.tokens)
            .sum::<f64>(),
        100.
    );
    let count = Arc::new(AtomicUsize::new(0));
    let c = count.clone();
    let abort = policy(vec![("pi", vec![f.0.clone()])])
        .with_revision(3)
        .with_check(move || c.fetch_add(1, Ordering::SeqCst) + 1 < calls - 1);
    assert!(spend::collect(&abort, None).is_empty());
    let restored = policy(vec![("claude", vec![f.0.clone()])]).with_revision(4);
    assert_eq!(
        spend::collect(&restored, None)
            .iter()
            .map(|r| r.last30.tokens)
            .sum::<f64>(),
        100.
    );
}
#[test]
#[cfg(unix)]
fn spend_cache_uses_private_unique_staging() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let f = Fixture::new("private-cache");
    fs::write(f.0.join("session.jsonl"), claude("MiniMax-M3", 100.)).unwrap();
    let cache = pane_scan_tests::providers::config_dir().join("spend_cache.json");
    fs::write(&cache, "old").unwrap();
    fs::set_permissions(&cache, fs::Permissions::from_mode(0o666)).unwrap();
    let victim = f.0.join("unrelated");
    fs::write(&victim, "keep me").unwrap();
    let predicted = cache.with_extension("json.tmp");
    let _ = fs::remove_file(&predicted);
    symlink(&victim, &predicted).unwrap();
    spend::collect(&policy(vec![("claude", vec![f.0.clone()])]), None);
    assert_eq!(fs::read_to_string(&victim).unwrap(), "keep me");
    assert_eq!(
        fs::metadata(&cache).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let _ = fs::remove_file(predicted);
}
