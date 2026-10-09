# rice monitor local HTTP API

The running desktop app starts a read-only HTTP listener for local scripts and widgets. It exposes already-published, currently authorized account snapshots. Requests do not query a provider, refresh credentials, enable an account, or scan logs.

```text
GET http://127.0.0.1:6736/v1/usage
GET http://127.0.0.1:6736/v1/usage/claude
```

The collection is an array. A single-provider route returns an object. Use the exact `providerId` returned by the collection; explicitly identified additional accounts may have IDs such as `claude@<hash8>`, `codex@<hash8>`, or `opencode@<hash8>`.

One/New API and Sub2API are removed from this fork. Their former `onenewapi@...` and `sub2api@...` routes are unsupported and return `provider_not_found`; they are not included in the collection. Independent local-log estimates are not served by this account-snapshot API.

## Example and freshness

Illustrative data, not a real account:

```json
[{
  "providerId": "claude",
  "displayName": "Claude",
  "plan": "Example plan",
  "fetchedAt": "2026-10-07T01:30:00Z",
  "status": "ok",
  "stale": false,
  "lines": [{
    "type": "progress",
    "label": "Session",
    "used": 22.0,
    "limit": 100,
    "format": { "kind": "percent" },
    "resetsAt": "2026-10-07T04:39:59Z",
    "periodDurationMs": 18000000,
    "color": null
  }]
}]
```

- `status` describes the projected snapshot, such as `ok`, `error`, `no_credentials`, or `manual` (an authorized account with quota auto-refresh off and no retained query result). This is no longer limited to Claude or CommandCode. Always check `stale` as well: a saved result from any auto-refresh-off family, restored last-good snapshot, or Kimi plan with historical wallet data can have `status: "ok"` and `stale: true`. Historical data does not by itself mean that the latest plan query failed.
- `fetchedAt` is the last successful fetch time when one is available. Restoring a previous success preserves that time. A query without a successful result uses its recorded attempt time when available; cooldown/saved replays preserve that time. Legacy error/no-credentials/manual snapshots without either time can use publication time; a timestamp alone does not prove that a query was attempted or succeeded.
- Progress lines report percentages (`used`, `limit: 100`), optional reset time and period. A progress line with an expiry rather than a reset has `"expires": true`.
- Text lines have `value` and `subtitle`; unavailable fields may be `null`.
- Rate Limit Resets are a text line such as `"2 available"` with the soonest expiry in `resetsAt`. Credit identifiers and redemption controls are not exposed.
- The projection omits raw credentials, configuration objects, dashboard URLs, and raw remote errors. Display names can still identify an account or organization, and plan/usage values are private information.

## Quota auto-refresh preferences

The app’s [per-provider auto-refresh](provider-auto-refresh.md) preference controls automatic quota queries, not HTTP reads or account authorization. Claude and CommandCode default off; other account-query families default on. With auto-refresh off, this API can still expose currently authorized saved history or a manual-query hint. The toggle does not erase history or renew its timestamps; ordinary cache expiry and safety invalidation still apply. Reading this API never triggers the skipped query. Card/global manual refresh remains available in the app under the same account permission and cooldown rules.

## After an exact-account refresh

The card refresh is an app action, not a new HTTP route. It updates only its bound account and merges that result into the complete currently authorized publication. Other accounts’ values, errors, and timestamps are retained unless an authorization or safety change requires removal. A later HTTP read does not itself refresh any timestamps or query providers.

<a id="saved-wallet-lines"></a>
## Saved Moonshot wallet lines

A Kimi Code card can combine a newly queried plan with saved Moonshot wallet rows. Refreshing that card does not query Moonshot. Automatic Kimi plan refreshes also preserve saved wallet history when Kimi API (`moonshot`) auto-refresh is off. The Kimi and Moonshot preferences do not authorize each other. Such wallet lines include these additional fields:

```json
{
  "type": "text",
  "label": "Balance",
  "value": "example saved balance",
  "sourceAccountId": "moonshot",
  "fetchedAt": "2026-10-07T01:00:00Z",
  "attemptedAt": "2026-10-07T01:00:00Z",
  "stale": true
}
```

This is an illustrative excerpt, not real account data or a complete response. `fetchedAt` and `attemptedAt` belong to the wallet’s saved success and attempt respectively; either can be `null` when unknown. The top-level `fetchedAt` is not proof that every line was queried at that time. Saved wallet provenance makes top-level `stale` true even when the plan query succeeds. Raw wallet warnings/errors are not exposed by this API.

Use global **Refresh / Ctrl+R** in the app to query the folded wallet under its separate Kimi API permission. Revoking that permission removes its wallet rows and provenance; retaining Kimi plan permission does not retain wallet access. See [单账号额度刷新](scoped-quota-refresh.md).

## Authorization and errors

The current backend permission revision filters every read. Disabled, unknown, or revoked accounts are absent and return 404 on the individual route. A permission change can clear the published view until the next authorized refresh, so an empty array is not proof of zero usage. Historical/retired snapshots are not permission to republish data.

| Situation | HTTP status and JSON |
|---|---|
| Missing/disabled provider | `404 {"error":"provider_not_found"}` |
| Unknown path | `404 {"error":"not_found"}` |
| Method other than GET | `405 {"error":"method_not_allowed"}` |
| Non-loopback Host header | `403 {"error":"forbidden_host"}` |

The API cannot redeem credits or change any settings. Use normal app controls for those actions.

## Security boundaries

- **Loopback binding:** `127.0.0.1:6736`, not a LAN listener. The `[::1]` Host spelling being accepted does not mean the server binds IPv6.
- **No authentication:** local processes, and other users able to connect on the same machine, may read these snapshots. Do not forward this port or expose it through a proxy to untrusted clients.
- **No CORS headers:** ordinary browser cross-origin reads are restricted. Native clients such as curl and PowerShell are unaffected.
- **Host check:** accepted names are `127.0.0.1`, `localhost`, and `[::1]`, with or without `:6736`; a missing Host is also accepted. Other aliases are rejected. This helps against DNS rebinding but does not authenticate the caller.
- **Port collision:** if binding fails, this app's API is unavailable for the session and an error is written to stderr. Another program may own the port, so do not assume a response belongs to rice monitor without checking the process.

See [privacy](privacy.md) and [build verification status](private-build.md#integration-status). These source-level properties do not claim a completed native Windows or adversarial network audit.
