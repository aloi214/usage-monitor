// Compile the actual usage IPC, cache restore and cooldown bodies. Only native
// application services and provider I/O are replaced by counted test boundaries.
use std::{env, fs, path::PathBuf};

fn main() {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../src-tauri/src");
    let path = root.join("lib.rs");
    println!("cargo:rerun-if-changed={}", path.display());
    let source = fs::read_to_string(path).unwrap();
    let mut config = String::new();
    let start = source.find("fn config_path_in(").unwrap();
    let end = source.find("use access_policy::{retired_id, sanitize_retired_config};").unwrap();
    config.push_str(&source[start..end]);
    let start = source.find("const CONFIG_KEYS:").unwrap();
    let end = source.find("#[tauri::command]\nfn set_config(app:").unwrap();
    config.push_str(&source[start..end]);
    fs::write(PathBuf::from(env::var("OUT_DIR").unwrap()).join("config_commands.rs"), config).unwrap();
    let mut output = String::new();
    for marker in [
        "fn empty_usage_result(",
        "struct FailState {",
        "fn change_access_policy(",
        "fn retain_authorized_snapshots(",
        "fn family_of(",
        "fn is_managed_key_card(",
        "fn is_plain_api_key_provider(",
        "fn is_credential_scoped_card(",
        "fn card_is_disabled(",
        "async fn guarded<",
        "async fn guarded_moonshot_wallet(",
        "fn fold_moonshot_into_kimi(",
        "fn is_kimi_wallet_label(",
        "fn restore_kimi_wallet_rows(",
        "fn preserve_unqueried_kimi_wallet(",
        "fn restore_last_success_after_error(",
        "fn hydrate_fetch_time(",
        "fn read_usage_identity<T>(",
        "async fn fetch_usage(",
        "async fn fetch_spend(",
        "enum UsageRefreshScope {",
        "fn usage_refresh_gate(",
        "fn cached_usage(",
        "fn retain_current_key_card_results(",
        "fn current_credential_scoped_generations(",
        "fn key_card_snapshot_generations(",
        "fn persist_last_ok_at(",
        "fn now_ms(",
    ] {
        let start = if marker == "enum UsageRefreshScope {" {
            source[..source.find(marker).expect(marker)]
                .rfind("#[derive")
                .expect("scope derive")
        } else {
            source.find(marker).expect(marker)
        };
        let end = source[start..].find("\n}\n").unwrap() + start + 3;
        output.push_str(&source[start..end]);
        output.push('\n');
    }
    let start = source.find("struct SpendResult {").unwrap();
    let end = source[start..].find("\n").unwrap() + start;
    output.push_str("#[derive(serde::Serialize)]\n");
    output.push_str(&source[start..end]); output.push('\n');
    let start = source.find("const API_KEY_PROVIDERS:").unwrap();
    let end = source[start..].find("\n];").unwrap() + start + 3;
    output.push_str(&source[start..end]);
    let start = source.find("#[derive(Clone, Copy, Default, serde::Deserialize)]\n#[serde(rename_all = \"camelCase\")]\nenum UsageRefreshReason {").expect("production usage reason");
    let end = source[start..].find("\n}\n").unwrap() + start + 3;
    output.push_str(&source[start..end]);
    let start = source.find("fn append_manual_usage_snapshots(").unwrap();
    let end = source[start..].find("\n}\n").unwrap() + start + 3;
    output.push_str(&source[start..end]);
    output.push_str("\nasync fn fetch_fixture(reason: Option<&str>, disabled: Option<Vec<String>>) -> usage_publication::UsageResult { fetch_usage(AppHandle, disabled, reason.map(|r| serde_json::from_value(serde_json::json!(r)).unwrap()), None).await.unwrap() }\n");
    output.push_str("\nasync fn fetch_scoped_fixture(id: &str, reason: Option<&str>, disabled: Option<Vec<String>>) -> Result<usage_publication::UsageResult, String> { fetch_usage(AppHandle, disabled, reason.map(|r| serde_json::from_value(serde_json::json!(r)).unwrap()), Some(UsageRefreshScope::Account { account_id: id.to_string() })).await }\n");
    // Exercise the real composite Kimi body, including its nested Moonshot
    // permit. Only credential/endpoint boundaries are synthetic and counted.
    let kimi_path = root.join("providers/kimi.rs");
    println!("cargo:rerun-if-changed={}", kimi_path.display());
    let kimi = fs::read_to_string(kimi_path).unwrap();
    let mut composite = String::new();
    for marker in ["pub async fn snapshot()", "async fn fetch()"] {
        let start = kimi.find(marker).unwrap();
        let end = kimi[start..].find("\n}\n").unwrap() + start + 3;
        composite.push_str(&kimi[start..end]);
        composite.push('\n');
    }
    fs::write(
        PathBuf::from(env::var("OUT_DIR").unwrap()).join("kimi_composite.rs"),
        composite,
    )
    .unwrap();
    let http_path = root.join("httpapi.rs");
    println!("cargo:rerun-if-changed={}", http_path.display());
    let http = fs::read_to_string(http_path).unwrap();
    for marker in ["pub(crate) fn provider_json(", "fn iso8601("] {
        let start = http.find(marker).unwrap();
        let end = http[start..].find("\n}\n").unwrap() + start + 3;
        output.push_str(&http[start..end]);
    }
    fs::write(
        PathBuf::from(env::var("OUT_DIR").unwrap()).join("usage_commands.rs"),
        output,
    )
    .unwrap();
}
