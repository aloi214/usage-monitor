#![allow(dead_code, unused_mut)]
use serde_json::{json, Value};
use snapshot::Snapshot;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

#[path = "../../../src-tauri/src/provider_auto_refresh.rs"]
mod provider_auto_refresh;
#[path = "../../../src-tauri/src/access_policy.rs"]
mod access_policy;
#[path = "../../../src-tauri/src/snapshot.rs"]
mod snapshot;
#[path = "../../../src-tauri/src/usage_publication.rs"]
mod usage_publication;

extern crate self as tauri;
extern crate self as tauri_plugin_notification;
pub struct AppHandle;
pub mod async_runtime {
    pub use tokio::{spawn, task::spawn_blocking};
}
pub trait NotificationExt {
    fn notification(&self) -> Notification;
}
impl NotificationExt for AppHandle {
    fn notification(&self) -> Notification {
        Notification
    }
}
pub struct Notification;
impl Notification {
    pub fn builder(self) -> Self {
        self
    }
    pub fn title(self, _: &str) -> Self {
        self
    }
    pub fn body(self, _: &str) -> Self {
        self
    }
    pub fn show(self) -> Result<(), String> {
        Ok(())
    }
}
mod alerts {
    pub struct Alert {
        pub title: String,
        pub body: String,
    }
    pub fn evaluate(_: &[crate::snapshot::Snapshot], _: &serde_json::Value) -> Vec<Alert> {
        vec![]
    }
}

fn access_runtime() -> &'static Arc<access_policy::AccessRuntime> {
    static RUNTIME: OnceLock<Arc<access_policy::AccessRuntime>> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        Arc::new(access_policy::AccessRuntime::new(
            access_policy::AccessPolicy::default(),
        ))
    })
}
fn usage_publications() -> &'static usage_publication::UsagePublication {
    static STORE: OnceLock<usage_publication::UsagePublication> = OnceLock::new();
    STORE.get_or_init(Default::default)
}
fn fixture_config() -> &'static Mutex<Value> {
    static CONFIG: OnceLock<Mutex<Value>> = OnceLock::new();
    CONFIG.get_or_init(|| Mutex::new(json!({})))
}
fn load_config() -> Value {
    let mut cfg = fixture_config().lock().unwrap().clone();
    cfg["accessPolicy"] = json!(access_runtime().snapshot().0);
    cfg
}
fn config_with_defaults(value: Value) -> Value {
    value
}
static CONFIG_WRITE: Mutex<()> = Mutex::new(());
fn persist_config_in(_: &std::path::Path, config: &Value) -> Result<(), String> {
    access_runtime().update(access_policy::AccessPolicy::from_config(config));
    Ok(())
}
mod spend {
    #[derive(serde::Serialize)]
    pub struct ProviderSpend { pub id: String }
    pub fn invalidate_published() {}
    pub fn collect(_: &super::FixtureScanPolicy, _: Option<String>) -> Vec<ProviderSpend> {
        super::providers::count("local:collect"); vec![ProviderSpend{id:"local:codex".into()}]
    }
    pub fn cursor_from_csv(_: &str) -> ProviderSpend { ProviderSpend{id:"cursor".into()} }
    pub fn provider_spend_has_data(_: &ProviderSpend) -> bool {true}
}
struct FixtureScanPolicy { revision: u64 }
fn local_scan_policy() -> FixtureScanPolicy { FixtureScanPolicy{ revision: access_runtime().snapshot().1 } }

const SNAPSHOT_CACHE_MS: i64 = 24 * 60 * 60 * 1000;
const STALE_GRACE_MS: i64 = 3 * 60 * 1000;
fn fail_state() -> &'static Mutex<HashMap<String, FailState>> {
    static STATE: OnceLock<Mutex<HashMap<String, FailState>>> = OnceLock::new();
    STATE.get_or_init(Default::default)
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct CachedSnap {
    at: i64,
    snap: snapshot::Snapshot,
}
fn last_ok() -> &'static Mutex<HashMap<String, CachedSnap>> {
    static CACHE: OnceLock<Mutex<HashMap<String, CachedSnap>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}
fn persist_last_ok(map: &HashMap<String, CachedSnap>) -> Result<(), String> {
    persist_last_ok_at(&providers::config_dir().join("last_snapshots.json"), map)
}
fn retired_id(_: &str) -> bool {
    false
}
static KEY_CARD_PUBLICATION: Mutex<()> = Mutex::new(());
static KEY_CARD_SNAPSHOT_GENERATIONS: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
struct CreditMeterBindGuard;
impl CreditMeterBindGuard {
    fn begin(_: Vec<String>) -> Self {
        Self
    }
}
fn credit_meter_bind_ids(id: &str) -> Vec<String> {
    vec![id.to_string()]
}
fn cached_kimi_ok() -> bool {
    providers::KIMI_LIVE.load(Ordering::SeqCst) != 0
}

