//! Exact production command bodies; synthetic IPC, config/key storage and native
//! dispatch boundaries. Real policy, mode transaction and cache cleanup helpers.
use crate::{access_policy, provider_modes};
use serde_json::{json, Value};
use std::{
    cell::RefCell,
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
};
pub(super) static SERIAL: Mutex<()> = Mutex::new(());
static CONFIG_WRITE: Mutex<()> = Mutex::new(());
mod tauri {
    pub type AppHandle = ();
}
struct Fixture {
    config: Value,
    runtime: &'static Arc<access_policy::AccessRuntime>,
    dir: PathBuf,
    fail: bool,
    cleared: Vec<String>,
    published: bool,
    native_cleared: bool,
}
thread_local! { static FIXTURE: RefCell<Option<Fixture>> = const { RefCell::new(None) }; }
fn fixture<T>(f: impl FnOnce(&mut Fixture) -> T) -> T {
    FIXTURE.with(|fixture| f(fixture.borrow_mut().as_mut().unwrap()))
}
pub(super) fn setup(config: Value) {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "pane-mode-bridge-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let runtime = Box::leak(Box::new(Arc::new(access_policy::AccessRuntime::new(
        access_policy::AccessPolicy::from_backend_config(&config),
    ))));
    FIXTURE.with(|f| {
        *f.borrow_mut() = Some(Fixture {
            config,
            runtime,
            dir,
            fail: false,
            cleared: Vec::new(),
            published: true,
            native_cleared: false,
        })
    });
}
pub(super) fn fail(value: bool) {
    fixture(|f| f.fail = value);
}
pub(super) fn finish() {
    FIXTURE.with(|f| {
        if let Some(old) = f.borrow_mut().take() {
            std::fs::remove_dir_all(old.dir).unwrap();
        }
    });
}
fn load_config() -> Value {
    fixture(|f| f.config.clone())
}
fn config_with_defaults(config: Value) -> Value {
    config
}
fn access_runtime() -> &'static Arc<access_policy::AccessRuntime> {
    fixture(|f| f.runtime)
}
fn persist_config_in(_: &Path, config: &Value) -> Result<(), String> {
    fixture(|f| {
        if f.fail {
            Err("synthetic persistence failure".into())
        } else {
            f.config = config.clone();
            Ok(())
        }
    })
}
fn change_access_policy(
    change: impl FnOnce(&mut access_policy::AccessPolicy) -> Result<(), String>,
) -> Result<Value, String> {
    let mut config = load_config();
    let mut policy = access_policy::AccessPolicy::from_config(&config);
    change(&mut policy)?;
    config["accessPolicy"] = json!(policy);
    persist_config_in(&providers::config_dir(), &config)?;
    access_runtime().update(policy);
    Ok(config)
}
fn set_api_key_in(_: &Path, _: &str, _: &str) -> Result<(), String> {
    fixture(|f| {
        if f.fail {
            Err("synthetic key save failure".into())
        } else {
            Ok(())
        }
    })
}
fn forget_provider_snapshots_inner(ids: &[String], force: bool) -> Result<(), String> {
    assert!(force);
    fixture(|f| f.cleared.extend_from_slice(ids));
    Ok(())
}
struct Publication;
impl Publication {
    fn clear(&self) {
        fixture(|f| f.published = false);
    }
}
fn usage_publications() -> Publication {
    Publication
}
mod spend {
    pub fn invalidate_published() {}
}
mod providers {
    pub use crate::providers::{forget_credit_baselines_in, minimax, qwen};
    pub fn config_dir() -> std::path::PathBuf {
        super::fixture(|f| f.dir.clone())
    }
}
async fn clear_consent_trays(_: ()) -> Result<(), String> {
    assert!(
        KEY_CARD_PUBLICATION.try_lock().is_ok(),
        "publication mutex crossed native await"
    );
    assert!(
        CONFIG_WRITE.try_lock().is_ok(),
        "config mutex crossed native await"
    );
    fixture(|f| f.native_cleared = true);
    Ok(())
}
include!(concat!(env!("OUT_DIR"), "/mode_commands.rs"));

#[tokio::test]
async fn actual_mode_command_clears_minimax_context_without_granting_access() {
    let _serial = SERIAL.lock().unwrap();
    setup(json!({"accessPolicy": access_policy::AccessPolicy::default()}));
    let dir = providers::config_dir();
    std::fs::write(
        dir.join("credit_baselines.json"),
        r#"{"minimax":80,"qwen":30}"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("cache_identities.json"),
        r#"{"minimax":"old","codex":"keep"}"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("minimax-plan.json"),
        r#"{"tier":"old-region","userId":"synthetic"}"#,
    )
    .unwrap();
    fn require_send<T: std::future::Future + Send>(future: T) -> T {
        future
    }
    let config = require_send(set_provider_region(
        (),
        "minimax".into(),
        "china:mcode".into(),
    ))
    .await
    .unwrap();
    assert_eq!(config["accessPolicy"]["regions"]["minimax"], "china:mcode");
    assert!(access_policy::AccessPolicy::from_config(&config)
        .enabled_accounts
        .is_empty());
    assert!(!crate::providers::minimax::remembered_tier_exists_in(&dir));
    assert!(!crate::providers::credit_baselines_contain(
        &dir,
        &["minimax".into()]
    ));
    assert!(crate::providers::credit_baselines_contain(
        &dir,
        &["qwen".into()]
    ));
    let identities: Value =
        serde_json::from_slice(&std::fs::read(dir.join("cache_identities.json")).unwrap()).unwrap();
    assert!(identities.get("minimax").is_none());
    assert_eq!(identities["codex"], "keep");
    fixture(|f| {
        assert_eq!(f.cleared, ["minimax"]);
        assert!(!f.published);
        assert!(f.native_cleared);
    });
    assert_eq!(get_provider_modes().len(), 7);
    finish();
}

#[test]
fn actual_mode_command_refuses_new_mode_when_identity_cleanup_fails() {
    let _serial = SERIAL.lock().unwrap();
    setup(json!({"accessPolicy": access_policy::AccessPolicy::default()}));
    std::fs::write(
        providers::config_dir().join("cache_identities.json"),
        "broken",
    )
    .unwrap();
    assert!(set_provider_region_inner("stepfun", "china:api_key").is_err());
    assert!(access_policy::AccessPolicy::from_config(&load_config())
        .regions
        .is_empty());
    assert!(access_runtime().snapshot().0.regions.is_empty());
    finish();
}
