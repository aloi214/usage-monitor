# Provider quota scheduling fixtures

Run from the repository root:

```sh
cargo test --locked --manifest-path tests/usage-harness/Cargo.toml
```

The build script compiles the unchanged production `fetch_usage`, `cached_usage`,
`guarded`, `change_access_policy`, `fetch_spend`, config persistence/normalization,
cache helpers and refresh-reason type from
`src-tauri/src/lib.rs`, plus the HTTP snapshot projection from `httpapi.rs`.
Production access-policy, snapshot and usage-publication modules are imported
directly. Provider I/O, native notifications, configuration location and credit
meter effects are synthetic boundaries. No application, actual credential,
session log, network connection or real provider endpoint is opened.

Tests count provider queries, credential/discovery/identity boundary calls,
exercise omitted/automatic/explicit IPC reasons, retained older manual data,
last-success versus attempt timestamps, real error retention across unrelated
access edits, Retry-After cooldown, disabled accounts, revoke/re-enable and
revocation during an in-flight manual query. Whole-module frontend
event routing and queue races are covered separately in
`scripts/settings-boot.test.mjs`. Actual OAuth and transport behavior remains in
the existing refresh/network harnesses.

This portable harness is not a Windows/WebView2 runtime or real-account test.

Per-family automatic preferences are tested through the actual usage commands:
old-schema defaults, partial/malformed/unknown preferences, persistence without
changing authorization, inherited extra accounts, selected/global manual intent,
zero credential/discovery/identity/provider I/O for automatic-off families, queued
intent and in-flight follow-on gates, historical cache clocks, independent Kimi
and Moonshot wallet behavior, and Cursor CSV suppression/replay hints while local
collection continues. Cursor export and local scan I/O are synthetic counted
boundaries; no live provider, user credential, log directory, or native app runs.
