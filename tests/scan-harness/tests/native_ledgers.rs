use pane_scan_tests::{providers, scan_policy::ScanPolicy, spend};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
struct Fixture(PathBuf);
impl Fixture {
    fn new(name: &str) -> Self {
        let p =
            std::env::temp_dir().join(format!("pane-native-ledger-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn policy(source: &str, root: &Path, rev: u64) -> ScanPolicy {
    ScanPolicy::new(BTreeMap::from([(source.into(), vec![root.into()])]))
        .unwrap()
        .with_revision(rev)
}
fn make(source: &str, root: &Path) -> PathBuf {
    let file = root.join(match source {
        "minimax" => "sqlite.db",
        "opencode" => "opencode.db",
        _ => "sessions.db",
    });
    let db = rusqlite::Connection::open(&file).unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    match source {
        "minimax" => {
            db.execute_batch("CREATE TABLE token_usage(ts INTEGER,model TEXT,input_tokens REAL,output_tokens REAL,reasoning_tokens REAL,cache_read_tokens REAL,cache_write_tokens REAL,cost_usd REAL);").unwrap();
            db.execute(
                "INSERT INTO token_usage VALUES (?1,'MiniMax-M3',10,5,0,0,0,1)",
                [now],
            )
            .unwrap();
        }
        "opencode" => {
            db.execute_batch("CREATE TABLE message(time_created INTEGER,data TEXT);")
                .unwrap();
            let data=serde_json::json!({"role":"assistant","providerID":"opencode-go","modelID":"test","cost":1,"tokens":{"input":10,"output":5}}).to_string();
            db.execute(
                "INSERT INTO message VALUES (?1,?2)",
                rusqlite::params![now, data],
            )
            .unwrap();
        }
        _ => {
            db.execute_batch("CREATE TABLE sessions(id TEXT PRIMARY KEY,model TEXT); CREATE TABLE message_nodes(row_id INTEGER PRIMARY KEY AUTOINCREMENT,session_id TEXT,chat_message TEXT,created_at INTEGER); CREATE TABLE refinery_schema_history(version INTEGER,name TEXT,applied_on TEXT,checksum TEXT); INSERT INTO refinery_schema_history VALUES(1,'first','synthetic-lineage',''); INSERT INTO sessions VALUES('s','claude-sonnet-4');").unwrap();
            let data=serde_json::json!({"role":"assistant","message_id":"m","metadata":{"metrics":{"input_tokens":10,"output_tokens":5}}}).to_string();
            db.execute(
                "INSERT INTO message_nodes(session_id,chat_message,created_at) VALUES('s',?1,?2)",
                rusqlite::params![data, now / 1000],
            )
            .unwrap();
        }
    }
    file
}
fn read(source: &str, file: &Path) -> usize {
    match source {
        "minimax" => providers::minimax::collect_usage_events_in(file).len(),
        "opencode" => providers::opencode::collect_cost_events_in(file.parent().unwrap()).len(),
        _ => providers::devin::collect_usage_events_in(file).len(),
    }
}
#[test]
fn every_native_adapter_requires_its_exact_source_and_new_grant() {
    for source in ["minimax", "opencode", "devin"] {
        let f = Fixture::new(source);
        let file = make(source, &f.0);
        assert_eq!(read(source, &file), 0);
        assert_eq!(
            policy("claude", &f.0, 20).scope("claude", || read(source, &file)),
            0
        );
        assert_eq!(
            policy(source, &f.0, 21).scope(source, || read(source, &file)),
            1
        );
        fs::write(&file, "corrupt replacement").unwrap();
        assert_eq!(
            policy(source, &f.0, 22).scope(source, || read(source, &file)),
            0,
            "{source} regrant must not restore revoked rows"
        );
    }
}
#[test]
fn devin_regrant_does_not_restore_deleted_rows_from_persistent_cache() {
    let f = Fixture::new("devin-deleted");
    let file = make("devin", &f.0);
    assert_eq!(
        policy("devin", &f.0, 30)
            .with_epochs(BTreeMap::from([("devin".into(), "a".repeat(32))]))
            .scope("devin", || read("devin", &file)),
        1
    );
    let db = rusqlite::Connection::open(&file).unwrap();
    db.execute("INSERT INTO message_nodes(session_id,chat_message,created_at) SELECT session_id,replace(chat_message,'\"m\"','\"m2\"'),created_at FROM message_nodes",[]).unwrap();
    drop(db);
    assert_eq!(
        policy("devin", &f.0, 30)
            .with_epochs(BTreeMap::from([("devin".into(), "a".repeat(32))]))
            .scope("devin", || read("devin", &file)),
        2
    );
    let db = rusqlite::Connection::open(&file).unwrap();
    db.execute("DELETE FROM message_nodes", []).unwrap();
    drop(db);
    // A process-restart-like runtime revision with the same durable grant can
    // retain within-grant history. A new grant epoch must start from the DB.
    assert_eq!(
        policy("devin", &f.0, 0)
            .with_epochs(BTreeMap::from([("devin".into(), "a".repeat(32))]))
            .scope("devin", || read("devin", &file)),
        2
    );
    assert_eq!(
        policy("devin", &f.0, 31)
            .with_epochs(BTreeMap::from([("devin".into(), "b".repeat(32))]))
            .scope("devin", || read("devin", &file)),
        0
    );
}
#[test]
fn opencode_local_windows_remain_source_details() {
    let f = Fixture::new("opencode-details");
    make("opencode", &f.0);
    let rows = spend::collect(&policy("opencode", &f.0, 40), None);
    let source = rows.iter().find(|r| r.id == "source:opencode").unwrap();
    assert_eq!(source.last30.tokens, 15.);
    assert!(source
        .local_stats
        .iter()
        .any(|m| m.label.contains("local estimate")));
}
