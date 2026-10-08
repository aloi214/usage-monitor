use pane_scan_tests::scan_policy::{self, ScanPolicy};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
struct Fixture(PathBuf);
impl Fixture {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("pane-scan-policy-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn policy(&self) -> ScanPolicy {
        ScanPolicy::new(BTreeMap::from([("claude".into(), vec![self.0.clone()])])).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn disabled_source_is_not_opened() {
    let f = Fixture::new("disabled");
    let file = f.0.join("session.jsonl");
    fs::write(&file, "{}\n").unwrap();
    ScanPolicy::default().scope("claude", || assert!(scan_policy::open_log(&file).is_err()));
    assert!(
        scan_policy::open_log(&file).is_err(),
        "missing context denies access"
    );
}
#[test]
fn scan_never_reads_auth_json() {
    let f = Fixture::new("auth");
    let auth = f.0.join("auth.json");
    fs::write(&auth, "fake credential").unwrap();
    f.policy()
        .scope("claude", || assert!(scan_policy::open_log(&auth).is_err()));
    #[cfg(unix)]
    {
        let alias = f.0.join("session.jsonl");
        std::os::unix::fs::symlink(&auth, &alias).unwrap();
        f.policy()
            .scope("claude", || assert!(scan_policy::open_log(&alias).is_err()));
    }
}
#[test]
#[cfg(unix)]
fn symlink_escape_is_rejected() {
    let f = Fixture::new("inside");
    let outside = Fixture::new("outside");
    let file = outside.0.join("secret.jsonl");
    fs::write(&file, "secret").unwrap();
    let alias = f.0.join("escape.jsonl");
    std::os::unix::fs::symlink(&file, &alias).unwrap();
    let dir = f.0.join("escape-dir");
    std::os::unix::fs::symlink(&outside.0, &dir).unwrap();
    let policy = f.policy();
    assert!(!policy.allows_path("claude", &alias));
    assert!(!policy.allows_path("claude", &dir));
    policy.scope("claude", || assert!(scan_policy::open_log(&alias).is_err()));
}
#[test]
fn revocation_stops_further_reads() {
    let f = Fixture::new("revocation");
    let file = f.0.join("session.jsonl");
    fs::write(&file, "{}\n").unwrap();
    let live = Arc::new(AtomicBool::new(true));
    let check = live.clone();
    let policy = f.policy().with_check(move || check.load(Ordering::SeqCst));
    policy.scope("claude", || {
        assert!(scan_policy::open_log(&file).is_ok());
        live.store(false, Ordering::SeqCst);
        assert!(scan_policy::open_log(&file).is_err());
    });
    assert!(!policy.is_current());
}
#[test]
fn invalid_roots_fail_closed() {
    assert!(ScanPolicy::new(BTreeMap::from([(
        "claude".into(),
        vec![PathBuf::from("relative")]
    )]))
    .is_err());
    assert!(ScanPolicy::new(BTreeMap::from([(
        "oneapi".into(),
        vec![PathBuf::from("/tmp")]
    )]))
    .is_err());
}
#[test]
#[cfg(unix)]
fn sqlite_sidecar_escape_is_rejected() {
    let f = Fixture::new("db");
    let outside = Fixture::new("wal");
    let db = f.0.join("opencode.db");
    fs::write(&db, "fake database").unwrap();
    let wal = outside.0.join("private");
    fs::write(&wal, "outside").unwrap();
    std::os::unix::fs::symlink(&wal, f.0.join("opencode.db-wal")).unwrap();
    let policy = ScanPolicy::new(BTreeMap::from([("opencode".into(), vec![f.0.clone()])])).unwrap();
    policy.scope("opencode", || {
        assert!(scan_policy::checked_sqlite(&db).is_err())
    });
}
#[test]
fn conflicting_sources_cannot_reparse_one_tree_with_different_codecs() {
    let f = Fixture::new("conflicting");
    fs::create_dir_all(f.0.join("nested")).unwrap();
    assert!(ScanPolicy::new(BTreeMap::from([
        ("claude".into(), vec![f.0.clone()]),
        ("pi".into(), vec![f.0.join("nested")]),
    ]))
    .is_err());
}
#[test]
#[cfg(unix)]
fn sqlite_alias_cannot_open_an_auth_file_inside_root() {
    let f = Fixture::new("db-auth");
    fs::write(f.0.join("auth.json"), "fake credentials").unwrap();
    let db = f.0.join("opencode.db");
    std::os::unix::fs::symlink(f.0.join("auth.json"), &db).unwrap();
    let p = ScanPolicy::new(BTreeMap::from([("opencode".into(), vec![f.0.clone()])])).unwrap();
    p.scope("opencode", || {
        assert!(scan_policy::checked_sqlite(&db).is_err())
    });
}
#[test]
#[cfg(unix)]
fn sqlite_implicit_files_cannot_alias_credentials_inside_root() {
    let f = Fixture::new("sidecar-auth");
    fs::write(f.0.join("opencode.db"), "synthetic db").unwrap();
    fs::write(f.0.join("auth.json"), "synthetic secret").unwrap();
    let p = ScanPolicy::new(BTreeMap::from([("opencode".into(), vec![f.0.clone()])])).unwrap();
    for suffix in ["-wal", "-shm", "-journal"] {
        let sidecar = f.0.join(format!("opencode.db{suffix}"));
        std::os::unix::fs::symlink(f.0.join("auth.json"), &sidecar).unwrap();
        p.scope("opencode", || {
            assert!(
                scan_policy::checked_sqlite(&f.0.join("opencode.db")).is_err(),
                "{suffix} cannot alias credentials"
            )
        });
        fs::remove_file(sidecar).unwrap();
    }
}
