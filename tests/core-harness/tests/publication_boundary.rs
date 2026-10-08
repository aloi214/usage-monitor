use pane_core_tests::{
    access_policy::{AccessPolicy, AccessRuntime, AccountBinding},
    snapshot::{Metric, Snapshot},
    usage_publication::{clear_both, strip_view, UsagePublication},
};
use serde_json::json;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
fn policy(accounts: &[&str]) -> AccessPolicy {
    AccessPolicy::from_config(
        &json!({"accessPolicy":{"version":1,"enabledFamilies":["kimi","moonshot","claude","codex"],"enabledAccounts":accounts}}),
    )
}
fn kimi() -> Snapshot {
    Snapshot::ok(
        "kimi",
        "Kimi",
        None,
        vec![
            Metric::progress("Weekly", 25.0, None),
            Metric::progress("API", 60.0, None),
        ],
    )
}
fn injected() -> (Arc<Mutex<AccessPolicy>>, Arc<AccessRuntime>) {
    let source = Arc::new(Mutex::new(policy(&["kimi", "moonshot"])));
    let s = source.clone();
    (
        source,
        Arc::new(AccessRuntime::with_checker(move || {
            s.lock().unwrap().clone()
        })),
    )
}
#[test]
fn delayed_strip_rejects_revoked_wallet_provenance() {
    let (source, runtime) = injected();
    let store = UsagePublication::default();
    let revision = runtime.snapshot().1;
    store.publish(&runtime, revision, vec![kimi()]).unwrap();
    let labels = vec!["Weekly".into(), "API".into()];
    let mut native = Vec::new();
    *source.lock().unwrap() = policy(&["kimi"]);
    let result = store.apply(&runtime, revision, |snaps, p| {
        native.push(strip_view(snaps, p, "kimi", &labels).unwrap());
    });
    assert!(result.is_none());
    assert!(native.is_empty());
    let p = runtime.snapshot().0;
    let current = strip_view(&[kimi()], &p, "kimi", &labels).unwrap();
    assert_eq!(current.values, vec![75]);
    assert!(!current.tooltip.contains("API"));
}
#[test]
fn delayed_main_and_strip_reject_old_revision_after_off_on() {
    let (source, runtime) = injected();
    let store = UsagePublication::default();
    let rev = runtime.snapshot().1;
    store.publish(&runtime, rev, vec![kimi()]).unwrap();
    *source.lock().unwrap() = policy(&[]);
    runtime.snapshot();
    *source.lock().unwrap() = policy(&["kimi", "moonshot"]);
    runtime.snapshot();
    let main = AtomicUsize::new(0);
    let strip = AtomicUsize::new(0);
    assert!(store
        .apply(&runtime, rev, |_, _| main.fetch_add(1, Ordering::SeqCst))
        .is_none());
    assert!(store
        .apply(&runtime, rev, |_, _| strip.fetch_add(1, Ordering::SeqCst))
        .is_none());
    assert_eq!(
        main.load(Ordering::SeqCst) + strip.load(Ordering::SeqCst),
        0
    );
}
#[test]
fn checker_revocation_invalidates_http_read_boundary() {
    let (source, runtime) = injected();
    let store = UsagePublication::default();
    let rev = runtime.snapshot().1;
    store.publish(&runtime, rev, vec![kimi()]).unwrap();
    assert_eq!(store.read(&runtime).unwrap().snapshots.len(), 1);
    *source.lock().unwrap() = AccessPolicy::from_config(&json!({"accessPolicy":"broken"}));
    runtime.snapshot();
    assert!(store.read(&runtime).is_none());
}
#[test]
fn final_publication_rechecks_after_post_fetch_work() {
    let (source, runtime) = injected();
    let store = UsagePublication::default();
    let rev = runtime.snapshot().1;
    assert!(runtime.is_current(rev));
    *source.lock().unwrap() = policy(&[]);
    assert!(store.publish(&runtime, rev, vec![kimi()]).is_none());
    assert!(store.read(&runtime).is_none());
}
#[test]
fn redemption_route_reads_only_target_and_never_sibling_or_default() {
    for family in ["claude", "codex"] {
        let target = format!("{family}@bbbbbbbb");
        let sibling = format!("{family}@aaaaaaaa");
        let root = std::env::temp_dir().join(format!("pane-route-{}-{family}", std::process::id()));
        let mut p = policy(&[]);
        for id in [&target, &sibling] {
            let dir = root.join(id.replace('@', "-"));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("auth.json"), "synthetic").unwrap();
            p.account_bindings.insert(
                id.clone(),
                AccountBinding {
                    family: family.into(),
                    directory: dir,
                    name: id.clone(),
                },
            );
            p.enabled_accounts.insert(id.clone());
        }
        let runtime = Arc::new(AccessRuntime::new(p));
        let permit = runtime.permit(&target, &[sibling.clone()]).unwrap();
        let mut reads = Vec::new();
        let route = permit.read_bound_target(family, |id, b| {
            reads.push(id.to_owned());
            Some(std::fs::read_to_string(b.directory.join("auth.json")).unwrap())
        });
        assert_eq!(route.as_deref(), Some("synthetic"));
        assert_eq!(reads, vec![target]);
        assert!(!reads.contains(&family.to_string()));
        std::fs::remove_dir_all(root).unwrap();
    }
}
#[tokio::test]
async fn failed_strip_clear_does_not_skip_main_clear() {
    let calls = AtomicUsize::new(0);
    let result = clear_both(
        async {
            calls.fetch_add(1, Ordering::SeqCst);
            Err("strip failed".into())
        },
        async {
            calls.fetch_add(1, Ordering::SeqCst);
            Err("main failed".into())
        },
    )
    .await;
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let err = result.unwrap_err();
    assert!(err.contains("strip failed") && err.contains("main failed"));
}