pub mod providers {
    use super::*;
    pub use crate::snapshot::{Metric, Snapshot};
    pub fn config_dir() -> PathBuf {
        std::env::temp_dir().join(format!("pane-usage-synthetic-{}", std::process::id()))
    }
    pub fn read_count(id: &str) -> usize {
        COUNTS
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|m| m.get(id))
            .copied()
            .unwrap_or(0)
    }
    pub(super) static COUNTS: Mutex<Option<HashMap<String, usize>>> = Mutex::new(None);
    pub fn count(id: &str) {
        *COUNTS
            .lock()
            .unwrap()
            .get_or_insert_default()
            .entry(id.into())
            .or_default() += 1;
    }
    pub fn reset_counts() {
        *COUNTS.lock().unwrap() = None;
    }
    pub static COMMANDCODE_WARNING: AtomicUsize = AtomicUsize::new(0);
    pub static COMMANDCODE_IDENTITY: Mutex<Option<String>> = Mutex::new(None);
    pub static CLAUDE_ERROR: AtomicUsize = AtomicUsize::new(0);
    pub static BLOCK_CLAUDE: AtomicUsize = AtomicUsize::new(0);
    pub static SKIP_EXTRA: AtomicUsize = AtomicUsize::new(0);
    pub static KIMI_LIVE: AtomicUsize = AtomicUsize::new(0);
    pub static CLOSE_WALLET_IN_PLAN_KEY: AtomicUsize = AtomicUsize::new(0);
    pub static CLOSE_WALLET_AFTER_QUERY: AtomicUsize = AtomicUsize::new(0);
    pub static WALLET_VALUE: AtomicUsize = AtomicUsize::new(77);
    pub static ERROR_ACCOUNT: Mutex<Option<String>> = Mutex::new(None);
    pub static BLOCK_ACCOUNT: Mutex<Option<String>> = Mutex::new(None);
    pub static FOLLOW_ON_ACCOUNT: Mutex<Option<String>> = Mutex::new(None);
    pub async fn query(id: &str, name: &str) -> Snapshot {
        count(&format!("query:{id}"));
        while BLOCK_ACCOUNT.lock().unwrap().as_deref() == Some(id) {
            tokio::task::yield_now().await;
        }
        if FOLLOW_ON_ACCOUNT.lock().unwrap().as_deref() == Some(id) && crate::access_policy::check_current_operation().is_ok() { count(&format!("follow_on:{id}")); }
        if ERROR_ACCOUNT.lock().unwrap().as_deref() == Some(id) {
            return Snapshot::error(id, name, "synthetic provider failure".into());
        }
        if id.starts_with("claude") {
            count("claude:auth");
            while BLOCK_CLAUDE.load(Ordering::SeqCst) != 0 {
                tokio::task::yield_now().await;
            }
            if CLAUDE_ERROR.load(Ordering::SeqCst) != 0 {
                return Snapshot::error(
                    id,
                    name,
                    "usage endpoint: HTTP 429 retry_after_s=600".into(),
                );
            }
        }
        let mut snapshot = Snapshot::ok(
            id,
            name,
            None,
            vec![Metric::progress("Session", 35.0, None)],
        );
        if id == "commandcode" && COMMANDCODE_WARNING.load(Ordering::SeqCst) != 0 {
            snapshot.warning = Some("CommandCode subscriptions: HTTP 429 retry_after_s=120".into());
        }
        snapshot
    }
    macro_rules! adapter {
        ($($name:ident),+) => {$ (pub mod $name { pub async fn snapshot() -> super::Snapshot { super::query(stringify!($name), stringify!($name)).await } })+};
    }
    adapter!(
        copilot,
        grok,
        devin,
        minimax,
        openrouter,
        zai,
        antigravity,
        deepseek,
        elevenlabs,
        ollama,
        codebuff,
        kilo,
        aihubmix,
        qwen,
        hermes
    );
    pub mod cursor {
        pub async fn snapshot() -> super::Snapshot {super::query("cursor", "Cursor").await}
        pub async fn fetch_usage_csv() -> Option<String> {
            super::count("cursor:csv_auth"); super::count("cursor:csv_query");
            while super::BLOCK_ACCOUNT.lock().unwrap().as_deref()==Some("cursor-csv") {tokio::task::yield_now().await;}
            Some("synthetic csv".into())
        }
    }
    pub mod commandcode {
        pub async fn snapshot() -> super::Snapshot {
            super::query("commandcode", "CommandCode").await
        }
        pub fn default_identity() -> Option<String> {
            super::count("commandcode:identity");
            super::COMMANDCODE_IDENTITY.lock().unwrap().clone()
        }
    }
    pub mod moonshot {
        pub async fn snapshot() -> super::Snapshot {
            super::query("moonshot", "Moonshot").await
        }
        pub async fn api_rows() -> Result<Vec<super::Metric>, String> {
            super::count("moonshot:wallet_auth");
            super::count("moonshot:wallet_query");
            if super::ERROR_ACCOUNT.lock().unwrap().as_deref() == Some("moonshot") {
                return Err("synthetic wallet HTTP 429 retry_after_s=600".into());
            }
            if super::CLOSE_WALLET_AFTER_QUERY.load(std::sync::atomic::Ordering::SeqCst) != 0 {
                crate::fixture_config().lock().unwrap()["providerAutoRefresh"] = serde_json::json!({"moonshot":false});
            }
            Ok(vec![
                super::Metric::progress("API", super::WALLET_VALUE.load(std::sync::atomic::Ordering::SeqCst) as f64, None),
                super::Metric::text("Balance", "$23".into()),
            ])
        }
    }
    pub mod kimi {
        use super::Snapshot;
        const ID: &str = "kimi";
        const NAME: &str = "Kimi";
        pub fn has_credentials() -> bool {
            super::count("kimi:credentials");
            super::KIMI_LIVE.load(std::sync::atomic::Ordering::SeqCst) != 0
        }
        fn cred_path() -> Option<std::path::PathBuf> {
            super::count("kimi:credential_path");
            Some(std::path::PathBuf::from("/synthetic-kimi-fixture"))
        }
        fn plan_key() -> Option<String> {
            super::count("kimi:plan_key");
            if super::CLOSE_WALLET_IN_PLAN_KEY.load(std::sync::atomic::Ordering::SeqCst) != 0 {
                crate::fixture_config().lock().unwrap()["providerAutoRefresh"] = serde_json::json!({"moonshot":false});
            }
            None
        }
        async fn load_usages(
            _: Option<&std::path::Path>,
            _: Option<&str>,
        ) -> Result<(serde_json::Value, Option<String>), String> {
            let snapshot = super::query(ID, NAME).await;
            if let Some(error) = snapshot.error {
                return Err(error);
            }
            Ok((serde_json::to_value(snapshot).unwrap(), None))
        }
        fn parse_snapshot(doc: &serde_json::Value) -> Result<Snapshot, String> {
            serde_json::from_value(doc.clone()).map_err(|e| e.to_string())
        }
        include!(concat!(env!("OUT_DIR"), "/kimi_composite.rs"));
    }
    pub mod stepfun {
        pub fn default_identity() -> Option<String> {
            super::count("stepfun:identity");
            Some("stepfun-synthetic".into())
        }
        pub async fn snapshot() -> super::Snapshot {
            super::query("stepfun", "StepFun").await
        }
    }
    pub struct Account {
        pub id: String,
        pub name: String,
        pub dir: PathBuf,
        pub fingerprint: String,
    }
    macro_rules! multi_adapter {
        ($name:ident) => {
            pub mod $name {
                pub fn default_identity() -> Option<String> {
                    super::count(concat!(stringify!($name), ":identity"));
                    Some(concat!(stringify!($name), "-synthetic").into())
                }
                pub fn discover_extra_accounts_for(
                    policy: &crate::access_policy::AccessPolicy,
                ) -> Vec<super::Account> {
                    super::count(concat!(stringify!($name), ":discovery"));
                    if super::SKIP_EXTRA.load(std::sync::atomic::Ordering::SeqCst) != 0 {
                        return Vec::new();
                    }
                    policy
                        .account_bindings
                        .iter()
                        .filter(|(id, binding)| {
                            binding.family == stringify!($name) && policy.allows_account(id)
                        })
                        .map(|(id, binding)| {
                            super::count(concat!(stringify!($name), ":discovery_auth"));
                            super::Account {
                                id: id.clone(),
                                name: binding.name.clone(),
                                dir: binding.directory.clone(),
                                fingerprint: "synthetic".into(),
                            }
                        })
                        .collect()
                }
                pub async fn snapshot() -> super::Snapshot {
                    super::query(stringify!($name), stringify!($name)).await
                }
                pub async fn snapshot_at(
                    _: std::path::PathBuf,
                    id: String,
                    name: String,
                ) -> super::Snapshot {
                    super::query(&id, &name).await
                }
            }
        };
    }
    multi_adapter!(claude);
    multi_adapter!(codex);
    pub mod opencode {
        pub fn default_identity() -> Option<String> {
            super::count("opencode:identity");
            Some("opencode-synthetic".into())
        }
        pub fn discover_extra_accounts_for(
            policy: &crate::access_policy::AccessPolicy,
        ) -> Vec<super::Account> {
            super::count("opencode:discovery");
            policy
                .account_bindings
                .iter()
                .filter(|(id, b)| b.family == "opencode" && policy.allows_account(id))
                .map(|(id, b)| {
                    super::count("opencode:discovery_auth");
                    super::Account {
                        id: id.clone(),
                        name: b.name.clone(),
                        dir: b.directory.clone(),
                        fingerprint: "synthetic".into(),
                    }
                })
                .collect()
        }
        pub async fn snapshot() -> super::Snapshot {
            super::query("opencode", "OpenCode").await
        }
        pub async fn snapshot_at(
            _: std::path::PathBuf,
            id: String,
            name: String,
            _: Option<String>,
        ) -> super::Snapshot {
            super::query(&id, &name).await
        }
    }
}
include!(concat!(env!("OUT_DIR"), "/usage_commands.rs"));

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) static SERIAL: Mutex<()> = Mutex::new(());
    pub(super) fn setup() {
        *fixture_config().lock().unwrap() = json!({});
        last_ok().lock().unwrap().clear();
        fail_state().lock().unwrap().clear();
        usage_publications().clear();
        providers::reset_counts();
        providers::CLAUDE_ERROR.store(0, Ordering::SeqCst);
        providers::COMMANDCODE_WARNING.store(0, Ordering::SeqCst);
        *providers::COMMANDCODE_IDENTITY.lock().unwrap() = Some("synthetic-a".into());
        providers::BLOCK_CLAUDE.store(0, Ordering::SeqCst);
        providers::SKIP_EXTRA.store(0, Ordering::SeqCst);
        providers::KIMI_LIVE.store(0, Ordering::SeqCst);
        providers::CLOSE_WALLET_IN_PLAN_KEY.store(0, Ordering::SeqCst);
        providers::CLOSE_WALLET_AFTER_QUERY.store(0, Ordering::SeqCst);
        providers::WALLET_VALUE.store(77, Ordering::SeqCst);
        *providers::ERROR_ACCOUNT.lock().unwrap() = None;
        *providers::BLOCK_ACCOUNT.lock().unwrap() = None;
        *providers::FOLLOW_ON_ACCOUNT.lock().unwrap() = None;
        let dir = providers::config_dir();
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut policy = access_policy::AccessPolicy::default();
        policy
            .enabled_families
            .extend(["claude".into(), "codex".into()]);
        policy.account_bindings.insert(
            "claude@abcd1234".into(),
            access_policy::AccountBinding {
                family: "claude".into(),
                directory: dir.join("extra"),
                name: "Claude — remembered synthetic".into(),
            },
        );
        policy
            .enabled_accounts
            .extend(["claude".into(), "claude@abcd1234".into(), "codex".into()]);
        access_runtime().update(policy);
    }
    fn claude<'a>(batch: &'a usage_publication::UsageResult, id: &str) -> &'a snapshot::Snapshot {
        batch
            .snapshots
            .iter()
            .find(|s| s.id == id)
            .expect("authorized Claude card")
    }
    fn assert_no_claude_io() {
        for key in [
            "query:claude",
            "query:claude@abcd1234",
            "claude:auth",
            "claude:identity",
            "claude:discovery",
            "claude:discovery_auth",
        ] {
            assert_eq!(
                providers::read_count(key),
                0,
                "automatic path performed {key}"
            );
        }
    }
    fn seed_old_cache() -> i64 {
        let at = now_ms() as i64 - 3 * SNAPSHOT_CACHE_MS;
        let mut snap = snapshot::Snapshot::ok(
            "claude",
            "Claude — saved synthetic account",
            None,
            vec![snapshot::Metric::progress("Session", 12.0, None)],
        );
        snap.fetched_at = Some(at);
        let mut cache = last_ok().lock().unwrap();
        cache.insert("claude".into(), CachedSnap { at, snap });
        persist_last_ok(&cache).unwrap();
        at
    }
    #[tokio::test]
    async fn automatic_usage_never_queries_or_discovers_claude_but_other_accounts_still_refresh() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        setup();
        for reason in [None, Some("automatic")] {
            let batch = fetch_fixture(reason, None).await;
            assert_no_claude_io();
            for id in ["claude", "claude@abcd1234"] {
                let s = claude(&batch, id);
                assert_eq!(s.status, "manual");
                assert!(s.error.is_none());
                assert!(s.fetched_at.is_none());
                assert!(s.metrics.is_empty());
            }
        }
        assert_eq!(providers::read_count("query:codex"), 2);
    }
    #[tokio::test]
    async fn cache_paint_and_automatic_refresh_keep_old_manual_success_without_auth_reads_or_new_success_time(
    ) {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        setup();
        let at = seed_old_cache();
        let cached = cached_usage();
        assert_no_claude_io();
        assert_eq!(claude(&cached, "claude").fetched_at, Some(at));
        assert_eq!(
            claude(&cached, "claude").metrics[0].used_percent,
            Some(12.0)
        );
        for _ in 0..2 {
            let batch = fetch_fixture(None, None).await;
            assert_no_claude_io();
            let s = claude(&batch, "claude");
            assert_eq!(s.status, "ok");
            assert_eq!(s.fetched_at, Some(at));
            assert!(!s.attempt_failed);
            assert!(s.warning.is_none());
            assert!(s.stale);
            assert_eq!(last_ok().lock().unwrap()["claude"].at, at);
        }
    }
    #[tokio::test]
    async fn explicit_refresh_queries_enabled_claude_and_retains_cooldown_and_real_failure_on_automatic_replay(
    ) {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        setup();
        let good = fetch_fixture(Some("userRefresh"), None).await;
        let at = claude(&good, "claude").fetched_at;
        assert!(at.is_some());
        assert_eq!(providers::read_count("query:claude"), 1);
        assert_eq!(providers::read_count("query:claude@abcd1234"), 1);
        providers::CLAUDE_ERROR.store(1, Ordering::SeqCst);
        let failed = fetch_fixture(Some("userRefresh"), None).await;
        assert!(claude(&failed, "claude").attempt_failed);
        assert!(claude(&failed, "claude")
            .warning
            .as_deref()
            .is_some_and(|w| w.contains("429")));
        assert_eq!(claude(&failed, "claude").fetched_at, at);
        let until = fail_state().lock().unwrap()["claude"].until_ms;
        assert!(until >= now_ms() as i64 + 590_000);
        providers::reset_counts();
        let replay = fetch_fixture(None, None).await;
        assert_no_claude_io();
        assert!(claude(&replay, "claude").attempt_failed);
        assert_eq!(claude(&replay, "claude").fetched_at, at);
        let _ = fetch_fixture(Some("userRefresh"), None).await;
        assert_eq!(
            providers::read_count("query:claude"),
            0,
            "manual clicks cannot bypass 429 cooldown"
        );
        assert_eq!(providers::read_count("query:claude@abcd1234"), 0);
        assert_eq!(fail_state().lock().unwrap()["claude"].until_ms, until);
    }
    #[tokio::test]
    async fn manual_intent_does_not_grant_disabled_accounts_and_revocation_fences_inflight_results()
    {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        setup();
        let batch = fetch_fixture(
            Some("userRefresh"),
            Some(vec!["claude".into(), "claude@abcd1234".into()]),
        )
        .await;
        assert!(batch.snapshots.iter().all(|s| !s.id.starts_with("claude")));
        assert_no_claude_io();
        providers::BLOCK_CLAUDE.store(1, Ordering::SeqCst);
        let pending = fetch_fixture(Some("userRefresh"), None);
        let revoke = async {
            while providers::read_count("query:claude") == 0 {
                tokio::task::yield_now().await;
            }
            access_runtime().update(access_policy::AccessPolicy::default());
            providers::BLOCK_CLAUDE.store(0, Ordering::SeqCst);
        };
        let (result, ()) = tokio::join!(pending, revoke);
        assert!(result.snapshots.is_empty());
        assert!(last_ok()
            .lock()
            .unwrap()
            .keys()
            .all(|id| !id.starts_with("claude")));
        assert!(cached_usage().snapshots.is_empty());
    }
    #[tokio::test]
    async fn unrelated_access_edit_preserves_claude_failure_but_revoke_and_reenable_clears_it() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        setup();
        fetch_fixture(Some("userRefresh"), None).await;
        providers::CLAUDE_ERROR.store(1, Ordering::SeqCst);
        let failed = fetch_fixture(Some("userRefresh"), None).await;
        let warning = claude(&failed, "claude").warning.clone();
        change_access_policy(|policy| policy.set_family("qwen", true)).unwrap();
        providers::reset_counts();
        let replay = fetch_fixture(None, None).await;
        assert_no_claude_io();
        assert_eq!(claude(&replay, "claude").warning, warning);
        assert!(claude(&replay, "claude").attempt_failed);
        change_access_policy(|policy| policy.set_account("claude", false)).unwrap();
        change_access_policy(|policy| policy.set_account("claude", true)).unwrap();
        let reset = fetch_fixture(None, None).await;
        assert_no_claude_io();
        assert_eq!(claude(&reset, "claude").status, "manual");
        assert!(claude(&reset, "claude").warning.is_none());
    }
    #[tokio::test]
    async fn failed_first_manual_query_preserves_attempt_time_without_inventing_success_on_auto_replay(
    ) {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        setup();
        providers::CLAUDE_ERROR.store(1, Ordering::SeqCst);
        let failed = fetch_fixture(Some("userRefresh"), None).await;
        let first = claude(&failed, "claude");
        assert_eq!(first.status, "error");
        assert!(first.fetched_at.is_none());
        let first_wire = provider_json(first, "publication-before");
        change_access_policy(|policy| policy.set_family("qwen", true)).unwrap();
        providers::reset_counts();
        let replay = fetch_fixture(None, None).await;
        assert_no_claude_io();
        let current = claude(&replay, "claude");
        assert_eq!(current.status, "error");
        assert!(current.fetched_at.is_none());
        assert_eq!(
            provider_json(current, "publication-after")["fetchedAt"],
            first_wire["fetchedAt"]
        );
        assert_ne!(
            first_wire["fetchedAt"], "publication-before",
            "the timestamp must come from the real attempt"
        );
    }
}

