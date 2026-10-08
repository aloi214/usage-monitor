use pane_core_tests::access_policy::{AccessPolicy, AccessRuntime};
use serde_json::json;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

fn policy(families: &[&str], accounts: &[&str]) -> AccessPolicy {
    AccessPolicy::from_config(&json!({"accessPolicy": {
        "version": 1, "enabledFamilies": families, "enabledAccounts": accounts,
        "scanRoots": {}, "regions": {}, "accountBindings": {}
    }}))
}

#[tokio::test]
async fn legacy_config_denies_access() {
    let policy = AccessPolicy::from_config(&json!({"disabled": []}));
    let reads = AtomicUsize::new(0);
    assert!(policy
        .read_account("codex", || reads.fetch_add(1, Ordering::SeqCst))
        .is_none());
    let runtime = Arc::new(AccessRuntime::new(policy));
    let requests = AtomicUsize::new(0);
    assert!(runtime
        .run_account("codex", &[], async {
            requests.fetch_add(1, Ordering::SeqCst)
        })
        .await
        .is_none());
    assert_eq!(reads.load(Ordering::SeqCst), 0);
    assert_eq!(requests.load(Ordering::SeqCst), 0);
}

#[test]
fn family_enable_does_not_enable_discovered_accounts() {
    let policy = policy(&["claude"], &[]);
    assert!(policy.allows_discovery("claude"));
    assert!(!policy.allows_account("claude"));
    assert!(!policy.allows_account("claude@12345678"));
    let reads = AtomicUsize::new(0);
    assert!(policy
        .read_account("claude", || reads.fetch_add(1, Ordering::SeqCst))
        .is_none());
    assert_eq!(reads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn frontend_filter_cannot_grant_access() {
    let runtime = Arc::new(AccessRuntime::new(policy(&["codex"], &[])));
    let requests = AtomicUsize::new(0);
    assert!(runtime
        .run_account("codex", &[], async {
            requests.fetch_add(1, Ordering::SeqCst)
        })
        .await
        .is_none());
    assert_eq!(requests.load(Ordering::SeqCst), 0);
    runtime.update(policy(&["codex"], &["codex"]));
    assert!(runtime
        .run_account("codex", &["codex".into()], async {
            requests.fetch_add(1, Ordering::SeqCst)
        })
        .await
        .is_none());
    assert_eq!(requests.load(Ordering::SeqCst), 0);
    assert_eq!(
        runtime.run_account("codex", &[], async { 7 }).await,
        Some(7)
    );
}

#[tokio::test]
async fn disabled_during_refresh_drops_result() {
    let runtime = Arc::new(AccessRuntime::new(policy(&["codex"], &["codex"])));
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let requests = Arc::new(AtomicUsize::new(0));
    let r = runtime.clone();
    let calls = requests.clone();
    let work = tokio::spawn(async move {
        r.run_account("codex", &[], async {
            calls.fetch_add(1, Ordering::SeqCst);
            started_tx.send(()).unwrap();
            release_rx.await.unwrap();
            calls.fetch_add(1, Ordering::SeqCst);
            42
        })
        .await
    });
    started_rx.await.unwrap();
    runtime.update(policy(&[], &[]));
    let _ = release_tx.send(());
    assert_eq!(work.await.unwrap(), None);
    assert_eq!(
        requests.load(Ordering::SeqCst),
        1,
        "revocation must stop the next request"
    );
}

#[test]
fn malformed_unknown_version_and_unknown_accounts_fail_closed() {
    for value in [
        json!({"version":2,"enabledFamilies":["codex"],"enabledAccounts":["codex"]}),
        json!({"version":1,"enabledFamilies":["codex"],"enabledAccounts":"codex"}),
        json!({"version":1,"enabledFamilies":["unknown"],"enabledAccounts":["unknown"]}),
        json!({"version":1,"enabledFamilies":["codex"],"enabledAccounts":["codex@12345678"]}),
    ] {
        let p = AccessPolicy::from_config(&json!({"accessPolicy":value}));
        assert!(!p.allows_account("codex"));
        assert!(!p.allows_account("codex@12345678"));
        assert!(!p.allows_discovery("unknown"));
    }
}

#[test]
fn disabling_family_revokes_accounts_without_enabling_siblings() {
    let mut p = policy(&["codex"], &["codex"]);
    assert!(p.allows_account("codex"));
    p.set_family("codex", false).unwrap();
    assert!(!p.allows_account("codex"));
    p.set_family("codex", true).unwrap();
    assert!(
        !p.allows_account("codex"),
        "re-enabling discovery must not restore account grants"
    );
    assert!(p.set_account("codex@12345678", true).is_err());
}

#[test]
fn private_state_path_never_imports_or_renames_upstream() {
    let base = std::path::Path::new("/synthetic/config");
    assert_eq!(
        pane_core_tests::access_policy::private_config_dir(base),
        base.join("PanePrivate")
    );
}

#[test]
fn retired_configuration_is_pruned_before_first_refresh() {
    let mut cfg = json!({
        "disabled":["claude","onenewapi@key","sub2api"],
        "trayProviders":["sub2api@key","codex"],
        "pinned":{"provider":"onenewapi@key","label":"Balance"},
        "layout":{"providerOrder":["claude","onenewapi","sub2api@key"],"providers":{"claude":{},"onenewapi":{},"sub2api@key":{}}},
        "oneNewApiSites":[{"secret":"fake"}],"sub2apiSites":[], "appearance":"dark"
    });
    pane_core_tests::access_policy::sanitize_retired_config(&mut cfg);
    assert_eq!(cfg["disabled"], json!(["claude"]));
    assert_eq!(cfg["trayProviders"], json!(["codex"]));
    assert_eq!(cfg["pinned"], serde_json::Value::Null);
    assert_eq!(cfg["layout"]["providerOrder"], json!(["claude"]));
    assert_eq!(cfg["layout"]["providers"], json!({"claude":{}}));
    assert!(cfg.get("oneNewApiSites").is_none());
    assert!(cfg.get("sub2apiSites").is_none());
    assert_eq!(cfg["appearance"], "dark");
}

#[tokio::test]
async fn revocation_and_reenable_never_resurrect_old_permit() {
    let runtime = Arc::new(AccessRuntime::new(policy(&["codex"], &["codex"])));
    let permit = runtime.permit("codex", &[]).unwrap();
    runtime.update(policy(&[], &[]));
    runtime.update(policy(&["codex"], &["codex"]));
    let calls = AtomicUsize::new(0);
    assert_eq!(
        permit
            .run(async { calls.fetch_add(1, Ordering::SeqCst) })
            .await,
        None
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn child_future_inherits_only_explicit_authorization() {
    assert!(pane_core_tests::access_policy::check_current_operation().is_err());
    let runtime = Arc::new(AccessRuntime::new(policy(&["cursor"], &["cursor"])));
    let result = runtime
        .run_account("cursor", &[], async {
            assert!(pane_core_tests::access_policy::check_current_operation().is_ok());
            assert!(tokio::spawn(async {
                pane_core_tests::access_policy::check_current_operation()
            })
            .await
            .unwrap()
            .is_err());
            let permit = pane_core_tests::access_policy::current_operation().unwrap();
            assert_eq!(permit.account_id(), "cursor");
            tokio::spawn(async move {
                permit
                    .run(async { pane_core_tests::access_policy::check_current_operation() })
                    .await
            })
            .await
            .unwrap()
        })
        .await;
    assert_eq!(result, Some(Some(Ok(()))));
}

#[test]
fn contained_account_file_rejects_symlink_escape() {
    let root = std::env::temp_dir().join(format!("pane-consent-{}", std::process::id()));
    let allowed = root.join("chosen");
    std::fs::create_dir_all(&allowed).unwrap();
    std::fs::write(allowed.join("inside.json"), "fake").unwrap();
    std::fs::write(root.join("outside.json"), "fake").unwrap();
    assert!(pane_core_tests::access_policy::contained_file(&allowed, "inside.json").is_ok());
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(root.join("outside.json"), allowed.join("auth.json")).unwrap();
        assert!(pane_core_tests::access_policy::contained_file(&allowed, "auth.json").is_err());
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn folded_wallet_rows_require_separate_moonshot_account_consent() {
    let p = policy(&["kimi", "moonshot"], &["kimi"]);
    assert!(p.allows_metric("kimi", "Weekly"));
    assert!(!p.allows_metric("kimi", "API"));
    assert!(!p.allows_metric("kimi", "Balance"));
    assert!(!p.allows_metric("moonshot", "Balance"));
    let p = policy(&["kimi", "moonshot"], &["kimi", "moonshot"]);
    assert!(p.allows_metric("kimi", "API"));
}

#[tokio::test]
async fn narrowing_legacy_filter_invalidates_inflight_permit() {
    let runtime = Arc::new(AccessRuntime::new(policy(&["codex"], &["codex"])));
    let permit = runtime.permit("codex", &[]).unwrap();
    runtime.invalidate();
    assert!(permit.check().is_err());
}

#[tokio::test]
async fn nested_account_operation_inherits_frontend_narrowing() {
    let runtime = Arc::new(AccessRuntime::new(policy(
        &["kimi", "moonshot"],
        &["kimi", "moonshot"],
    )));
    let calls = AtomicUsize::new(0);
    let result = runtime
        .run_account("kimi", &["moonshot".into()], async {
            runtime
                .run_account("moonshot", &[], async {
                    calls.fetch_add(1, Ordering::SeqCst)
                })
                .await
        })
        .await;
    assert_eq!(result, Some(None));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn stale_batch_cannot_start_after_disable_and_reenable() {
    let runtime = Arc::new(AccessRuntime::new(policy(&["codex"], &["codex"])));
    let revision = runtime.snapshot().1;
    runtime.update(policy(&[], &[]));
    runtime.update(policy(&["codex"], &["codex"]));
    let calls = AtomicUsize::new(0);
    assert!(runtime
        .run_account_at("codex", &[], revision, async {
            calls.fetch_add(1, Ordering::SeqCst)
        })
        .await
        .is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn injected_backend_reload_denies_corrupt_or_revoked_configuration() {
    let backend = Arc::new(std::sync::Mutex::new(policy(&["codex"], &["codex"])));
    let source = backend.clone();
    let runtime = Arc::new(AccessRuntime::with_checker(move || {
        source.lock().unwrap().clone()
    }));
    let permit = runtime.permit("codex", &[]).unwrap();
    *backend.lock().unwrap() = AccessPolicy::from_config(&json!({"accessPolicy":"corrupt"}));
    assert!(permit.check().is_err());
    let calls = AtomicUsize::new(0);
    assert!(runtime
        .run_account("codex", &[], async { calls.fetch_add(1, Ordering::SeqCst) })
        .await
        .is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn mode_mutation_suspends_new_permits_and_all_publications_until_drop() {
    let mut policy = AccessPolicy::default();
    policy.set_family("qwen", true).unwrap();
    policy.set_account("qwen", true).unwrap();
    let runtime = Arc::new(AccessRuntime::new(policy));
    let old = runtime.permit("qwen", &[]).unwrap();
    let (_, old_revision) = runtime.snapshot();
    let pending_revision;
    {
        let _mutation = runtime.suspend();
        pending_revision = runtime.snapshot().1;
        assert!(old.check().is_err());
        assert!(runtime.permit("qwen", &[]).is_none());
        assert!(runtime
            .commit_if_current(pending_revision, |_| ())
            .is_none());
        assert!(!runtime.is_current_cached(pending_revision));
    }
    assert!(runtime.permit("qwen", &[]).is_some());
    assert!(!runtime.is_current(old_revision));
    assert!(!runtime.is_current(pending_revision));
}

#[test]
fn nested_mode_mutation_keeps_access_suspended_after_inner_drop() {
    let mut policy = AccessPolicy::default();
    policy.set_family("minimax", true).unwrap();
    policy.set_account("minimax", true).unwrap();
    let runtime = Arc::new(AccessRuntime::new(policy));
    let outer = runtime.suspend();
    {
        let _inner = runtime.suspend();
    }
    assert!(runtime.permit("minimax", &[]).is_none());
    drop(outer);
    assert!(runtime.permit("minimax", &[]).is_some());
}

#[test]
fn legacy_log_grants_migrate_to_durable_custom_intent_without_directory_discovery() {
    let a = std::env::temp_dir().join("pane-legacy-missing-a");
    let b = std::env::temp_dir().join("pane-legacy-missing-b");
    let loaded = AccessPolicy::from_config(&json!({"accessPolicy": {
        "version": 1, "enabledFamilies": [], "enabledAccounts": [],
        "scanRoots": {"claude": [a, b]}, "scanEpochs": {"claude": "saved-generation"}
    }}));
    let serialized = serde_json::to_value(&loaded).unwrap();
    assert_eq!(
        serialized["scanSources"]["claude"],
        json!({
            "enabled": true, "mode": "custom", "customDirectories": [a, b],
            "defaultDirectories": []
        })
    );
    assert_eq!(serialized["scanRoots"]["claude"], json!([a, b]));
    assert_eq!(serialized["scanEpochs"]["claude"], "saved-generation");
    assert!(loaded.enabled_families.is_empty());
    assert!(loaded.enabled_accounts.is_empty());
}
