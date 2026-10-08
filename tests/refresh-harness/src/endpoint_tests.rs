use crate::{
    access_policy::{AccessPolicy, AccessRuntime},
    credential_refresh::*,
    network_transport::with_test_transport,
    providers,
};
use serde_json::json;
use std::{sync::Arc, time::Duration};
struct TlsServer {
    addr: std::net::SocketAddr,
    hits: Arc<std::sync::atomic::AtomicUsize>,
    requests: Arc<std::sync::Mutex<Vec<String>>>,
    gate: Arc<std::sync::atomic::AtomicBool>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl TlsServer {
    fn new(status: u16, body: &str) -> Self {
        Self::response(status, body, None)
    }
    fn response(status: u16, body: &str, location: Option<&str>) -> Self {
        use std::io::{Read, Write};
        use std::sync::atomic::Ordering;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let gate = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let response_gate = gate.clone();
        let received = requests.clone();
        let (h, s) = (hits.clone(), stop.clone());
        let location = location
            .map(|v| format!("Location: {v}\r\n"))
            .unwrap_or_default();
        let body = body.to_string();
        let thread = std::thread::spawn(move || {
            let provider = rustls::crypto::ring::default_provider();
            let config = rustls::ServerConfig::builder_with_provider(Arc::new(provider))
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_no_client_auth()
                .with_single_cert(
                    vec![include_bytes!("../fixtures/synthetic-cert.der")
                        .to_vec()
                        .into()],
                    rustls::pki_types::PrivatePkcs8KeyDer::from(
                        include_bytes!("../fixtures/synthetic-key.der").to_vec(),
                    )
                    .into(),
                )
                .unwrap();
            let config = Arc::new(config);
            while !s.load(Ordering::SeqCst) {
                let (socket, _) = match listener.accept() {
                    Ok(v) => v,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(e) => panic!("{e}"),
                };
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                socket
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut tls = rustls::StreamOwned::new(
                    rustls::ServerConnection::new(config.clone()).unwrap(),
                    socket,
                );
                let mut request = Vec::new();
                loop {
                    let mut buf = [0; 2048];
                    match tls.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            request.extend_from_slice(&buf[..n]);
                            if request.windows(4).any(|w| w == b"\r\n\r\n") {
                                break;
                            }
                        }
                    }
                }
                if request.is_empty() {
                    continue;
                }
                h.fetch_add(1, Ordering::SeqCst);
                received
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&request).to_string());
                while !response_gate.load(Ordering::SeqCst) && !s.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(2));
                }
                if status == 0 {
                    continue;
                }
                let answer = format!("HTTP/1.1 {status} Synthetic\r\n{location}Content-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = tls.write_all(answer.as_bytes());
                let _ = tls.flush();
            }
        });
        Self {
            addr,
            hits,
            requests,
            gate,
            stop,
            thread: Some(thread),
        }
    }
    fn hits(&self) -> usize {
        self.hits.load(std::sync::atomic::Ordering::SeqCst)
    }
}
impl Drop for TlsServer {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        self.thread.take().unwrap().join().unwrap();
    }
}
fn test_certificate() -> reqwest::Certificate {
    reqwest::Certificate::from_der(include_bytes!("../fixtures/synthetic-cert.der")).unwrap()
}
fn runtime(provider: &str) -> Arc<AccessRuntime> {
    let mut p = AccessPolicy::default();
    p.set_family(provider, true).unwrap();
    p.set_account(provider, true).unwrap();
    if provider == "antigravity" {
        p.regions.insert(provider.into(), "cloud".into());
    }
    Arc::new(AccessRuntime::new(p))
}
struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        let mut bytes = [0; 16];
        getrandom::getrandom(&mut bytes).unwrap();
        let p = std::env::temp_dir().join(format!(
            "pane-refresh-tls-fixture-{:x}",
            u128::from_le_bytes(bytes)
        ));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn jwt(sub: &str) -> String {
    use base64::Engine;
    let doc = json!({"sub":sub,"exp":4102444800_i64,"https://api.openai.com/auth":{"chatgpt_account_id":"account-A","chatgpt_plan_type":"plus"}});
    format!(
        "fake.{}.sig",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(doc.to_string())
    )
}
#[tokio::test]
async fn all_six_adapters_refresh_through_checked_tls_transport() {
    for (provider, host) in [
        ("claude", "platform.claude.com"),
        ("codex", "auth.openai.com"),
        ("kimi", "auth.kimi.com"),
        ("grok", "auth.x.ai"),
        ("cursor", "api2.cursor.sh"),
        ("antigravity", "oauth2.googleapis.com"),
    ] {
        let f = Fixture::new();
        let refresh = f.0.to_string_lossy().to_string();
        let token = jwt("provider|A");
        let response = json!({"access_token":token,"expires_in":3600});
        let server = TlsServer::new(200, &response.to_string());
        let rt = runtime(provider);
        with_test_transport(vec![(host.into(),server.addr)],test_certificate(),async{
   rt.run_account(provider,&[],async{
    let actual=match provider{
     "claude"=>{std::fs::write(f.0.join(".credentials.json"),json!({"claudeAiOauth":{"accessToken":"expired","refreshToken":refresh,"expiresAt":0,"unknown":8}}).to_string()).unwrap();providers::claude::fixture_load(&f.0).await.unwrap()},
     "codex"=>{std::fs::write(f.0.join("auth.json"),json!({"tokens":{"access_token":"expired","refresh_token":refresh,"account_id":"account-A","id_token":jwt("provider|A")}}).to_string()).unwrap();providers::codex::fixture_load(&f.0).await.unwrap()},
     "kimi"=>{let p=f.0.join("fake.json");std::fs::write(&p,json!({"access_token":"expired","refresh_token":refresh,"expires_at":0}).to_string()).unwrap();providers::kimi::fixture_load(&p).await.unwrap()},
     "grok"=>{let p=f.0.join("fake.json");std::fs::write(&p,json!({"https://auth.x.ai::A":{"key":"expired","refresh_token":refresh,"expires_at":"2000-01-01T00:00:00Z","oidc_client_id":"fake-client","oidc_issuer":"https://auth.x.ai"}}).to_string()).unwrap();providers::grok::fixture_load(&p).await.unwrap()},
     "cursor"=>{let old=jwt("provider|A");memory_access(&refresh,||Ok((old.clone(),refresh.clone())),Some(credential_version(old.as_bytes())),crate::access_policy::check_current_operation,providers::cursor::fixture_refresh).await.unwrap().access},
     "antigravity"=>{let source=json!({"token":{"access_token":"expired","refresh_token":refresh,"expiry":"2000-01-01T00:00:00Z"}}).to_string();providers::TEST_CONFIG.scope(f.0.clone(),providers::TEST_KEYRING.scope(source,providers::antigravity::fixture_load())).await.unwrap()},_=>unreachable!()
    };assert_eq!(actual,token);
   }).await.unwrap();
  }).await;
        assert_eq!(server.hits(), 1, "{provider}");
        let requests = server.requests.lock().unwrap();
        assert!(requests[0].starts_with("POST "));
        assert!(requests[0]
            .to_ascii_lowercase()
            .contains(&format!("host: {host}")));
    }
}
#[tokio::test]
async fn actual_antigravity_save_error_reaches_cloud_result() {
    let f = Fixture::new();
    let source=json!({"token":{"access_token":"expired","refresh_token":f.0.to_string_lossy(),"expiry":"2000-01-01T00:00:00Z"}}).to_string();
    let server = TlsServer::new(200, "{\"access_token\":\"fresh\",\"expires_in\":3600}");
    server
        .gate
        .store(false, std::sync::atomic::Ordering::SeqCst);
    let rt = runtime("antigravity");
    let work = with_test_transport(
        vec![("oauth2.googleapis.com".into(), server.addr)],
        test_certificate(),
        providers::TEST_CONFIG.scope(
            f.0.clone(),
            providers::TEST_KEYRING.scope(source, async {
                rt.run_account("antigravity", &[], async {
                    let first = providers::antigravity::fixture_cloud_error().await.unwrap();
                    assert!(first.contains("refresh succeeded, save failed"));
                    let second = providers::antigravity::fixture_cloud_error().await.unwrap();
                    assert!(second.contains("refresh succeeded, save failed"));
                })
                .await
                .unwrap();
            }),
        ),
    );
    let sabotage = async {
        while server.hits() == 0 {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        for e in std::fs::read_dir(&f.0).unwrap() {
            let e = e.unwrap();
            if e.file_name().to_string_lossy().ends_with(".tmp") {
                std::fs::remove_file(e.path()).unwrap();
            }
        }
        server.gate.store(true, std::sync::atomic::Ordering::SeqCst);
    };
    tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(work, sabotage);
    })
    .await
    .unwrap();
    assert_eq!(server.hits(), 1);
}
#[tokio::test]
async fn actual_permit_revocation_cancels_rotation_and_preserves_latch() {
    use std::sync::atomic::Ordering;
    let f = Fixture::new();
    let path = f.0.join("fake.json");
    let before =
        json!({"access_token":"expired","refresh_token":f.0.to_string_lossy(),"expires_at":0})
            .to_string();
    std::fs::write(&path, &before).unwrap();
    let server = TlsServer::new(
        200,
        "{\"access_token\":\"fresh\",\"refresh_token\":\"rotated\"}",
    );
    server.gate.store(false, Ordering::SeqCst);
    let rt = runtime("kimi");
    let original = rt.snapshot().0;
    let work = with_test_transport(
        vec![("auth.kimi.com".into(), server.addr)],
        test_certificate(),
        async {
            rt.run_account("kimi", &[], providers::kimi::fixture_load(&path))
                .await
        },
    );
    let revoke = async {
        while server.hits() == 0 {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        let mut p = rt.snapshot().0;
        p.set_account("kimi", false).unwrap();
        rt.update(p);
        server.gate.store(true, Ordering::SeqCst);
    };
    let (result, ()) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(work, revoke)
    })
    .await
    .unwrap();
    assert!(result.is_none());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    rt.update(original);
    with_test_transport(
        vec![("auth.kimi.com".into(), server.addr)],
        test_certificate(),
        async {
            assert!(rt
                .run_account("kimi", &[], providers::kimi::fixture_load(&path))
                .await
                .unwrap()
                .is_err());
        },
    )
    .await;
    assert_eq!(server.hits(), 1);
    assert_eq!(std::fs::read_dir(&f.0).unwrap().count(), 1);
}
#[test]
fn adapter_refresh_futures_remain_send() {
    fn assert_send<T: Send>(_: T) {}
    let synthetic = std::path::Path::new("synthetic-not-opened");
    assert_send(providers::claude::fixture_load(synthetic));
    assert_send(providers::codex::fixture_load(synthetic));
    assert_send(providers::kimi::fixture_load(synthetic));
    assert_send(providers::grok::fixture_load(synthetic));
    assert_send(providers::cursor::fixture_refresh(
        "synthetic-not-sent".into(),
    ));
    assert_send(providers::antigravity::fixture_load());
}