#[cfg(test)]
mod scoped_tests {
    use super::tests::{setup, SERIAL};
    use super::*;
    pub(super) fn setup_all() {
        setup();
        let (mut policy, _) = access_runtime().snapshot();
        for id in access_policy::FAMILIES {
            policy.enabled_families.insert((*id).into());
            policy.enabled_accounts.insert((*id).into());
        }
        for family in ["claude", "codex", "opencode"] {
            for suffix in ["abcd1234", "eeee1234"] {
                let id = format!("{family}@{suffix}");
                policy.account_bindings.insert(
                    id.clone(),
                    access_policy::AccountBinding {
                        family: family.into(),
                        directory: providers::config_dir().join(&id),
                        name: id.clone(),
                    },
                );
                policy.enabled_accounts.insert(id);
            }
        }
        access_runtime().update(policy);
    }
    fn value(batch: &usage_publication::UsageResult, id: &str) -> Value {
        serde_json::to_value(batch.snapshots.iter().find(|s| s.id == id).unwrap()).unwrap()
    }
    #[tokio::test]
    async fn exact_account_scope_precedes_every_unrelated_query_identity_and_discovery_read() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        for target in [
            "claude",
            "claude@abcd1234",
            "codex",
            "codex@abcd1234",
            "opencode@abcd1234",
            "cursor",
            "moonshot",
        ] {
            setup_all();
            let batch = fetch_scoped_fixture(target, Some("userRefresh"), None)
                .await
                .unwrap();
            assert_eq!(
                providers::read_count(&format!("query:{target}")),
                1,
                "target {target}"
            );
            assert_eq!(
                batch.snapshots.len(),
                1,
                "target {target} must be the only initial publication"
            );
            for id in access_runtime().snapshot().0.enabled_accounts {
                if id != target {
                    assert_eq!(
                        providers::read_count(&format!("query:{id}")),
                        0,
                        "{target} queried {id}"
                    );
                }
            }
            for family in ["claude", "codex", "opencode", "stepfun"] {
                if target != family {
                    assert_eq!(
                        providers::read_count(&format!("{family}:identity")),
                        0,
                        "{target} opened default {family} identity"
                    );
                }
                if !target.starts_with(&format!("{family}@")) {
                    assert_eq!(
                        providers::read_count(&format!("{family}:discovery_auth")),
                        0,
                        "{target} opened {family} extras"
                    );
                }
            }
            for key in [
                "kimi:credentials",
                "kimi:credential_path",
                "kimi:plan_key",
                "moonshot:wallet_auth",
                "moonshot:wallet_query",
            ] {
                assert_eq!(
                    providers::read_count(key),
                    0,
                    "{target} crossed unrelated boundary {key}"
                );
            }
        }
    }
    #[tokio::test]
    async fn invalid_and_disabled_targets_reject_before_any_provider_io() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        for target in [
            "missing",
            "claude@unknown0",
            "local:claude",
            "claude@",
            "claude@../../",
            "claude",
        ] {
            setup_all();
            assert!(
                fetch_scoped_fixture(target, Some("userRefresh"), Some(vec!["claude".into()]))
                    .await
                    .is_err()
            );
            assert!(providers::COUNTS
                .lock()
                .unwrap()
                .as_ref()
                .is_none_or(|m| m.is_empty()));
        }
    }
    #[tokio::test]
    async fn scoped_refresh_preserves_unrelated_errors_success_times_cache_and_identity_stamps() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        setup_all();
        let _first = fetch_fixture(Some("userRefresh"), None).await;
        providers::CLAUDE_ERROR.store(1, Ordering::SeqCst);
        let failed = fetch_fixture(Some("userRefresh"), None).await;
        let prior = value(&failed, "claude");
        let codex_cache = serde_json::to_value(&last_ok().lock().unwrap()["codex"]).unwrap();
        let stamps = std::fs::read(providers::config_dir().join("cache_identities.json")).unwrap();
        providers::reset_counts();
        let current = fetch_scoped_fixture("cursor", Some("userRefresh"), None)
            .await
            .unwrap();
        assert_eq!(value(&current, "claude"), prior);
        assert_eq!(value(&current, "codex"), value(&failed, "codex"));
        assert_eq!(
            serde_json::to_value(&last_ok().lock().unwrap()["codex"]).unwrap(),
            codex_cache
        );
        assert_eq!(
            std::fs::read(providers::config_dir().join("cache_identities.json")).unwrap(),
            stamps
        );
        assert_eq!(providers::read_count("query:codex"), 0);
    }
    #[tokio::test]
    async fn kimi_scope_preserves_saved_wallet_without_nested_credentials_or_requests() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        setup_all();
        let before = fetch_fixture(Some("userRefresh"), None).await;
        let old = value(&before, "kimi");
        let wallet_at = old["fetched_at"].as_i64().unwrap();
        let moonshot_cache = serde_json::to_value(&last_ok().lock().unwrap()["moonshot"]).unwrap();
        let mut failure = snapshot::Snapshot::error(
            "moonshot",
            "Moonshot",
            "Moonshot API wallet couldn't refresh — retrying next cycle".into(),
        );
        failure.attempted_at = Some(wallet_at - 12);
        let (policy, rev) = access_runtime().snapshot();
        let mut published = before.snapshots.clone();
        published
            .iter_mut()
            .find(|s| s.id == "kimi")
            .unwrap()
            .warning = Some("Moonshot API wallet couldn't refresh — retrying next cycle".into());
        published.push(failure.clone());
        usage_publications().publish(access_runtime(), rev, published);
        providers::reset_counts();
        let current = fetch_scoped_fixture("kimi", Some("userRefresh"), None)
            .await
            .unwrap();
        assert_eq!(providers::read_count("query:kimi"), 1);
        assert_eq!(providers::read_count("query:moonshot"), 0);
        assert_eq!(providers::read_count("moonshot:wallet_auth"), 0);
        assert_eq!(providers::read_count("moonshot:wallet_query"), 0);
        let kimi = value(&current, "kimi");
        assert_eq!(
            kimi["metrics"], old["metrics"],
            "saved wallet rows must survive plan-only refresh"
        );
        assert_eq!(kimi["wallet_history"]["fetched_at"], wallet_at);
        assert_eq!(
            kimi["wallet_history"]["warning"],
            "Moonshot API wallet couldn't refresh — retrying next cycle"
        );
        assert_eq!(
            value(&current, "moonshot"),
            serde_json::to_value(failure).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&last_ok().lock().unwrap()["moonshot"]).unwrap(),
            moonshot_cache
        );
        let snap = current.snapshots.iter().find(|s| s.id == "kimi").unwrap();
        let wire = provider_json(snap, "new publication");
        let wallet = wire["lines"]
            .as_array()
            .unwrap()
            .iter()
            .find(|line| line["label"] == "API")
            .unwrap();
        assert_eq!(wallet["fetchedAt"], iso8601(wallet_at));
        assert_eq!(wallet["stale"], true);
        assert_eq!(wallet["sourceAccountId"], "moonshot");
        assert_eq!(wire["stale"], true);
        let strip =
            usage_publication::strip_view(&current.snapshots, &policy, "kimi", &["API".into()])
                .unwrap();
        assert!(strip.tooltip.contains("saved wallet"), "{}", strip.tooltip);
        let refreshed = fetch_fixture(Some("userRefresh"), None).await;
        assert!(value(&refreshed, "kimi").get("wallet_history").is_none());
    }
    #[tokio::test]
    async fn scoped_query_does_not_invent_wallet_or_refresh_automatic_claude() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        setup_all();
        let kimi = fetch_scoped_fixture("kimi", Some("userRefresh"), None)
            .await
            .unwrap();
        assert!(value(&kimi, "kimi")["metrics"]
            .as_array()
            .unwrap()
            .iter()
            .all(|m| m["label"] != "API"));
        providers::reset_counts();
        let claude = fetch_scoped_fixture("claude@abcd1234", None, None)
            .await
            .unwrap();
        assert_eq!(value(&claude, "claude@abcd1234")["status"], "manual");
        assert_eq!(providers::read_count("query:claude@abcd1234"), 0);
        assert_eq!(providers::read_count("claude:identity"), 0);
        assert_eq!(providers::read_count("claude:discovery_auth"), 0);
    }

    #[tokio::test]
    async fn target_error_has_real_attempt_clock_and_unrelated_error_http_clock_never_moves() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        setup_all();
        *providers::ERROR_ACCOUNT.lock().unwrap() = Some("cursor".into());
        let start = now_ms() as i64;
        let failed = fetch_scoped_fixture("cursor", Some("userRefresh"), None)
            .await
            .unwrap();
        let error = failed.snapshots.iter().find(|s| s.id == "cursor").unwrap();
        assert!(error.attempted_at.is_some_and(|at| at >= start));
        let first_wire = provider_json(error, "original publication");
        assert_ne!(first_wire["fetchedAt"], "original publication");
        let bench = fail_state().lock().unwrap()["cursor"].until_ms;
        let current = fetch_scoped_fixture("codex", Some("userRefresh"), None)
            .await
            .unwrap();
        let retained = current.snapshots.iter().find(|s| s.id == "cursor").unwrap();
        assert_eq!(provider_json(retained, "later publication"), first_wire);
        let retry = fetch_scoped_fixture("cursor", Some("userRefresh"), None)
            .await
            .unwrap();
        assert_eq!(
            value(&retry, "cursor")["attempted_at"],
            value(&failed, "cursor")["attempted_at"]
        );
        assert_eq!(providers::read_count("query:cursor"), 1);
        assert_eq!(fail_state().lock().unwrap()["cursor"].until_ms, bench);
    }
    #[tokio::test]
    async fn backend_serializes_target_and_full_refresh_without_losing_cards_or_newer_results() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        setup_all();
        providers::BLOCK_CLAUDE.store(1, Ordering::SeqCst);
        let targeted = fetch_scoped_fixture("claude@abcd1234", Some("userRefresh"), None);
        let queued = async {
            while providers::read_count("query:claude@abcd1234") == 0 {
                tokio::task::yield_now().await;
            }
            let full = fetch_fixture(None, None);
            let release = async {
                for _ in 0..10 {
                    tokio::task::yield_now().await;
                }
                assert_eq!(
                    providers::read_count("query:codex"),
                    0,
                    "global must wait for target publication"
                );
                providers::BLOCK_CLAUDE.store(0, Ordering::SeqCst);
            };
            let (full, ()) = tokio::join!(full, release);
            full
        };
        let (targeted, full) = tokio::join!(targeted, queued);
        let targeted = targeted.unwrap();
        assert_eq!(
            value(&full, "claude@abcd1234")["fetched_at"],
            value(&targeted, "claude@abcd1234")["fetched_at"]
        );
        assert!(full.snapshots.iter().any(|s| s.id == "cursor"));
        assert_eq!(providers::read_count("query:claude"), 0);
        assert_eq!(providers::read_count("query:claude@abcd1234"), 1);
        assert_eq!(providers::read_count("query:codex"), 1);
        assert_eq!(
            serde_json::to_value(usage_publications().read(access_runtime()).unwrap()).unwrap(),
            serde_json::to_value(full).unwrap()
        );
    }
    #[tokio::test]
    async fn scoped_revocation_discards_inflight_data_and_revalidates_queued_target() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        setup_all();
        providers::BLOCK_CLAUDE.store(1, Ordering::SeqCst);
        let pending = fetch_scoped_fixture("claude", Some("userRefresh"), None);
        let revoke = async {
            while providers::read_count("query:claude") == 0 {
                tokio::task::yield_now().await;
            }
            let queued = fetch_scoped_fixture("cursor", Some("userRefresh"), None);
            let change = async {
                for _ in 0..3 {
                    tokio::task::yield_now().await;
                }
                access_runtime().update(access_policy::AccessPolicy::default());
                providers::BLOCK_CLAUDE.store(0, Ordering::SeqCst);
            };
            let (queued, ()) = tokio::join!(queued, change);
            assert!(queued.is_err());
        };
        let (result, ()) = tokio::join!(pending, revoke);
        assert!(result.unwrap().snapshots.is_empty());
        assert_eq!(providers::read_count("query:cursor"), 0);
        assert!(last_ok().lock().unwrap().is_empty());
        assert!(usage_publications().read(access_runtime()).is_none());
    }
    #[tokio::test]
    async fn scoped_generation_rotation_cannot_publish_old_key_or_replace_other_cache() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        setup_all();
        let initial = fetch_scoped_fixture("codex", Some("userRefresh"), None)
            .await
            .unwrap();
        *providers::BLOCK_ACCOUNT.lock().unwrap() = Some("deepseek".into());
        let pending = fetch_scoped_fixture("deepseek", Some("userRefresh"), None);
        let rotate = async {
            while providers::read_count("query:deepseek") == 0 {
                tokio::task::yield_now().await;
            }
            KEY_CARD_SNAPSHOT_GENERATIONS
                .get_or_init(Default::default)
                .lock()
                .unwrap()
                .insert("deepseek".into(), 9);
            *providers::BLOCK_ACCOUNT.lock().unwrap() = None;
        };
        let (result, ()) = tokio::join!(pending, rotate);
        let result = result.unwrap();
        assert_eq!(value(&result, "codex"), value(&initial, "codex"));
        assert!(!result.snapshots.iter().any(|s| s.id == "deepseek"));
        assert!(!last_ok().lock().unwrap().contains_key("deepseek"));
        assert!(!fail_state().lock().unwrap().contains_key("deepseek"));
    }
    #[test]
    fn typed_scope_rejects_unknown_shapes_and_preserves_exact_account_id() {
        for invalid in [
            json!({}),
            json!({"kind":"family","accountId":"claude"}),
            json!({"kind":"account","accountId":12}),
            json!({"kind":"account"}),
            json!({"kind":"account","accountId":"claude","extra":true}),
            json!({"kind":"all","extra":true}),
            json!([]),
        ] {
            assert!(
                serde_json::from_value::<UsageRefreshScope>(invalid.clone()).is_err(),
                "{invalid}"
            );
        }
        assert!(matches!(
            serde_json::from_value::<UsageRefreshScope>(json!({"kind":"all"})).unwrap(),
            UsageRefreshScope::All {}
        ));
    }

    #[tokio::test]
    async fn wallet_revocation_purges_saved_values_errors_and_provenance_from_every_surface() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        setup_all();
        fetch_fixture(Some("userRefresh"), None).await;
        fetch_scoped_fixture("kimi", Some("userRefresh"), None)
            .await
            .unwrap();
        assert!(last_ok().lock().unwrap()["kimi"]
            .snap
            .wallet_history
            .is_some());
        change_access_policy(|policy| policy.set_account("moonshot", false)).unwrap();
        let cache = serde_json::to_value(&last_ok().lock().unwrap()["kimi"].snap).unwrap();
        assert!(
            cache.get("wallet_history").is_none(),
            "revocation must purge wallet provenance/error"
        );
        assert!(cache["metrics"]
            .as_array()
            .unwrap()
            .iter()
            .all(|m| !is_kimi_wallet_label(m["label"].as_str().unwrap())));
        for batch in [
            cached_usage(),
            fetch_scoped_fixture("kimi", Some("userRefresh"), None)
                .await
                .unwrap(),
        ] {
            let snap = batch.snapshots.iter().find(|s| s.id == "kimi").unwrap();
            let wire = provider_json(snap, "publication");
            assert!(snap.wallet_history.is_none());
            assert!(snap.warning.is_none());
            assert!(wire["lines"]
                .as_array()
                .unwrap()
                .iter()
                .all(|m| !is_kimi_wallet_label(m["label"].as_str().unwrap())));
            assert!(usage_publication::strip_view(
                &batch.snapshots,
                &access_runtime().snapshot().0,
                "kimi",
                &["API".into()]
            )
            .is_none());
        }
    }
    #[tokio::test]
    async fn wallet_revoked_during_plan_query_cannot_republish_saved_wallet() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        setup_all();
        fetch_fixture(Some("userRefresh"), None).await;
        *providers::BLOCK_ACCOUNT.lock().unwrap() = Some("kimi".into());
        providers::reset_counts();
        let pending = fetch_scoped_fixture("kimi", Some("userRefresh"), None);
        let revoke = async {
            while providers::read_count("query:kimi") == 0 {
                tokio::task::yield_now().await;
            }
            change_access_policy(|policy| policy.set_account("moonshot", false)).unwrap();
            *providers::BLOCK_ACCOUNT.lock().unwrap() = None;
        };
        let (result, ()) = tokio::join!(pending, revoke);
        assert!(result.unwrap().snapshots.is_empty());
        assert!(usage_publications().read(access_runtime()).is_none());
        let cache = &last_ok().lock().unwrap()["kimi"].snap;
        assert!(cache
            .metrics
            .iter()
            .all(|m| !is_kimi_wallet_label(&m.label)));
        assert!(cache.wallet_history.is_none());
    }

    #[tokio::test]
    async fn nonidentity_target_does_not_rewrite_or_invent_other_identity_stamps() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        setup_all();
        let path = providers::config_dir().join("cache_identities.json");
        assert!(!path.exists());
        fetch_scoped_fixture("cursor", Some("userRefresh"), None)
            .await
            .unwrap();
        assert!(
            !path.exists(),
            "a cursor-only query cannot create identity stamps for other accounts"
        );
        let sparse = br#"{"claude":"saved identity"}"#;
        std::fs::write(&path, sparse).unwrap();
        fetch_scoped_fixture("cursor", Some("userRefresh"), None)
            .await
            .unwrap();
        assert_eq!(std::fs::read(path).unwrap(), sparse);
    }
    #[tokio::test]
    async fn late_cached_usage_cannot_replace_current_target_error_or_full_publication() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        setup_all();
        fetch_fixture(Some("userRefresh"), None).await;
        *providers::ERROR_ACCOUNT.lock().unwrap() = Some("cursor".into());
        let latest = fetch_scoped_fixture("cursor", Some("userRefresh"), None)
            .await
            .unwrap();
        let expected = serde_json::to_value(latest).unwrap();
        providers::reset_counts();
        assert_eq!(serde_json::to_value(cached_usage()).unwrap(), expected);
        assert_eq!(
            serde_json::to_value(usage_publications().read(access_runtime()).unwrap()).unwrap(),
            expected
        );
        assert!(providers::COUNTS
            .lock()
            .unwrap()
            .as_ref()
            .is_none_or(|m| m.is_empty()));
    }

    #[tokio::test]
    async fn global_disabled_wallet_is_not_restored_and_kimi_plan_error_survives_sanitization() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        setup_all();
        fetch_fixture(Some("userRefresh"), None).await;
        fetch_scoped_fixture("kimi", Some("userRefresh"), None)
            .await
            .unwrap();
        {
            let mut cache = last_ok().lock().unwrap();
            let saved = cache.get_mut("kimi").unwrap();
            saved.at = now_ms() as i64 - STALE_GRACE_MS - 1_000;
            saved.snap.fetched_at = Some(saved.at);
        }
        *providers::ERROR_ACCOUNT.lock().unwrap() = Some("kimi".into());
        let failed = fetch_fixture(Some("userRefresh"), Some(vec!["moonshot".into()])).await;
        let kimi = failed.snapshots.iter().find(|s| s.id == "kimi").unwrap();
        assert!(kimi.metrics.iter().all(|m| !is_kimi_wallet_label(&m.label)));
        assert!(kimi.wallet_history.is_none());
        assert_eq!(kimi.warning.as_deref(), Some("synthetic provider failure"));
    }
    #[tokio::test]
    async fn unavailable_remembered_target_reports_its_own_revalidation_time() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        setup_all();
        fetch_scoped_fixture("cursor", Some("userRefresh"), None)
            .await
            .unwrap();
        providers::SKIP_EXTRA.store(1, Ordering::SeqCst);
        let started = now_ms() as i64;
        let batch = fetch_scoped_fixture("claude@abcd1234", Some("userRefresh"), None)
            .await
            .unwrap();
        let target = batch
            .snapshots
            .iter()
            .find(|s| s.id == "claude@abcd1234")
            .unwrap();
        assert_eq!(target.status, "error");
        assert!(target.attempted_at.is_some_and(|at| at >= started));
        assert_eq!(providers::read_count("query:claude@abcd1234"), 0);
        assert!(batch.snapshots.iter().any(|s| s.id == "cursor"));
    }
    #[tokio::test]
    async fn failed_wallet_attempt_clock_comes_from_wallet_result_and_cooldown_never_retimes_it() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        setup_all();
        let before = fetch_fixture(Some("userRefresh"), None).await;
        let prior = before.snapshots.iter().find(|s| s.id == "kimi").unwrap();
        let prior_fetched = prior.fetched_at;
        providers::KIMI_LIVE.store(1, Ordering::SeqCst);
        providers::reset_counts();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        *providers::ERROR_ACCOUNT.lock().unwrap() = Some("moonshot".into());
        let started = now_ms() as i64;
        let failed = fetch_fixture(Some("userRefresh"), None).await;
        let current = failed.snapshots.iter().find(|s| s.id == "kimi").unwrap();
        let history = current.wallet_history.as_ref().unwrap();
        assert_eq!(history.fetched_at, prior_fetched);
        assert!(
            history.attempted_at.is_some_and(|at| at >= started),
            "wallet failure inherited an old attempt clock"
        );
        assert_eq!(
            history.attempted_at,
            fail_state().lock().unwrap()["moonshot"]
                .observed_error
                .as_ref()
                .unwrap()
                .attempted_at
        );
        let attempted = history.attempted_at;
        assert_eq!(providers::read_count("moonshot:wallet_query"), 1);
        assert_eq!(
            providers::read_count("query:moonshot"),
            0,
            "live Kimi folds the standalone wallet"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let replayed = fetch_fixture(Some("userRefresh"), None).await;
        let replay = replayed
            .snapshots
            .iter()
            .find(|s| s.id == "kimi")
            .unwrap()
            .wallet_history
            .as_ref()
            .unwrap();
        assert_eq!(
            replay.attempted_at, attempted,
            "benched wallet was never queried again"
        );
        assert_eq!(replay.fetched_at, prior_fetched);
        assert_eq!(providers::read_count("moonshot:wallet_query"), 1);
        providers::reset_counts();
        let plan_only = fetch_scoped_fixture("kimi", Some("userRefresh"), None)
            .await
            .unwrap();
        let saved = plan_only
            .snapshots
            .iter()
            .find(|s| s.id == "kimi")
            .unwrap()
            .wallet_history
            .as_ref()
            .unwrap();
        assert_eq!(saved.attempted_at, attempted);
        assert_eq!(saved.fetched_at, prior_fetched);
        assert_eq!(providers::read_count("moonshot:wallet_query"), 0);
        let wallet_until = fail_state().lock().unwrap()["moonshot"].until_ms;
        *providers::ERROR_ACCOUNT.lock().unwrap() = Some("kimi".into());
        fetch_scoped_fixture("kimi", Some("userRefresh"), None)
            .await
            .unwrap();
        assert_eq!(
            fail_state().lock().unwrap()["moonshot"].until_ms,
            wallet_until
        );
        assert!(fail_state().lock().unwrap().contains_key("kimi"));
        assert_eq!(providers::read_count("moonshot:wallet_query"), 0);
        // Expire only synthetic cooldown clocks; recovery still runs the real
        // command, composite body and guard instead of bypassing those checks.
        for failure in fail_state().lock().unwrap().values_mut() {
            failure.until_ms = now_ms() as i64 - 1;
        }
        *providers::ERROR_ACCOUNT.lock().unwrap() = None;
        let recovered = fetch_fixture(Some("userRefresh"), None).await;
        let fresh = recovered.snapshots.iter().find(|s| s.id == "kimi").unwrap();
        assert!(fresh.wallet_history.is_none());
        assert!(fresh.warning.is_none());
        assert!(!fail_state().lock().unwrap().contains_key("moonshot"));
        assert!(!fail_state().lock().unwrap().contains_key("kimi"));
        assert_eq!(providers::read_count("moonshot:wallet_query"), 1);
    }

    #[tokio::test]
    async fn wallet_failure_preserves_unknown_source_success_clocks() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        for explicit_unknown in [false, true] {
            setup_all();
            let at = now_ms() as i64 - 5_000;
            let mut snap = Snapshot::ok(
                "kimi",
                "Legacy Kimi",
                None,
                vec![snapshot::Metric::progress("API", 42.0, None)],
            );
            if explicit_unknown {
                snap.wallet_history = Some(snapshot::WalletHistory {
                    source_account_id: "moonshot".into(),
                    fetched_at: None,
                    attempted_at: None,
                    warning: None,
                });
            }
            last_ok()
                .lock()
                .unwrap()
                .insert("kimi".into(), CachedSnap { at, snap });
            providers::KIMI_LIVE.store(1, Ordering::SeqCst);
            *providers::ERROR_ACCOUNT.lock().unwrap() = Some("moonshot".into());
            let failed = fetch_fixture(Some("userRefresh"), None).await;
            let kimi = failed.snapshots.iter().find(|s| s.id == "kimi").unwrap();
            assert_eq!(kimi.wallet_history.as_ref().unwrap().fetched_at, None);
            assert!(kimi
                .wallet_history
                .as_ref()
                .unwrap()
                .attempted_at
                .is_some_and(|t| t > at));
        }
    }
    #[tokio::test]
    async fn plan_only_refresh_preserves_first_wallet_failure_without_inventing_wallet_rows() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        setup_all();
        *providers::ERROR_ACCOUNT.lock().unwrap() = Some("moonshot".into());
        let failed = fetch_fixture(Some("userRefresh"), None).await;
        let before = failed.snapshots.iter().find(|s| s.id == "kimi").unwrap();
        let before_history = serde_json::to_value(before.wallet_history.as_ref().unwrap()).unwrap();
        assert!(before
            .metrics
            .iter()
            .all(|m| !is_kimi_wallet_label(&m.label)));
        assert!(before.wallet_history.as_ref().unwrap().fetched_at.is_none());
        providers::reset_counts();
        let current = fetch_scoped_fixture("kimi", Some("userRefresh"), None)
            .await
            .unwrap();
        let kimi = current.snapshots.iter().find(|s| s.id == "kimi").unwrap();
        assert_eq!(
            serde_json::to_value(kimi.wallet_history.as_ref()).unwrap(),
            before_history
        );
        assert!(kimi.metrics.iter().all(|m| !is_kimi_wallet_label(&m.label)));
        assert_eq!(providers::read_count("moonshot:wallet_query"), 0);
        assert_eq!(providers::read_count("query:moonshot"), 0);
        change_access_policy(|policy| policy.set_account("moonshot", false)).unwrap();
        let clean = fetch_scoped_fixture("kimi", Some("userRefresh"), None)
            .await
            .unwrap();
        assert!(clean
            .snapshots
            .iter()
            .find(|s| s.id == "kimi")
            .unwrap()
            .wallet_history
            .is_none());
    }
}

