use pane_scan_tests::{providers, scan_policy::ScanPolicy, spend};
use serde_json::json;
use std::fs;
#[test]
fn legacy_and_corrupt_cache_invalidated_with_no_scan_grants_then_rebuilt_once() {
    let path = providers::config_dir().join("spend_cache.json");
    for raw in [json!({"version":4,"scan_roots":{},"scan_epochs":{},"entries":[{"prefix_head":[83,69,67,82,69,84]}]}).to_string(),"{\"prefix_head\":[83,69,67,82,69,84], CORRUPT".into()] {
        fs::write(&path,raw).unwrap();
        spend::migrate_legacy_cache().unwrap();
        assert!(!path.exists());
    }
    fs::write(
        &path,
        b"{\"version\":4,\"prefix_head\":[83,69,67,82,69,84],\xff}",
    )
    .unwrap();
    spend::migrate_legacy_cache().unwrap();
    assert!(
        !path.exists(),
        "corrupt non-UTF-8 raw-fragment cache was retained"
    );
    fs::write(
        &path,
        json!({"version":4,"entries":[{"days":[[123,"fixture",99.,999999.]],"prefix_head":[83]}]})
            .to_string(),
    )
    .unwrap();
    assert!(spend::collect(&ScanPolicy::default(), None).is_empty());
    let empty: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(empty["version"], 5);
    assert_eq!(empty["entries"], json!([]));
    let root = providers::config_dir().join("migration-logs");
    fs::create_dir_all(&root).unwrap();
    let log = root.join("session.jsonl");
    let raw=json!({"type":"assistant","timestamp":chrono::Utc::now().to_rfc3339(),"requestId":"fixture","message":{"id":"fixture","model":"claude-sonnet-4-6","usage":{"input_tokens":100.,"output_tokens":0}}}).to_string()+"\n";
    fs::write(&log, &raw).unwrap();
    let policy = ScanPolicy::new([("claude".into(), vec![root])].into_iter().collect())
        .unwrap()
        .with_epochs(
            [("claude".into(), "durable-grant".into())]
                .into_iter()
                .collect(),
        );
    for _ in 0..2 {
        assert_eq!(
            spend::collect(&policy, None)
                .iter()
                .map(|s| s.last30.tokens)
                .sum::<f64>(),
            100.
        );
    }
    let saved: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(saved["scan_epochs"]["claude"], "durable-grant");
    assert_eq!(
        saved["entries"][0]["prefix_head"]["sha256"]
            .as_str()
            .unwrap()
            .len(),
        64
    );
    assert_eq!(fs::read_to_string(log).unwrap(), raw);
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(
        spend::migrate_legacy_cache().is_err(),
        "genuine read failure was swallowed"
    );
    assert!(path.is_dir());
    fs::remove_dir(&path).unwrap();
}
