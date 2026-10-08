//! Exact desktop command bodies, with only storage/environment/native IPC
//! boundaries replaced by isolated synthetic fixtures. Never touch user logs.
use crate::{
    access_policy::{self, AccessPolicy, AccessRuntime},
    scan_policy, provider_auto_refresh,
};
use serde_json::{json, Value};
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
static CONFIG_WRITE: Mutex<()> = Mutex::new(());
mod tauri {
    pub type AppHandle = ();
}
struct Fixture {
    config: Value,
    dir: PathBuf,
    runtime: &'static Arc<AccessRuntime>,
    native_cleared: bool,
    fail: bool,
}
thread_local! { static FIXTURE: RefCell<Option<Fixture>> = const { RefCell::new(None) }; }
fn fixture<T>(f: impl FnOnce(&mut Fixture) -> T) -> T {
    FIXTURE.with(|s| f(s.borrow_mut().as_mut().unwrap()))
}
struct Guard;
impl Guard {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("pane-source-command-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let config = json!({"accessPolicy": AccessPolicy::default()});
        let runtime = Box::leak(Box::new(Arc::new(AccessRuntime::new(
            AccessPolicy::default(),
        ))));
        FIXTURE.with(|f| {
            *f.borrow_mut() = Some(Fixture {
                config,
                dir,
                runtime,
                native_cleared: false,
                fail: false,
            })
        });
        Self
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        FIXTURE.with(|f| {
            if let Some(f) = f.borrow_mut().take() {
                std::fs::remove_dir_all(f.dir).unwrap();
            }
        });
    }
}
fn load_config() -> Value {
    fixture(|f| f.config.clone())
}
fn load_config_from(_: &Path) -> Value {
    load_config()
}
fn config_with_defaults(config: Value) -> Value {
    config
}
fn access_runtime() -> &'static Arc<AccessRuntime> {
    fixture(|f| f.runtime)
}
fn sanitize_retired_config(config: &mut Value) {
    access_policy::sanitize_retired_config(config);
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
    change: impl FnOnce(&mut AccessPolicy) -> Result<(), String>,
) -> Result<Value, String> {
    let mut config = load_config();
    let mut policy = AccessPolicy::from_config(&config);
    change(&mut policy)?;
    if !policy.is_valid() {
        return Err("Invalid policy".into());
    }
    config["accessPolicy"] = json!(policy);
    persist_config_in(&providers::config_dir(), &config)?;
    access_runtime().update(policy);
    Ok(config)
}
async fn clear_consent_trays(_: ()) -> Result<(), String> {
    fixture(|f| f.native_cleared = true);
    Ok(())
}
mod providers {
    pub fn config_dir() -> std::path::PathBuf {
        super::fixture(|f| f.dir.clone())
    }
    pub mod qwen {
        pub fn reset_quota_cooldown() {}
    }
}
mod scan_sources {
    pub use crate::scan_sources::{
        catalog, configure, runtime_policy, set_legacy, ScanSourceStatus,
    };
    pub struct SourceEnvironment;
    impl SourceEnvironment {
        pub fn current() -> crate::scan_sources::SourceEnvironment {
            crate::scan_sources::SourceEnvironment {
                home: Some(super::providers::config_dir()),
                vars: Default::default(),
            }
        }
    }
}
include!(concat!(env!("OUT_DIR"), "/source_commands.rs"));

