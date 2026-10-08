#![allow(dead_code)]
#[path = "../../../src-tauri/src/access_policy.rs"]
mod access_policy;
#[path = "../../../src-tauri/src/credential_refresh.rs"]
mod credential_refresh;
#[path = "../../../src-tauri/src/private_file.rs"]
mod private_file;
#[cfg(test)]
mod tests;

#[path = "../../../src-tauri/src/network_policy.rs"]
pub mod network_policy;
#[path = "../../../src-tauri/src/network_transport.rs"]
pub mod network_transport;
#[path = "../../../src-tauri/src/scan_policy.rs"]
pub mod scan_policy;
#[path = "../../../src-tauri/src/snapshot.rs"]
pub mod snapshot;
extern crate self as tauri;
pub mod async_runtime {
    pub use tokio::task::spawn_blocking;
    pub fn block_on<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Runtime::new().unwrap().block_on(f)
    }
}
fn access_runtime() -> std::sync::Arc<access_policy::AccessRuntime> {
    std::sync::Arc::new(access_policy::AccessRuntime::new(
        access_policy::AccessPolicy::default(),
    ))
}
pub mod providers {
    include!(concat!(env!("OUT_DIR"), "/providers_common.rs"));
    pub fn proxy_url() -> Option<&'static str> {
        None
    }
    pub fn config_value(_: &str) -> Option<serde_json::Value> {
        None
    }
    pub fn provider_disabled(_: &str) -> bool {
        false
    }
    tokio::task_local! {pub static TEST_CONFIG: PathBuf; pub static TEST_KEYRING: String;}
    pub fn config_dir() -> PathBuf {
        TEST_CONFIG.try_with(Clone::clone).unwrap_or_else(|_| {
            std::env::temp_dir().join(format!(
                "pane-refresh-harness-config-{}",
                std::process::id()
            ))
        })
    }
    pub fn account_scan_roots() -> Vec<PathBuf> {
        Vec::new()
    }
    pub fn stored_api_key(_: &str, _: &[&str]) -> Option<String> {
        None
    }
    pub fn credential_string(_: &str) -> Option<String> {
        TEST_KEYRING.try_with(Clone::clone).ok()
    }
}

#[cfg(test)]
mod endpoint_tests;

// This harness tests provider credential-refresh/transport boundaries only.
// Composite quota orchestration and its real root guard run in usage-harness.
async fn guarded_moonshot_wallet() -> snapshot::Snapshot {
    panic!("composite wallet orchestration belongs to the actual-command usage harness")
}