#[cfg(test)]
mod commandcode_tests {
    use super::*;
    fn setup() {
        tests::setup();
        let mut policy = access_runtime().snapshot().0;
        policy.enabled_families.insert("commandcode".into());
        policy.enabled_accounts.insert("commandcode".into());
        access_runtime().update(policy);
    }
    #[tokio::test]
    async fn commandcode_automatic_and_cached_paths_are_manual_without_provider_reads() {
        let _serial = tests::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        setup();
        for reason in [None, Some("automatic")] {
            let batch = fetch_fixture(reason, None).await;
            let snapshot = batch
                .snapshots
                .iter()
                .find(|s| s.id == "commandcode")
                .expect("manual CommandCode card");
            assert_eq!(snapshot.status, "manual");
            assert_eq!(providers::read_count("query:commandcode"), 0);
            assert_eq!(providers::read_count("commandcode:identity"), 0);
        }
    }
    #[tokio::test]
    async fn commandcode_explicit_scope_queries_once_and_global_manual_includes_it() {
        let _serial = tests::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        setup();
        let result = fetch_scoped_fixture("commandcode", Some("userRefresh"), None)
            .await
            .unwrap();
        assert_eq!(
            result
                .snapshots
                .iter()
                .find(|s| s.id == "commandcode")
                .unwrap()
                .status,
            "ok"
        );
        assert_eq!(providers::read_count("query:commandcode"), 1);
        assert_eq!(providers::read_count("query:codex"), 0);
        fetch_fixture(Some("userRefresh"), None).await;
        assert_eq!(providers::read_count("query:commandcode"), 2);
    }

