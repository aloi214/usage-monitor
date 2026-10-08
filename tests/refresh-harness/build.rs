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
    for name in ["claude", "codex", "kimi", "grok", "cursor", "moonshot"] {
        let file = root
            .join(format!("providers/{name}.rs"))
            .canonicalize()
            .unwrap();
        println!("cargo:rerun-if-changed={}", file.display());
        let wrapper=match name {
            "claude" => "pub(crate) async fn fixture_load(path:&std::path::Path)->Result<String,String>{oauth_access(path).await.map(|v|v.0)}",
            "codex" => "pub(crate) async fn fixture_load(path:&std::path::Path)->Result<String,String>{load_access(path).await.map(|v|v.token)}",
            "kimi" => "pub(crate) async fn fixture_load(path:&std::path::Path)->Result<String,String>{load_access(path,None).await}",
            "grok" => "pub(crate) async fn fixture_load(path:&std::path::Path)->Result<String,String>{load_access(path).await}",
            "cursor" => "pub(crate) async fn fixture_refresh(refresh:String)->Result<serde_json::Value,crate::credential_refresh::RemoteError>{refresh_access_token(refresh).await}",
            _ => "",
        };
        let output = PathBuf::from(env::var("OUT_DIR").unwrap()).join(format!("{name}_fixture.rs"));
        fs::write(
            &output,
            format!(
                "{}\n{}\n",
                fs::read_to_string(&file).unwrap(),
                if wrapper.is_empty() {
                    String::new()
                } else {
                    format!("#[cfg(test)] {wrapper}")
                }
            ),
        )
        .unwrap();
        generated.push_str(&format!("#[path = {:?}] pub mod {name};\n", output));
    }
    let anti = root.join("providers/antigravity.rs");
    println!("cargo:rerun-if-changed={}", anti.display());
    let source = fs::read_to_string(anti).unwrap();
    let start = source.find("fn run_hidden(").unwrap();
    let end = source[start..].find("/// `--flag").unwrap() + start;
    let portable=format!("{}fn run_hidden(_: &str, _: &[&str]) -> Option<String> {{ panic!(\"native process discovery is excluded from fixture tests\") }}\n{}",&source[..start],&source[end..]);
    let output = PathBuf::from(env::var("OUT_DIR").unwrap()).join("antigravity_fixture.rs");
    fs::write(&output,format!("{portable}\n#[cfg(test)] pub(crate) async fn fixture_load()->Result<String,String>{{cloud_access(None).await.map(|v|v.access)}}\n#[cfg(test)] pub(crate) async fn fixture_cloud_error()->Option<String>{{match try_cloud().await{{CloudResult::RefreshError(e)=>Some(e),_=>None}}}}\n")).unwrap();
    generated.push_str(&format!("#[path = {:?}] pub mod antigravity;\n", output));
    fs::write(
        PathBuf::from(env::var("OUT_DIR").unwrap()).join("providers_common.rs"),
        generated,
    )
    .unwrap();
}
