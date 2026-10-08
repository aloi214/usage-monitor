# Pane Private privacy

This document applies to the private source fork, not upstream Pane binaries. Pane Private removes upstream telemetry and the upstream updater. It has no Pane account or analytics upload path. It still makes the provider requests you authorize, and its local HTTP API is not an authentication boundary.

**Verification scope:** the behavior below is implemented in this private source fork. See [verification limits](private-build.md#integration-status). This is a source-level description, not an exhaustive network audit or a claim of native Windows or rendered UI validation.

## Two independent permissions

### Account discovery and queries

New configurations grant no provider-family or account access. The **Queries** switch in **Platform management** permits that family’s supported account choices/discovery; a separate account switch permits that account’s queries and applicable token refresh. Turning the family off revokes all its account query/refresh permissions; turning it on again does not restore them. Saving a key, changing the region, opening Settings, or permitting local logs does not enable account queries. Account controls, modes, keys, and metrics/layout are in the same platform panel. Metric visibility, order, stars, and layout resets are display-only; they do not revoke permission.

Each account-query provider family has a separate **Auto-refresh quotas** scheduling preference. Claude and CommandCode default off; the other account-query families default on, without enabling any account grants. Explicitly bound extra accounts inherit their family’s choice, and enabled families reuse the global interval. Hermes is a local-log source and has no quota-auto switch. With a family off, automatic startup, popover, interval, settings, and post-reset quota refreshes skip it before credential, identity, or extra-account discovery reads; currently authorized saved results or a manual-query hint can still be published. Toggling this preference does not revoke permission, erase history, or advance successful-fetch times. Existing cache expiry and safety invalidation still apply. Already-sent requests may finish; subsequent queued automatic refreshes use the latest setting. See [per-provider auto-refresh](provider-auto-refresh.md).

A card’s ↻ queries only that exact authorized account; global Refresh / Ctrl+R queries all authorized accounts, including families whose auto-refresh is off. Manual requests still obey authorization and rate-limit cooldowns. Explicit directory identification and reset-credit actions remain separately authorized. Independent local-log permissions, public-price synchronization, and other explicit actions are unchanged; the quota switch is not an application-wide network or credential-read block.

CommandCode has its own default-off family/account permissions and uses only the key explicitly saved in its local settings. Saving it does not authorize a query. Its auto-refresh preference defaults off: the card’s ↻, global Refresh, or Ctrl+R can query it manually, or the user can enable automatic queries for an already-authorized account. When auto-refresh is off, automatic quota paths do not read its key. Both query modes use the same two fixed billing GET endpoints. It does not discover environment/CLI credentials, use OAuth, write CLI authentication files, or send model prompts. The saved key uses the private-file writer, not encryption. See [CommandCode setup and limits](commandcode-provider.md).

Extra Claude, Codex, and OpenCode accounts are identified only when you submit a particular account directory. The identification action reads that directory to bind an account; it does not recursively search home directories or automatically enable the discovered account.

Enabled account adapters may read their documented CLI/editor credential files, environment keys, saved keys, or Windows Credential Manager entries. This includes provider-specific fallback credential sources described in [providers.md](providers.md); account permission is not a promise that only a manually pasted key will be read.

### Exact-account refresh boundaries

A card’s refresh is narrowed to its complete bound account ID before credential, identity, or extra-account discovery work. Unknown/disabled targets are rejected. It does not trigger sibling-account queries, global log scans, or price downloads; independently scheduled or already-running work can still occur. Scope narrowing is not a permission revocation: the backend merges the target into the current authorized publication and preserves other accounts’ data, errors, and timestamps. Permission changes invalidate queued intent and are checked again before publication. See [scoped refresh](scoped-quota-refresh.md).

Refreshing a Kimi Code card queries only its plan. Folded Moonshot wallet values remain explicitly saved, with their own historical time and warning, without reading the wallet credential or querying Moonshot. Use global Refresh to update the folded wallet. Revoking the separate Kimi API permission removes wallet rows, source metadata, and wallet warnings from publication. The card’s plan authorization cannot authorize the wallet. Kimi Code and Kimi API also have separate auto-refresh preferences: turning off Kimi API prevents automatic nested wallet queries even when Kimi Code auto-refresh is on; saved authorized wallet history retains its own time. Turning off Kimi Code does not turn off independently authorized Kimi API automatic queries.

### Local log sources

Each supported tool has its own On/Off switch and **Default location / Custom locations** selection under **Local log sources**. New configurations start off with no granted roots. Turning on Default explicitly grants only the finite known log locations shown for that tool; the list is not a filesystem-wide search or account discovery. Custom Save replaces all selected roots for that source, including defaults, and preserves its current On/Off state. Saving invalid paths or failing to persist the change leaves the prior selection and authority unchanged, with an error; it never falls back to defaults. To stop existing reads while correcting a path, turn that source off. A local-only setup needs no remote account permission. Local scans do not harvest account credentials to label or assign logs to an account. Results retain tool/source provenance and disclose that account ownership is unverified. Model or billing-route classification is not identity proof.

Only the currently granted source directories authorize log reads. Supported directory environment variables determine known default candidates, but do not authorize scanning by themselves. Account directories and hidden cards do not grant log access. Default grants freeze the selected paths, including known locations not created yet; environment changes or a link retarget do not silently add a new target. A missing directory stays distinct from Off and can become readable if created at the same authorized path. Path availability checks and **Recheck locations** inspect directory metadata, not credentials or log contents; Found does not guarantee usable records.

Existing grants from the previous private version migrate to enabled Custom with all original roots retained, without adding defaults. Off removes the active grants while remembering locations; a full access reset clears them. Roots are canonicalized and checked again during access; revocation invalidates in-flight scans and their publication. One missing or invalid root is excluded without disabling other valid roots/sources. These checks reduce ordinary path escape and replacement risks; they are not an OS-level no-follow guarantee against every concurrent filesystem attack.

Cursor's authenticated usage CSV is a remote request: it requires Cursor account access and can be downloaded even with no local log sources enabled. Automatic CSV downloads also require Cursor auto-refresh to be on; off skips the credential/request path and can retain applicable authorized saved statistics. Global manual Refresh / Ctrl+R can still update them. Local scans continue under their independent grants. Source-only recomputation after price synchronization does not fetch this CSV and preserves its already reported costs. Hermes is local-only and requires a Hermes log-source grant, not a remote account login.

## Network behavior

The following describes implemented request categories, not a claim that every possible network effect has been exhaustively proved absent.

| Request category | Trigger and data |
|---|---|
| Authorized provider APIs | Enabled account refreshes and explicit account actions. The relevant provider credential and API parameters are sent to that provider's approved destinations. [Provider details](providers.md) |
| OAuth token endpoints | An authorized account needs refresh. This can exchange a refresh token and change the provider's credential generation even if local saving later fails. |
| Public model-price tables | Manual **Sync latest price catalog** action only. Fixed catalog URLs listed below; no account credential, usage, log content, or model-search payload. |
| Ollama | Explicit account permission and selected loopback origin; local version, installed-model, and loaded-model requests. |
| Antigravity local process | Explicit `local_process` selection plus account permission; process discovery and requests to a matching local service. No automatic cloud fallback. |
| Links opened by the user | Dashboard/status/source links open a browser and are subject to that browser's networking and session. |

No upstream update checks/downloads or daily analytics events are scheduled. One/New API and Sub2API no longer have active adapters, configured-site requests, or supported API entries.

Exact regional modes bind both region and credential type; requests do not try another region or alternate authentication header after a failure. Provider transport rejects unapproved origins and automatic redirects. Fixed same-provider endpoints can still serve different supported API functions. Local transport bypasses proxies. A configured outbound proxy affects remote requests and can observe network metadata; HTTPS does not make the destination or the fact of a connection invisible.

Manual price sources:

- `https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json`
- `https://models.dev/api.json`
- `https://robinebers.github.io/openusage/pricing_supplement.json`

Catalog download servers still see ordinary request/network information, including a source IP. The catalogs are public inputs, not signed billing records. Only a complete, validated set of all three sources replaces the local bundle. A failed download, validation, or write leaves the prior successful bundle and timestamp unchanged; unknown rates remain unknown, including aggregated “Other” rows. First run uses built-in prices when no valid bundle exists; legacy split catalog files are ignored until manual synchronization. Startup, routine quota refresh, log scanning, and missing models do not authorize a download.

## Data stored locally

Windows private-build configuration lives in `%APPDATA%\PanePrivate`. The fork does not automatically migrate, rename, or import `%APPDATA%\Pane` or `%APPDATA%\OpenUsage`.

- **Saved API keys:** provider-specific JSON files in the private configuration directory, protected using the private-file writer. File permissions restrict ordinary access; they are not encryption and do not protect against a compromised owner account or administrator.
- **Account bindings:** selected account directory, local identifier, and display name. Names can contain an email or organization. They are distinct from local log grants.
- **Query snapshots:** local copies of plan, quota, reset, and freshness data. Authorized stale snapshots may be shown with their last-success timestamp.
- **Log-derived caches:** file paths, model names, token/cost aggregates, time buckets, and parser checkpoints. The fragment format stores SHA-256 plus sample length instead of raw bounded head/tail bytes. Hashes are comparison indexes, not encryption or anonymization of the rest of the cache.
- **Price catalogs:** manually obtained public data and last-success metadata.
- **Credentials owned by other tools:** applicable OAuth refresh can replace a CLI's credential file, so this permission can affect the CLI's login. MiniMax mcode credentials remain read-only.

Caches are not a complete transcript archive, but they remain private data. Revoking access prevents current authorized use/publication; it is not a promise to securely erase every derived file, old backup, database copy, or filesystem history. Obsolete or malformed spend-cache files are invalidated without reading logs, including when no source is granted; removal is attempted and filesystem failures are reported. Ordinary removal does not erase copies outside this application's control. Do not publish configuration directories or raw diagnostic output.

## OAuth refresh safety and limits

Coordinated refresh covers Claude, Codex, Kimi Code, and Grok credential files: a per-source in-process lock, rereading inside that lock, a whole-file digest comparison before replacement, and an owner-restricted unique temporary file. It preserves unrelated JSON fields and does not create new backup copies or restore old backups to recover a login.

Refresh success followed by failed saving, cancellation with uncertain outcome, or reuse of a consumed rotating token can require signing in again. In-process safeguards stop automatic reuse of a known consumed/uncertain generation; they do not persist across an application restart. Cursor's refreshed credentials remain in memory rather than being written to its database. Antigravity's access-token cache is bound to its credential source; its raw keyring/refresh data is not newly persisted by that cache.

**There is no shared transaction or lock with official CLIs or other processes.** The compare-to-replace window, pathname races, identical-byte change-and-restore, and platform-specific aliases remain limitations. Available JWT/account fields are consistency checks, not independent identity attestation for opaque responses. Native Windows file behavior and concurrent use with real CLIs require separate validation.

## Local HTTP API

The running app starts a read-only listener on `127.0.0.1:6736`. It serves current authorized account snapshots, not raw credentials or local log files, and it does not perform an on-demand remote query. Disallowed or retired provider entries are excluded; revoked results are invalidated before later publication.

There is no authentication. Any process able to connect to this loopback port may see display names, plan labels, usage values, and reset times. Loopback prevents direct LAN access; it does not isolate users or programs sharing a computer. No CORS headers and a loopback `Host` check restrict browser access and DNS-rebinding attempts; they are not a substitute for authentication or protection against a local proxy. Details: [local-http-api.md](local-http-api.md).

## Review the source and the result

Useful entry points are [account policy](../src-tauri/src/access_policy.rs), [log policy](../src-tauri/src/scan_policy.rs), [source locations](../src-tauri/src/scan_sources.rs), [providers](../src-tauri/src/providers/), [pricing](../src-tauri/src/pricing.rs), [spend](../src-tauri/src/spend.rs), [HTTP API](../src-tauri/src/httpapi.rs), and [Tauri configuration](../src-tauri/tauri.conf.json). Passing isolated tests or reading these files does not prove every application path, dependency, OS behavior, or future edit safe. Build and test the exact revision you intend to use, and keep the [known limitations](../SECURITY.md) in view.