    #[tokio::test]
    async fn commandcode_partial_metadata_rate_limit_keeps_good_credits_and_benches_next_manual_query(
    ) {
        let _serial = tests::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        setup();
        providers::COMMANDCODE_WARNING.store(1, Ordering::SeqCst);
        let first = fetch_scoped_fixture("commandcode", Some("userRefresh"), None)
            .await
            .unwrap();
        let original = first
            .snapshots
            .iter()
            .find(|s| s.id == "commandcode")
            .unwrap();
        assert_eq!(original.status, "ok");
        assert!(original.warning.is_some());
        let at = original.fetched_at;
        let second = fetch_scoped_fixture("commandcode", Some("userRefresh"), None)
            .await
            .unwrap();
        assert_eq!(providers::read_count("query:commandcode"), 1);
        let saved = second
            .snapshots
            .iter()
            .find(|s| s.id == "commandcode")
            .unwrap();
        assert_eq!(saved.fetched_at, at);
        assert!(saved.stale);
        assert!(saved.warning.is_some());
    }
    #[tokio::test]
    async fn commandcode_external_identity_swap_cannot_restore_previous_account_history() {
        let _serial = tests::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        setup();
        fetch_scoped_fixture("commandcode", Some("userRefresh"), None)
            .await
            .unwrap();
        *providers::COMMANDCODE_IDENTITY.lock().unwrap() = Some("synthetic-b".into());
        *providers::ERROR_ACCOUNT.lock().unwrap() = Some("commandcode".into());
        let result = fetch_scoped_fixture("commandcode", Some("userRefresh"), None)
            .await
            .unwrap();
        let s = result
            .snapshots
            .iter()
            .find(|s| s.id == "commandcode")
            .unwrap();
        assert_eq!(s.status, "error");
        assert!(s.metrics.is_empty());
        assert!(!last_ok().lock().unwrap().contains_key("commandcode"));
    }

