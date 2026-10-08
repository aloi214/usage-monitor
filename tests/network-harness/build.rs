// Compile the exact shared production types/body readers/accounting helpers,
// excluding Windows Credential Manager and real credential/config discovery.
// The network policy and transport are imported directly, never copied here.
use std::{env, fs, path::PathBuf};
fn main() {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../src-tauri/src");
    let common_path = root.join("providers/mod.rs");
    println!("cargo:rerun-if-changed={}", common_path.display());
    let source = fs::read_to_string(&common_path).unwrap();
    let mut generated = String::new();
    for (from, to) in [
        (
            "use std::collections::HashMap;",
            "/// Optional outbound proxy",
        ),
        (
            "pub fn http(provider:",
            "/// Where Pane keeps its own settings",
        ),
        ("/// Percent-used meter", "/// Candidate roots"),
        (
            "/// Hard cap for any leftover",
            "/// Drop leftover Pane temp snapshots",
        ),
    ] {
        let start = source.find(from).unwrap();
        let end = source[start..].find(to).unwrap() + start;
        generated.push_str(&source[start..end]);
        generated.push('\n');
    }
    for name in [
        "zai",
        "minimax",
        "moonshot",
        "qwen",
        "stepfun",
        "commandcode",
    ] {
        let file = root
            .join(format!("providers/{name}.rs"))
            .canonicalize()
            .unwrap();
        println!("cargo:rerun-if-changed={}", file.display());
        generated.push_str(&format!("#[path = {:?}] pub mod {name};\n", file));
    }
    // Compile the real root mode/key/reset bridge bodies with synthetic storage
    // and native-window boundaries. The command logic itself is never copied.
    let bridge_path = root.join("lib.rs");
    println!("cargo:rerun-if-changed={}", bridge_path.display());
    let bridge = fs::read_to_string(bridge_path).unwrap();
    let mut commands = String::new();
    for name in [
        "get_provider_modes",
        "set_provider_region_inner",
        "set_api_key",
        "reset_provider_access",
        "set_provider_region",
    ] {
        let marker = if name == "reset_provider_access" || name == "set_provider_region" {
            format!("async fn {name}(")
        } else {
            format!("fn {name}(")
        };
        let start = bridge.find(&marker).unwrap();
        let end = bridge[start..].find("\n}\n").unwrap() + start + 3;
        commands.push_str("pub(super) ");
        commands.push_str(&bridge[start..end]);
        commands.push('\n');
    }
    let start = bridge.find("static KEY_CARD_MUTATION_GENERATION:").unwrap();
    let end = bridge[start..].find("/// Strip deleted key cards").unwrap() + start;
    commands.push_str(&bridge[start..end]);
    fs::write(
        PathBuf::from(env::var("OUT_DIR").unwrap()).join("mode_commands.rs"),
        commands,
    )
    .unwrap();
    let mut keys = String::new();
    for marker in [
        "fn set_api_key_in(",
        "fn stored_pane_api_key(",
        "fn is_plain_api_key_provider(",
        "fn forget_provider_snapshots_inner(",
        "fn persist_last_ok_at(",
    ] {
        let start = bridge.find(marker).unwrap();
        let end = bridge[start..].find("\n}\n").unwrap() + start + 3;
        keys.push_str(&bridge[start..end]);
        keys.push('\n');
    }
    let start = bridge.find("const API_KEY_PROVIDERS:").unwrap();
    let end = bridge[start..].find("\n];").unwrap() + start + 3;
    keys.push_str(&bridge[start..end]);
    keys.push('\n');
    let start = bridge.find("static KEY_CARD_MUTATION_GENERATION:").unwrap();
    let end = bridge[start..].find("/// Strip deleted key cards").unwrap() + start;
    keys.push_str(&bridge[start..end]);
    fs::write(
        PathBuf::from(env::var("OUT_DIR").unwrap()).join("commandcode_keys.rs"),
        keys,
    )
    .unwrap();
    fs::write(
        PathBuf::from(env::var("OUT_DIR").unwrap()).join("providers_common.rs"),
        generated,
    )
    .unwrap();
}
