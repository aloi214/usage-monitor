use pane_scan_tests::scan_policy::{update_source_grant, ScanPolicy};
use std::{collections::BTreeMap, fs, path::PathBuf};
struct Fixture {
    base: PathBuf,
    a: PathBuf,
    b: PathBuf,
    c: PathBuf,
}
impl Fixture {
    fn new(name: &str) -> Self {
        let base = std::env::temp_dir().join(format!(
            "pane-source-mutation-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&base);
        let (a, b, c) = (base.join("a"), base.join("b"), base.join("c"));
        for path in [&a, &b, &c] {
            fs::create_dir_all(path).unwrap();
        }
        Self { base, a, b, c }
    }
    fn grants(&self) -> (BTreeMap<String, Vec<PathBuf>>, BTreeMap<String, String>) {
        (
            ScanPolicy::new(BTreeMap::from([
                ("claude".into(), vec![self.a.clone(), self.b.clone()]),
                ("qwen".into(), vec![self.c.clone()]),
            ]))
            .unwrap()
            .roots,
            BTreeMap::from([
                ("claude".into(), "a".repeat(32)),
                ("qwen".into(), "b".repeat(32)),
            ]),
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}
#[test]
fn partial_revocation_succeeds_with_unrelated_offline_root_and_survives_restore() {
    let f = Fixture::new("unrelated");
    let (mut roots, mut epochs) = f.grants();
    let old = epochs["claude"].clone();
    fs::remove_dir(&f.c).unwrap();
    assert!(update_source_grant(&mut roots, &mut epochs, "claude", vec![f.b.clone()]).unwrap());
    assert_eq!(roots["claude"], vec![f.b.clone()]);
    assert_ne!(epochs["claude"], old);
    assert_eq!(epochs["qwen"], "b".repeat(32));
    let saved = serde_json::to_string(&roots).unwrap();
    fs::create_dir_all(&f.c).unwrap();
    let restored = ScanPolicy::new(serde_json::from_str(&saved).unwrap()).unwrap();
    assert!(!restored.roots["claude"].contains(&f.a));
}
#[test]
fn partial_revocation_succeeds_when_retained_root_is_offline() {
    let f = Fixture::new("retained");
    let (mut roots, mut epochs) = f.grants();
    let old = epochs["claude"].clone();
    fs::remove_dir(&f.b).unwrap();
    assert!(update_source_grant(&mut roots, &mut epochs, "claude", vec![f.b.clone()]).unwrap());
    assert_eq!(roots["claude"], vec![f.b.clone()]);
    assert_eq!(epochs["claude"].len(), 32);
    assert_ne!(epochs["claude"], old);
    let saved = serde_json::to_string(&roots).unwrap();
    fs::create_dir_all(&f.b).unwrap();
    let restored = ScanPolicy::new(serde_json::from_str(&saved).unwrap()).unwrap();
    assert_eq!(restored.roots["claude"], vec![f.b.clone()]);
    assert!(!restored.roots["claude"].contains(&f.a));
}
#[test]
fn identical_and_duplicate_inputs_preserve_epoch_without_resolving_offline_roots() {
    let f = Fixture::new("noop");
    let (mut roots, mut epochs) = f.grants();
    let before = (roots.clone(), epochs.clone());
    assert!(!update_source_grant(
        &mut roots,
        &mut epochs,
        "claude",
        vec![f.a.clone(), f.b.clone()]
    )
    .unwrap());
    fs::remove_dir(&f.b).unwrap();
    fs::remove_dir(&f.c).unwrap();
    assert!(!update_source_grant(
        &mut roots,
        &mut epochs,
        "claude",
        vec![f.b.clone(), f.a.clone(), f.a.clone()]
    )
    .unwrap());
    assert_eq!((roots, epochs), before);
}
#[test]
fn whole_source_off_works_offline_but_additions_and_unrecognized_spellings_still_validate() {
    let f = Fixture::new("validation");
    let (mut roots, mut epochs) = f.grants();
    let before = (roots.clone(), epochs.clone());
    fs::remove_dir(&f.c).unwrap();
    assert!(update_source_grant(
        &mut roots,
        &mut epochs,
        "claude",
        vec![f.b.clone(), f.base.join("missing")]
    )
    .is_err());
    assert!(
        update_source_grant(&mut roots, &mut epochs, "claude", vec![f.b.join(".")]).is_err(),
        "an arbitrary spelling must not enter the exact removal-only branch"
    );
    assert_eq!((roots.clone(), epochs.clone()), before);
    assert!(update_source_grant(&mut roots, &mut epochs, "claude", Vec::new()).unwrap());
    assert!(!roots.contains_key("claude"));
    assert!(!epochs.contains_key("claude"));
    assert!(!update_source_grant(&mut roots, &mut epochs, "claude", Vec::new()).unwrap());
}
#[test]
fn actual_addition_is_canonicalized_and_rotates_epoch() {
    let f = Fixture::new("addition");
    let (mut roots, mut epochs) = f.grants();
    let old = epochs["claude"].clone();
    let d = f.base.join("d");
    fs::create_dir_all(&d).unwrap();
    assert!(update_source_grant(
        &mut roots,
        &mut epochs,
        "claude",
        vec![f.a.clone(), f.b.clone(), d.join(".")]
    )
    .unwrap());
    assert_eq!(roots["claude"].last(), Some(&d.canonicalize().unwrap()));
    assert_ne!(epochs["claude"], old);
}