    #[tokio::test]
    async fn commandcode_cached_and_automatic_replay_keep_old_success_and_no_key_identity_reads() {
        let _serial = tests::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        setup();
        fetch_scoped_fixture("commandcode", Some("userRefresh"), None)
            .await
            .unwrap();
        let at = now_ms() as i64 - 3 * SNAPSHOT_CACHE_MS;
        {
            let mut map = last_ok().lock().unwrap();
            let s = map.get_mut("commandcode").unwrap();
            s.at = at;
            s.snap.fetched_at = Some(at);
            persist_last_ok(&map).unwrap();
        }
        usage_publications().clear();
        providers::reset_counts();
        for batch in [
            cached_usage(),
            fetch_fixture(None, None).await,
            fetch_scoped_fixture("commandcode", Some("automatic"), None)
                .await
                .unwrap(),
        ] {
            let s = batch
                .snapshots
                .iter()
                .find(|s| s.id == "commandcode")
                .unwrap();
            assert_eq!(s.fetched_at, Some(at));
            assert!(s.stale);
            assert_eq!(s.status, "ok");
        }
        assert_eq!(providers::read_count("query:commandcode"), 0);
        assert_eq!(providers::read_count("commandcode:identity"), 0);
    }
    #[tokio::test]
    async fn commandcode_disabled_or_revoked_scope_never_queries_or_publishes() {
        let _serial = tests::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        setup();
        let mut policy = access_runtime().snapshot().0;
        policy.set_account("commandcode", false).unwrap();
        access_runtime().update(policy);
        assert!(
            fetch_scoped_fixture("commandcode", Some("userRefresh"), None)
                .await
                .is_err()
        );
        assert_eq!(providers::read_count("commandcode:identity"), 0);
        assert_eq!(providers::read_count("query:commandcode"), 0);
        let mut policy = access_runtime().snapshot().0;
        policy.set_account("commandcode", true).unwrap();
        access_runtime().update(policy);
        *providers::BLOCK_ACCOUNT.lock().unwrap() = Some("commandcode".into());
        let pending = fetch_scoped_fixture("commandcode", Some("userRefresh"), None);
        let revoke = async {
            while providers::read_count("query:commandcode") == 0 {
                tokio::task::yield_now().await;
            }
            change_access_policy(|policy| policy.set_account("commandcode", false)).unwrap();
            *providers::BLOCK_ACCOUNT.lock().unwrap() = None;
        };
        let (result, ()) = tokio::join!(pending, revoke);
        assert!(result.unwrap().snapshots.is_empty());
        assert!(!last_ok().lock().unwrap().contains_key("commandcode"));
    }
    #[tokio::test]
    async fn commandcode_key_generation_change_fences_old_result_without_restoring_cache() {
        let _serial = tests::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        setup();
        *providers::BLOCK_ACCOUNT.lock().unwrap() = Some("commandcode".into());
        let before = key_card_snapshot_generations(["commandcode".into()])["commandcode"];
        let pending = fetch_scoped_fixture("commandcode", Some("userRefresh"), None);
        let rotate = async {
            while providers::read_count("query:commandcode") == 0 {
                tokio::task::yield_now().await;
            }
            KEY_CARD_SNAPSHOT_GENERATIONS
                .get_or_init(Default::default)
                .lock()
                .unwrap()
                .insert("commandcode".into(), before + 1);
            *providers::COMMANDCODE_IDENTITY.lock().unwrap() = Some("synthetic-new".into());
            *providers::BLOCK_ACCOUNT.lock().unwrap() = None;
        };
        let (result, ()) = tokio::join!(pending, rotate);
        assert!(!result
            .unwrap()
            .snapshots
            .iter()
            .any(|s| s.id == "commandcode"));
        assert!(!last_ok().lock().unwrap().contains_key("commandcode"));
        assert!(!fail_state().lock().unwrap().contains_key("commandcode"));
    }
}

