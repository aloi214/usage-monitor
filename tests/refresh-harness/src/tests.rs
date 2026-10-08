use crate::{credential_refresh::*, private_file::PrivateStage};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let mut bytes = [0; 16];
        getrandom::getrandom(&mut bytes).unwrap();
        let p = std::env::temp_dir().join(format!(
            "pane-refresh-fixture-{:x}",
            u128::from_le_bytes(bytes)
        ));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn file(&self) -> PathBuf {
        let p = self.0.join("fake.json");
        std::fs::write(&p,json!({"access_token":"old","refresh_token":self.0.to_string_lossy(),"expires_at":0,"unknown":{"keep":true}}).to_string()).unwrap();
        p
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn allow() -> Result<(), String> {
    Ok(())
}
fn unchanged(_: &Value) -> Result<(), String> {
    Ok(())
}
fn reply() -> Value {
    json!({"access_token":"fresh","refresh_token":"new-fake","expires_in":3600})
}
#[tokio::test]
async fn concurrent_refresh_calls_provider_once() {
    let f = Fixture::new();
    let p = f.file();
    let calls = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let p1 = p.clone();
    let c = calls.clone();
    let n = entered.clone();
    let gate = release.clone();
    let first = tokio::spawn(async move {
        refresh_file(
            &p1,
            Flavor::Kimi,
            Reason::Expiry,
            allow,
            unchanged,
            |_| async move {
                c.fetch_add(1, Ordering::SeqCst);
                n.notify_one();
                gate.notified().await;
                Ok(reply())
            },
        )
        .await
        .unwrap()
    });
    entered.notified().await;
    let queued = Arc::new(tokio::sync::Notify::new());
    let q = queued.clone();
    let c = calls.clone();
    let second = tokio::spawn(async move {
        q.notify_one();
        refresh_file(
            &p,
            Flavor::Kimi,
            Reason::Expiry,
            allow,
            unchanged,
            |_| async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(reply())
            },
        )
        .await
        .unwrap()
    });
    queued.notified().await;
    release.notify_one();
    let a = first.await.unwrap();
    let b = second.await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(a.doc["access_token"], "fresh");
    assert_eq!(b.doc["access_token"], "fresh");
    assert_eq!(std::fs::read_dir(&f.0).unwrap().count(), 1);
}
#[tokio::test]
async fn external_rotation_wins() {
    let f = Fixture::new();
    let p = f.file();
    let p2 = p.clone();
    let external = json!({"access_token":"external","refresh_token":"external-new","expires_at":4102444800_i64,"unknown":7});
    let bytes = external.to_string();
    let r = refresh_file(
        &p,
        Flavor::Kimi,
        Reason::Expiry,
        allow,
        unchanged,
        |_| async move {
            std::fs::write(p2, &bytes).unwrap();
            Ok(reply())
        },
    )
    .await
    .unwrap();
    assert_eq!(r.outcome, Some(CommitOutcome::ChangedExternally));
    assert_eq!(r.doc, external);
    assert_eq!(
        serde_json::from_slice::<Value>(&std::fs::read(&p).unwrap()).unwrap(),
        external
    );
}
#[test]
fn unique_temp_files_do_not_collide() {
    let f = Fixture::new();
    let p = f.file();
    let planted = p.with_extension("json.tmp");
    std::fs::write(&planted, "unrelated").unwrap();
    let mut a = PrivateStage::new(&p).unwrap();
    let mut b = PrivateStage::new(&p).unwrap();
    assert_ne!(a.path(), b.path());
    a.write(b"a").unwrap();
    b.write(b"b").unwrap();
    drop(a);
    assert_eq!(std::fs::read(b.path()).unwrap(), b"b");
    assert_eq!(std::fs::read(planted).unwrap(), b"unrelated");
}
#[tokio::test]
async fn unknown_json_fields_survive() {
    let f = Fixture::new();
    let p = f.file();
    let r = refresh_file(
        &p,
        Flavor::Kimi,
        Reason::Expiry,
        allow,
        unchanged,
        |_| async { Ok(reply()) },
    )
    .await
    .unwrap();
    assert_eq!(r.doc["unknown"], json!({"keep":true}));
    assert_eq!(r.doc["access_token"], "fresh");
}
#[tokio::test]
async fn remote_success_local_failure_stops_retry() {
    let f = Fixture::new();
    let p = f.file();
    let p2 = p.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let r = refresh_file(
        &p,
        Flavor::Kimi,
        Reason::Expiry,
        allow,
        unchanged,
        |_| async move {
            c.fetch_add(1, Ordering::SeqCst);
            for e in std::fs::read_dir(p2.parent().unwrap()).unwrap() {
                let e = e.unwrap();
                if e.file_name().to_string_lossy().ends_with(".tmp") {
                    std::fs::remove_file(e.path()).unwrap();
                }
            }
            Ok(reply())
        },
    )
    .await;
    assert!(r.unwrap_err().contains("refresh succeeded, save failed"));
    let r = refresh_file(
        &p,
        Flavor::Kimi,
        Reason::Expiry,
        allow,
        unchanged,
        |_| async {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(reply())
        },
    )
    .await;
    assert!(r.unwrap_err().contains("refresh succeeded, save failed"));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn cancelled_refresh_cannot_delete_other_temp() {
    let f = Fixture::new();
    let p = f.file();
    let entered = Arc::new(tokio::sync::Notify::new());
    let n = entered.clone();
    let p2 = p.clone();
    let task = tokio::spawn(async move {
        refresh_file(
            &p2,
            Flavor::Kimi,
            Reason::Expiry,
            allow,
            unchanged,
            |_| async move {
                n.notify_one();
                std::future::pending::<Result<Value, RemoteError>>().await
            },
        )
        .await
    });
    entered.notified().await;
    let mut b = PrivateStage::new(&p).unwrap();
    b.write(b"other stage").unwrap();
    task.abort();
    let _ = task.await;
    assert_eq!(std::fs::read(b.path()).unwrap(), b"other stage");
    assert_eq!(std::fs::read_dir(&f.0).unwrap().count(), 2);
    let r = refresh_file(
        &p,
        Flavor::Kimi,
        Reason::Expiry,
        allow,
        unchanged,
        |_| async { panic!("must not retry cancelled rotation") },
    )
    .await;
    assert!(r.is_err());
}
fn jwt(sub: &str, account: Option<&str>) -> String {
    use base64::Engine;
    let mut v = json!({"sub":sub,"exp":4102444800_i64});
    if let Some(a) = account {
        v["https://api.openai.com/auth"] = json!({"chatgpt_account_id":a});
    }
    format!(
        "fake.{}.sig",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(v.to_string())
    )
}
#[tokio::test]
async fn account_switch_invalidates_memory_and_csv() {
    let f = Fixture::new();
    let key = f.0.to_string_lossy().to_string();
    let a = jwt("provider|A", None);
    let b = jwt("provider|B", None);
    let first = memory_access(
        &key,
        || Ok((a.clone(), "refresh-A".into())),
        None,
        allow,
        |_| async { panic!("no refresh needed") },
    )
    .await
    .unwrap();
    put_memory_csv(&key, first.source, 1, "account A rows".into(), allow).unwrap();
    let second = memory_access(
        &key,
        || Ok((b.clone(), "refresh-B".into())),
        None,
        allow,
        |_| async { panic!("no refresh needed") },
    )
    .await
    .unwrap();
    assert_eq!(second.access, b);
    assert!(memory_csv(&key, second.source).is_none());
    assert!(memory_csv(&key, first.source).is_none());
}
#[tokio::test]
async fn returned_identity_mismatch_never_commits() {
    let f = Fixture::new();
    let p = f.file();
    let old = json!({"tokens":{"access_token":"expired","refresh_token":f.0.to_string_lossy(),"account_id":"account-A","id_token":jwt("sub-A",Some("account-A"))}});
    std::fs::write(&p, old.to_string()).unwrap();
    let r=refresh_file(&p,Flavor::Codex,Reason::Expiry,allow,unchanged,|_|async{Ok(json!({"access_token":jwt("sub-B",Some("account-B")),"id_token":jwt("sub-B",Some("account-B"))}))}).await;
    assert!(r.is_err());
    assert_eq!(
        serde_json::from_slice::<Value>(&std::fs::read(&p).unwrap()).unwrap(),
        old
    );
    let key = f.0.join("memory").to_string_lossy().to_string();
    let token = jwt("provider|A", None);
    let r = memory_access(
        &key,
        || Ok((token.clone(), format!("{}-memory", key))),
        Some(credential_version(token.as_bytes())),
        allow,
        |_| async { Ok(json!({"access_token":jwt("provider|B",None)})) },
    )
    .await;
    assert!(r.is_err());
}
#[tokio::test]
async fn antigravity_cache_is_bound_to_source() {
    let f = Fixture::new();
    let path = f.0.join("cache.json");
    let key = format!("fake-keyring:{}", f.0.display());
    let a = credential_version(b"source-A");
    let b = credential_version(b"source-B");
    std::fs::write(
        &path,
        json!({"accessToken":"previous-account","expiresAtMs":4102444800000_i64,"unknown":7})
            .to_string(),
    )
    .unwrap();
    let read = || {
        Ok(CachedSource {
            version: a,
            access: "expired".into(),
            refresh: Some(key.clone()),
            expires: 0,
        })
    };
    let first = cached_access(&key, &path, read, None, allow, |_| async {
        Ok(json!({"access_token":"fresh","expires_in":3600}))
    })
    .await
    .unwrap();
    assert_eq!(first.access, "fresh");
    let doc: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(doc["unknown"], 7);
    let second = cached_access(
        &key,
        &path,
        || {
            Ok(CachedSource {
                version: b,
                access: "account-B".into(),
                refresh: None,
                expires: 4102444800000,
            })
        },
        None,
        allow,
        |_| async { panic!("valid source") },
    )
    .await
    .unwrap();
    assert_eq!(second.access, "account-B");
}
#[tokio::test]
async fn absent_cache_external_creation_wins() {
    let f = Fixture::new();
    let path = f.0.join("cache.json");
    let p2 = path.clone();
    let key = format!("fake-keyring:{}", f.0.display());
    let r = cached_access(
        &key,
        &path,
        || {
            Ok(CachedSource {
                version: credential_version(b"one"),
                access: "expired".into(),
                refresh: Some(key.clone()),
                expires: 0,
            })
        },
        None,
        allow,
        |_| async move {
            std::fs::write(p2, b"external creation").unwrap();
            Ok(json!({"access_token":"fresh"}))
        },
    )
    .await;
    assert!(r.is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"external creation");
}
#[tokio::test]
async fn disabled_during_refresh_prevents_commit_and_retry() {
    use std::sync::atomic::AtomicBool;
    let f = Fixture::new();
    let p = f.file();
    let enabled = Arc::new(AtomicBool::new(true));
    let e = enabled.clone();
    let check = || {
        if enabled.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err("revoked".into())
        }
    };
    let result = refresh_file(
        &p,
        Flavor::Kimi,
        Reason::Expiry,
        check,
        unchanged,
        |_| async move {
            e.store(false, Ordering::SeqCst);
            Ok(reply())
        },
    )
    .await;
    assert!(result.is_err());
    let old: Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    assert_eq!(old["access_token"], "old");
    enabled.store(true, Ordering::SeqCst);
    assert!(refresh_file(
        &p,
        Flavor::Kimi,
        Reason::Expiry,
        check,
        unchanged,
        |_| async { panic!("late retry after revocation") }
    )
    .await
    .is_err());
}
#[tokio::test]
async fn disabled_while_waiting_prevents_late_send() {
    use std::sync::atomic::AtomicBool;
    let f = Fixture::new();
    let p = f.file();
    let key = normalized_source(&p).unwrap().to_string_lossy().to_string();
    let guard = lock_source(&key).await;
    let entered = Arc::new(tokio::sync::Notify::new());
    let n = entered.clone();
    let enabled = Arc::new(AtomicBool::new(true));
    let e = enabled.clone();
    let task = tokio::spawn(async move {
        refresh_file(
            &p,
            Flavor::Kimi,
            Reason::Expiry,
            || {
                n.notify_one();
                if e.load(Ordering::SeqCst) {
                    Ok(())
                } else {
                    Err("revoked".into())
                }
            },
            unchanged,
            |_| async { panic!("disabled request sent") },
        )
        .await
    });
    entered.notified().await;
    enabled.store(false, Ordering::SeqCst);
    drop(guard);
    assert!(task.await.unwrap().is_err());
}
#[tokio::test]
async fn rejection_rereads_external_rotation() {
    let f = Fixture::new();
    let p = f.file();
    let p2 = p.clone();
    let r=refresh_file(&p,Flavor::Kimi,Reason::Expiry,allow,unchanged,|_|async move{std::fs::write(p2,json!({"access_token":"external","refresh_token":"external","expires_at":4102444800_i64}).to_string()).unwrap();Err(RemoteError::Rejected)}).await.unwrap();
    assert_eq!(r.outcome, Some(CommitOutcome::ChangedExternally));
    assert_eq!(r.doc["access_token"], "external");
}
#[tokio::test]
async fn rejected_generation_reuses_newer_token() {
    let f = Fixture::new();
    let p = f.file();
    let old = credential_version(b"old");
    let calls = AtomicUsize::new(0);
    let first = refresh_file(
        &p,
        Flavor::Kimi,
        Reason::Rejected(old),
        allow,
        unchanged,
        |_| async {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(reply())
        },
    )
    .await
    .unwrap();
    let second = refresh_file(
        &p,
        Flavor::Kimi,
        Reason::Rejected(old),
        allow,
        unchanged,
        |_| async { panic!("duplicate forced refresh") },
    )
    .await
    .unwrap();
    assert_eq!(first.doc, second.doc);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn nonrotating_tokens_remain_reusable() {
    for omit in [true, false] {
        let f = Fixture::new();
        let p = f.file();
        let refresh = f.0.to_string_lossy().to_string();
        let first = refresh_file(
            &p,
            Flavor::Kimi,
            Reason::Expiry,
            allow,
            unchanged,
            |_| async {
                let mut v = json!({"access_token":"first"});
                if !omit {
                    v["refresh_token"] = Value::from(refresh.clone());
                }
                Ok(v)
            },
        )
        .await
        .unwrap();
        assert_eq!(first.doc["refresh_token"], refresh);
        let second = refresh_file(
            &p,
            Flavor::Kimi,
            Reason::Rejected(credential_version(b"first")),
            allow,
            unchanged,
            |_| async { Ok(json!({"access_token":"second"})) },
        )
        .await
        .unwrap();
        assert_eq!(second.doc["access_token"], "second");
    }
}
#[tokio::test]
async fn copied_rotating_refresh_token_is_not_redeemed_again() {
    let f = Fixture::new();
    let p = f.file();
    let copy = f.0.join("copied.json");
    std::fs::copy(&p, &copy).unwrap();
    refresh_file(
        &p,
        Flavor::Kimi,
        Reason::Expiry,
        allow,
        unchanged,
        |_| async { Ok(reply()) },
    )
    .await
    .unwrap();
    let r = refresh_file(
        &copy,
        Flavor::Kimi,
        Reason::Expiry,
        allow,
        unchanged,
        |_| async { panic!("copied refresh consumed twice") },
    )
    .await;
    assert!(r.is_err());
}
#[tokio::test]
async fn malformed_success_stops_retry() {
    let f = Fixture::new();
    let p = f.file();
    let r = refresh_file(
        &p,
        Flavor::Kimi,
        Reason::Expiry,
        allow,
        unchanged,
        |_| async { Err(RemoteError::SucceededInvalid) },
    )
    .await;
    assert!(r.unwrap_err().contains("refresh succeeded, save failed"));
    assert!(refresh_file(
        &p,
        Flavor::Kimi,
        Reason::Expiry,
        allow,
        unchanged,
        |_| async { panic!("malformed successful response retried") }
    )
    .await
    .is_err());
}
#[tokio::test]
async fn missing_objects_never_send() {
    for (flavor, doc) in [
        (
            Flavor::Claude { account: None },
            json!({"claudeAiOauth":[]}),
        ),
        (Flavor::Codex, json!({"tokens":null})),
        (Flavor::Kimi, json!([])),
        (
            Flavor::Grok {
                entry: "chosen".into(),
            },
            json!({"other":{}}),
        ),
    ] {
        let f = Fixture::new();
        let p = f.file();
        std::fs::write(&p, doc.to_string()).unwrap();
        assert!(
            refresh_file(&p, flavor, Reason::Expiry, allow, unchanged, |_| async {
                panic!("malformed document sent")
            })
            .await
            .is_err()
        );
    }
}
#[test]
fn all_provider_patchers_preserve_unknown_fields() {
    let fixtures = [
        (
            Flavor::Claude {
                account: Some("A".into()),
            },
            json!({"root":1,"claudeAiOauth":{"unknown":2,"refreshToken":"r"}}),
            "/claudeAiOauth/unknown",
        ),
        (
            Flavor::Codex,
            json!({"root":1,"tokens":{"unknown":2,"account_id":"A","refresh_token":"r"}}),
            "/tokens/unknown",
        ),
        (
            Flavor::Kimi,
            json!({"root":1,"unknown":2,"refresh_token":"r"}),
            "/unknown",
        ),
        (
            Flavor::Grok {
                entry: "issuer::A".into(),
            },
            json!({"root":1,"issuer::A":{"unknown":2,"refresh_token":"r"},"issuer::B":{"untouched":7}}),
            "/issuer::A/unknown",
        ),
    ];
    for (flavor, old, pointer) in fixtures {
        let mut response = reply();
        response["access_token"] = Value::from(jwt("A", Some("A")));
        let new = flavor.patch(old.clone(), &response).unwrap();
        assert_eq!(new["root"], 1);
        assert_eq!(new.pointer(pointer), Some(&json!(2)));
        if old.get("issuer::B").is_some() {
            assert_eq!(new["issuer::B"], old["issuer::B"]);
        }
    }
}
#[test]
fn replacing_owned_temp_never_redirects_write_or_cleanup() {
    let f = Fixture::new();
    let p = f.file();
    let mut stage = PrivateStage::new(&p).unwrap();
    let tmp = stage.path().to_path_buf();
    std::fs::remove_file(&tmp).unwrap();
    std::fs::write(&tmp, b"other owner").unwrap();
    assert!(stage.write(b"secret fake payload").is_err());
    drop(stage);
    assert_eq!(std::fs::read(&tmp).unwrap(), b"other owner");
}
#[tokio::test]
async fn source_deletion_is_not_recreated() {
    let f = Fixture::new();
    let p = f.file();
    let p2 = p.clone();
    assert!(refresh_file(
        &p,
        Flavor::Kimi,
        Reason::Expiry,
        allow,
        unchanged,
        |_| async move {
            std::fs::remove_file(p2).unwrap();
            Ok(reply())
        }
    )
    .await
    .is_err());
    assert!(!p.exists());
}
#[tokio::test]
async fn normalized_alias_calls_coalesce() {
    let f = Fixture::new();
    let p = f.file();
    let alias = f.0.join(".").join("fake.json");
    assert_eq!(
        normalized_source(&p).unwrap(),
        normalized_source(&alias).unwrap()
    );
    let first = refresh_file(
        &p,
        Flavor::Kimi,
        Reason::Expiry,
        allow,
        unchanged,
        |_| async { Ok(reply()) },
    )
    .await
    .unwrap();
    let second = refresh_file(
        &alias,
        Flavor::Kimi,
        Reason::Expiry,
        allow,
        unchanged,
        |_| async { panic!("alias repeated refresh") },
    )
    .await
    .unwrap();
    assert_eq!(first.doc, second.doc);
}

#[tokio::test]
async fn source_update_recovers_after_save_failure() {
    let f = Fixture::new();
    let p = f.file();
    let p2 = p.clone();
    assert!(refresh_file(
        &p,
        Flavor::Kimi,
        Reason::Expiry,
        allow,
        unchanged,
        |_| async move {
            for e in std::fs::read_dir(p2.parent().unwrap()).unwrap() {
                let e = e.unwrap();
                if e.file_name().to_string_lossy().ends_with(".tmp") {
                    std::fs::remove_file(e.path()).unwrap();
                }
            }
            Ok(reply())
        }
    )
    .await
    .is_err());
    std::fs::write(&p,json!({"access_token":"expired-new-source","refresh_token":format!("{}-relogin",f.0.display()),"expires_at":0}).to_string()).unwrap();
    let r = refresh_file(
        &p,
        Flavor::Kimi,
        Reason::Expiry,
        allow,
        unchanged,
        |_| async { Ok(reply()) },
    )
    .await
    .unwrap();
    assert_eq!(r.doc["access_token"], "fresh");
}
#[test]
fn symlink_and_hardlink_sources_are_rejected() {
    #[cfg(unix)]
    {
        let f = Fixture::new();
        let p = f.file();
        let link = f.0.join("link");
        std::os::unix::fs::symlink(&p, &link).unwrap();
        assert!(normalized_source(&link).is_err());
        let hard = f.0.join("hard");
        std::fs::hard_link(&p, &hard).unwrap();
        assert!(normalized_source(&hard).is_err());
        assert!(read_bytes(&p, 65536).is_err());
    }
}
#[test]
fn final_pre_replace_checkpoint_prevents_commit() {
    let f = Fixture::new();
    let p = f.file();
    let before = std::fs::read(&p).unwrap();
    let mut stage = PrivateStage::new(&p).unwrap();
    stage.write(b"new fake pair").unwrap();
    assert!(stage
        .commit_checked(&p, || Err("revoked immediately before replacement".into()))
        .is_err());
    assert_eq!(std::fs::read(&p).unwrap(), before);
    assert_eq!(std::fs::read_dir(&f.0).unwrap().count(), 1);
}
#[tokio::test]
async fn cursor_rejects_expired_returned_token() {
    use base64::Engine;
    let f = Fixture::new();
    let key = f.0.to_string_lossy().to_string();
    let original = jwt("provider|A", None);
    let expired = format!(
        "fake.{}.sig",
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(json!({"sub":"provider|A","exp":1}).to_string())
    );
    let result = memory_access(
        &key,
        || Ok((original.clone(), key.clone())),
        Some(credential_version(original.as_bytes())),
        allow,
        |_| async { Ok(json!({"access_token":expired})) },
    )
    .await;
    assert!(
        result.is_err(),
        "expired returned token must not become the effective token"
    );
}

#[tokio::test]
async fn review_cursor_external_nonrotating_success_remains_refreshable() {
    let f = Fixture::new();
    let key = format!("{}-cursor-review", f.0.display());
    let refresh = format!("{key}-refresh");
    let old = jwt("provider|review", None);
    let external = format!("{old}-external");
    let latest = format!("{old}-latest");
    let source = std::sync::Mutex::new((old.clone(), refresh.clone()));
    let result = memory_access(
        &key,
        || Ok(source.lock().unwrap().clone()),
        Some(credential_version(old.as_bytes())),
        allow,
        |_| async {
            *source.lock().unwrap() = (external.clone(), refresh.clone());
            Ok(json!({"access_token":latest}))
        },
    )
    .await
    .unwrap();
    assert_eq!(result.access, external);
    let second = memory_access(
        &key,
        || Ok(source.lock().unwrap().clone()),
        Some(credential_version(external.as_bytes())),
        allow,
        |_| async { Ok(json!({"access_token":latest})) },
    )
    .await;
    assert!(
        second.is_ok(),
        "successful nonrotating exchange poisoned future refresh: {}",
        second.err().unwrap_or_default()
    );
}
#[tokio::test]
async fn review_preflight_denial_does_not_poison_unspent_refresh() {
    let f = Fixture::new();
    let p = f.file();
    let checks = AtomicUsize::new(0);
    let first = refresh_file(
        &p,
        Flavor::Kimi,
        Reason::Expiry,
        || {
            if checks.fetch_add(1, Ordering::SeqCst) == 4 {
                Err("preflight denied".into())
            } else {
                Ok(())
            }
        },
        unchanged,
        |_| async { panic!("denied before any request") },
    )
    .await;
    assert!(first.is_err());
    let second = refresh_file(
        &p,
        Flavor::Kimi,
        Reason::Expiry,
        allow,
        unchanged,
        |_| async { Ok(reply()) },
    )
    .await;
    assert!(
        second.is_ok(),
        "no request was made but refresh is poisoned: {:?}",
        second
    );
}
#[tokio::test]
async fn reservation_drop_is_unspent_but_started_drop_remains_uncertain() {
    let f = Fixture::new();
    let key = f.0.to_string_lossy().to_string();
    let version = credential_version(b"initial");
    drop(
        begin_attempt(&key, "kimi", &key, version, allow)
            .await
            .unwrap(),
    );
    let mut sent = begin_attempt(&key, "kimi", &key, version, allow)
        .await
        .unwrap();
    sent.started();
    drop(sent);
    assert!(begin_attempt(&key, "kimi", &key, version, allow)
        .await
        .is_err());
}
#[tokio::test]
async fn abandoned_reservation_preserves_prior_nonrotating_success() {
    let f = Fixture::new();
    let key = f.0.to_string_lossy().to_string();
    let mut completed = begin_attempt(&key, "kimi", &key, credential_version(b"old"), allow)
        .await
        .unwrap();
    completed.started();
    completed.finish(false);
    drop(completed);
    drop(
        begin_attempt(&key, "kimi", &key, credential_version(b"new"), allow)
            .await
            .unwrap(),
    );
    assert!(
        begin_attempt(&key, "kimi", &key, credential_version(b"new"), allow)
            .await
            .is_ok()
    );
}
#[tokio::test]
async fn cursor_external_rotating_or_ambiguous_outcomes_still_block_old_refresh() {
    for rotated in [true, false] {
        let f = Fixture::new();
        let key = f.0.to_string_lossy().to_string();
        let old = jwt("provider|sticky", None);
        let external = format!("{old}-external");
        let source = std::sync::Mutex::new((old.clone(), key.clone()));
        let result=memory_access(&key,||Ok(source.lock().unwrap().clone()),Some(credential_version(old.as_bytes())),allow,|_|async{
            *source.lock().unwrap()=(external.clone(),key.clone());
            if rotated {Ok(json!({"access_token":format!("{old}-new"),"refresh_token":format!("{key}-rotated")}))} else {Err(RemoteError::Unavailable)}
        }).await.unwrap();
        assert_eq!(result.access, external);
        assert!(memory_access(
            &key,
            || Ok(source.lock().unwrap().clone()),
            Some(credential_version(external.as_bytes())),
            allow,
            |_| async { panic!("consumed or ambiguous old token must not be sent") }
        )
        .await
        .is_err());
    }
}
#[tokio::test]
async fn memory_and_cache_local_reread_errors_do_not_spend_refresh() {
    let f = Fixture::new();
    let key = f.0.to_string_lossy().to_string();
    let token = jwt("provider|preflight", None);
    let reads = AtomicUsize::new(0);
    let reader = || {
        if reads.fetch_add(1, Ordering::SeqCst) == 1 {
            Err("local source reread unavailable".into())
        } else {
            Ok((token.clone(), key.clone()))
        }
    };
    assert!(memory_access(
        &key,
        reader,
        Some(credential_version(token.as_bytes())),
        allow,
        |_| async { panic!("failed reread must not send") }
    )
    .await
    .is_err());
    assert!(memory_access(
        &key,
        reader,
        Some(credential_version(token.as_bytes())),
        allow,
        |_| async { Ok(json!({"access_token":format!("{token}-new")})) }
    )
    .await
    .is_ok());
    let path = f.0.join("cache.json");
    let cache_key = format!("{key}-cache");
    let reads = AtomicUsize::new(0);
    let reader = || {
        if reads.fetch_add(1, Ordering::SeqCst) == 1 {
            Err("keyring reread unavailable".into())
        } else {
            Ok(CachedSource {
                version: credential_version(b"same-source"),
                access: "expired".into(),
                refresh: Some(cache_key.clone()),
                expires: 0,
            })
        }
    };
    assert!(
        cached_access(&cache_key, &path, reader, None, allow, |_| async {
            panic!("failed keyring reread must not send")
        })
        .await
        .is_err()
    );
    assert!(
        cached_access(&cache_key, &path, reader, None, allow, |_| async {
            Ok(json!({"access_token":"new"}))
        })
        .await
        .is_ok()
    );
}
