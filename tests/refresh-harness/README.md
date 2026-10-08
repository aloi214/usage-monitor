# OAuth refresh fixtures

Run from the repository root:

```sh
cargo test --manifest-path tests/refresh-harness/Cargo.toml
```

This harness exercises the production `credential_refresh`, `private_file`,
`access_policy`, network policy/transport, and all six OAuth adapters. The build
script imports the production adapters and adds thin test entrypoints. It replaces
only Antigravity's Windows process-launch function with a panic on Linux. Provider
configuration, discovery roots, and Credential Manager reads are synthetic harness
boundaries. The refresh/storage algorithms are not duplicated in these tests.

Fixtures use temporary JSON/SQLite files, fake tokens, and local TLS servers.
The committed certificate/private key are intentionally public synthetic test
material, not credentials for any service. A test-only DNS/TLS route maps fixed
vendor hosts to these local listeners; production destination and permit checks
remain enabled. No production endpoint is contacted.

Covered cases include single-flight and normalized aliases, forced-refresh
generation deduplication, external rotation/rejection/deletion, unknown JSON
fields, copied refresh tokens, rotating and nonrotating responses, malformed or
identity-inconsistent responses, save failures and source-update recovery,
cancellation, revoked permits, private temporary ownership, Cursor token/CSV
binding, and Antigravity access-cache binding/save-error propagation.

The existing Antigravity `live_probe` test remains ignored. Do not run ignored
probes as part of this synthetic verification.

Linux results do not validate Windows replacement/sharing behavior or coexistence
with real CLI/Desktop clients. Native Windows tests include a destination sharing
violation and private DACL/replacement checks. Full application compilation and
client coexistence must be verified separately on Windows.