#[cfg(test)]
mod auto_refresh_tests {
    use super::*;
    fn configure(value: Value) { fixture_config().lock().unwrap()["providerAutoRefresh"] = value; }
    fn saved(batch: &usage_publication::UsageResult, id: &str) -> snapshot::Snapshot {
        batch.snapshots.iter().find(|s| s.id == id).unwrap().clone()
    }
    #[tokio::test]
    async fn automatic_policy_filters_all_provider_io_before_identity_and_discovery() {
        let _serial = tests::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        scoped_tests::setup_all();
        configure(json!({"codex":false,"opencode":false,"kimi":false,"stepfun":false}));
        let batch = fetch_fixture(Some("automatic"), None).await;
        for family in ["codex", "opencode", "kimi", "stepfun"] {
            for key in [format!("query:{family}"), format!("{family}:identity"), format!("{family}:discovery"), format!("{family}:discovery_auth")] {
                assert_eq!(providers::read_count(&key), 0, "off family crossed {key}");
            }
            assert_eq!(saved(&batch, family).status, "manual");
        }
        for key in ["kimi:credentials", "kimi:credential_path", "kimi:plan_key", "query:codex@abcd1234", "query:opencode@abcd1234"] {
            assert_eq!(providers::read_count(key), 0, "off family crossed {key}");
        }
        assert_eq!(providers::read_count("query:cursor"), 1);
    }
    #[tokio::test]
    async fn enabling_former_manual_defaults_queries_all_authorized_family_accounts() {
        let _serial = tests::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        scoped_tests::setup_all();
        configure(json!({"claude":true,"commandcode":true}));
        fetch_fixture(None, None).await;
        for id in ["claude", "claude@abcd1234", "claude@eeee1234", "commandcode"] {
            assert_eq!(providers::read_count(&format!("query:{id}")), 1, "enabled {id}");
        }
    }
    #[tokio::test]
    async fn moonshot_auto_off_cannot_be_queried_through_kimi_and_preserves_wallet_clocks() {
        let _serial = tests::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        scoped_tests::setup_all();
        providers::KIMI_LIVE.store(1, Ordering::SeqCst);
        let original = saved(&fetch_fixture(Some("userRefresh"), None).await, "kimi");
        providers::reset_counts();
        configure(json!({"moonshot":false}));
        let next = saved(&fetch_fixture(None, None).await, "kimi");
        assert_eq!(providers::read_count("query:kimi"), 1);
        for key in ["query:moonshot", "moonshot:wallet_auth", "moonshot:wallet_query"] { assert_eq!(providers::read_count(key), 0, "{key}"); }
        let wallet = |s: &snapshot::Snapshot| serde_json::to_value(s.metrics.iter().filter(|m| is_kimi_wallet_label(&m.label)).collect::<Vec<_>>()).unwrap();
        assert_eq!(wallet(&next), wallet(&original));
        assert_eq!(next.wallet_history.as_ref().unwrap().fetched_at, original.fetched_at);
        assert_eq!(next.wallet_history.as_ref().unwrap().attempted_at, original.attempted_at);
    }
    #[tokio::test]
    async fn disabled_auto_replays_old_cache_without_credentials_or_new_success_clock() {
        let _serial = tests::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        scoped_tests::setup_all();
        fetch_scoped_fixture("codex", Some("userRefresh"), None).await.unwrap();
        let at = now_ms() as i64 - 3 * SNAPSHOT_CACHE_MS;
        { let mut map = last_ok().lock().unwrap(); let old = map.get_mut("codex").unwrap(); old.at=at; old.snap.fetched_at=Some(at); persist_last_ok(&map).unwrap(); }
        configure(json!({"codex":false}));
        usage_publications().clear(); providers::reset_counts();
        for batch in [cached_usage(), fetch_fixture(None, None).await] {
            let old = saved(&batch, "codex"); assert_eq!(old.fetched_at, Some(at)); assert!(old.stale);
        }
        assert_eq!(providers::read_count("codex:identity"), 0);
        assert_eq!(providers::read_count("codex:discovery"), 0);
        assert_eq!(providers::read_count("query:codex"), 0);
        assert_eq!(last_ok().lock().unwrap()["codex"].at, at);
    }
}

#[cfg(test)]
mod auto_refresh_race_tests {
    use super::*;
    #[tokio::test]
    async fn closing_auto_gate_stops_follow_on_io_without_revoking_inflight_publication() {
        let _serial = tests::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        scoped_tests::setup_all();
        *providers::BLOCK_ACCOUNT.lock().unwrap() = Some("codex".into());
        *providers::FOLLOW_ON_ACCOUNT.lock().unwrap() = Some("codex".into());
        let revision = access_runtime().snapshot().1;
        let pending = fetch_scoped_fixture("codex", Some("automatic"), None);
        let close = async {
            while providers::read_count("query:codex") == 0 { tokio::task::yield_now().await; }
            fixture_config().lock().unwrap()["providerAutoRefresh"] = json!({"codex":false});
            *providers::BLOCK_ACCOUNT.lock().unwrap() = None;
        };
        let (result, ()) = tokio::join!(pending, close);
        assert_eq!(providers::read_count("follow_on:codex"), 0);
        assert_eq!(access_runtime().snapshot().1, revision);
        assert_eq!(result.unwrap().snapshots[0].status, "ok");
        assert_eq!(providers::read_count("codex:identity"), 0, "codex identity must not be opened after gate closes");
    }
    #[tokio::test]
    async fn independently_refreshed_moonshot_is_not_hidden_by_saved_kimi_wallet() {
        let _serial = tests::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        scoped_tests::setup_all(); providers::KIMI_LIVE.store(1, Ordering::SeqCst);
        let old = fetch_fixture(Some("userRefresh"), None).await;
        let original = old.snapshots.iter().find(|s| s.id == "kimi").unwrap().clone();
        fixture_config().lock().unwrap()["providerAutoRefresh"] = json!({"kimi":false});
        providers::reset_counts();
        let next = fetch_fixture(None, None).await;
        assert_eq!(providers::read_count("query:moonshot"), 1);
        for key in ["query:kimi", "kimi:credentials", "kimi:credential_path", "kimi:plan_key"] { assert_eq!(providers::read_count(key), 0, "{key}"); }
        assert!(next.snapshots.iter().any(|s| s.id == "moonshot"), "fresh independent wallet must remain visible");
        let kimi = next.snapshots.iter().find(|s| s.id == "kimi").unwrap();
        assert_eq!(kimi.fetched_at, original.fetched_at);
        assert_eq!(serde_json::to_value(&kimi.metrics).unwrap(), serde_json::to_value(&original.metrics).unwrap());
    }
}

#[cfg(test)]
mod config_commands {
    use super::*;
    use std::path::Path;
    use std::sync::atomic::AtomicU64;
    use access_policy::sanitize_retired_config;
    mod httpapi { pub fn forget_disabled_snapshots(_: &[String]) {} }
    include!(concat!(env!("OUT_DIR"), "/config_commands.rs"));
    #[test]
    fn canonical_auto_map_persists_partial_old_schema_and_rejects_unknown_keys() {
        let _serial = tests::SERIAL.lock().unwrap_or_else(|e| e.into_inner()); tests::setup();
        let dir = providers::config_dir();
        let old = json!({"refreshMinutes":7, "accessPolicy":access_runtime().snapshot().0});
        std::fs::write(dir.join("config.json"), old.to_string()).unwrap();
        let initial = config_with_defaults(load_config());
        assert_eq!(initial["providerAutoRefresh"]["claude"], false);
        assert_eq!(initial["providerAutoRefresh"]["commandcode"], false);
        assert_eq!(initial["providerAutoRefresh"]["codex"], true);
        let next = set_config_in(&dir, json!({"providerAutoRefresh":{"codex":false,"claude":true,"cursor":"yes","claude@abcd1234":false,"unknown":true,"hermes":true}})).unwrap();
        assert_eq!(next["providerAutoRefresh"]["codex"], false);
        assert_eq!(next["providerAutoRefresh"]["claude"], true);
        assert_eq!(next["providerAutoRefresh"]["cursor"], false);
        assert_eq!(next["providerAutoRefresh"]["opencode"], true);
        for key in ["unknown","claude@abcd1234","hermes"] { assert!(next["providerAutoRefresh"].get(key).is_none()); }
        assert_eq!(next["accessPolicy"], old["accessPolicy"]);
        assert_eq!(next["refreshMinutes"], 7);
        assert_eq!(load_config()["providerAutoRefresh"], next["providerAutoRefresh"]);
        assert_eq!(next["providerAutoRefresh"].as_object().unwrap().len(), access_policy::FAMILIES.len()-1);
        for malformed in [Value::Null,json!([]),json!("on"),json!(true)] {
            let next = set_config_in(&dir,json!({"providerAutoRefresh":malformed})).unwrap();
            assert!(next["providerAutoRefresh"].as_object().unwrap().values().all(|v|v==false));
            assert_eq!(next["accessPolicy"], old["accessPolicy"]);
        }
    }
    #[test]
    fn partial_auto_patch_preserves_other_families_saved_preferences() {
        let _serial=tests::SERIAL.lock().unwrap_or_else(|e|e.into_inner()); tests::setup();
        let dir=providers::config_dir();
        set_config_in(&dir,json!({"providerAutoRefresh":{"claude":true,"codex":false,"cursor":false}})).unwrap();
        let next=set_config_in(&dir,json!({"providerAutoRefresh":{"codex":true}})).unwrap();
        assert_eq!(next["providerAutoRefresh"]["codex"],true);
        assert_eq!(next["providerAutoRefresh"]["claude"],true,"unrelated explicit enable must survive");
        assert_eq!(next["providerAutoRefresh"]["cursor"],false,"unrelated explicit disable must survive");
        let next=set_config_in(&dir,json!({"providerAutoRefresh":{"claude":"invalid","unknown":true}})).unwrap();
        assert_eq!(next["providerAutoRefresh"]["claude"],false);
        assert_eq!(next["providerAutoRefresh"]["cursor"],false);
        assert!(next["providerAutoRefresh"].get("unknown").is_none());
    }
    #[test]
    fn scheduler_only_save_of_old_config_does_not_change_access_revision_or_cached_history() {
        let _serial = tests::SERIAL.lock().unwrap_or_else(|e| e.into_inner()); tests::setup();
        let dir=providers::config_dir(); let policy=access_runtime().snapshot().0;
        std::fs::write(dir.join("config.json"), json!({"accessPolicy":policy}).to_string()).unwrap();
        let revision=access_runtime().snapshot().1;
        let cached=CachedSnap{at:123,snap:Snapshot::ok("codex","Codex",None,vec![])};
        last_ok().lock().unwrap().insert("codex".into(),cached);
        set_config_inner(json!({"providerAutoRefresh":{"codex":false}})).unwrap();
        assert_eq!(access_runtime().snapshot().1,revision,"scheduler-only changes must not invalidate authorization");
        assert_eq!(last_ok().lock().unwrap()["codex"].at,123);
    }
}

