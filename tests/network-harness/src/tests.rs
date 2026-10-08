use crate::access_policy::{AccessPolicy, AccessRuntime};
use crate::network_policy::{validate_destination, validate_selection, ProviderEndpoint};
use crate::network_transport::CheckedClient;
use reqwest::Url;
use std::{sync::Arc, time::Duration};

fn runtime(provider: &str, selection: Option<&str>) -> Arc<AccessRuntime> {
    let mut p = AccessPolicy::default();
    p.set_family(provider, true).unwrap();
    p.set_account(provider, true).unwrap();
    if let Some(s) = selection {
        p.regions.insert(provider.into(), s.into());
    }
    Arc::new(AccessRuntime::new(p))
}
fn server() -> (tiny_http::Server, String) {
    let s = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let url = format!("http://{}", s.server_addr());
    (s, url)
}
fn empty(s: &tiny_http::Server) {
    assert!(s.recv_timeout(Duration::from_millis(30)).unwrap().is_none());
}
fn endpoint(provider: &str, region: &str, origin: &str) -> ProviderEndpoint {
    ProviderEndpoint {
        provider: provider.into(),
        region: region.into(),
        origin: origin.into(),
    }
}

#[test]
fn embedded_credentials_rejected() {
    for origin in [
        "https://token@api.deepseek.com",
        "https://user:token@api.deepseek.com",
    ] {
        assert!(validate_destination(
            &endpoint("deepseek", "global", origin),
            &Url::parse(origin).unwrap()
        )
        .is_err());
    }
}
#[test]
fn unconfigured_host_rejected() {
    for origin in [
        "https://api.github.com",
        "https://api.deepseek.com.evil.test",
        "https://sub.api.deepseek.com",
        "https://api.deepseek.com.",
        "https://api.deepseek.com:444",
        "http://api.deepseek.com",
        "https://raw.githubusercontent.com",
    ] {
        assert!(
            validate_destination(
                &endpoint("deepseek", "global", origin),
                &Url::parse(origin).unwrap()
            )
            .is_err(),
            "{origin}"
        );
    }
    assert!(validate_destination(
        &endpoint("onenewapi", "global", "https://api.deepseek.com"),
        &Url::parse("https://api.deepseek.com").unwrap()
    )
    .is_err());
}
#[test]
fn missing_region_and_unsupported_kind_are_rejected() {
    for provider in ["zai", "minimax", "moonshot", "qwen", "stepfun"] {
        assert!(validate_selection(provider, "").is_err());
        assert!(validate_selection(provider, "international:unknown").is_err());
    }
}
#[tokio::test]
async fn loopback_requires_explicit_configuration() {
    let (s, url) = server();
    for (selection, reason) in [
        (
            None,
            "Choose this account's region and credential type in Settings",
        ),
        (
            Some("local:http://127.0.0.1:1"),
            "Local destination was not explicitly bound",
        ),
    ] {
        let rt = runtime("ollama", selection);
        let result = rt
            .run_account("ollama", &[], async {
                CheckedClient::new("ollama", None).get(&url).send().await
            })
            .await
            .unwrap();
        let error = result.unwrap_err().to_string();
        assert!(error.contains(reason), "wrong denial: {error}");
        assert!(!error.contains("no account authorization"));
        empty(&s);
    }
}
#[tokio::test]
async fn credential_redirect_is_not_followed() {
    for status in [301, 302, 303, 307, 308] {
        let (source, from) = server();
        let (target, to) = server();
        let rt = runtime("antigravity", Some("local_process"));
        let thread = std::thread::spawn(move || {
            let mut request = source
                .recv_timeout(Duration::from_secs(3))
                .unwrap()
                .expect("initial request");
            let mut body = String::new();
            request.as_reader().read_to_string(&mut body).unwrap();
            assert!(body.contains("synthetic-refresh"));
            request
                .respond(
                    tiny_http::Response::empty(status)
                        .with_header(tiny_http::Header::from_bytes("Location", to).unwrap()),
                )
                .unwrap();
        });
        let response = rt
            .run_account("antigravity", &[], async {
                CheckedClient::antigravity_local(&from)
                    .unwrap()
                    .post(&from)
                    .bearer_auth("synthetic-access")
                    .header("Cookie", "synthetic-cookie")
                    .header("x-codeium-csrf-token", "synthetic-csrf")
                    .json(&serde_json::json!({"refresh_token":"synthetic-refresh"}))
                    .send()
                    .await
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(response.status().as_u16(), status);
        thread.join().unwrap();
        empty(&target);
    }
}
#[tokio::test]
async fn revoked_builder_does_not_send() {
    let (s, url) = server();
    let rt = runtime("antigravity", Some("local_process"));
    let permit = rt.permit("antigravity", &[]).unwrap();
    permit
        .run(async {
            let request = CheckedClient::antigravity_local(&url)
                .unwrap()
                .post(&url)
                .form(&[("refresh_token", "synthetic")]);
            rt.invalidate();
            assert!(request.send().await.is_err());
            empty(&s);
        })
        .await;
}
#[tokio::test]
async fn no_context_or_wrong_family_never_sends() {
    let (s, url) = server();
    assert!(CheckedClient::new("antigravity", None)
        .post(&url)
        .body("synthetic")
        .send()
        .await
        .is_err());
    runtime("ollama", Some(&format!("local:{url}")))
        .run_account("ollama", &[], async {
            assert!(CheckedClient::new("antigravity", None)
                .post(&url)
                .body("synthetic")
                .send()
                .await
                .is_err());
        })
        .await
        .unwrap();
    empty(&s);
}

#[test]
fn antigravity_ports_match_exact_pid_not_suffix() {
    let raw = "TCP 127.0.0.1:12345 0.0.0.0:0 LISTENING 142\nTCP 127.0.0.1:23456 0.0.0.0:0 LISTENING 42\nTCP 192.168.1.3:34567 0.0.0.0:0 LISTENING 42\nTCP 0.0.0.0:45678 0.0.0.0:0 LISTENING 42";
    assert_eq!(
        crate::network_policy::antigravity_process_ports(raw, 42),
        vec![23456, 45678]
    );
}

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
        Self::scripted(status, body, location, None)
    }
    fn scripted(
        status: u16,
        body: &str,
        location: Option<&str>,
        subscription: Option<(u16, &str)>,
    ) -> Self {
        let subscription = subscription.map(|(status, body)| (status, body.to_string()));
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
                let (status, body) = if request.starts_with(b"GET /alpha/billing/subscriptions ") {
                    subscription
                        .as_ref()
                        .map(|(s, b)| (*s, b))
                        .unwrap_or((status, &body))
                } else {
                    (status, &body)
                };
                let retry = if status == 429 {
                    "Retry-After: 120\r\n"
                } else {
                    ""
                };
                let answer = format!("HTTP/1.1 {status} Synthetic\r\n{location}{retry}Content-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}", body.len());
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

#[tokio::test]
async fn wrong_region_does_not_probe_sibling() {
    for (provider, kind) in [
        ("zai", "api_key"),
        ("minimax", "api_key"),
        ("moonshot", "api_key"),
        ("qwen", "bearer"),
        ("stepfun", "api_key"),
    ] {
        let selected = format!("international:{kind}");
        let china = format!("china:{kind}");
        let first = crate::network_policy::remote_origins(provider, &selected)[0];
        let other = crate::network_policy::remote_origins(provider, &china)[0];
        for (status, body) in [
            (401, "{}"),
            (403, "{}"),
            (429, "{}"),
            (503, "{}"),
            (200, "malformed"),
        ] {
            let primary = TlsServer::new(status, body);
            let sibling = TlsServer::new(200, "{}");
            let routes = vec![
                (
                    Url::parse(first).unwrap().host_str().unwrap().into(),
                    primary.addr,
                ),
                (
                    Url::parse(other).unwrap().host_str().unwrap().into(),
                    sibling.addr,
                ),
            ];
            let rt = runtime(provider, Some(&selected));
            crate::network_transport::with_test_transport(routes, test_certificate(), async {
                rt.run_account(provider, &[], async {
                    let response = CheckedClient::new(provider, None)
                        .get(first)
                        .bearer_auth("synthetic-only")
                        .send()
                        .await
                        .unwrap();
                    assert_eq!(response.status().as_u16(), status);
                    // A retry path can never reuse this credential for the sibling.
                    assert!(CheckedClient::new(provider, None)
                        .get(other)
                        .bearer_auth("synthetic-only")
                        .send()
                        .await
                        .is_err());
                })
                .await
                .unwrap();
            })
            .await;
            assert_eq!(primary.hits(), 1);
            assert_eq!(sibling.hits(), 0);
        }
    }
}

#[tokio::test]
async fn actual_regional_adapters_never_try_sibling_hosts_or_credential_kinds() {
    for (provider, kind) in [
        ("zai", "api_key"),
        ("minimax", "api_key"),
        ("moonshot", "api_key"),
        ("qwen", "bearer"),
        ("stepfun", "api_key"),
        ("stepfun", "plan_key"),
    ] {
        let selection = format!("international:{kind}");
        let other_selection = format!("china:{kind}");
        let first = crate::network_policy::remote_origins(provider, &selection)[0];
        let second = crate::network_policy::remote_origins(provider, &other_selection)[0];
        for (status, body) in [
            (0, ""),
            (401, "{}"),
            (403, "{}"),
            (429, "{}"),
            (500, "{}"),
            (200, "invalid json"),
        ] {
            let primary = TlsServer::new(status, body);
            let sibling = TlsServer::new(200, "{}");
            let routes = vec![
                (
                    Url::parse(first).unwrap().host_str().unwrap().into(),
                    primary.addr,
                ),
                (
                    Url::parse(second).unwrap().host_str().unwrap().into(),
                    sibling.addr,
                ),
            ];
            crate::network_transport::with_test_transport(routes, test_certificate(), async {
                runtime(provider, Some(&selection))
                    .run_account(provider, &[], async {
                        use crate::providers::*;
                        let snap = match provider {
                            "zai" => zai::snapshot().await,
                            "minimax" => minimax::snapshot().await,
                            "moonshot" => moonshot::snapshot().await,
                            "stepfun" => stepfun::snapshot().await,
                            "qwen" => {
                                assert!(qwen::quota_with_synthetic_key("synthetic")
                                    .await
                                    .is_none());
                                return;
                            }
                            _ => unreachable!(),
                        };
                        assert_eq!(
                            snap.status,
                            if kind == "plan_key" && status == 200 {
                                "ok"
                            } else {
                                "error"
                            },
                            "{provider} {status}"
                        );
                    })
                    .await
                    .unwrap();
            })
            .await;
            assert_eq!(
                primary.hits(),
                if matches!(provider, "zai" | "minimax") {
                    2
                } else {
                    1
                },
                "{provider} {status}"
            );
            assert_eq!(sibling.hits(), 0, "{provider} {status}");
            let requests = primary.requests.lock().unwrap();
            if provider == "stepfun" {
                let expected_path = if kind == "plan_key" {
                    "/step_plan/v1/models"
                } else {
                    "/v1/accounts"
                };
                assert!(requests
                    .iter()
                    .all(|r| r.starts_with(&format!("GET {expected_path} "))));
            }
            if provider == "qwen" {
                assert!(requests
                    .iter()
                    .all(|r| r.to_lowercase().contains("authorization: bearer synthetic")));
            }
        }
    }
}

#[test]
fn all_retained_official_origins_are_covered_and_provider_scoped() {
    let cases: &[(&str, &str, &[&str])] = &[
        ("aihubmix", "global", &["https://aihubmix.com"]),
        (
            "antigravity",
            "cloud",
            &[
                "https://oauth2.googleapis.com",
                "https://daily-cloudcode-pa.googleapis.com",
                "https://cloudcode-pa.googleapis.com",
            ],
        ),
        (
            "claude",
            "global",
            &["https://api.anthropic.com", "https://platform.claude.com"],
        ),
        ("codebuff", "global", &["https://www.codebuff.com"]),
        (
            "codex",
            "global",
            &["https://chatgpt.com", "https://auth.openai.com"],
        ),
        ("copilot", "global", &["https://api.github.com"]),
        (
            "cursor",
            "global",
            &["https://api2.cursor.sh", "https://cursor.com"],
        ),
        ("deepseek", "global", &["https://api.deepseek.com"]),
        ("devin", "global", &["https://server.codeium.com"]),
        ("elevenlabs", "global", &["https://api.elevenlabs.io"]),
        (
            "grok",
            "global",
            &[
                "https://auth.x.ai",
                "https://cli-chat-proxy.grok.com",
                "https://grok.com",
            ],
        ),
        ("kilo", "global", &["https://app.kilo.ai"]),
        (
            "kimi",
            "global",
            &["https://api.kimi.com", "https://auth.kimi.com"],
        ),
        (
            "minimax",
            "international:api_key",
            &["https://api.minimax.io"],
        ),
        ("minimax", "china:api_key", &["https://api.minimaxi.com"]),
        (
            "minimax",
            "international:mcode",
            &["https://agent.minimax.io", "https://platform.minimax.io"],
        ),
        (
            "minimax",
            "china:mcode",
            &["https://agent.minimaxi.com", "https://www.minimaxi.com"],
        ),
        (
            "moonshot",
            "international:api_key",
            &["https://api.moonshot.ai"],
        ),
        ("moonshot", "china:api_key", &["https://api.moonshot.cn"]),
        ("opencode", "global", &["https://opencode.ai"]),
        ("openrouter", "global", &["https://openrouter.ai"]),
        (
            "qwen",
            "international:bearer",
            &["https://modelstudio.console.alibabacloud.com"],
        ),
        (
            "qwen",
            "china:bearer",
            &["https://bailian.console.aliyun.com"],
        ),
        (
            "stepfun",
            "international:api_key",
            &["https://api.stepfun.ai"],
        ),
        ("stepfun", "china:plan_key", &["https://api.stepfun.com"]),
        ("zai", "international:api_key", &["https://api.z.ai"]),
        ("zai", "china:api_key", &["https://open.bigmodel.cn"]),
    ];
    for (provider, selection, origins) in cases {
        assert_eq!(
            crate::network_policy::remote_origins(provider, selection),
            *origins
        );
        for origin in *origins {
            let url = Url::parse(&format!("{origin}:443/test?public=value")).unwrap();
            assert!(
                validate_destination(&endpoint(provider, selection, origin), &url).is_ok(),
                "{provider} {origin}"
            );
            assert!(validate_destination(&endpoint("ollama", "global", origin), &url).is_err());
        }
    }
    assert!(crate::network_policy::remote_origins("hermes", "global").is_empty());
    assert!(crate::network_policy::remote_origins("sub2api", "global").is_empty());
}

#[tokio::test]
async fn escaped_revoked_builder_cannot_be_reused_in_new_operation() {
    let (s, url) = server();
    let rt = runtime("ollama", Some(&format!("local:{url}")));
    let request = rt
        .run_account("ollama", &[], async {
            CheckedClient::new("ollama", None).get(&url)
        })
        .await
        .unwrap();
    rt.invalidate();
    rt.run_account("ollama", &[], async {
        assert!(request.send().await.is_err());
    })
    .await
    .unwrap();
    empty(&s);
}

#[tokio::test]
async fn denial_does_not_serialize_credentials_and_errors_are_sanitized() {
    struct Secret;
    impl serde::Serialize for Secret {
        fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
            panic!("Denied destination must not serialize credential bodies");
        }
    }
    let request = CheckedClient::new("deepseek", None)
        .post("https://synthetic-secret@evil.test/?token=synthetic-secret")
        .json(&Secret);
    let error = request.send().await.unwrap_err();
    assert!(!format!("{error:?} {error}").contains("synthetic-secret"));
}

#[test]
fn fixed_dynamic_origins_reject_userinfo_ports_paths_queries_and_fragments() {
    for fixed in ["https://auth.x.ai", "https://server.codeium.com"] {
        assert_eq!(
            crate::network_policy::fixed_origin(&format!("{fixed}:443/"), fixed).unwrap(),
            fixed
        );
        for bad in [
            format!("{fixed}:444"),
            format!("{fixed}/path"),
            format!("{fixed}?key=synthetic"),
            format!("{fixed}#fragment"),
            fixed.replace("https://", "https://synthetic@"),
        ] {
            assert!(crate::network_policy::fixed_origin(&bad, fixed).is_err());
        }
    }
}

#[tokio::test]
async fn missing_region_denies_before_actual_adapter_credential_discovery() {
    for provider in ["zai", "minimax", "moonshot", "qwen", "stepfun"] {
        let reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        crate::providers::DISCOVERY_READS
            .scope(reads.clone(), async {
                runtime(provider, None)
                    .run_account(provider, &[], async {
                        use crate::providers::*;
                        let snapshot = match provider {
                            "zai" => zai::snapshot().await,
                            "minimax" => minimax::snapshot().await,
                            "moonshot" => moonshot::snapshot().await,
                            "qwen" => qwen::snapshot().await,
                            "stepfun" => stepfun::snapshot().await,
                            _ => unreachable!(),
                        };
                        assert_eq!(snapshot.status, "error");
                        assert!(snapshot.error.unwrap().contains("Settings"));
                    })
                    .await
                    .unwrap();
            })
            .await;
        assert_eq!(
            reads.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "{provider}"
        );
    }
}

#[tokio::test]
async fn explicit_loopback_ipv4_ipv6_and_localhost_are_exact_and_ignore_proxy() {
    for host in ["127.0.0.1", "localhost", "[::1]"] {
        let listener = tiny_http::Server::http(if host == "[::1]" {
            "[::1]:0"
        } else {
            "127.0.0.1:0"
        })
        .unwrap();
        let port = listener.server_addr().to_ip().unwrap().port();
        let origin = format!("http://{host}:{port}");
        let (proxy, proxy_url) = server();
        let rt = runtime("ollama", Some(&format!("local:{origin}")));
        let worker = std::thread::spawn(move || {
            let request = listener
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
                .unwrap();
            request.respond(tiny_http::Response::empty(200)).unwrap();
        });
        rt.run_account("ollama", &[], async {
            assert_eq!(
                CheckedClient::new("ollama", Some(&proxy_url))
                    .get(format!("{origin}/api/version"))
                    .send()
                    .await
                    .unwrap()
                    .status(),
                200
            );
            let other = format!(
                "http://{host}:{}",
                if port == 65535 { 65534 } else { port + 1 }
            );
            assert!(CheckedClient::new("ollama", None)
                .get(other)
                .send()
                .await
                .is_err());
            if host != "localhost" {
                assert!(CheckedClient::new("ollama", None)
                    .get(format!("http://localhost:{port}/"))
                    .send()
                    .await
                    .is_err());
            }
        })
        .await
        .unwrap();
        worker.join().unwrap();
        empty(&proxy);
    }
}

#[test]
fn local_selection_rejects_lan_unspecified_suffix_and_other_providers() {
    for origin in [
        "http://0.0.0.0:1",
        "http://[::]:1",
        "http://192.168.1.1:1",
        "http://localhost.evil.test:1",
        "http://sub.localhost:1",
        "http://127.0.0.1:0",
        "http://token@127.0.0.1:1",
        "http://127.0.0.1:1/path",
    ] {
        assert!(
            validate_selection("ollama", &format!("local:{origin}")).is_err(),
            "{origin}"
        );
    }
    for provider in ["claude", "zai", "onenewapi", "sub2api"] {
        assert!(validate_selection(provider, "local:http://127.0.0.1:1").is_err());
    }
    assert!(validate_selection("antigravity", "local_process").is_ok());
}

#[tokio::test]
async fn antigravity_local_process_mode_cannot_use_unbound_caller_url() {
    let (s, url) = server();
    runtime("antigravity", Some("local_process"))
        .run_account("antigravity", &[], async {
            assert!(CheckedClient::new("antigravity", None)
                .post(&url)
                .header("x-codeium-csrf-token", "synthetic")
                .send()
                .await
                .is_err());
            assert!(CheckedClient::antigravity_local("http://192.168.1.1:1234").is_err());
        })
        .await
        .unwrap();
    empty(&s);
}

#[tokio::test]
async fn public_prices_have_no_credentials_and_never_follow_redirects() {
    let (target, to) = server();
    let source = TlsServer::response(307, "", Some(&to));
    let url = crate::network_policy::PUBLIC_PRICE_URLS[0];
    let routes = vec![("raw.githubusercontent.com".into(), source.addr)];
    crate::network_transport::with_test_transport(routes, test_certificate(), async {
        // A credential context cannot contribute anything to this API.
        runtime("deepseek", None)
            .run_account("deepseek", &[], async {
                let response = crate::network_transport::public_price_request(url, None)
                    .etag("synthetic-public-etag")
                    .send()
                    .await
                    .unwrap();
                assert_eq!(response.status(), 307);
            })
            .await
            .unwrap();
    })
    .await;
    empty(&target);
    let requests = source.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let request = requests[0].to_lowercase();
    assert!(request.contains("if-none-match: synthetic-public-etag"));
    for secret in [
        "authorization:",
        "cookie:",
        "api-key:",
        "csrf",
        "synthetic-key-never-real",
    ] {
        assert!(!request.contains(secret));
    }
    for bad in [
        "https://models.dev/api.json?account=synthetic",
        "https://models.dev/other",
        "https://raw.githubusercontent.com/other",
        "https://api.deepseek.com/user/balance",
    ] {
        assert!(crate::network_transport::public_price_request(bad, None)
            .send()
            .await
            .is_err());
    }
}

#[tokio::test]
async fn nested_account_contexts_do_not_authorize_another_provider() {
    let mut policy = AccessPolicy::default();
    for name in ["kimi", "moonshot"] {
        policy.set_family(name, true).unwrap();
        policy.set_account(name, true).unwrap();
    }
    policy
        .regions
        .insert("moonshot".into(), "china:api_key".into());
    let rt = Arc::new(AccessRuntime::new(policy));
    rt.run_account("kimi", &[], async {
        assert!(crate::network_policy::current_selection("moonshot").is_err());
        assert_eq!(
            rt.run_account("moonshot", &[], async {
                crate::network_policy::current_selection("moonshot").unwrap()
            })
            .await
            .unwrap(),
            "china:api_key"
        );
        assert_eq!(
            crate::network_policy::current_selection("kimi").unwrap(),
            "global"
        );
    })
    .await
    .unwrap();
    rt.run_account("kimi", &["moonshot".into()], async {
        assert!(rt
            .run_account("moonshot", &[], async {
                panic!("narrowed account must never execute")
            })
            .await
            .is_none());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn explicitly_bound_extra_account_uses_its_own_selection_and_family() {
    use crate::access_policy::AccountBinding;
    let mut policy = AccessPolicy::default();
    policy.set_family("claude", true).unwrap();
    policy.account_bindings.insert(
        "claude@abcd1234".into(),
        AccountBinding {
            family: "claude".into(),
            directory: std::env::temp_dir(),
            name: "Synthetic".into(),
        },
    );
    policy.set_account("claude@abcd1234", true).unwrap();
    let rt = Arc::new(AccessRuntime::new(policy));
    rt.run_account("claude@abcd1234", &[], async {
        assert_eq!(
            crate::network_policy::current_selection("claude").unwrap(),
            "global"
        );
        assert!(crate::network_policy::current_selection("codex").is_err());
    })
    .await
    .unwrap();
}

#[test]
fn production_provider_network_has_no_raw_client_escape_hatch() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/src");
    let mut adapters = 0;
    for entry in std::fs::read_dir(root.join("providers")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|x| x.to_str()) != Some("rs")
            || path.file_name().unwrap() == "mod.rs"
        {
            continue;
        }
        adapters += 1;
        let source = std::fs::read_to_string(&path).unwrap();
        for bypass in ["reqwest::Client", "http()", "http_no_redirect()"] {
            assert!(
                !source.contains(bypass),
                "{} exposes {bypass}",
                path.display()
            );
        }
    }
    assert_eq!(adapters, 23);
    let transport = std::fs::read_to_string(root.join("network_transport.rs")).unwrap();
    for escape in [
        "impl Deref",
        "impl AsRef<reqwest::",
        "pub fn into_inner",
        "pub fn build(",
        "pub fn execute(",
    ] {
        assert!(!transport.contains(escape));
    }
}

#[tokio::test]
async fn qwen_uses_only_the_explicitly_selected_header() {
    for (kind, header) in [
        ("bearer", "authorization: Bearer synthetic"),
        ("x_api_key", "x-api-key: synthetic"),
        ("dashscope_api_key", "x-dashscope-api-key: synthetic"),
    ] {
        let source = TlsServer::new(401, "{}");
        crate::network_transport::with_test_transport(
            vec![("modelstudio.console.alibabacloud.com".into(), source.addr)],
            test_certificate(),
            async {
                runtime("qwen", Some(&format!("international:{kind}")))
                    .run_account("qwen", &[], async {
                        assert!(
                            crate::providers::qwen::quota_with_synthetic_key("synthetic")
                                .await
                                .is_none()
                        );
                    })
                    .await
                    .unwrap();
            },
        )
        .await;
        assert_eq!(source.hits(), 1);
        let request = source.requests.lock().unwrap()[0].to_lowercase();
        assert!(request.contains(&header.to_lowercase()));
        let selected_headers = ["authorization:", "x-api-key:", "x-dashscope-api-key:"]
            .iter()
            .filter(|h| request.contains(**h))
            .count();
        assert_eq!(selected_headers, 1);
    }
}

#[tokio::test]
async fn form_and_binary_credentials_do_not_follow_same_origin_redirect() {
    for form in [true, false] {
        let (source, url) = server();
        let target = format!("{url}/redirected");
        let thread = std::thread::spawn(move || {
            let mut request = source
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
                .unwrap();
            let mut body = String::new();
            request.as_reader().read_to_string(&mut body).unwrap();
            assert!(body.contains("synthetic"));
            request
                .respond(
                    tiny_http::Response::empty(307)
                        .with_header(tiny_http::Header::from_bytes("Location", target).unwrap()),
                )
                .unwrap();
            empty(&source);
        });
        runtime("antigravity", Some("local_process"))
            .run_account("antigravity", &[], async {
                let request = CheckedClient::antigravity_local(&url)
                    .unwrap()
                    .post(&url)
                    .timeout(Duration::from_secs(2));
                let request = if form {
                    request.form(&[("refresh_token", "synthetic")])
                } else {
                    request.body(b"synthetic-binary-body".to_vec())
                };
                assert_eq!(request.send().await.unwrap().status(), 307);
            })
            .await
            .unwrap();
        thread.join().unwrap();
    }
}

#[tokio::test]
async fn stepfun_query_snapshot_does_not_read_local_spend() {
    let reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let source = TlsServer::new(200, "{}");
    crate::spend::READS
        .scope(reads.clone(), async {
            crate::network_transport::with_test_transport(
                vec![("api.stepfun.ai".into(), source.addr)],
                test_certificate(),
                async {
                    runtime("stepfun", Some("international:plan_key"))
                        .run_account("stepfun", &[], async {
                            assert_eq!(crate::providers::stepfun::snapshot().await.status, "ok");
                        })
                        .await
                        .unwrap();
                },
            )
            .await;
        })
        .await;
    assert_eq!(
        reads.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "query snapshot must not read local spend"
    );
}

#[tokio::test]
async fn qwen_refusal_is_bound_to_credential_and_mode() {
    use crate::providers::qwen;
    let source = TlsServer::new(401, "{}");
    qwen::with_isolated_quota_cooldown(async {
        qwen::reset_quota_cooldown();
        crate::network_transport::with_test_transport(
            vec![("modelstudio.console.alibabacloud.com".into(), source.addr)],
            test_certificate(),
            async {
                let bearer = runtime("qwen", Some("international:bearer"));
                bearer
                    .run_account("qwen", &[], async {
                        assert!(qwen::quota_with_synthetic_key("synthetic-A")
                            .await
                            .is_none());
                        assert!(qwen::quota_with_synthetic_key("synthetic-A")
                            .await
                            .is_none());
                        assert_eq!(source.hits(), 1, "same refused credential keeps cooldown");
                        assert!(qwen::quota_with_synthetic_key("synthetic-B")
                            .await
                            .is_none());
                        assert_eq!(
                            source.hits(),
                            2,
                            "changed credential must not inherit refusal"
                        );
                    })
                    .await
                    .unwrap();
                runtime("qwen", Some("international:x_api_key"))
                    .run_account("qwen", &[], async {
                        assert!(qwen::quota_with_synthetic_key("synthetic-B")
                            .await
                            .is_none());
                        assert_eq!(source.hits(), 3, "changed mode must not inherit refusal");
                    })
                    .await
                    .unwrap();
                bearer
                    .run_account("qwen", &[], async {
                        assert!(qwen::quota_with_synthetic_key("synthetic-B")
                            .await
                            .is_none());
                        assert_eq!(
                            source.hits(),
                            4,
                            "returning to a prior mode must not revive old refusal"
                        );
                        qwen::reset_quota_cooldown();
                        assert!(qwen::quota_with_synthetic_key("synthetic-B")
                            .await
                            .is_none());
                        assert_eq!(source.hits(), 5, "explicit context reset permits a retry");
                    })
                    .await
                    .unwrap();
            },
        )
        .await;
    })
    .await;
}

#[tokio::test]
async fn qwen_test_contexts_are_independent() {
    use crate::providers::qwen;
    qwen::reset_quota_cooldown();
    for _ in 0..2 {
        let source = TlsServer::new(401, "{}");
        qwen::with_isolated_quota_cooldown(async {
            crate::network_transport::with_test_transport(
                vec![("modelstudio.console.alibabacloud.com".into(), source.addr)],
                test_certificate(),
                async {
                    runtime("qwen", Some("international:bearer"))
                        .run_account("qwen", &[], async {
                            assert!(qwen::quota_with_synthetic_key("same-synthetic-key")
                                .await
                                .is_none());
                            assert!(qwen::quota_with_synthetic_key("same-synthetic-key")
                                .await
                                .is_none());
                        })
                        .await
                        .unwrap();
                },
            )
            .await;
            assert_eq!(
                source.hits(),
                1,
                "each isolated context gets its own first request"
            );
        })
        .await;
    }
}

#[tokio::test]
async fn qwen_reset_during_request_does_not_rearm_old_refusal() {
    use crate::providers::qwen;
    use std::sync::atomic::Ordering;
    let source = TlsServer::new(401, "{}");
    source.gate.store(false, Ordering::SeqCst);
    qwen::with_isolated_quota_cooldown(async {
        crate::network_transport::with_test_transport(
            vec![("modelstudio.console.alibabacloud.com".into(), source.addr)],
            test_certificate(),
            async {
                runtime("qwen", Some("international:bearer"))
                    .run_account("qwen", &[], async {
                        let request = qwen::quota_with_synthetic_key("synthetic");
                        let reset_after_send = async {
                            tokio::time::timeout(Duration::from_secs(2), async {
                                while source.hits() == 0 {
                                    tokio::time::sleep(Duration::from_millis(1)).await;
                                }
                            })
                            .await
                            .unwrap();
                            qwen::reset_quota_cooldown();
                            source.gate.store(true, Ordering::SeqCst);
                        };
                        let (response, ()) = tokio::join!(request, reset_after_send);
                        assert!(response.is_none());
                        assert!(qwen::quota_with_synthetic_key("synthetic").await.is_none());
                        assert_eq!(
                            source.hits(),
                            2,
                            "late pre-reset refusal must not block the retry"
                        );
                    })
                    .await
                    .unwrap();
            },
        )
        .await;
    })
    .await;
}

#[tokio::test]
async fn qwen_concurrent_test_scopes_do_not_share_refusals() {
    use crate::providers::qwen;
    let first = TlsServer::new(401, "{}");
    let second = TlsServer::new(401, "{}");
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();
    let a = Box::pin(qwen::with_isolated_quota_cooldown(async {
        crate::network_transport::with_test_transport(
            vec![("modelstudio.console.alibabacloud.com".into(), first.addr)],
            test_certificate(),
            async {
                runtime("qwen", Some("international:bearer"))
                    .run_account("qwen", &[], async {
                        assert!(qwen::quota_with_synthetic_key("same-synthetic")
                            .await
                            .is_none());
                        ready_tx.send(()).unwrap();
                        done_rx.await.unwrap();
                        assert!(qwen::quota_with_synthetic_key("same-synthetic")
                            .await
                            .is_none());
                    })
                    .await
                    .unwrap();
            },
        )
        .await;
    }));
    let b = Box::pin(qwen::with_isolated_quota_cooldown(async {
        ready_rx.await.unwrap();
        crate::network_transport::with_test_transport(
            vec![("modelstudio.console.alibabacloud.com".into(), second.addr)],
            test_certificate(),
            async {
                runtime("qwen", Some("international:bearer"))
                    .run_account("qwen", &[], async {
                        assert!(qwen::quota_with_synthetic_key("same-synthetic")
                            .await
                            .is_none());
                        assert!(qwen::quota_with_synthetic_key("same-synthetic")
                            .await
                            .is_none());
                    })
                    .await
                    .unwrap();
            },
        )
        .await;
        done_tx.send(()).unwrap();
    }));
    tokio::time::timeout(Duration::from_secs(3), async {
        tokio::join!(a, b);
    })
    .await
    .unwrap();
    assert_eq!(first.hits(), 1);
    assert_eq!(second.hits(), 1);
}

#[tokio::test]
async fn mode_change_stops_old_response_recreating_balance_after_generation_rebind() {
    let dir = std::env::temp_dir().join(format!("pane-mode-stale-balance-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let rt = runtime("stepfun", Some("china:api_key"));
    rt.run_account("stepfun", &[], async {
        let _mode_change = rt.suspend();
        crate::providers::forget_credit_baselines_in(&dir, &["stepfun".into()]).unwrap();
        // A subsequent cycle can replace this shared inflight-generation slot.
        // The old response still carries its own revoked account permit.
        crate::providers::bind_credit_meter_generation(
            "stepfun",
            crate::providers::credit_baseline_generation("stepfun"),
        );
        crate::providers::credit_meter_labeled_in(&dir, "stepfun", "$", 100.0, "Credits used", "");
        crate::providers::unbind_credit_meter_generation("stepfun");
        assert!(
            !crate::providers::credit_baselines_contain(&dir, &["stepfun".into()]),
            "revoked mode wrote an old balance under the new generation"
        );
    })
    .await;
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn actual_root_commands_reset_qwen_only_after_successful_key_mode_or_settings_mutation() {
    use crate::{mode_commands as commands, providers::qwen};
    let _serial = commands::SERIAL.lock().unwrap();
    let source = TlsServer::new(401, "{}");
    let rt = runtime("qwen", Some("international:bearer"));
    commands::setup(serde_json::json!({"accessPolicy": rt.snapshot().0}));
    qwen::with_isolated_quota_cooldown(crate::network_transport::with_test_transport(
        vec![("modelstudio.console.alibabacloud.com".into(), source.addr)],
        test_certificate(),
        async {
            async fn attempt(rt: &Arc<AccessRuntime>) {
                rt.run_account(
                    "qwen",
                    &[],
                    qwen::quota_with_synthetic_key("synthetic-stable-key"),
                )
                .await
                .unwrap();
            }
            attempt(&rt).await;
            attempt(&rt).await;
            assert_eq!(source.hits(), 1);
            commands::fail(true);
            assert!(commands::set_api_key("qwen".into(), "synthetic-stable-key".into()).is_err());
            attempt(&rt).await;
            assert_eq!(source.hits(), 1, "failed save must not reset cooldown");
            commands::fail(false);
            commands::set_api_key("zai".into(), "synthetic-other-key".into()).unwrap();
            attempt(&rt).await;
            assert_eq!(source.hits(), 1, "another provider's key is unrelated");
            commands::set_api_key("qwen".into(), "synthetic-stable-key".into()).unwrap();
            attempt(&rt).await;
            assert_eq!(
                source.hits(),
                2,
                "successful Qwen save resets even an unchanged key"
            );
            commands::set_api_key("qwen".into(), String::new()).unwrap();
            attempt(&rt).await;
            assert_eq!(source.hits(), 3, "successful Qwen clear resets");
            commands::fail(true);
            assert!(commands::reset_provider_access(()).await.is_err());
            attempt(&rt).await;
            assert_eq!(
                source.hits(),
                3,
                "failed settings reset must not reset cooldown"
            );
            commands::fail(false);
            commands::reset_provider_access(()).await.unwrap();
            attempt(&rt).await;
            assert_eq!(
                source.hits(),
                4,
                "successful settings reset resets cooldown"
            );
            commands::fail(true);
            assert!(commands::set_provider_region_inner("qwen", "china:bearer").is_err());
            attempt(&rt).await;
            assert_eq!(source.hits(), 4, "failed mode save must not reset cooldown");
            commands::fail(false);
            commands::set_provider_region_inner("qwen", "china:bearer").unwrap();
            attempt(&rt).await;
            assert_eq!(source.hits(), 5, "successful mode change resets cooldown");
            commands::set_provider_region_inner("qwen", "international:bearer").unwrap();
            attempt(&rt).await;
            assert_eq!(
                source.hits(),
                6,
                "away-and-back mode change cannot inherit a refusal"
            );
            commands::set_provider_region_inner("qwen", "international:bearer").unwrap();
            attempt(&rt).await;
            assert_eq!(source.hits(), 6, "saving an unchanged mode is a no-op");
        },
    ))
    .await;
    commands::finish();
}

#[test]
fn commandcode_consent_and_exact_billing_destinations() {
    let mut policy = AccessPolicy::default();
    assert!(!policy.allows_account("commandcode"));
    assert!(policy.set_family("commandcode", true).is_ok());
    policy.set_account("commandcode", true).unwrap();
    assert!(policy.allows_account("commandcode"));
    let endpoint = endpoint("commandcode", "global", "https://api.commandcode.ai");
    for path in ["/alpha/billing/credits", "/alpha/billing/subscriptions"] {
        assert!(validate_destination(
            &endpoint,
            &Url::parse(&format!("https://api.commandcode.ai{path}")).unwrap()
        )
        .is_ok());
    }
    for url in [
        "https://api.commandcode.ai/alpha/whoami",
        "https://api.commandcode.ai/alpha/usage/summary",
        "https://api.commandcode.ai/v1/chat/completions",
        "https://api.commandcode.ai/alpha/billing/credits?key=synthetic",
        "https://api.commandcode.ai.evil.test/alpha/billing/credits",
        "http://api.commandcode.ai/alpha/billing/credits",
    ] {
        assert!(
            validate_destination(&endpoint, &Url::parse(url).unwrap()).is_err(),
            "{url}"
        );
    }
}

#[tokio::test]
async fn commandcode_actual_adapter_is_bounded_read_only_and_preserves_partial_credit_success() {
    use crate::{network_transport::with_test_transport, providers};
    let credits = r#"{"credits":{"monthlyCredits":52.5,"purchasedCredits":0,"freeCredits":1},"windowLimits":{"fiveHour":{"used":3.5,"cap":14,"resetAt":1893474000000},"weekly":{"used":7,"cap":35,"resetAt":1894060800000}}}"#;
    for (status, body) in [
        (
            200,
            r#"{"data":{"planId":"individual-goat","currentPeriodEnd":1896134400000}}"#,
        ),
        (503, r#"{"error":"synthetic-key-never-real"}"#),
        (429, "{}"),
        (200, "not json"),
    ] {
        let source = TlsServer::scripted(200, credits, None, Some((status, body)));
        let rt = runtime("commandcode", None);
        let result = with_test_transport(
            vec![("api.commandcode.ai".into(), source.addr)],
            test_certificate(),
            rt.run_account("commandcode", &[], providers::commandcode::snapshot()),
        )
        .await
        .unwrap();
        assert_eq!(result.status, "ok");
        assert_eq!(source.hits(), 2);
        assert!(result
            .metrics
            .iter()
            .any(|m| m.label == "Monthly credits remaining"));
        let requests = source.requests.lock().unwrap();
        assert!(requests[0].starts_with("GET /alpha/billing/credits HTTP/"));
        assert!(requests[1].starts_with("GET /alpha/billing/subscriptions HTTP/"));
        for request in requests.iter() {
            assert!(request.contains("authorization: Bearer synthetic-key-never-real"));
            assert!(request.contains("user-agent:"));
            assert!(!request.contains("cookie:"));
        }
        if status != 200 || body == "not json" {
            assert!(result.warning.is_some());
            assert!(!result
                .metrics
                .iter()
                .any(|m| m.label == "Subscription period ends"));
        }
        if status == 429 {
            assert!(result
                .warning
                .as_ref()
                .unwrap()
                .contains("retry_after_s=120"));
        }
        assert!(!serde_json::to_string(&result)
            .unwrap()
            .contains("synthetic-key-never-real"));
    }
}
#[tokio::test]
async fn commandcode_failures_stop_before_subscription_and_never_follow_redirects() {
    use crate::{network_transport::with_test_transport, providers};
    let (target, url) = server();
    for (status, body) in [
        (401, "synthetic-key-never-real"),
        (403, "{}"),
        (429, "{}"),
        (500, "{}"),
        (307, "{}"),
        (200, "not json"),
        (200, r#"{"success":false,"credits":{"monthlyCredits":10}}"#),
    ] {
        let source = TlsServer::response(status, body, Some(&url));
        let rt = runtime("commandcode", None);
        let result = with_test_transport(
            vec![("api.commandcode.ai".into(), source.addr)],
            test_certificate(),
            rt.run_account("commandcode", &[], providers::commandcode::snapshot()),
        )
        .await
        .unwrap();
        assert_eq!(result.status, "error");
        assert_eq!(source.hits(), 1);
        if status == 429 {
            assert!(result.error.as_ref().unwrap().contains("retry_after_s=120"));
        }
        assert!(!serde_json::to_string(&result)
            .unwrap()
            .contains("synthetic-key-never-real"));
    }
    empty(&target);
}
#[tokio::test]
async fn commandcode_missing_wrong_or_revoked_grant_never_reads_key_or_sends() {
    use crate::providers;
    let reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    providers::DISCOVERY_READS
        .scope(reads.clone(), async {
            assert_eq!(providers::commandcode::snapshot().await.status, "error");
            runtime("zai", Some("international:api_key"))
                .run_account("zai", &[], async {
                    assert_eq!(providers::commandcode::snapshot().await.status, "error");
                })
                .await;
            let rt = runtime("commandcode", None);
            let permit = rt.permit("commandcode", &[]).unwrap();
            rt.invalidate();
            permit
                .run(async {
                    assert_eq!(providers::commandcode::snapshot().await.status, "error");
                })
                .await;
        })
        .await;
    assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 0);
    runtime("commandcode", None)
        .run_account("commandcode", &[], async {
            assert!(CheckedClient::new("commandcode", None)
                .post("https://api.commandcode.ai/alpha/billing/credits")
                .bearer_auth("synthetic")
                .send()
                .await
                .is_err());
        })
        .await;
}

#[tokio::test]
async fn commandcode_absent_and_midflight_changed_keys_fail_closed() {
    use crate::{network_transport::with_test_transport, providers};
    for (keys, expected_hits, expected_status) in [
        (vec![None], 0, "no_credentials"),
        (vec![Some("a"), Some("b")], 1, "error"),
        (vec![Some("a"), Some("a"), None], 2, "error"),
    ] {
        let source = TlsServer::scripted(
            200,
            r#"{"credits":{"monthlyCredits":5}}"#,
            None,
            Some((200, r#"{"data":{"planId":"individual-goat"}}"#)),
        );
        let keys =
            std::cell::RefCell::new(keys.into_iter().map(|v| v.map(str::to_string)).collect());
        let rt = runtime("commandcode", None);
        let snapshot = providers::KEY_SEQUENCE
            .scope(
                keys,
                with_test_transport(
                    vec![("api.commandcode.ai".into(), source.addr)],
                    test_certificate(),
                    rt.run_account("commandcode", &[], providers::commandcode::snapshot()),
                ),
            )
            .await
            .unwrap();
        assert_eq!(snapshot.status, expected_status);
        assert_eq!(source.hits(), expected_hits);
        assert!(snapshot.metrics.is_empty());
    }
}
#[tokio::test]
async fn commandcode_billing_payload_cap_stops_before_metadata() {
    let source = TlsServer::new(
        200,
        &format!(
            "{{\"credits\":{{\"monthlyCredits\":1}},\"padding\":\"{}\"}}",
            "x".repeat(70_000)
        ),
    );
    let rt = runtime("commandcode", None);
    let snapshot = crate::network_transport::with_test_transport(
        vec![("api.commandcode.ai".into(), source.addr)],
        test_certificate(),
        rt.run_account(
            "commandcode",
            &[],
            crate::providers::commandcode::snapshot(),
        ),
    )
    .await
    .unwrap();
    assert_eq!(snapshot.status, "error");
    assert_eq!(source.hits(), 1);
    assert!(snapshot.error.unwrap().contains("oversized"));
}
