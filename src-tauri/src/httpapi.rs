//! Local read-only HTTP API, Mac parity: GET http://127.0.0.1:6736/v1/usage
//! returns the latest snapshots in the original app's documented wire format
//! (docs/local-http-api.md), so scripts written for the Mac app work here too.

use crate::providers::Snapshot;
use serde_json::{json, Value};

pub(crate) fn forget_snapshots(ids: &[String]) {
    crate::usage_publications().retain(|id| !ids.iter().any(|removed| removed == id));
}
pub(crate) fn forget_disabled_snapshots(disabled: &[String]) {
    crate::usage_publications().retain(|id| !crate::card_is_disabled(id, disabled));
}
pub(crate) fn rename_snapshots(names: &std::collections::HashMap<String, String>) {
    crate::usage_publications().rename(names);
}

fn current_view(
    store: &crate::usage_publication::UsagePublication,
    runtime: &crate::access_policy::AccessRuntime,
) -> Value {
    Value::Array(
        store
            .read_with_published_at(runtime)
            .map(|(result, published_at_ms)| {
                let fetched_at = iso8601(published_at_ms);
                result
                    .snapshots
                    .iter()
                    .map(|snapshot| provider_json(snapshot, &fetched_at))
                    .collect()
            })
            .unwrap_or_default(),
    )
}

pub(crate) fn provider_json(s: &Snapshot, fetched_at: &str) -> Value {
    let lines: Vec<Value> = s
        .metrics
        .iter()
        .map(|m| {
            let mut line = if m.kind == "progress" {
                let mut line = json!({
                    "type": "progress",
                    "label": m.label,
                    "used": m.used_percent,
                    "limit": 100,
                    "format": { "kind": "percent" },
                    "resetsAt": m.resets_at.map(iso8601),
                    "periodDurationMs": m.period_ms,
                    "color": Value::Null,
                });
                if m.expires {
                    line["expires"] = json!(true);
                }
                if s.id.starts_with("sub2api@") {
                    line["value"] = json!(m.value);
                    line["subtitle"] = json!(m.detail);
                }
                line
            } else if m.kind == "resets" {
                // The credit list in `detail` carries per-credit ids —
                // internals that never leave the app. The wire gets the
                // count and the soonest expiry only.
                json!({
                    "type": "text",
                    "label": m.label,
                    "value": format!("{} available", m.value.as_deref().unwrap_or("0")),
                    "subtitle": Value::Null,
                    "resetsAt": m.resets_at.map(iso8601),
                    "color": Value::Null,
                })
            } else {
                json!({
                    "type": "text",
                    "label": m.label,
                    "value": m.value,
                    "subtitle": m.detail,
                    "resetsAt": m.resets_at.map(iso8601),
                    "color": Value::Null,
                })
            };
            if s.has_saved_wallet_metric(&m.label) {
                let history = s.wallet_history.as_ref().expect("saved wallet provenance");
                line["sourceAccountId"] = json!(history.source_account_id);
                line["fetchedAt"] = json!(history.fetched_at.map(iso8601));
                line["attemptedAt"] = json!(history.attempted_at.map(iso8601));
                line["stale"] = json!(true);
            }
            line
        })
        .collect();
    // fetchedAt is when the data was last successfully fetched — for a
    // snapshot restored after a failed refresh that is the ORIGINAL
    // success time, not this publish. The fallback (fresh successes,
    // error states) is the attempt time, which the status field
    // disambiguates from a success.
    let fetched_at = s
        .fetched_at
        .or(s.attempted_at)
        .map(iso8601)
        .unwrap_or_else(|| fetched_at.to_string());
    let mut output = json!({
        "providerId": s.id,
        "displayName": s.name,
        "plan": s.plan,
        "lines": lines,
        "fetchedAt": fetched_at,
        // Freshness for every provider — only display facts. Diagnostic
        // text stays Sub2API-only: its errors are whitelisted strings,
        // other families may embed remote error text that must not leak.
        "status": s.status,
        "stale": s.stale || s.attempt_failed || s.wallet_history.is_some(),
    });
    if s.id.starts_with("sub2api@") {
        output["error"] = json!(s.error);
        output["warning"] = json!(s.warning);
    }
    output
}

fn iso8601(epoch_ms: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(epoch_ms)
        .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_default()
}

/// Only loopback spellings may appear in the Host header. A missing header
/// is allowed (HTTP/1.0 scripts); a rebound hostname is not.
fn host_ok(host: Option<&str>) -> bool {
    let Some(host) = host else { return true };
    let host = host.trim().to_ascii_lowercase();
    let bare = host.strip_suffix(":6736").unwrap_or(&host);
    matches!(bare, "127.0.0.1" | "localhost" | "[::1]")
}

