use pane_scan_tests::{cache_fingerprint::SampleFingerprint, providers, spend};
use serde_json::{json, Value};
use std::fs;

#[test]
fn sources_off_migration_rejects_malformed_v5_fingerprints_without_logs() {
    let root = providers::config_dir();
    let path = root.join("spend_cache.json");
    let unopened = root.join("no-grant-no-log/session.jsonl");
    assert!(!unopened.exists());
    let valid = serde_json::to_value(SampleFingerprint::of(b"abc")).unwrap();
    let doc = |head: Value, tail: Value| {
        json!({
            "version":5,"scan_roots":{},"scan_epochs":{},"pricing_stamp":"builtin","corrections":16,
            "entries":[{"path":unopened,"mtime_secs":0,"mtime_nanos":0,"size":3,"days":[],"unpriced":[],
                "prefix_head":head,"prefix_tail":tail}]
        })
    };
    // Exact empty/missing sentinels and real digest objects remain readable.
    for mark in [valid.clone(), json!({"sampled_len":0,"sha256":""})] {
        let text = doc(mark.clone(), mark).to_string();
        fs::write(&path, &text).unwrap();
        spend::migrate_legacy_cache().unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), text);
    }
    let mut missing = doc(valid.clone(), valid.clone());
    missing["entries"][0]
        .as_object_mut()
        .unwrap()
        .remove("prefix_head");
    missing["entries"][0]
        .as_object_mut()
        .unwrap()
        .remove("prefix_tail");
    fs::write(&path, missing.to_string()).unwrap();
    spend::migrate_legacy_cache().unwrap();
    assert!(path.exists());
    for (field, cap) in [("prefix_head", 64), ("prefix_tail", 32)] {
        for malformed in [
            json!({"sampled_len":10,"sha256":"RAW_SYNTHETIC_LOG_FRAGMENT"}),
            json!({"sampled_len":3,"sha256":"a".repeat(63)}),
            json!({"sampled_len":3,"sha256":"g".repeat(64)}),
            json!({"sampled_len":0,"sha256":valid["sha256"]}),
            json!({"sampled_len":3,"sha256":""}),
            json!({"sampled_len":cap+1,"sha256":valid["sha256"]}),
            json!({"sampled_len":-1,"sha256":valid["sha256"]}),
        ] {
            let mut corrupted = doc(valid.clone(), valid.clone());
            corrupted["entries"][0][field] = malformed;
            fs::write(&path, corrupted.to_string()).unwrap();
            spend::migrate_legacy_cache().unwrap();
            assert!(
                !path.exists(),
                "malformed {field} retained with every source off: {corrupted}"
            );
            assert!(!unopened.exists());
        }
    }
}
