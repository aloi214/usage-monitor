// This portable harness intentionally compiles a subset of desktop entry points.
#![allow(dead_code)]
#[path = "../../../src-tauri/src/access_policy.rs"]
pub mod access_policy;
#[path = "../../../src-tauri/src/network_policy.rs"]
pub mod network_policy;
#[path = "../../../src-tauri/src/network_transport.rs"]
pub mod network_transport;
#[path = "../../../src-tauri/src/private_file.rs"]
mod private_file;
#[path = "../../../src-tauri/src/scan_policy.rs"]
pub mod scan_policy;
#[path = "../../../src-tauri/src/snapshot.rs"]
pub mod snapshot;
#[cfg(test)]
mod tests;
// Only discovery/configuration/scan boundaries are synthetic. Actual adapters,
// HTTP wrappers, policy, parsing, and body readers compile unchanged.
pub mod providers {
    include!(concat!(env!("OUT_DIR"), "/providers_common.rs"));
    pub fn proxy_url() -> Option<&'static str> {
        None
    }
    tokio::task_local! { pub static DISCOVERY_READS: std::sync::Arc<std::sync::atomic::AtomicUsize>; }
    tokio::task_local! { pub static KEY_SEQUENCE: std::cell::RefCell<Vec<Option<String>>>; }
    pub fn stored_api_key(provider: &str, env_vars: &[&str]) -> Option<String> {
        if provider == "commandcode" {
            assert!(
                env_vars.is_empty(),
                "CommandCode must not inspect environment keys"
            );
        }
        if let Ok(key) = KEY_SEQUENCE.try_with(|keys| keys.borrow_mut().remove(0)) {
            return key;
        }
        let _ = DISCOVERY_READS
            .try_with(|reads| reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst));
        Some("synthetic-key-never-real".into())
    }
    pub fn config_value(_: &str) -> Option<serde_json::Value> {
        None
    }
    pub fn provider_disabled(_: &str) -> bool {
        false
    }
    pub fn config_dir() -> PathBuf {
        std::env::temp_dir().join(format!("pane-network-fixture-{}", std::process::id()))
    }
}
mod spend {
    static SCAN_GAPS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    pub fn note_scan_gap() {
        SCAN_GAPS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    tokio::task_local! { pub static READS: std::sync::Arc<std::sync::atomic::AtomicUsize>; }
    pub fn month_to_date_cost(_: &str) -> Option<f64> {
        let _ = READS.try_with(|reads| reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst));
        None
    }
}

fn access_runtime() -> std::sync::Arc<access_policy::AccessRuntime> {
    std::sync::Arc::new(access_policy::AccessRuntime::new(
        access_policy::AccessPolicy::default(),
    ))
}

#[path = "../../../src-tauri/src/provider_modes.rs"]
pub mod provider_modes;

#[cfg(test)]
mod mode_commands;

#[cfg(test)]
mod commandcode_keys;
