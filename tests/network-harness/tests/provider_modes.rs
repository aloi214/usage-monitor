use pane_network_tests::{
    access_policy::{AccessPolicy, AccessRuntime, AccountBinding},
    provider_modes,
};
use serde_json::json;
use std::{cell::RefCell, sync::Arc};

fn config() -> serde_json::Value {
    json!({"accessPolicy": AccessPolicy::default(), "locale": "zh"})
}

#[test]
fn catalog_uses_real_policy_choices_without_reading_or_granting_accounts() {
    let policy = AccessPolicy::default();
    let catalog = provider_modes::catalog(&policy);
    assert_eq!(catalog.len(), 7);
    for item in &catalog {
        assert_eq!(
            item.choices.len(),
            pane_network_tests::network_policy::region_choices(&item.family).len()
        );
        assert!(item.selected.is_none());
    }
    assert_eq!(
        catalog
            .iter()
            .find(|row| row.id == "antigravity")
            .unwrap()
            .default_selection,
        Some("cloud")
    );
    assert!(
        catalog
            .iter()
            .find(|row| row.id == "ollama")
            .unwrap()
            .allow_local_origin
    );
    assert!(policy.enabled_accounts.is_empty());
}

#[test]
fn disabled_account_selection_does_not_grant_any_access() {
    let change = provider_modes::prepare(&config(), "qwen", "china:x_api_key").unwrap();
    let policy = AccessPolicy::from_config(change.config());
    assert_eq!(
        policy.regions.get("qwen").map(String::as_str),
        Some("china:x_api_key")
    );
    assert!(policy.enabled_families.is_empty());
    assert!(policy.enabled_accounts.is_empty());
    assert_eq!(change.config()["locale"], "zh");
}

#[test]
fn unknown_accounts_wrong_modes_and_remote_local_origins_are_rejected() {
    for (id, mode) in [
        ("qwen@unbound1", "china:bearer"),
        ("https://api.z.ai", "china:api_key"),
        ("qwen", "global"),
        ("qwen", "china:api_key"),
        ("ollama", "local:http://192.168.1.2:11434"),
        ("ollama", "local:http://localhost:11434/v1"),
        ("ollama", "local:http://token@localhost:11434"),
        ("onenewapi", "global"),
    ] {
        assert!(
            provider_modes::prepare(&config(), id, mode).is_err(),
            "{id} {mode}"
        );
    }
    for mode in [
        "local:http://127.0.0.1:12000",
        "local:http://[::1]:12001",
        "local:http://localhost:12002",
    ] {
        assert!(provider_modes::prepare(&config(), "ollama", mode).is_ok());
    }
}

#[test]
fn known_extra_accounts_resolve_through_validated_bindings() {
    let mut p = AccessPolicy::default();
    p.account_bindings.insert(
        "codex@abc12345".into(),
        AccountBinding {
            family: "codex".into(),
            directory: std::env::temp_dir().join("synthetic-mode-account"),
            name: "Work".into(),
        },
    );
    let change =
        provider_modes::prepare(&json!({"accessPolicy": p}), "codex@abc12345", "global").unwrap();
    assert_eq!(change.family(), "codex");
    assert_eq!(change.snapshot_ids(), ["codex@abc12345"]);
    assert!(provider_modes::prepare(
        &json!({"accessPolicy": p}),
        "codex@abc12345",
        "china:api_key"
    )
    .is_err());
}

#[test]
fn successful_mode_change_clears_before_persist_and_rejects_old_or_mid_change_work() {
    let mut p = AccessPolicy::default();
    p.set_family("moonshot", true).unwrap();
    p.set_account("moonshot", true).unwrap();
    p.regions.insert("moonshot".into(), "china:api_key".into());
    let cfg = json!({"accessPolicy": p});
    let runtime = Arc::new(AccessRuntime::new(p));
    let old = runtime.permit("moonshot", &[]).unwrap();
    let steps = RefCell::new(Vec::new());
    let change = provider_modes::prepare(&cfg, "moonshot", "international:api_key").unwrap();
    assert_eq!(change.snapshot_ids(), ["moonshot", "kimi"]);
    let result = change
        .commit(
            &runtime,
            || {
                assert!(old.check().is_err());
                assert!(runtime.permit("moonshot", &[]).is_none());
                steps.borrow_mut().push("clear");
                Ok(())
            },
            |new| {
                assert_eq!(
                    new["accessPolicy"]["regions"]["moonshot"],
                    "international:api_key"
                );
                steps.borrow_mut().push("persist");
                Ok(())
            },
            || {
                steps.borrow_mut().push("success");
            },
        )
        .unwrap();
    assert_eq!(*steps.borrow(), ["clear", "persist", "success"]);
    assert_eq!(runtime.snapshot().0, AccessPolicy::from_config(&result));
    assert!(old.check().is_err());
    assert!(runtime.permit("moonshot", &[]).is_some());
}

#[test]
fn failed_cleanup_or_persistence_keeps_old_mode_and_never_calls_success() {
    for fail_cleanup in [true, false] {
        let cfg = config();
        let old = AccessPolicy::from_config(&cfg);
        let runtime = AccessRuntime::new(old.clone());
        let calls = RefCell::new(Vec::new());
        let change = provider_modes::prepare(&cfg, "minimax", "china:mcode").unwrap();
        let result = change.commit(
            &runtime,
            || {
                calls.borrow_mut().push("clear");
                if fail_cleanup {
                    Err("cleanup blocked".into())
                } else {
                    Ok(())
                }
            },
            |_| {
                calls.borrow_mut().push("persist");
                Err("write blocked".into())
            },
            || {
                calls.borrow_mut().push("success");
            },
        );
        assert!(result.is_err());
        assert_eq!(runtime.snapshot().0, old);
        assert_eq!(
            *calls.borrow(),
            if fail_cleanup {
                vec!["clear"]
            } else {
                vec!["clear", "persist"]
            }
        );
    }
}

#[test]
fn identity_cleanup_preserves_other_accounts_and_handles_bad_files_fail_closed() {
    let dir = std::env::temp_dir().join(format!("pane-mode-identities-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("cache_identities.json");
    std::fs::write(&file, r#"{"stepfun":"old-cn","codex":"other"}"#).unwrap();
    provider_modes::forget_identities_in(&dir, &["stepfun".into()]).unwrap();
    let result: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert!(result.get("stepfun").is_none());
    assert_eq!(result["codex"], "other");
    std::fs::write(&file, "broken").unwrap();
    assert!(provider_modes::forget_identities_in(&dir, &["stepfun".into()]).is_err());
    std::fs::remove_file(file).unwrap();
    provider_modes::forget_identities_in(&dir, &["stepfun".into()]).unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}
