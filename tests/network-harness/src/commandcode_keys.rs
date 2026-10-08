//! Production key command, mutation guard and cache eviction, with only app
//! publication/native services and unrelated-provider paths stubbed out.
use crate::{private_file, providers};
use serde_json::Value;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Mutex, OnceLock,
    },
};
#[derive(Clone, serde::Serialize)]
struct CachedSnap {
    at: i64,
    snap: crate::snapshot::Snapshot,
}
static SERIAL: Mutex<()> = Mutex::new(());
static DIR: Mutex<Option<PathBuf>> = Mutex::new(None);
static FAIL: AtomicBool = AtomicBool::new(false);
static PUBLICATION: AtomicBool = AtomicBool::new(false);
fn last_ok() -> &'static Mutex<HashMap<String, CachedSnap>> {
    static M: OnceLock<Mutex<HashMap<String, CachedSnap>>> = OnceLock::new();
    M.get_or_init(Default::default)
}
fn fail_state() -> &'static Mutex<HashMap<String, String>> {
    static M: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    M.get_or_init(Default::default)
}
fn persist_last_ok(map: &HashMap<String, CachedSnap>) -> Result<(), String> {
    assert!(
        KEY_CARD_PUBLICATION.try_lock().is_err(),
        "key/cache transaction must hold the publication lock"
    );
    if FAIL.load(Ordering::SeqCst) {
        return Err("synthetic cache flush failure".into());
    }
    persist_last_ok_at(
        &DIR.lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .join("last_snapshots.json"),
        map,
    )
}
fn retired_id(_: &str) -> bool {
    false
}
fn api_key_context_is_dirty(_: &Path, _: &str) -> bool {
    panic!("unrelated key branch")
}
fn invalidate_api_key_context(_: &Path, _: &str) -> Result<(), String> {
    panic!("unrelated key branch")
}
fn context_cleanup_error(s: String) -> String {
    s
}
mod alerts {
    pub fn forget_snapshot(_: &str) {}
}
mod httpapi {
    pub fn forget_snapshots(_: &[String]) {
        super::PUBLICATION.store(false, super::Ordering::SeqCst);
    }
}
include!(concat!(env!("OUT_DIR"), "/commandcode_keys.rs"));
fn setup() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "pane-commandcode-command-fixture-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    *DIR.lock().unwrap() = Some(dir.clone());
    FAIL.store(false, Ordering::SeqCst);
    last_ok().lock().unwrap().clear();
    fail_state().lock().unwrap().clear();
    std::fs::write(
        dir.join("commandcode.json"),
        r#"{"apiKey":"old-synthetic"}"#,
    )
    .unwrap();
    let entry = CachedSnap {
        at: 1,
        snap: crate::snapshot::Snapshot::ok("commandcode", "CommandCode", None, vec![]),
    };
    last_ok()
        .lock()
        .unwrap()
        .insert("commandcode".into(), entry);
    persist_last_ok_at(&dir.join("last_snapshots.json"), &last_ok().lock().unwrap()).unwrap();
    fail_state()
        .lock()
        .unwrap()
        .insert("commandcode".into(), "old error".into());
    PUBLICATION.store(true, Ordering::SeqCst);
    dir
}
#[test]
fn actual_key_command_fences_generation_and_preinvalidates_before_save_clear_or_failed_cleanup() {
    let _serial = SERIAL.lock().unwrap();
    for key in ["new-synthetic", ""] {
        let dir = setup();
        let old_generation = key_card_snapshot_generations(["commandcode".into()])["commandcode"];
        FAIL.store(true, Ordering::SeqCst);
        assert!(set_api_key_in(&dir, "commandcode", key).is_err());
        assert_eq!(
            stored_pane_api_key(&dir.join("commandcode.json")).as_deref(),
            Some("old-synthetic")
        );
        assert!(std::fs::read_to_string(dir.join("last_snapshots.json"))
            .unwrap()
            .contains("commandcode"));
        assert!(last_ok().lock().unwrap().is_empty());
        assert!(fail_state().lock().unwrap().is_empty());
        assert!(!PUBLICATION.load(Ordering::SeqCst));
        assert!(
            key_card_snapshot_generations(["commandcode".into()])["commandcode"] > old_generation
        );
        FAIL.store(false, Ordering::SeqCst);
        set_api_key_in(&dir, "commandcode", key).unwrap();
        assert!(!std::fs::read_to_string(dir.join("last_snapshots.json"))
            .unwrap()
            .contains("commandcode"));
        assert_eq!(
            stored_pane_api_key(&dir.join("commandcode.json")).as_deref(),
            if key.is_empty() { None } else { Some(key) }
        );
        assert_eq!(KEY_CARD_ACTIVE_MUTATIONS.load(Ordering::SeqCst), 0);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
#[test]
fn actual_key_command_failed_write_and_remove_leave_no_cached_quota() {
    let _serial = SERIAL.lock().unwrap();
    for key in ["new-synthetic", ""] {
        let dir = setup();
        let path = dir.join("commandcode.json");
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(set_api_key_in(&dir, "commandcode", key).is_err());
        assert!(last_ok().lock().unwrap().is_empty());
        assert!(!PUBLICATION.load(Ordering::SeqCst));
        assert!(!std::fs::read_to_string(dir.join("last_snapshots.json"))
            .unwrap()
            .contains("commandcode"));
        assert_eq!(KEY_CARD_ACTIVE_MUTATIONS.load(Ordering::SeqCst), 0);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
