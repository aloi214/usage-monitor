use pane_scan_tests::{providers, scan_policy::ScanPolicy, spend};
use serde_json::json;
#[test]
fn unknown_models_keep_their_flag_when_folded_or_name_list_is_capped() {
    let root = providers::config_dir().join("unknown-model-flags");
    std::fs::create_dir_all(&root).unwrap();
    let now = chrono::Utc::now().to_rfc3339();
    let lines=(0..8).map(|n|json!({"type":"assistant","timestamp":now,"requestId":format!("request-{n}"),"message":{"id":format!("message-{n}"),"model":format!("unknown-flag-{n}"),"usage":{"input_tokens":100.,"output_tokens":0.}}}).to_string()).collect::<Vec<_>>().join("\n")+"\n";
    std::fs::write(root.join("session.jsonl"), &lines).unwrap();
    let policy = ScanPolicy::new(
        [("claude".into(), vec![root.clone()])]
            .into_iter()
            .collect(),
    )
    .unwrap();
    for _ in 0..2 {
        let rows = spend::collect(&policy, None);
        let row = &rows[0];
        assert_eq!(row.unpriced, 8);
        assert_eq!(row.unpriced_models.len(), 5);
        assert_eq!(row.last30.tokens, 800.);
        let doc = serde_json::to_value(row).unwrap();
        let models = doc["today"]["models"].as_array().unwrap();
        assert!(models.iter().any(|m| m["model"] == "Other"));
        assert!(
            models.iter().all(|m| m["unpriced"] == true),
            "unknown flag lost in aggregate: {models:?}"
        );
    }
    assert_eq!(
        std::fs::read_to_string(root.join("session.jsonl")).unwrap(),
        lines
    );
}
