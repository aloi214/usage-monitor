use pane_scan_tests::access_policy::AccessPolicy;
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