#[test]
fn barrier_revocation_blocks_final_cache_alert_and_result_commit() {
    let (source, runtime) = injected();
    let store = Arc::new(UsagePublication::default());
    let revision = runtime.snapshot().1;
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let cache = Arc::new(AtomicUsize::new(0));
    let alerts = Arc::new(AtomicUsize::new(0));
    let (r, s, b, c, a) = (
        runtime.clone(),
        store.clone(),
        barrier.clone(),
        cache.clone(),
        alerts.clone(),
    );
    let worker = std::thread::spawn(move || {
        assert!(r.is_current(revision));
        b.wait();
        b.wait();
        s.commit(&r, revision, vec![kimi()], |_| {
            c.fetch_add(1, Ordering::SeqCst);
            a.fetch_add(1, Ordering::SeqCst);
        })
    });
    barrier.wait();
    *source.lock().unwrap() = policy(&[]);
    barrier.wait();
    assert!(worker.join().unwrap().is_none());
    assert_eq!(cache.load(Ordering::SeqCst), 0);
    assert_eq!(alerts.load(Ordering::SeqCst), 0);
    assert!(store.read(&runtime).is_none());
}

#[test]
fn persisted_narrowing_rejects_old_publication_and_folded_rows() {
    let cfg = Arc::new(Mutex::new(
        json!({"accessPolicy":{"version":1,"enabledFamilies":["kimi","moonshot"],"enabledAccounts":["kimi","moonshot"]},"disabled":[]}),
    ));
    let backend = cfg.clone();
    let runtime = AccessRuntime::with_checker(move || {
        AccessPolicy::from_backend_config(&backend.lock().unwrap())
    });
    let store = UsagePublication::default();
    let rev = runtime.snapshot().1;
    store.publish(&runtime, rev, vec![kimi()]).unwrap();
    cfg.lock().unwrap()["disabled"] = json!(["moonshot"]);
    assert!(store.read(&runtime).is_none());
    let current = runtime.snapshot();
    let result = store.publish(&runtime, current.1, vec![kimi()]).unwrap();
    assert_eq!(result.snapshots[0].metrics.len(), 1);
    assert_eq!(result.snapshots[0].metrics[0].label, "Weekly");
}

#[test]
fn strip_tooltip_preserves_stale_warning_without_marking_fresh_data() {
    let p = policy(&["kimi"]);
    let labels = vec!["Weekly".to_string()];
    let fresh = strip_view(&[kimi()], &p, "kimi", &labels).unwrap();
    assert_eq!(fresh.tooltip, "Kimi\nWeekly: 75% left");
    let mut cached = kimi();
    cached.stale = true;
    let stale = strip_view(&[cached], &p, "kimi", &labels).unwrap();
    assert_eq!(stale.tooltip, "⚠ Kimi\nWeekly: 75% left");
    assert_eq!(stale.values, fresh.values);
}

#[test]
fn publication_drops_legacy_local_only_opencode_snapshot() {
    let policy = AccessPolicy::from_config(&json!({"accessPolicy": {
        "version": 1, "enabledFamilies": ["opencode"], "enabledAccounts": ["opencode"]
    }}));
    let runtime = AccessRuntime::new(policy);
    let store = UsagePublication::default();
    let legacy = Snapshot::ok(
        "opencode",
        "OpenCode",
        Some("Go — this PC only".into()),
        vec![Metric::progress("Weekly", 40.0, None)],
    );
    let result = store
        .publish(&runtime, runtime.snapshot().1, vec![legacy])
        .unwrap();
    assert!(
        result.snapshots.is_empty(),
        "legacy local-log estimates are not authorized account-query snapshots"
    );
}