// An injected token reservation exercises the final identity-check invariant.
// This is not a claim that this exact schedule occurs in ordinary file flows.
#[tokio::test]
async fn review_claude_sidecar_switch_during_token_queue_never_sends() {
    let f = Fixture::new();
    let cred = f.0.join(".credentials.json");
    let sidecar = f.0.join(".claude.json");
    let refresh = format!("{}-claude-review", f.0.display());
    std::fs::write(
        &cred,
        json!({"claudeAiOauth":{"accessToken":"expired","refreshToken":refresh,"expiresAt":0}})
            .to_string(),
    )
    .unwrap();
    std::fs::write(
        &sidecar,
        json!({"oauthAccount":{"accountUuid":"aaaaaaaa-old"}}).to_string(),
    )
    .unwrap();
    let key = cred.canonicalize().unwrap().to_string_lossy().to_string();
    let mut held = begin_attempt(
        &key,
        "claude",
        &refresh,
        credential_version(b"prior-generation"),
        || Ok(()),
    )
    .await
    .unwrap();
    let server = TlsServer::new(200, "{\"access_token\":\"fresh\",\"expires_in\":3600}");
    let rt = runtime("claude");
    let work = with_test_transport(
        vec![("platform.claude.com".into(), server.addr)],
        test_certificate(),
        async {
            rt.run_account("claude", &[], providers::claude::fixture_load(&f.0))
                .await
                .unwrap()
        },
    );
    let switch = async {
        loop {
            if std::fs::read_dir(&f.0)
                .unwrap()
                .any(|e| e.unwrap().file_name().to_string_lossy().ends_with(".tmp"))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        std::fs::write(
            &sidecar,
            json!({"oauthAccount":{"accountUuid":"bbbbbbbb-new"}}).to_string(),
        )
        .unwrap();
        held.finish(false);
        drop(held);
    };
    let (result, _) = tokio::join!(work, switch);
    assert!(result.is_err());
    assert_eq!(
        server.hits(),
        0,
        "identity changed while queued, but a refresh was transmitted: {:?}",
        result
    );
}
