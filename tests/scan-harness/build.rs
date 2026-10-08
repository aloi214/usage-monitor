// Compile unchanged production plan helpers and the app-owned config reader.
// Exclude the account/credential/network adapter entirely from scan tests.
use std::{env, fs, path::PathBuf};

fn section<'a>(source: &'a str, from: &str, to: &str) -> &'a str {
    let start = source.find(from).expect(from);
    let end = source[start..].find(to).expect(to) + start;
    &source[start..end]
}

fn main() {
    let providers = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("../../src-tauri/src/providers");
    let stepfun = providers.join("stepfun.rs");
    let common = providers.join("mod.rs");
    println!("cargo:rerun-if-changed={}", stepfun.display());
    println!("cargo:rerun-if-changed={}", common.display());
    let stepfun = fs::read_to_string(stepfun).unwrap();
    let common = fs::read_to_string(common).unwrap();
    let bridge_path = providers.parent().unwrap().join("lib.rs");
    println!("cargo:rerun-if-changed={}", bridge_path.display());
    let bridge = fs::read_to_string(bridge_path).unwrap();
    let mut commands = section(&bridge, "const CONFIG_KEYS:", "static CONFIG_WRITE:").to_string();
    for marker in [
        "fn get_scan_sources(",
        "async fn configure_scan_source(",
        "async fn set_scan_source(",
        "async fn reset_provider_access(",
        "pub(crate) fn local_scan_policy(",
        "fn apply_config_patch(",
        "fn set_config_in(",
    ] {
        let start = bridge.find(marker).expect(marker);
        let end = bridge[start..].find("\n}\n").unwrap() + start + 3;
        commands.push_str(&bridge[start..end]);
        commands.push('\n');
    }
    fs::write(
        PathBuf::from(env::var("OUT_DIR").unwrap()).join("source_commands.rs"),
        commands,
    )
    .unwrap();
    let generated = format!(
        "use chrono::{{Datelike, Local, NaiveDate}};\nuse super::{{config_dir, Metric}};\n{}\n{}\n{}",
        section(&common, "pub fn config_value(", "/// True when Customize"),
        section(&stepfun, "const CNY_PER_USD:", "pub async fn snapshot()"),
        section(&stepfun, "pub(crate) fn plan_tier()", "/// `GET /v1/accounts` body:"),
    );
    fs::write(
        PathBuf::from(env::var("OUT_DIR").unwrap()).join("stepfun_plan.rs"),
        generated,
    )
    .unwrap();
}
