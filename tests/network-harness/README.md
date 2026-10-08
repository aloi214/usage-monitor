# Portable destination/transport tests

Run from this checkout:

```
cargo test --manifest-path tests/network-harness/Cargo.toml --offline
```

This Linux-capable crate imports the actual `access_policy.rs`, `snapshot.rs`, `scan_policy.rs`, `network_policy.rs`, and `network_transport.rs`. Its build script compiles unchanged sections of shared provider types/body readers/accounting helpers and imports the actual Z.ai, MiniMax, Moonshot, Qwen, and StepFun files. Only application configuration, key discovery, and local-spend boundaries are synthetic. The real scan policy gates the integrated MiniMax ledger helpers and their synthetic-file tests. Qwen's quota-only test entry avoids reading the host's local log directory. No actual credential or user log is read.

HTTP listeners use random loopback ports. TLS fixtures use a generated, explicitly nonproduction certificate/key; the private key is intentionally a public synthetic test fixture and must never be used for a real service. A `cfg(test)` task-local DNS/trust-root hook directs the exact official test URLs to these listeners. It does not alter provider/region validation, credential construction, redirects, or authorization checks. Release builds contain no hook and cannot configure alternate remote origins.

Coverage includes all retained official origin records, five actual regional adapter failure paths (including transport failure), no sibling requests, single-header Qwen selection, explicit StepFun credential type, missing-region-before-key-discovery, revocation, nested account contexts, exact IPv4/IPv6/localhost origin and port, proxy bypass, Antigravity PID listener ownership, all redirect status families, JSON/form/binary/header/cookie/CSRF payloads, and fixed credential-free public prices.

This is not a Windows native build, Tauri IPC/UI test, live provider compatibility test, or process-discovery smoke test. The harness is integrated with the extracted snapshot model and source-consent policy; keep these production modules directly referenced when adding coverage.

Qwen cooldown tests use task-local isolated instances of the real production cooldown state for complete request operations. Repeated-request tests explicitly share one isolated scope. Coverage verifies credential/mode rebinding, switch-back invalidation, late-response reset safety, and concurrent test-scope independence. The backend must call `providers::qwen::reset_quota_cooldown()` when successfully changing/clearing its key, changing its mode, or resetting settings; those UI/command integrations remain outside this harness.
