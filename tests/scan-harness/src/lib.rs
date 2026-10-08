#[path = "../../../src-tauri/src/provider_auto_refresh.rs"]
mod provider_auto_refresh;
#[path = "../../../src-tauri/src/cache_fingerprint.rs"]
pub mod cache_fingerprint;
// These are actual production modules; the harness supplies only desktop I/O.
#[path = "../../../src-tauri/src/providers/devin.rs"]
pub mod devin_adapter;
#[path = "../../../src-tauri/src/providers/hermes.rs"]
pub mod hermes_adapter;
#[path = "../../../src-tauri/src/providers/minimax.rs"]
pub mod minimax_adapter;
#[path = "../../../src-tauri/src/providers/opencode.rs"]
pub mod opencode_adapter;
#[path = "../../../src-tauri/src/pricing.rs"]
pub mod pricing;
#[path = "../../../src-tauri/src/scan_policy.rs"]
pub mod scan_policy;
#[path = "../../../src-tauri/src/snapshot.rs"]
pub mod snapshot;
#[path = "../../../src-tauri/src/spend.rs"]
pub mod spend;
// Compile the production public-pricing module without pulling the Windows
// Tauri runtime. No test invokes its network function.
extern crate self as tauri;
pub mod async_runtime {
    pub fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Runtime::new().unwrap().block_on(future)
    }
}
pub fn local_scan_policy() -> scan_policy::ScanPolicy {
    scan_policy::ScanPolicy::default()
}
#[path = "../../../src-tauri/src/network_policy.rs"]
pub mod network_policy;
#[path = "../../../src-tauri/src/network_transport.rs"]
pub mod network_transport;
pub mod providers {
    pub fn proxy_url() -> Option<&'static str> {
        None
    }
    pub use crate::hermes_adapter as hermes;
    pub use crate::snapshot::{Metric, Snapshot};
    pub mod stepfun {
        include!(concat!(env!("OUT_DIR"), "/stepfun_plan.rs"));
    }
    pub const MAX_LEDGER_ROWS: u64 = 2_100_000;
    pub fn config_dir() -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("pane-scan-harness-config-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }
    pub fn http(_provider: &str) -> reqwest::Client {
        panic!("A local scan must not request network access")
    }
    pub fn open_readonly_sqlite(path: &std::path::Path) -> Result<rusqlite::Connection, String> {
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| e.to_string())
    }
    pub use crate::devin_adapter as devin;
    pub use crate::minimax_adapter as minimax;
    pub use crate::opencode_adapter as opencode;
    pub fn http_no_redirect(_provider: &str) -> reqwest::Client {
        panic!("A local scan must not make a credential-bearing request")
    }
    pub fn stored_api_key(_provider: &str, _vars: &[&str]) -> Option<String> {
        None
    }
    pub fn account_scan_roots() -> Vec<std::path::PathBuf> {
        Vec::new()
    }
    pub fn credit_meter_labeled(
        _provider: &str,
        _sign: &str,
        _balance: f64,
        _label: &str,
        _suffix: &str,
    ) -> Option<Metric> {
        panic!("Remote-only credit projection is not part of local scan tests")
    }
    pub async fn json_body(
        _response: reqwest::Response,
        _max: usize,
        _what: &str,
    ) -> Result<serde_json::Value, String> {
        panic!("Network response not allowed in local scan tests")
    }
    pub fn read_small_text(
        path: &std::path::Path,
        cap: u64,
        _what: &str,
    ) -> Result<String, String> {
        if !path.starts_with(std::env::temp_dir()) {
            return Err("Non-fixture file read denied by harness".into());
        }
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let mut text = String::new();
        std::io::Read::read_to_string(&mut std::io::Read::take(file, cap + 1), &mut text)
            .map_err(|e| e.to_string())?;
        if text.len() as u64 > cap {
            return Err("Fixture exceeds read cap".into());
        }
        Ok(text)
    }
    pub const MAX_TEMP_SQLITE_BYTES: u64 = 512 * 1024 * 1024;
    pub fn temp_sqlite_copy_allowed(path: &std::path::Path) -> bool {
        path.metadata()
            .map(|m| m.len() <= MAX_TEMP_SQLITE_BYTES)
            .unwrap_or(false)
    }
    pub fn remove_sqlite_files(path: &std::path::Path) {
        assert!(path.starts_with(std::env::temp_dir()));
        for suffix in ["", "-wal", "-shm", "-journal"] {
            let mut p = path.as_os_str().to_os_string();
            p.push(suffix);
            let _ = std::fs::remove_file(p);
        }
    }
}
// Hermes is included at crate root, so its ordinary provider siblings are
// re-exported here as well. Their behavior is the same harness I/O boundary.
pub use providers::*;

#[path = "../../../src-tauri/src/access_policy.rs"]
pub mod access_policy;
#[path = "../../../src-tauri/src/private_file.rs"]
pub mod private_file;

pub fn access_runtime() -> &'static std::sync::Arc<access_policy::AccessRuntime> {
    static RUNTIME: std::sync::OnceLock<std::sync::Arc<access_policy::AccessRuntime>> =
        std::sync::OnceLock::new();
    RUNTIME
        .get_or_init(|| std::sync::Arc::new(access_policy::AccessRuntime::new(Default::default())))
}

#[path = "../../../src-tauri/src/scan_sources.rs"]
pub mod scan_sources;

#[cfg(test)]
mod source_commands;
