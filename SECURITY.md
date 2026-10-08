# Pane Private security

This policy is for the private source fork, not upstream Pane releases. Read the [private build guide](docs/private-build.md), [privacy contract](docs/privacy.md), and [provider access reference](docs/providers.md) before allowing account or local-log access.

## Report privately

Do not publish credentials, logs, account identifiers, configuration directories, or exploitable details in a public issue. Report fork-specific problems privately to whoever supplied or maintains your copy of Pane Private, with a minimal redacted reproduction and the exact source revision.

If a problem also affects unmodified upstream Pane, its [security reporting page](https://github.com/ItsJazii/pane/security) is the upstream channel. That link does not establish support or a response-time commitment for this fork.

## Security boundaries

- Account access starts off. Family discovery and individual-account query/refresh permissions are separate backend decisions.
- Local log access starts off and is independently scoped to explicitly selected source directories. It does not require opening account credentials to infer ownership.
- Extra-account identification uses a single user-selected directory, not broad home-directory discovery.
- Private state uses `%APPDATA%\PanePrivate` on Windows and does not automatically import upstream state.
- Upstream telemetry and the updater are removed. One/New API and Sub2API are removed.
- The provider transport binds exact provider origins and selected regional/credential modes, disables automatic redirects, and requires explicit local modes for loopback services. It does not retry the same key in another region or try alternate authentication headers.
- The price-sync action downloads only fixed public catalog URLs when requested. It sends no account keys or log data. Catalog values are display inputs, not signed invoices.
- The local HTTP API checks permissions at read time, binds loopback, omits credentials, sends no CORS headers, and restricts Host. It has **no authentication**; local programs may read account display data.
- Tauri's content security policy and capability configuration constrain the webview. They do not replace backend authorization or guarantee that every dependency is harmless.

The [verification limits](docs/private-build.md#integration-status) distinguish implemented source protections from native/runtime evidence. Statements here are not certification of every network call or runtime path, and rendered UI interaction has not been validated.

## Credential replacement

Refresh coordination for Claude, Codex, Kimi Code, and Grok serializes the same source within this process, rereads under that lock, compares complete source bytes before replacement, and writes through a unique owner-restricted temporary file. Unrelated JSON fields are preserved. No new backup copies are created, and an old backup is not automatically restored as a valid login.

Failed saving after a successful remote refresh is a meaningful failure: the server may already have consumed the old refresh token. In-process generation guards stop known consumed or uncertain token reuse; signing in again may be necessary. These guards do not survive app restart. Cursor's refresh cache stays in memory; Antigravity's access-token cache is source-bound. MiniMax mcode credentials are read-only.

## Known limitations

- **No verified private Windows installer or native execution.** Frontend compilation, isolated tests, or a cross-target typecheck do not validate Windows packaging, tray/UI behavior, native ACLs, or real CLI coexistence.
- **No cross-process OAuth transaction.** The official CLI does not share this fork's lock. The final comparison-to-replacement window, path aliases, change-and-restore cases, and other concurrent filesystem races remain. There is no guarantee a refresh cannot disrupt a login.
- **No independent identity proof for opaque tokens.** Available account/JWT fields can catch inconsistencies; unverified claims or opaque refresh responses do not establish account ownership on their own.
- **No hostile-filesystem guarantee.** Canonical root checks and revocation fences reduce ordinary path escapes; they do not provide OS-level no-follow semantics across every read. Native Windows reparse behavior still needs validation.
- **Local data remains sensitive.** Owner-restricted files are not encryption. Paths, model names, usage aggregates, and parser checkpoints can remain in caches. SHA-256 fragment fingerprints avoid retaining raw head/tail samples; old or malformed caches are invalidated and removal is attempted without log reads, but this does not securely erase backups or filesystem history.
- **Local HTTP is readable by local clients.** A loopback binding and browser-origin restrictions do not isolate other processes or users able to connect on the same computer.
- **Network requests reveal metadata.** Providers, public catalog hosts, and configured proxies may observe ordinary connection information. Authorizing a provider does not eliminate its own logging or policies.
- **Local Antigravity uses an IDE loopback service.** Its local TLS compatibility path may accept the service's self-signed certificate; process/port matching is the intended boundary, not public-PKI identity verification. This does not apply to remote provider TLS.
- **Estimates are not account balances.** Public prices may be stale or wrong, some models have no rate, and local logs may omit other machines. Account quotas must not be inferred from logs with unverified account ownership.

## Updates and supported revisions

There is no automatic upstream update channel in Pane Private and no claim that an upstream release supports this fork. Review, rebuild, and test the exact private source revision you use. Keep the original MIT license and attribution with redistributed copies. Do not use upstream installation instructions as instructions for this build.
