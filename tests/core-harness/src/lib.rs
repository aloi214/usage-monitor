// Linux-capable tests reference production modules directly. This is not a substitute
// for compiling the Windows Tauri application.
#[path = "../../../src-tauri/src/access_policy.rs"]
pub mod access_policy;
#[path = "../../../src-tauri/src/private_file.rs"]
mod private_file;

#[path = "../../../src-tauri/src/snapshot.rs"]
pub mod snapshot;
#[path = "../../../src-tauri/src/usage_publication.rs"]
pub mod usage_publication;

// Adapter shims expose the production snapshot model and synthetic in-memory
// runtime only. No app, port, credential source, or home directory is opened.
pub mod providers {
    pub use crate::snapshot::{Metric, ResetCredit, Snapshot};
}
fn access_runtime() -> &'static std::sync::Arc<access_policy::AccessRuntime> {
    static RUNTIME: std::sync::OnceLock<std::sync::Arc<access_policy::AccessRuntime>> =
        std::sync::OnceLock::new();
    RUNTIME.get_or_init(|| {
        std::sync::Arc::new(access_policy::AccessRuntime::new(
            access_policy::AccessPolicy::default(),
        ))
    })
}
fn usage_publications() -> &'static usage_publication::UsagePublication {
    static STORE: std::sync::OnceLock<usage_publication::UsagePublication> =
        std::sync::OnceLock::new();
    STORE.get_or_init(Default::default)
}
fn card_is_disabled(id: &str, disabled: &[String]) -> bool {
    disabled.iter().any(|d| d == id)
}
#[path = "../../../src-tauri/src/httpapi.rs"]
mod httpapi;

#[path = "../../../src-tauri/src/startup_cleanup.rs"]
mod startup_cleanup;
