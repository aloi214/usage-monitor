use pane_core_tests::access_policy::AccessPolicy;
#[test]
fn legacy_local_estimates_cannot_reenter_query_snapshots() {
    let mut policy = AccessPolicy::default();
    for family in ["claude", "codex", "qwen", "hermes", "opencode"] {
        policy.set_family(family, true).unwrap();
        policy.set_account(family, true).unwrap();
    }
    assert!(!policy.allows_metric("claude", "Weekly capacity"));
    assert!(!policy.allows_metric("qwen", "Requests today"));
    assert!(!policy.allows_metric("hermes", "Recent models"));
    assert!(!policy.allows_snapshot("opencode", Some("Go — this PC only")));
    assert!(policy.allows_snapshot("opencode", Some("Go")));
    assert!(policy.allows_metric("opencode", "Weekly"));
    for label in ["Session", "Weekly", "Monthly"] {
        assert!(policy.allows_metric("qwen", label));
    }
}
#[test]
fn scan_hot_loop_checks_revision_without_reloading_config() {
    use pane_core_tests::access_policy::AccessRuntime;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    let reads = Arc::new(AtomicUsize::new(0));
    let r = reads.clone();
    let runtime = AccessRuntime::with_checker(move || {
        r.fetch_add(1, Ordering::SeqCst);
        AccessPolicy::default()
    });
    let (_, revision) = runtime.snapshot();
    let before = reads.load(Ordering::SeqCst);
    for _ in 0..100_000 {
        assert!(runtime.is_current_cached(revision));
    }
    assert_eq!(reads.load(Ordering::SeqCst), before);
    runtime.invalidate();
    assert!(!runtime.is_current_cached(revision));
}