#[cfg(test)]
mod cursor_auto_tests {
    use super::*;
    #[tokio::test]
    async fn auto_off_cursor_csv_does_not_open_credentials_but_local_collection_continues() {
        let _serial = tests::SERIAL.lock().unwrap_or_else(|e| e.into_inner()); scoped_tests::setup_all();
        fixture_config().lock().unwrap()["providerAutoRefresh"]=json!({"cursor":false});
        let result=fetch_spend(None).await;
        assert_eq!(providers::read_count("cursor:csv_auth"),0);
        assert_eq!(providers::read_count("cursor:csv_query"),0);
        assert_eq!(providers::read_count("local:collect"),1);
        assert_eq!(serde_json::to_value(result).unwrap()["preserveCursor"],true);
    }
}

#[cfg(test)]
mod complete_auto_contract_tests {
    use super::*;
    #[tokio::test]
    async fn manual_scope_and_global_bypass_scheduler_without_granting_access() {
        let _serial=tests::SERIAL.lock().unwrap_or_else(|e|e.into_inner()); scoped_tests::setup_all();
        fixture_config().lock().unwrap()["providerAutoRefresh"]=json!(false);
        for target in ["codex@abcd1234","cursor","commandcode","kimi"] {
            providers::reset_counts(); let batch=fetch_scoped_fixture(target,Some("userRefresh"),None).await.unwrap();
            assert!(batch.snapshots.iter().any(|s|s.id==target));
            assert_eq!(providers::read_count(&format!("query:{target}")),1);
            assert_eq!(providers::read_count("moonshot:wallet_query"),0,"exact Kimi refresh stays plan-only");
        }
        providers::reset_counts(); fetch_fixture(Some("userRefresh"),None).await;
        for id in ["codex","codex@abcd1234","claude","commandcode","cursor"] {assert_eq!(providers::read_count(&format!("query:{id}")),1);}
        let mut policy=access_runtime().snapshot().0; policy.set_account("cursor",false).unwrap(); access_runtime().update(policy);
        providers::reset_counts(); assert!(fetch_scoped_fixture("cursor",Some("userRefresh"),None).await.is_err());
        assert_eq!(providers::read_count("query:cursor"),0);
        let csv=fetch_spend(Some(UsageRefreshReason::UserRefresh)).await; assert!(!csv.preserve_cursor); assert_eq!(providers::read_count("cursor:csv_auth"),0);
    }
    #[tokio::test]
    async fn manual_cursor_csv_bypasses_auto_setting_and_revoke_never_preserves_it() {
        let _serial=tests::SERIAL.lock().unwrap_or_else(|e|e.into_inner()); scoped_tests::setup_all();
        fixture_config().lock().unwrap()["providerAutoRefresh"]=json!({"cursor":false});
        let manual=fetch_spend(Some(UsageRefreshReason::UserRefresh)).await;
        assert_eq!(providers::read_count("cursor:csv_query"),1); assert!(!manual.preserve_cursor);
        assert!(manual.rows.iter().any(|r|r.id=="cursor"));
        let mut policy=access_runtime().snapshot().0; policy.set_account("cursor",false).unwrap(); access_runtime().update(policy);
        let auto=fetch_spend(None).await; assert!(!auto.preserve_cursor); assert_eq!(providers::read_count("cursor:csv_query"),1);
        assert_eq!(providers::read_count("local:collect"),2);
    }
    #[tokio::test]
    async fn queued_automatic_intent_uses_latest_setting_without_erasing_manual_history() {
        let _serial=tests::SERIAL.lock().unwrap_or_else(|e|e.into_inner()); scoped_tests::setup_all();
        let guard=usage_refresh_gate().lock().await;
        let task=tokio::spawn(async {fetch_scoped_fixture("codex",Some("automatic"),None).await});
        tokio::task::yield_now().await;
        fixture_config().lock().unwrap()["providerAutoRefresh"]=json!({"codex":false}); drop(guard);
        let result=task.await.unwrap().unwrap(); assert_eq!(providers::read_count("query:codex"),0);
        assert_eq!(result.snapshots[0].status,"manual");
    }
    #[tokio::test]
    async fn old_schema_unknown_and_malformed_preferences_never_grant_or_override_family() {
        let _serial=tests::SERIAL.lock().unwrap_or_else(|e|e.into_inner()); scoped_tests::setup_all();
        fixture_config().lock().unwrap()["providerAutoRefresh"]=json!({"codex":false,"codex@abcd1234":true,"unknown":true,"cursor":"true"});
        fetch_fixture(None,None).await; assert_eq!(providers::read_count("query:codex@abcd1234"),0);assert_eq!(providers::read_count("query:cursor"),0);
        assert_eq!(providers::read_count("query:opencode"),1);
        providers::reset_counts(); fixture_config().lock().unwrap()["providerAutoRefresh"]=json!(null);
        fetch_fixture(None,None).await;
        for id in access_policy::FAMILIES.iter().filter(|id|**id!="hermes") {assert_eq!(providers::read_count(&format!("query:{id}")),0,"malformed {id}");}
        let policy=access_policy::AccessPolicy::default(); access_runtime().update(policy);
        fixture_config().lock().unwrap()["providerAutoRefresh"]=json!({"claude":true,"codex":true});providers::reset_counts();
        assert!(fetch_fixture(None,None).await.snapshots.is_empty()); assert_eq!(providers::read_count("query:claude"),0);
    }
}

#[cfg(test)]
mod auto_history_tests {
    use super::*;
    #[tokio::test]
    async fn generic_manual_placeholders_keep_provider_display_names() {
        let _serial=tests::SERIAL.lock().unwrap_or_else(|e|e.into_inner()); scoped_tests::setup_all();
        fixture_config().lock().unwrap()["providerAutoRefresh"]=json!(false);
        let batch=fetch_fixture(None,None).await;
        for (id,name) in [("claude","Claude"),("commandcode","CommandCode"),("codex","Codex"),("opencode","OpenCode")] {
            assert_eq!(batch.snapshots.iter().find(|s|s.id==id).unwrap().name,name);
        }
    }
    #[tokio::test]
    async fn completed_cursor_export_is_new_data_even_if_auto_setting_closed_inflight() {
        let _serial=tests::SERIAL.lock().unwrap_or_else(|e|e.into_inner()); scoped_tests::setup_all();
        *providers::BLOCK_ACCOUNT.lock().unwrap()=Some("cursor-csv".into());
        let pending=fetch_spend(None);
        let close=async {while providers::read_count("cursor:csv_query")==0 {tokio::task::yield_now().await;} fixture_config().lock().unwrap()["providerAutoRefresh"]=json!({"cursor":false}); *providers::BLOCK_ACCOUNT.lock().unwrap()=None;};
        let (result,())=tokio::join!(pending,close);
        assert!(result.rows.iter().any(|row|row.id=="cursor"));
        assert!(!result.preserve_cursor,"new CSV must not be replaced by historical rows");
    }
}

#[cfg(test)]
mod wallet_auto_race_tests {
    use super::*;
    fn kimi(batch: &usage_publication::UsageResult)->Snapshot {batch.snapshots.iter().find(|s|s.id=="kimi").unwrap().clone()}
    #[tokio::test]
    async fn wallet_gate_closed_at_nested_start_preserves_rows_error_and_original_clocks() {
        let _serial=tests::SERIAL.lock().unwrap_or_else(|e|e.into_inner());
        for prior_error in [false,true] {
            scoped_tests::setup_all();providers::KIMI_LIVE.store(1,Ordering::SeqCst);
            let mut old=kimi(&fetch_fixture(Some("userRefresh"),None).await);
            if prior_error {
                *providers::ERROR_ACCOUNT.lock().unwrap()=Some("moonshot".into());
                old=kimi(&fetch_fixture(Some("userRefresh"),None).await);
            }
            providers::reset_counts();providers::CLOSE_WALLET_IN_PLAN_KEY.store(1,Ordering::SeqCst);
            let next=kimi(&fetch_fixture(None,None).await);
            assert_eq!(providers::read_count("moonshot:wallet_auth"),0);assert_eq!(providers::read_count("query:moonshot"),0);
            let rows=|s:&Snapshot|serde_json::to_value(s.metrics.iter().filter(|m|is_kimi_wallet_label(&m.label)).collect::<Vec<_>>()).unwrap();
            assert_eq!(rows(&next),rows(&old),"nested scheduler skip must not erase wallet");
            if prior_error {assert_eq!(serde_json::to_value(&next.wallet_history).unwrap(),serde_json::to_value(&old.wallet_history).unwrap());}
            else {assert_eq!(next.wallet_history.as_ref().unwrap().fetched_at,old.fetched_at);}
            change_access_policy(|policy|policy.set_account("moonshot",false)).unwrap();
            let revoked=kimi(&fetch_fixture(None,None).await);assert!(revoked.wallet_history.is_none());assert!(revoked.metrics.iter().all(|m|!is_kimi_wallet_label(&m.label)));
        }
    }
    #[tokio::test]
    async fn completed_wallet_is_not_overwritten_when_setting_closes_after_query() {
        let _serial=tests::SERIAL.lock().unwrap_or_else(|e|e.into_inner());scoped_tests::setup_all();providers::KIMI_LIVE.store(1,Ordering::SeqCst);
        fetch_fixture(Some("userRefresh"),None).await;
        providers::WALLET_VALUE.store(11,Ordering::SeqCst);providers::CLOSE_WALLET_AFTER_QUERY.store(1,Ordering::SeqCst);providers::reset_counts();
        let next=kimi(&fetch_fixture(None,None).await);
        assert_eq!(providers::read_count("moonshot:wallet_query"),1);
        assert_eq!(next.metrics.iter().find(|m|m.label=="API").unwrap().used_percent,Some(11.0));
        assert!(next.wallet_history.is_none(),"completed fresh wallet is not historical replay");
    }
}
