#![cfg(unix)]
use pane_scan_tests::scan_policy::{update_source_grant, ScanPolicy};
use std::{collections::BTreeMap, fs, path::PathBuf};
struct Fixture {
    base: PathBuf,
    a: PathBuf,
    b: PathBuf,
    outside: PathBuf,
    fresh: PathBuf,
}
impl Fixture {
    fn new(name: &str) -> Self {
        let base =
            std::env::temp_dir().join(format!("pane-root-retarget-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let (a, b, outside, fresh) = (
            base.join("a"),
            base.join("b"),
            base.join("outside"),
            base.join("fresh"),
        );
        for path in [&a, &b, &outside, &fresh] {
            fs::create_dir_all(path).unwrap();
        }
        Self {
            base,
            a,
            b,
            outside,
            fresh,
        }
    }
    fn grants(&self) -> (BTreeMap<String, Vec<PathBuf>>, BTreeMap<String, String>) {
        (
            ScanPolicy::new(BTreeMap::from([
                ("claude".into(), vec![self.a.clone()]),
                ("qwen".into(), vec![self.b.clone()]),
            ]))
            .unwrap()
            .roots,
            BTreeMap::from([
                ("claude".into(), "a".repeat(32)),
                ("qwen".into(), "b".repeat(32)),
            ]),
        )
    }
    fn retarget(&self, path: &std::path::Path) {
        fs::remove_dir(path).unwrap();
        std::os::unix::fs::symlink(&self.outside, path).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}
#[test]
fn persisted_root_retarget_is_denied_on_reload_before_target_logs_open() {
    let f = Fixture::new("reload");
    let (roots, _) = f.grants();
    let saved = serde_json::to_string(&roots).unwrap();
    fs::write(f.outside.join("session.jsonl"), "synthetic outside content").unwrap();
    f.retarget(&f.a);
    let reloaded: BTreeMap<String, Vec<PathBuf>> = serde_json::from_str(&saved).unwrap();
    assert!(
        ScanPolicy::from_saved(reloaded).is_err(),
        "restoration cannot adopt a new root target"
    );
    let denied = ScanPolicy::from_saved(roots).unwrap_or_default();
    assert!(!denied.allows_path("claude", &f.outside.join("session.jsonl")));
    denied.scope("claude", || {
        assert!(pane_scan_tests::scan_policy::open_log(&f.outside.join("session.jsonl")).is_err())
    });
}
#[test]
fn unrelated_source_addition_cannot_reauthorize_a_retargeted_saved_root() {
    let f = Fixture::new("unrelated");
    let (mut roots, mut epochs) = f.grants();
    let before = (roots.clone(), epochs.clone());
    f.retarget(&f.b);
    assert!(update_source_grant(
        &mut roots,
        &mut epochs,
        "claude",
        vec![f.a.clone(), f.fresh.clone()]
    )
    .is_err());
    assert_eq!((roots, epochs), before);
}
#[test]
fn adding_to_same_source_does_not_follow_its_retained_retargeted_root() {
    let f = Fixture::new("retained");
    let (mut roots, mut epochs) = f.grants();
    let before = (roots.clone(), epochs.clone());
    f.retarget(&f.a);
    assert!(update_source_grant(
        &mut roots,
        &mut epochs,
        "claude",
        vec![f.a.clone(), f.fresh.clone()]
    )
    .is_err());
    assert_eq!((roots, epochs), before);
}
#[test]
fn pure_revocation_keeps_saved_spelling_and_runtime_denies_its_retarget() {
    let f = Fixture::new("revoke");
    let (mut roots, mut epochs) = f.grants();
    update_source_grant(
        &mut roots,
        &mut epochs,
        "claude",
        vec![f.a.clone(), f.fresh.clone()],
    )
    .unwrap();
    f.retarget(&f.a);
    assert!(update_source_grant(&mut roots, &mut epochs, "claude", vec![f.a.clone()]).unwrap());
    assert_eq!(roots["claude"], vec![f.a.clone()]);
    assert!(ScanPolicy::from_saved(roots).is_err());
}
#[test]
fn explicit_new_input_can_choose_a_symlink_target_and_persists_the_target() {
    let f = Fixture::new("explicit");
    let (mut roots, mut epochs) = f.grants();
    let old = epochs["claude"].clone();
    let alias = f.base.join("explicit-alias");
    std::os::unix::fs::symlink(&f.outside, &alias).unwrap();
    assert!(update_source_grant(&mut roots, &mut epochs, "claude", vec![alias.clone()]).unwrap());
    assert_eq!(roots["claude"], vec![f.outside.canonicalize().unwrap()]);
    assert_ne!(epochs["claude"], old);
    fs::remove_file(&alias).unwrap();
    std::os::unix::fs::symlink(&f.fresh, &alias).unwrap();
    let runtime = ScanPolicy::from_saved(roots).unwrap();
    assert_eq!(
        runtime.roots["claude"],
        vec![f.outside.canonicalize().unwrap()]
    );
}