#[tokio::test]
async fn exact_source_commands_default_custom_off_reset_and_publication_fences() {
    let _guard = Guard::new("roundtrip");
    let dir = providers::config_dir();
    let on = configure_scan_source((), "claude".into(), true, "default".into(), None)
        .await
        .unwrap();
    assert_eq!(on["accessPolicy"]["scanSources"]["claude"]["enabled"], true);
    assert_eq!(
        get_scan_sources()
            .iter()
            .find(|s| s.source == "claude")
            .unwrap()
            .directories[0]
            .status,
        "notFound"
    );
    assert!(fixture(|f| f.native_cleared));
    let custom = dir.join("custom");
    std::fs::create_dir(&custom).unwrap();
    let log = custom.join("fixture.jsonl");
    std::fs::write(&log, "{}\n").unwrap();
    configure_scan_source(
        (),
        "claude".into(),
        true,
        "custom".into(),
        Some(vec![custom.to_string_lossy().into()]),
    )
    .await
    .unwrap();
    let before_off = local_scan_policy();
    assert!(before_off.allows_path("claude", &log));
    let before_epoch = before_off.epochs["claude"].clone();
    configure_scan_source((), "claude".into(), false, "custom".into(), None)
        .await
        .unwrap();
    assert!(!before_off.is_current());
    assert!(!before_off.allows_path("claude", &log));
    configure_scan_source((), "claude".into(), true, "custom".into(), None)
        .await
        .unwrap();
    assert_ne!(local_scan_policy().epochs["claude"], before_epoch);
    let reset = reset_provider_access(()).await.unwrap();
    assert_eq!(reset["accessPolicy"]["scanSources"], json!({}));
    assert_eq!(reset["accessPolicy"]["scanRoots"], json!({}));
    assert_eq!(reset["accessPolicy"]["scanEpochs"], json!({}));
    assert!(local_scan_policy().roots.is_empty());
}
#[tokio::test]
async fn generic_config_patch_cannot_inject_source_consent_or_frozen_roots() {
    let _guard = Guard::new("injection");
    let before = load_config()["accessPolicy"].clone();
    let patch = json!({"locale":"zh", "accessPolicy":{"scanSources":{"claude":{"enabled":true,"mode":"default","defaultDirectories":["/"]}},"scanRoots":{"claude":["/"]}}, "scanSources":{"claude":{"enabled":true}}, "scanRoots":{"claude":["/"]}});
    let after = set_config_in(&providers::config_dir(), patch).unwrap();
    assert_eq!(after["locale"], "zh");
    assert_eq!(after["accessPolicy"], before);
    assert!(local_scan_policy().roots.is_empty());
}
#[tokio::test]
async fn malformed_source_mode_or_invalid_custom_command_never_changes_existing_grant() {
    let _guard = Guard::new("malformed");
    configure_scan_source((), "claude".into(), true, "default".into(), None)
        .await
        .unwrap();
    let before = load_config();
    let revision = access_runtime().snapshot().1;
    for (source, mode, directories) in [
        ("unknown", "default", None),
        ("claude", "automatic", None),
        ("claude", "default", Some(vec!["relative".into()])),
        ("claude", "custom", Some(vec!["relative".into()])),
        ("claude", "custom", Some(Vec::new())),
    ] {
        assert!(
            configure_scan_source((), source.into(), true, mode.into(), directories)
                .await
                .is_err()
        );
        assert_eq!(load_config(), before);
        assert_eq!(access_runtime().snapshot().1, revision);
    }
}
#[tokio::test]
async fn source_save_failure_preserves_config_and_revision() {
    let _guard = Guard::new("failure");
    let before = load_config();
    let revision = access_runtime().snapshot().1;
    fixture(|f| f.fail = true);
    assert!(
        configure_scan_source((), "codex".into(), true, "default".into(), None)
            .await
            .is_err()
    );
    assert_eq!(load_config(), before);
    assert_eq!(access_runtime().snapshot().1, revision);
}
#[tokio::test]
async fn exact_legacy_command_updates_new_intent_and_revokes_when_empty() {
    let _guard = Guard::new("legacy");
    let custom = providers::config_dir().join("custom");
    std::fs::create_dir(&custom).unwrap();
    set_scan_source((), "qwen".into(), vec![custom.to_string_lossy().into()])
        .await
        .unwrap();
    assert_eq!(
        load_config()["accessPolicy"]["scanSources"]["qwen"]["enabled"],
        true
    );
    std::fs::remove_dir(&custom).unwrap();
    set_scan_source((), "qwen".into(), Vec::new())
        .await
        .unwrap();
    assert_eq!(
        load_config()["accessPolicy"]["scanSources"]["qwen"]["enabled"],
        false
    );
    assert!(local_scan_policy().roots.is_empty());
}

#[tokio::test]
#[cfg(unix)]
async fn exact_legacy_command_cannot_retarget_remembered_grant_after_off() {
    let _guard = Guard::new("legacy-retarget");
    let root = providers::config_dir().join("custom");
    let outside = providers::config_dir().join("outside");
    std::fs::create_dir(&root).unwrap();
    std::fs::create_dir(&outside).unwrap();
    let spelling = root.to_string_lossy().into_owned();
    configure_scan_source(
        (),
        "claude".into(),
        true,
        "custom".into(),
        Some(vec![spelling.clone()]),
    )
    .await
    .unwrap();
    configure_scan_source((), "claude".into(), false, "custom".into(), None)
        .await
        .unwrap();
    let before = load_config();
    let revision = access_runtime().snapshot().1;
    std::fs::remove_dir(&root).unwrap();
    std::os::unix::fs::symlink(&outside, &root).unwrap();
    assert!(set_scan_source((), "claude".into(), vec![spelling])
        .await
        .is_err());
    assert_eq!(load_config(), before);
    assert_eq!(access_runtime().snapshot().1, revision);
    let log = outside.join("synthetic.jsonl");
    std::fs::write(&log, "{}\n").unwrap();
    assert!(!local_scan_policy().allows_path("claude", &log));
}
