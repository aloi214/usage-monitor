use pane_scan_tests::{
    access_policy::AccessPolicy,
    scan_sources::{self, SourceEnvironment},
};
use serde_json::json;
use std::{collections::BTreeMap, fs, path::PathBuf};

struct Fixture {
    base: PathBuf,
}
impl Fixture {
    fn new(name: &str) -> Self {
        let base =
            std::env::temp_dir().join(format!("pane-source-mode-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        Self {
            base: base.canonicalize().unwrap(),
        }
    }
    fn dir(&self, name: &str) -> PathBuf {
        let p = self.base.join(name);
        fs::create_dir_all(&p).unwrap();
        p
    }
    fn env(&self) -> SourceEnvironment {
        SourceEnvironment {
            home: Some(self.base.clone()),
            vars: BTreeMap::new(),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}
fn configure(
    p: &mut AccessPolicy,
    env: &SourceEnvironment,
    source: &str,
    enabled: bool,
    mode: &str,
    dirs: Option<Vec<PathBuf>>,
) -> Result<(), String> {
    scan_sources::configure(p, env, source, enabled, mode, dirs)
}
fn status(p: &AccessPolicy, env: &SourceEnvironment, source: &str) -> serde_json::Value {
    serde_json::to_value(
        scan_sources::catalog(p, env)
            .into_iter()
            .find(|s| s.source == source)
            .unwrap(),
    )
    .unwrap()
}

#[test]
fn missing_default_stays_on_and_can_appear_at_frozen_path() {
    let f = Fixture::new("missing-default");
    let env = f.env();
    let mut p = AccessPolicy::default();
    configure(&mut p, &env, "claude", true, "default", None).unwrap();
    assert_eq!(status(&p, &env, "claude")["enabled"], true);
    assert_eq!(
        status(&p, &env, "claude")["directories"][0]["status"],
        "notFound"
    );
    assert_eq!(
        p.scan_roots["claude"],
        vec![f.base.join(".claude/projects")]
    );
    assert!(scan_sources::runtime_policy(&p).roots.is_empty());
    let root = f.dir(".claude/projects");
    let file = root.join("fixture.jsonl");
    fs::write(&file, "{}\n").unwrap();
    assert!(scan_sources::runtime_policy(&p).allows_path("claude", &file));
    assert_eq!(
        status(&p, &env, "claude")["directories"][0]["status"],
        "found"
    );
    assert!(p.enabled_accounts.is_empty());
    assert!(p.enabled_families.is_empty());
}

#[test]
fn valid_custom_replaces_defaults_invalid_custom_leaves_policy_unchanged() {
    let f = Fixture::new("custom-replace");
    let env = f.env();
    let mut p = AccessPolicy::default();
    f.dir(".claude/projects");
    let custom = f.dir("custom");
    configure(&mut p, &env, "claude", true, "default", None).unwrap();
    configure(
        &mut p,
        &env,
        "claude",
        true,
        "custom",
        Some(vec![custom.clone()]),
    )
    .unwrap();
    assert_eq!(p.scan_roots["claude"], vec![custom]);
    let before = p.clone();
    assert!(configure(
        &mut p,
        &env,
        "claude",
        true,
        "custom",
        Some(vec![f.base.join("missing")])
    )
    .is_err());
    assert_eq!(p, before);
}

#[test]
fn off_retains_custom_location_revokes_and_reenable_rotates_epoch() {
    let f = Fixture::new("off");
    let env = f.env();
    let mut p = AccessPolicy::default();
    let custom = f.dir("custom");
    configure(
        &mut p,
        &env,
        "claude",
        true,
        "custom",
        Some(vec![custom.clone()]),
    )
    .unwrap();
    let old = p.scan_epochs["claude"].clone();
    configure(&mut p, &env, "claude", false, "custom", None).unwrap();
    assert!(!p.scan_roots.contains_key("claude"));
    assert!(!p.scan_epochs.contains_key("claude"));
    assert_eq!(
        status(&p, &env, "claude")["customDirectories"],
        json!([custom])
    );
    assert_eq!(status(&p, &env, "claude")["enabled"], false);
    configure(&mut p, &env, "claude", true, "custom", None).unwrap();
    assert_ne!(p.scan_epochs["claude"], old);
    p = AccessPolicy::default();
    assert!(p.scan_roots.is_empty());
    assert!(p.scan_epochs.is_empty());
    assert_eq!(status(&p, &env, "claude")["enabled"], false);
}

#[test]
fn unavailable_source_does_not_disable_or_block_changes_to_others() {
    let f = Fixture::new("independent");
    let env = f.env();
    let mut p = AccessPolicy::default();
    let a = f.dir("a");
    let b = f.dir("b");
    let c = f.dir("c");
    configure(
        &mut p,
        &env,
        "claude",
        true,
        "custom",
        Some(vec![a.clone()]),
    )
    .unwrap();
    configure(&mut p, &env, "qwen", true, "custom", Some(vec![b.clone()])).unwrap();
    fs::remove_dir(&a).unwrap();
    let file = b.join("fixture.jsonl");
    fs::write(&file, "{}\n").unwrap();
    assert!(scan_sources::runtime_policy(&p).allows_path("qwen", &file));
    configure(&mut p, &env, "qwen", true, "custom", Some(vec![c.clone()])).unwrap();
    assert_eq!(p.scan_roots["qwen"], vec![c]);
    assert_eq!(status(&p, &env, "claude")["enabled"], true);
    assert_eq!(
        status(&p, &env, "claude")["directories"][0]["status"],
        "notFound"
    );
}

#[test]
fn environment_changes_do_not_expand_frozen_defaults() {
    let f = Fixture::new("frozen-env");
    let mut env = f.env();
    let mut p = AccessPolicy::default();
    configure(&mut p, &env, "codex", true, "default", None).unwrap();
    let before = p.clone();
    env.vars.insert("CODEX_HOME".into(), f.dir("new-codex"));
    configure(&mut p, &env, "codex", true, "default", None).unwrap();
    assert_eq!(p, before);
    assert_eq!(
        status(&p, &env, "codex")["directories"][0]["path"],
        json!(f.base.join(".codex/sessions"))
    );
}

#[test]
fn defaults_use_only_known_leaf_directories_and_explicit_environment_roots() {
    let f = Fixture::new("known-defaults");
    let mut env = f.env();
    env.vars
        .insert("CODEX_HOME".into(), f.base.join("custom-codex"));
    assert_eq!(
        scan_sources::default_directories("codex", &env).unwrap(),
        vec![
            f.base.join("custom-codex/sessions"),
            f.base.join("custom-codex/archived_sessions")
        ]
    );
    assert_eq!(
        scan_sources::default_directories("claude", &env).unwrap(),
        vec![f.base.join(".claude/projects")]
    );
    assert_eq!(
        scan_sources::default_directories("qwen", &env).unwrap(),
        vec![f.base.join(".qwen/usage")]
    );
    env.vars
        .insert("CODEX_HOME".into(), PathBuf::from("relative"));
    assert!(scan_sources::default_directories("codex", &env).is_err());
}

#[test]
#[cfg(unix)]
fn missing_default_symlink_creation_is_denied_without_affecting_other_sources() {
    let f = Fixture::new("missing-symlink");
    let env = f.env();
    let mut p = AccessPolicy::default();
    f.dir(".claude");
    configure(&mut p, &env, "claude", true, "default", None).unwrap();
    let outside = f.dir("outside");
    let file = outside.join("fixture.jsonl");
    fs::write(&file, "{}\n").unwrap();
    std::os::unix::fs::symlink(&outside, f.base.join(".claude/projects")).unwrap();
    assert!(!scan_sources::runtime_policy(&p).allows_path("claude", &file));
    assert_eq!(
        status(&p, &env, "claude")["directories"][0]["status"],
        "unavailable"
    );
    let before = p.clone();
    configure(&mut p, &env, "claude", true, "default", None).unwrap();
    assert_eq!(p, before);
    let good = f.dir("good");
    configure(&mut p, &env, "qwen", true, "custom", Some(vec![good])).unwrap();
    assert!(!scan_sources::runtime_policy(&p)
        .roots
        .contains_key("claude"));
}

#[test]
#[cfg(unix)]
fn remembered_custom_cannot_silently_retarget_after_off() {
    let f = Fixture::new("custom-symlink");
    let env = f.env();
    let mut p = AccessPolicy::default();
    let custom = f.dir("custom");
    configure(
        &mut p,
        &env,
        "claude",
        true,
        "custom",
        Some(vec![custom.clone()]),
    )
    .unwrap();
    configure(&mut p, &env, "claude", false, "custom", None).unwrap();
    let before = p.clone();
    fs::remove_dir(&custom).unwrap();
    let outside = f.dir("outside");
    std::os::unix::fs::symlink(outside, &custom).unwrap();
    assert!(configure(&mut p, &env, "claude", true, "custom", None).is_err());
    assert!(configure(&mut p, &env, "claude", true, "custom", Some(vec![custom])).is_err());
    assert_eq!(p, before);
}

#[test]
fn mixed_legacy_paths_are_preserved_and_good_paths_remain_usable() {
    let f = Fixture::new("legacy");
    let env = f.env();
    let a = f.dir("a");
    let b = f.base.join("missing");
    let p = AccessPolicy::from_config(
        &json!({"accessPolicy": {"version":1,"enabledFamilies":[],"enabledAccounts":[],"scanRoots":{"claude":[a,b]}}}),
    );
    assert_eq!(status(&p, &env, "claude")["mode"], "custom");
    assert_eq!(
        status(&p, &env, "claude")["customDirectories"],
        json!([a, b])
    );
    assert_eq!(scan_sources::runtime_policy(&p).roots["claude"], vec![a]);
}

#[test]
fn explicit_off_clears_stale_authority_even_when_intent_already_says_off() {
    let f = Fixture::new("stale-off");
    let env = f.env();
    let mut p = AccessPolicy::default();
    p.scan_roots.insert("claude".into(), vec![f.dir("stale")]);
    p.scan_epochs.insert("claude".into(), "old".into());
    configure(&mut p, &env, "claude", false, "default", None).unwrap();
    assert!(!p.scan_roots.contains_key("claude"));
    assert!(!p.scan_epochs.contains_key("claude"));
}

#[test]
fn off_and_legacy_setter_keep_durable_intent_coherent() {
    let f = Fixture::new("legacy-setter");
    let env = f.env();
    let mut p = AccessPolicy::default();
    let a = f.dir("a");
    let b = f.dir("b");
    scan_sources::set_legacy(&mut p, "claude", vec![a.clone(), b.clone()]).unwrap();
    assert_eq!(status(&p, &env, "claude")["enabled"], true);
    assert_eq!(
        status(&p, &env, "claude")["customDirectories"],
        json!([a, b])
    );
    fs::remove_dir(&b).unwrap();
    scan_sources::set_legacy(&mut p, "claude", vec![b.clone()]).unwrap();
    assert_eq!(status(&p, &env, "claude")["customDirectories"], json!([b]));
    scan_sources::set_legacy(&mut p, "claude", Vec::new()).unwrap();
    assert_eq!(status(&p, &env, "claude")["enabled"], false);
    assert!(p.scan_roots.is_empty());
    assert!(p.scan_epochs.is_empty());
}

#[test]
fn windows_environment_locations_are_narrow_and_do_not_read_credentials() {
    // Use injected platform env variables on every host; Windows native path
    // parsing itself is covered by the cfg(windows) case below and GNU check.
    let f = Fixture::new("windows-layout");
    let mut env = f.env();
    let roaming = f.base.join("AppData/Roaming");
    let local = f.base.join("AppData/Local");
    env.vars.insert("APPDATA".into(), roaming.clone());
    env.vars.insert("LOCALAPPDATA".into(), local.clone());
    assert_eq!(
        scan_sources::default_directories("devin", &env).unwrap(),
        vec![roaming.join("devin/cli")]
    );
    assert_eq!(
        scan_sources::default_directories("hermes", &env).unwrap(),
        vec![local.join("hermes")]
    );
    assert_eq!(
        scan_sources::default_directories("stepcode", &env).unwrap(),
        vec![f.base.join(".stepcode/agent/sessions")]
    );
    assert_eq!(
        scan_sources::default_directories("grok", &env).unwrap(),
        vec![f.base.join(".grok/logs")]
    );
    assert_eq!(
        scan_sources::default_directories("minimax", &env).unwrap(),
        vec![f.base.join(".minimax")]
    );
    let p = AccessPolicy::default();
    let before = p.clone();
    assert_eq!(
        scan_sources::catalog(&p, &env).len(),
        pane_scan_tests::scan_policy::SOURCES.len()
    );
    assert_eq!(p, before);
}

#[test]
#[cfg(windows)]
fn windows_drive_and_unc_candidates_remain_absolute() {
    let env = SourceEnvironment {
        home: Some(PathBuf::from(r"C:\Users\synthetic")),
        vars: BTreeMap::from([
            ("CODEX_HOME".into(), PathBuf::from(r"\\server\share\codex")),
            (
                "LOCALAPPDATA".into(),
                PathBuf::from(r"C:\Users\synthetic\AppData\Local"),
            ),
        ]),
    };
    assert_eq!(
        scan_sources::default_directories("claude", &env).unwrap(),
        vec![PathBuf::from(r"C:\Users\synthetic\.claude\projects")]
    );
    let codex = scan_sources::default_directories("codex", &env).unwrap();
    assert!(codex.iter().all(|path| path.is_absolute()));
    assert_eq!(codex[0], PathBuf::from(r"\\server\share\codex\sessions"));
    assert!(scan_sources::default_directories("hermes", &env).unwrap()[0].is_absolute());
}

#[test]
fn malformed_overlapping_saved_authority_is_diagnosed_without_disabling_unrelated_source() {
    let f = Fixture::new("overlap-diagnostic");
    let env = f.env();
    let root = f.dir("overlap");
    let other = f.dir("other");
    let p = AccessPolicy::from_config(
        &json!({"accessPolicy":{"version":1,"enabledFamilies":[],"enabledAccounts":[],"scanRoots":{"claude":[root],"pi":[root],"qwen":[other]}}}),
    );
    assert_eq!(
        status(&p, &env, "claude")["directories"][0]["status"],
        "unavailable"
    );
    assert_eq!(
        status(&p, &env, "pi")["directories"][0]["status"],
        "unavailable"
    );
    assert_eq!(scan_sources::runtime_policy(&p).roots["qwen"], vec![other]);
}

#[test]
#[cfg(unix)]
fn dangling_retargeted_saved_directory_is_unavailable_not_merely_missing() {
    let f = Fixture::new("dangling");
    let env = f.env();
    let mut p = AccessPolicy::default();
    let root = f.dir(".claude/projects");
    configure(&mut p, &env, "claude", true, "default", None).unwrap();
    fs::remove_dir(&root).unwrap();
    std::os::unix::fs::symlink(f.base.join("outside-missing"), &root).unwrap();
    assert_eq!(
        status(&p, &env, "claude")["directories"][0]["status"],
        "unavailable"
    );
    assert!(scan_sources::runtime_policy(&p).roots.is_empty());
}

#[test]
#[cfg(unix)]
fn legacy_setter_cannot_retarget_remembered_custom_after_off() {
    let f = Fixture::new("legacy-off-retarget");
    let env = f.env();
    let mut p = AccessPolicy::default();
    let root = f.dir("custom");
    let outside = f.dir("outside");
    configure(
        &mut p,
        &env,
        "claude",
        true,
        "custom",
        Some(vec![root.clone()]),
    )
    .unwrap();
    configure(&mut p, &env, "claude", false, "custom", None).unwrap();
    let before = p.clone();
    fs::remove_dir(&root).unwrap();
    std::os::unix::fs::symlink(&outside, &root).unwrap();
    assert!(scan_sources::set_legacy(&mut p, "claude", vec![root]).is_err());
    assert_eq!(p, before);
    let log = outside.join("synthetic.jsonl");
    fs::write(&log, "{}\n").unwrap();
    assert!(!scan_sources::runtime_policy(&p).allows_path("claude", &log));
}

#[test]
#[cfg(unix)]
fn switching_to_custom_cannot_retarget_remembered_default_spelling() {
    let f = Fixture::new("default-custom-retarget");
    let env = f.env();
    let mut p = AccessPolicy::default();
    let root = f.dir(".claude/projects");
    let outside = f.dir("outside");
    configure(&mut p, &env, "claude", true, "default", None).unwrap();
    configure(&mut p, &env, "claude", false, "default", None).unwrap();
    let before = p.clone();
    fs::remove_dir(&root).unwrap();
    std::os::unix::fs::symlink(&outside, &root).unwrap();
    assert!(configure(
        &mut p,
        &env,
        "claude",
        true,
        "custom",
        Some(vec![root.clone()])
    )
    .is_err());
    assert_eq!(p, before);
    assert!(scan_sources::set_legacy(&mut p, "claude", vec![root]).is_err());
    assert_eq!(p, before);
}