fn route_with(
    method: &tiny_http::Method,
    url: &str,
    store: &crate::usage_publication::UsagePublication,
    runtime: &crate::access_policy::AccessRuntime,
) -> (u16, String) {
    let path = url.split('?').next().unwrap_or(url);
    match method {
        tiny_http::Method::Get => {
            let view = current_view(store, runtime);
            if path == "/v1/usage" {
                (200, view.to_string())
            } else if let Some(id) = path.strip_prefix("/v1/usage/") {
                view.as_array()
                    .and_then(|items| {
                        items
                            .iter()
                            .find(|p| p.get("providerId").and_then(Value::as_str) == Some(id))
                    })
                    .map(|p| (200, p.to_string()))
                    .unwrap_or((404, json!({"error":"provider_not_found"}).to_string()))
            } else {
                (404, json!({"error":"not_found"}).to_string())
            }
        }
        _ => (405, json!({"error":"method_not_allowed"}).to_string()),
    }
}
fn route(method: &tiny_http::Method, url: &str) -> (u16, String) {
    route_with(
        method,
        url,
        crate::usage_publications(),
        crate::access_runtime(),
    )
}

/// Binds 127.0.0.1:6736 and serves until the app exits. If the port is
/// taken the API is silently unavailable this session (Mac parity).
pub fn start() {
    std::thread::spawn(|| {
        let server = match tiny_http::Server::http("127.0.0.1:6736") {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[pane] local API: port 6736 unavailable ({e}) — API off");
                return;
            }
        };
        eprintln!("[pane] local API: http://127.0.0.1:6736/v1/usage");
        for request in server.incoming_requests() {
            // DNS-rebinding guard: a page can point its own hostname at
            // 127.0.0.1, making this server "same-origin" in the victim's
            // browser — CORS never applies then. Legitimate local clients
            // address us by loopback names only; anything else is refused.
            let host = request
                .headers()
                .iter()
                .find(|h| h.field.equiv("Host"))
                .map(|h| h.value.as_str().to_string());
            let (status, body) = if host_ok(host.as_deref()) {
                route(request.method(), request.url())
            } else {
                (403, json!({"error": "forbidden_host"}).to_string())
            };
            let mut response = tiny_http::Response::from_string(body).with_status_code(status);
            // Deliberately NO Access-Control-Allow-Origin header: with
            // permissive CORS, any website the user visits could silently
            // read their usage data from this port. Browsers now block
            // cross-origin reads; scripts, widgets, and curl are unaffected
            // (CORS only constrains browsers). The Mac app allows "*" and
            // discloses it — we chose the stricter default.
            for (k, v) in [("Content-Type", "application/json")] {
                if let Ok(h) = tiny_http::Header::from_bytes(k.as_bytes(), v.as_bytes()) {
                    response.add_header(h);
                }
            }
            let _ = request.respond(response);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{host_ok, provider_json};
    use crate::providers::{Metric, ResetCredit, Snapshot};

    #[test]
    fn sub2api_public_projection_preserves_stale_and_display_amounts_only() {
        let mut snap = Snapshot::ok(
            "sub2api@http-state",
            "Site · Key",
            None,
            vec![Metric::progress("5h", 25.0, Some("$5.00 / $20.00".into()))],
        );
        snap.dashboard_url = Some("https://private.example.com".into());
        snap.stale = true;
        snap.warning = Some("HTTP 401".into());
        let output = provider_json(&snap, "2026-09-05T00:00:00Z");
        assert_eq!(output["stale"], true);
        assert_eq!(output["warning"], "HTTP 401");
        assert_eq!(output["status"], "ok");
        assert_eq!(output["lines"][0]["subtitle"], "$5.00 / $20.00");
        assert!(!output.to_string().contains("private.example.com"));
        let error = Snapshot::error("sub2api@http-state", "Site · Key", "HTTP 403".into());
        assert_eq!(provider_json(&error, "now")["error"], "HTTP 403");
        // status/stale are universal freshness fields now; diagnostic
        // error/warning text stays Sub2API-only (its errors are
        // whitelisted strings, other families may embed remote text).
        let onenewapi = provider_json(&onenewapi_snap(), "now");
        assert_eq!(onenewapi["status"], "ok");
        assert_eq!(onenewapi["stale"], false);
        assert!(onenewapi.get("error").is_none());
        assert!(onenewapi.get("warning").is_none());
    }

    #[test]
    fn resets_metric_serves_count_and_soonest_expiry_not_credit_ids() {
        let snap = Snapshot::ok(
            "codex",
            "Codex",
            None,
            vec![Metric::resets(
                2,
                Some(vec![
                    ResetCredit {
                        id: Some("cred-abc123".into()),
                        expires_at: Some(1_800_000_000_000),
                    },
                    ResetCredit {
                        id: Some("cred-def456".into()),
                        expires_at: Some(1_800_100_000_000),
                    },
                ]),
            )],
        );
        let output = provider_json(&snap, "2026-09-05T00:00:00Z");
        let line = &output["lines"][0];
        assert_eq!(line["type"], "text");
        assert_eq!(line["label"], "Rate Limit Resets");
        assert_eq!(line["value"], "2 available");
        assert_eq!(line["subtitle"], serde_json::Value::Null);
        assert_eq!(line["resetsAt"], "2027-01-15T08:00:00Z");
        let raw = output.to_string();
        for leak in ["cred-abc123", "cred-def456", "expires_at"] {
            assert!(!raw.contains(leak), "local HTTP leaked {leak}: {raw}");
        }
    }

    #[test]
    fn restored_snapshot_keeps_its_original_fetch_time() {
        let mut snap = Snapshot::ok(
            "codex",
            "Codex",
            None,
            vec![Metric::progress("Weekly", 25.0, None)],
        );
        snap.stale = true;
        snap.fetched_at = Some(1_800_000_000_000); // 2027-01-15T08:00:00Z
        let output = provider_json(&snap, "2026-09-05T00:00:00Z");
        assert_eq!(output["fetchedAt"], "2027-01-15T08:00:00Z");
        assert_eq!(output["stale"], true);

        let fresh = provider_json(
            &Snapshot::ok("codex", "Codex", None, vec![]),
            "2026-09-05T00:00:00Z",
        );
        assert_eq!(fresh["fetchedAt"], "2026-09-05T00:00:00Z");
        assert_eq!(fresh["status"], "ok");
        let failed = Snapshot::error("codex", "Codex", "timeout".into());
        let failed = provider_json(&failed, "2026-09-05T00:00:00Z");
        assert_eq!(failed["fetchedAt"], "2026-09-05T00:00:00Z");
        assert_eq!(failed["status"], "error");
        assert!(
            failed.get("error").is_none(),
            "remote error text must not leak for non-Sub2API"
        );
    }

    #[test]
    fn failed_attempt_within_grace_is_stale_on_the_wire() {
        // UI grace leaves `stale` false so the Outdated chip stays off.
        // The API still reports stale so a widget cannot treat restored
        // numbers as a live success.
        let mut snap = Snapshot::ok(
            "codex",
            "Codex",
            None,
            vec![Metric::progress("Weekly", 25.0, None)],
        );
        snap.fetched_at = Some(1_800_000_000_000);
        snap.attempt_failed = true;
        let output = provider_json(&snap, "2026-09-05T00:00:00Z");
        assert!(!snap.stale);
        assert_eq!(output["stale"], true);
        assert_eq!(output["fetchedAt"], "2027-01-15T08:00:00Z");
        assert_eq!(output["status"], "ok");
    }

    fn onenewapi_snap() -> Snapshot {
        let mut snap = Snapshot::ok(
            "onenewapi@abc",
            "Site · Key 1",
            None,
            vec![Metric::progress(
                "Usage",
                48.63,
                Some("$592.18 of $1217.82".into()),
            )],
        );
        snap.dashboard_url = Some("https://panel.example.com".into());
        snap
    }

    #[test]
    fn onenewapi_json_omits_origin_dashboard_and_secrets() {
        let snap = onenewapi_snap();
        let json = provider_json(&snap, "2026-07-26T00:00:00Z");
        assert_eq!(json["providerId"], "onenewapi@abc");
        assert_eq!(json["displayName"], "Site · Key 1");
        assert!(json.get("dashboardUrl").is_none());
        assert!(json.get("dashboard_url").is_none());
        assert!(json.get("baseUrl").is_none());
        assert!(json.get("origin").is_none());
        let raw = json.to_string();
        for leak in [
            "https://panel.example.com",
            "panel.example.com",
            "dashboard",
            "sk-",
            "apiKey",
            "api_key",
        ] {
            assert!(
                !raw.to_ascii_lowercase()
                    .contains(&leak.to_ascii_lowercase()),
                "local HTTP leaked {leak}: {raw}"
            );
        }
    }

    #[test]
    fn routes_read_only_current_authorized_publication() {
        use crate::access_policy::{AccessPolicy, AccessRuntime};
        use crate::usage_publication::UsagePublication;
        use std::sync::{Arc, Mutex};
        let cfg = serde_json::json!({"accessPolicy": {"version":1,"enabledFamilies":["kimi","moonshot"],"enabledAccounts":["kimi","moonshot"]}});
        let source = Arc::new(Mutex::new(AccessPolicy::from_config(&cfg)));
        let backend = source.clone();
        let runtime = AccessRuntime::with_checker(move || backend.lock().unwrap().clone());
        let store = UsagePublication::default();
        let revision = runtime.snapshot().1;
        let snapshot = Snapshot::ok(
            "kimi",
            "Kimi",
            None,
            vec![
                Metric::progress("Weekly", 25.0, None),
                Metric::progress("API", 60.0, None),
            ],
        );
        store.publish(&runtime, revision, vec![snapshot]).unwrap();
        let get = |path| super::route_with(&tiny_http::Method::Get, path, &store, &runtime);
        assert_eq!(get("/v1/usage/kimi").0, 200);
        assert!(get("/v1/usage").1.contains("API"));
        *source.lock().unwrap() =
            AccessPolicy::from_config(&serde_json::json!({"accessPolicy":"corrupt"}));
        runtime.snapshot();
        assert_eq!(get("/v1/usage").1, "[]");
        assert_eq!(get("/v1/usage/kimi").0, 404);
        assert_eq!(get("/v1/usage/onenewapi@abc").0, 404);
        assert_eq!(get("/v1/usage/sub2api@key").0, 404);
    }

    #[test]
    fn http_fallback_is_publish_time_and_stays_stable_across_gets() {
        use crate::access_policy::{AccessPolicy, AccessRuntime};
        use crate::usage_publication::UsagePublication;
        let runtime = AccessRuntime::new(AccessPolicy::from_config(&serde_json::json!({
            "accessPolicy": {"version":1,"enabledFamilies":["kimi","moonshot","codex"],
            "enabledAccounts":["kimi","moonshot","codex"]}
        })));
        let store = UsagePublication::default();
        let revision = runtime.snapshot().1;
        let error = Snapshot::error("kimi", "Kimi", "synthetic failure".into());
        let missing = Snapshot::no_credentials("moonshot", "Kimi API", "synthetic hint");
        let mut known = Snapshot::ok("codex", "Codex", None, Vec::new());
        known.fetched_at = Some(1_700_000_000_123);
        known.stale = true;
        let snapshots = vec![error, missing, known];
        let before_publish = chrono::Utc::now().timestamp();
        store
            .publish(&runtime, revision, snapshots.clone())
            .unwrap();
        let after_publish = chrono::Utc::now().timestamp();
        // The HTTP wire format has second precision. Waiting past a whole
        // second separates publication from both subsequent read times.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let get = |id: &str| {
            let (status, body) = super::route_with(
                &tiny_http::Method::Get,
                &format!("/v1/usage/{id}"),
                &store,
                &runtime,
            );
            assert_eq!(status, 200);
            serde_json::from_str::<serde_json::Value>(&body).unwrap()
        };
        let first = get("kimi");
        let first_missing = get("moonshot");
        let timestamp = chrono::DateTime::parse_from_rfc3339(first["fetchedAt"].as_str().unwrap())
            .unwrap()
            .timestamp();
        assert!(
            (before_publish..=after_publish).contains(&timestamp),
            "fallback timestamp must describe publication, not the GET time"
        );
        assert_eq!(first["fetchedAt"], first_missing["fetchedAt"]);
        assert_eq!(get("codex")["fetchedAt"], super::iso8601(1_700_000_000_123));
        std::thread::sleep(std::time::Duration::from_millis(1100));
        assert_eq!(get("kimi")["fetchedAt"], first["fetchedAt"]);
        assert_eq!(get("moonshot")["fetchedAt"], first_missing["fetchedAt"]);
        assert_eq!(get("codex")["fetchedAt"], super::iso8601(1_700_000_000_123));
        store.publish(&runtime, revision, snapshots).unwrap();
        assert_ne!(
            get("kimi")["fetchedAt"],
            first["fetchedAt"],
            "a new publication may advance the fallback attempt timestamp"
        );
        assert_eq!(
            get("codex")["fetchedAt"],
            super::iso8601(1_700_000_000_123),
            "publishing must never overwrite a known fetch timestamp"
        );
    }

    #[test]
    fn host_header_must_be_loopback() {
        // Loopback spellings, with and without the port.
        for good in [
            "127.0.0.1:6736",
            "127.0.0.1",
            "localhost:6736",
            "LOCALHOST",
            "[::1]:6736",
        ] {
            assert!(host_ok(Some(good)), "{good} should be allowed");
        }
        // Absent header (HTTP/1.0 scripts) stays allowed.
        assert!(host_ok(None));
        // A rebound hostname resolving to 127.0.0.1 is refused — this is
        // the DNS-rebinding case CORS can't catch.
        for bad in [
            "evil.example:6736",
            "evil.example",
            "127.0.0.1.evil.example:6736",
            "localhost.evil.example",
        ] {
            assert!(!host_ok(Some(bad)), "{bad} should be refused");
        }
    }
}
