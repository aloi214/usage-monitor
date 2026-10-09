# rice monitor providers and access modes

rice monitor has **23 provider families**: the previous **22 are retained**, with experimental CommandCode added. One/New API and Sub2API are removed, including their site/key management and supported local API entries. Retaining a provider does not mean granting it permission or guaranteeing that its undocumented vendor endpoints still work.

**Verification scope:** these access modes and safeguards are implemented in the private source fork. See [verification limits](private-build.md#integration-status). No real-account, native Windows, or rendered UI validation is implied by this reference.

## Before connecting

- New configurations enable no provider discovery, account queries, or local log sources.
- Open the side-bar **☰ Platform management** entry. Each provider expands to show accounts, **Region / connection mode**, applicable keys, and **Metrics & layout**. Turn on its header **Queries** switch, then authorize the specific accounts. Turning the header on alone grants no account; turning it off revokes all that family’s account queries and refreshes. Saving a key or choosing a mode does not grant permission.
- Each account-query platform has a separate **Auto-refresh quotas** switch, shared by its extra accounts and the global interval. Claude and CommandCode default off; other account-query families default on. No account is authorized by this preference. Off keeps applicable saved history/manual hints and skips automatic quota credential/identity/discovery work; authorized card/global manual refresh remains available. Already-sent requests may finish. Hermes has no quota-auto switch. See [the Chinese guide](provider-auto-refresh.md).
- An enabled account may read the documented existing CLI/editor credential source as well as a pasted key. This account permission includes applicable token refresh; see the limits below.
- Extra Claude, Codex, and OpenCode accounts require **Identify directory** on one exact account directory, followed by a separate account enable action. They are not found by broadly scanning home directories.
- Local spend requires an independent tool switch under **Settings → Local log sources**, with **Default location** or saved **Custom locations**. Defaults are finite known log paths, not broad discovery. Custom Save replaces the full selection without adding defaults and does not turn an off source on. An invalid save preserves the old grant and reports an error, with no fallback. Query permission and local directory permission do not imply each other. Logs are grouped by source/model/route with account ownership unverified, never matched to credentials harvested for that purpose.
- Metric visibility, order, stars, and layout resets only change the display. Revoke account or source access with the corresponding permission switches; hiding a metric does not stop queries.
- Pasted keys are stored under `%APPDATA%\PanePrivate\<provider>.json` on Windows. They are not sent to a Pane service. The separate private directory does not import upstream Pane/OpenUsage state.

The destinations below are a guide to the adapter source, not an exhaustive independent network proof. Provider transport permits approved HTTPS origins, rejects redirects, and rechecks the current account/mode before sending. A provider may use several fixed paths or hosts within one approved mode; that is different from trying a rejected key in a different region or with another authentication header.

## Exact-account refresh

An authorized account card’s **↻** queries its exact account ID, including an explicitly bound extra Claude, Codex, or OpenCode account. It does not query sibling/default accounts, start global log scanning, or synchronize public prices. Scope is validated before credential/identity reads or extra-account discovery; existing independent background work may still run. The backend returns the complete authorized merged snapshot, preserving untouched data, errors, and times subject to current permission filtering. Source-only cards have no quota-refresh button. Hermes remains local-statistics-only and has no remote quota endpoint. Global **Refresh / Ctrl+R** remains available. See the [Chinese usage guide](scoped-quota-refresh.md).

## Region and credential selection

The following values identify exact modes. Choose the region where the credential was issued; do not rely on region inference from the key or local machine. Missing/invalid selection blocks the regional operation.

| Provider | Exact selection | Selected origin(s) |
|---|---|---|
| Z.ai | `international:api_key` / `china:api_key` | `https://api.z.ai` / `https://open.bigmodel.cn` |
| Kimi API (`moonshot`) | `international:api_key` / `china:api_key` | `https://api.moonshot.ai` / `https://api.moonshot.cn` |
| MiniMax key | `international:api_key` / `china:api_key` | `https://api.minimax.io` / `https://api.minimaxi.com` |
| MiniMax Code login | `international:mcode` / `china:mcode` | International: `https://agent.minimax.io`, `https://platform.minimax.io`; China: `https://agent.minimaxi.com`, `https://www.minimaxi.com` |
| Qwen Code | Region `international` or `china`, paired with `bearer`, `x_api_key`, or `dashscope_api_key` | `https://modelstudio.console.alibabacloud.com` / `https://bailian.console.aliyun.com` |
| StepFun | Region `international` or `china`, paired with `api_key` or `plan_key` | `https://api.stepfun.ai` / `https://api.stepfun.com` |
| Antigravity | `cloud` / `local_process` | Fixed Google Cloud Code/OAuth hosts, or the identified local Antigravity process |
| Ollama | `local:<origin>` | One exact loopback origin, for example `local:http://127.0.0.1:11434` |

Qwen's complete selections include, for example, `china:x_api_key` or `international:bearer`; each sends only its selected header. StepFun's `api_key` is a wallet key and `plan_key` is the explicitly chosen Step Plan path. No sibling region, credential type, or authentication header is attempted after a refusal.

Antigravity defaults to cloud mode when no choice has been saved, but still requires account authorization. Its local-process mode must be explicitly selected. Other listed regional providers and Ollama require a saved valid choice. Mode changes clear relevant old display/cache state and require fresh results; selecting a mode while access is off does not read credentials or enable queries.

## 1. Claude (`claude`)

- **Default-manual quota queries:** **Auto-refresh quotas** starts off for Claude. Use an account card’s **↻** for that exact bound account, or global **Refresh / Ctrl+R** for all authorized accounts. With auto-refresh off, startup, popover, interval, settings, and post-reset quota refreshes skip Claude before credential/identity/discovery reads. Enable the separate preference to query already-authorized Claude accounts automatically on the shared interval. Manual queries ignore the preference but still respect permission and cooldown. Explicit directory identification and reset-credit redemption keep their separate authorization; applicable token refresh remains available when an authorized operation needs it.
- **Credential source:** the default Claude Code `.credentials.json` under `CLAUDE_CONFIG_DIR` or `%USERPROFILE%\.claude`; explicit extra directories use their selected `.credentials.json` and account metadata.
- **Requests:** usage/profile and reset-credit operations at `https://api.anthropic.com`; OAuth refresh at `https://platform.claude.com/v1/oauth/token`. Redeeming a reset is an explicit app action, not a local-API feature.
- **Displays:** session/weekly limits, supported per-model windows, extra usage, cloud-session credits, and reset credits when supplied by the API.
- **Saved results:** before the first query, the card asks for a manual refresh. Later automatic refreshes keep the last saved successful numbers and original success time, including across restarts, with a manual/saved-result notice. Saved account details are not reverified until a manual query; they may reflect a prior CLI login. A real failed manual query retains the saved numbers with its warning. Revocation and relevant configuration changes still invalidate cached access normally.
- **Local statistics:** separately authorize the Claude projects log tree. Logs do not establish the selected Claude account's spending or weekly capacity.

## 2. Codex (`codex`)

- **Credential source:** `auth.json` under `CODEX_HOME` or `%USERPROFILE%\.codex`; extra directories must be explicitly identified and enabled.
- **Requests:** `https://chatgpt.com/backend-api/wham/usage`, related reset-credit routes, and OAuth refresh at `https://auth.openai.com`. Credit consumption requires the explicit redemption action for the selected account.
- **Displays:** session/weekly windows, supported additional limits, balances, and reset credits.
- **Local statistics:** separately authorize a Codex home or rollout sessions tree, including an explicitly selected synced copy. Local and child-session deduplication does not prove account ownership or completeness across devices. No credential file is needed to label a permitted log scan.

## 3. Cursor (`cursor`)

- **Credential source:** Cursor's `state.vscdb`, normally `%APPDATA%\Cursor\User\globalStorage\state.vscdb`.
- **Requests:** fixed usage/plan RPCs at `https://api2.cursor.sh` and supported dashboard/session endpoints at `https://cursor.com`, including the usage CSV export. Same-provider dashboard fallbacks remain distinct from regional/header probing.
- **Displays:** available plan, credit, allowance, and CSV-derived spend fields. The CSV's reported costs are retained; manual catalog synchronization neither fetches a new CSV nor reprices/overwrites those amounts.
- **Important:** CSV spend is an authenticated network download under Cursor account permission, not a local-log source. Automatic CSV downloads also obey the Cursor auto-refresh setting; off preserves applicable saved statistics while local scans continue. Use global manual Refresh / Ctrl+R to update the authorized CSV. Refresh credentials stay in memory; Pane does not write refreshed Cursor tokens to its state database. Token/CSV caches are bound to the selected source and account identity.

## 4. OpenCode (`opencode`)

- **Credential source:** the `opencode-go` entry in `auth.json` under its default data directory, normally `%USERPROFILE%\.local\share\opencode`; an extra profile requires exact-directory identification and its own permission.
- **Requests:** `https://opencode.ai/zen/go/v1/usage` for account-wide Go windows.
- **Displays:** server-reported session, weekly, and monthly usage. Local database estimates are not silently substituted as authenticated account quotas.
- **Local statistics:** separately grant the directory containing `opencode.db`; its data stays in local statistics with unverified account ownership.

## 5. GitHub Copilot (`copilot`)

- **Credential source:** Copilot editor `apps.json`/`hosts.json`, or GitHub CLI's `github.com` token from Windows Credential Manager/legacy `hosts.yml`.
- **Requests:** `https://api.github.com/copilot_internal/user`.
- **Displays:** returned plan/credits/quota. A GitHub Enterprise token is not a credential for this `api.github.com` adapter.

## 6. Grok (`grok`)

- **Credential source:** `%USERPROFILE%\.grok\auth.json`.
- **Requests:** billing/settings/subscription at `https://cli-chat-proxy.grok.com`, reset information at `https://grok.com`, and token refresh at `https://auth.x.ai`.
- **Displays:** available plan, weekly pool, pay-as-you-go cap, and reset credits. Local spend requires a separate Grok log-directory grant.

## 7. Devin (`devin`)

- **Credential source:** supported Devin `credentials.toml` locations, including `%APPDATA%\devin`, the user's `.local\share\devin`, and `%LOCALAPPDATA%\devin`.
- **Requests:** the fixed `https://server.codeium.com` service for supported auth/status RPCs. A server URL in a credential file does not authorize arbitrary destinations.
- **Displays:** reported quota, balance, and plan. Separately granted `sessions.db` supplies local CLI estimates; it cannot account for cloud-only Devin sessions.

## 8. MiniMax (`minimax`)

- **Key mode:** a pasted key, `MINIMAX_API_KEY`, or `provider.minimax.options.apiKey` from `%USERPROFILE%\.minimax\config.yaml`. It queries the chosen regional host's token-plan/coding-plan remains paths using the selected key mode; both supported paths may be tried on that same host.
- **MiniMax Code sign-in mode:** only the selected region's `.minimax\auth\prod\<en|cn>\mcode-public\auth.json` and `.minimax\cli-auth\prod\<en|cn>\account-identity.json`. Only a usable unexpired mcode login is selected. Pane does not refresh or write mcode's credential file.
- **Requests:** only the mode/region hosts listed above. mcode uses its signed matrix/workspace flow and coding-plan remains endpoint. An unavailable login does not fall back to a key or the other region.
- **Displays:** available plan/window/video allowance. Local MiniMax databases require their own directory grants.

## 9. OpenRouter (`openrouter`)

- **Credential source:** pasted key, `OPENROUTER_API_KEY`, or OpenCode's OpenRouter auth entry, under OpenRouter account permission.
- **Requests:** `https://openrouter.ai/api/v1/credits` and `/api/v1/key`.
- **Displays:** available balance, credit use, and key limit. A locally logged OpenRouter route is not an authorization to read its key.

## 10. Z.ai (`zai`)

- **Credential source:** pasted key, `ZAI_API_KEY`/`GLM_API_KEY`, or `%USERPROFILE%\.config\zai\key.json`.
- **Requests:** quota and subscription endpoints on the explicitly selected `api.z.ai` or `open.bigmodel.cn` origin.
- **Displays:** supported usage windows, search quota, and plan. Failure on one region does not probe the other.

## 11. Antigravity (`antigravity`)

- **Cloud mode:** Windows Credential Manager's `gemini:antigravity` entry; OAuth at `https://oauth2.googleapis.com`, quota requests at `https://daily-cloudcode-pa.googleapis.com` and `https://cloudcode-pa.googleapis.com`. It does not start local process discovery in this mode.
- **Local process mode:** explicit choice plus account permission permits inspecting running process command lines and listening ports to identify Antigravity's language server. The local CSRF token is used only with that matching process's loopback service. It is not an arbitrary port scanner and it does not fall back to cloud credentials.
- **Displays:** available Gemini/Claude pools and plan. If the selected mode is unavailable, choose a different mode deliberately or start/sign in to the official app.
- **Limit:** local TLS compatibility can accept the IDE service's self-signed certificate; native process/port binding remains subject to the [security limitations](../SECURITY.md).

## 12. DeepSeek (`deepseek`)

- **Credential source:** pasted key or `DEEPSEEK_API_KEY`.
- **Requests:** `https://api.deepseek.com/user/balance`.
- **Displays:** balance and a local high-water-mark credits-used meter. That meter is not a provider-reported subscription allowance.

## 13. Kimi API (`moonshot`)

- **Credential source:** pasted platform/wallet key, `MOONSHOT_API_KEY`, or `KIMI_API_KEY`.
- **Requests:** `/v1/users/me/balance` on the explicitly selected Moonshot region.
- **Displays:** wallet balance, available cash/vouchers, and a local high-water-mark meter. Kimi API permission and region are required even if wallet rows are displayed inside a Kimi Code card. A Kimi Code card refresh does not query this wallet; use global **Refresh / Ctrl+R** to update folded wallet rows. A separately displayed Kimi API card targets `moonshot` only. Its auto-refresh choice is independent of Kimi Code; off blocks automatic wallet requests, including nested Kimi requests, while preserving applicable authorized wallet history.

## 14. ElevenLabs (`elevenlabs`)

- **Credential source:** pasted key or `ELEVENLABS_API_KEY`.
- **Requests:** `https://api.elevenlabs.io/v1/user/subscription`.
- **Displays:** character allowance and reset information returned by the API.

## 15. Ollama (`ollama`)

- **Credential source:** none. Account permission is still required to make local service requests.
- **Selection:** one explicit loopback origin, using `127.0.0.1`, `[::1]`, or `localhost`; common presets use port 11434. A custom http(s) origin can select another port. Paths, query strings, fragments, embedded credentials, LAN addresses, and remote hosts are rejected. `localhost` is pinned to loopback by the transport.
- **Requests:** `/api/version`, `/api/tags`, and `/api/ps`, only at the selected origin, without proxies or redirects. No alternate host/port discovery.
- **Displays:** version, installed models, and loaded models.

## 16. Codebuff (`codebuff`)

- **Credential source:** pasted key, `CODEBUFF_API_KEY`, or `%USERPROFILE%\.config\manicode\credentials.json` from `codebuff login`.
- **Requests:** `https://www.codebuff.com/api/v1/usage` and `/api/user/subscription`.
- **Displays:** credits, weekly limit, and plan.

## 17. Kilo (`kilo`)

- **Credential source:** pasted key, `KILO_API_KEY`, or `%USERPROFILE%\.local\share\kilo\auth.json`.
- **Requests:** credit-block and Kilo Pass RPCs at `https://app.kilo.ai`.
- **Displays:** credit blocks, Pass window, and tier when returned.

## 18. AihubMix (`aihubmix`)

- **Credential source:** pasted key, `AIHUBMIX_API_KEY`, or OpenCode's AihubMix auth entry.
- **Requests:** `https://aihubmix.com/v1/dashboard/billing/subscription` and `/v1/dashboard/billing/usage`.
- **Displays:** usage and available spending limit. This built-in provider is retained; removing generic One/New API does not remove AihubMix. Log-derived amounts remain separate estimates.

## 19. Qwen Code (`qwen`)

- **Credential source:** pasted Coding Plan key, `BAILIAN_TOKEN_PLAN_API_KEY`, or `DASHSCOPE_API_KEY`.
- **Requests:** the Coding Plan quota RPC on the selected international or China console, with exactly one selected header: Bearer authorization, `x-api-key`, or `x-dashscope-api-key`.
- **Displays:** reported 5-hour/weekly/monthly quota when supported. Refusal does not probe another region, header, or model endpoint. A refusal cooldown is scoped to the account/key/mode context.
- **Local statistics:** separately grant the Qwen usage directory for request/token statistics. These are not an authenticated fallback quota or proof of billing-account ownership.

## 20. Hermes (`hermes`)

- **Credential source and network:** none. Hermes is retained as a local statistics source.
- **Reads:** `state.db` only inside a separately granted Hermes directory.
- **Displays:** available recent models, billing routes, sessions, and estimated local spend. Merely enabling an account-style Hermes card does not grant database access. A route label or custom URL in the ledger does not cause a request to that URL.

## 21. Kimi Code (`kimi`)

- **Credential source:** the official CLI's `credentials/kimi-code.json` under `KIMI_CODE_HOME`, `%USERPROFILE%\.kimi-code`, or the supported `.kimi` fallback; alternatively a pasted Kimi For Coding plan key in private settings. The CLI login is preferred when available; the pasted key is distinct from a Kimi API wallet key.
- **Requests:** `https://api.kimi.com/coding/v1/usages`; OAuth login may also query `/coding/v1/me` and refresh via `https://auth.kimi.com/api/oauth/token`. A pasted plan key does not query the OAuth-only profile path.
- **Displays:** supported session/weekly limits and membership name. Optional wallet rows use the independently authorized **Kimi API** account and its selected region; enabling Kimi Code alone does not permit that wallet request.
- **Auto-refresh:** Kimi Code and Kimi API (`moonshot`) use separate switches and account grants. Kimi Code automatic queries cannot read/query a wallet whose own auto-refresh is off. A Kimi Code switch set to off does not disable independently authorized Kimi API automatic queries.
- **Card refresh:** queries the Kimi plan only, without Moonshot credential reads or requests. Existing authorized wallet values, errors, and wallet timestamps remain explicitly marked as saved; unknown wallet time remains unknown. Revoking Kimi API permission removes those rows and their wallet metadata/warning. A new plan success cannot freshen the wallet.
- **Local statistics:** `wire.jsonl` sessions require a separate Kimi source grant. Routed Kimi usage in another tool's logs retains that producing source's grant and does not establish Kimi login ownership.

## 22. StepFun (`stepfun`)

- **Credential source:** pasted key or `STEPFUN_API_KEY`/`STEP_API_KEY`. Wallet mode may additionally read Step Code `platform_*` credentials from `STEP_CODING_AGENT_DIR/auth.json`, `.stepcode/agent/auth.json`, or `.stepcode/auth.json`. Plan mode does not infer a credential type from those platform entries.
- **Wallet mode:** `/v1/accounts` on the chosen region; displays balance, vouchers, and supported credits-used information in the region's currency.
- **Step Plan mode:** `/step_plan/v1/models` on the chosen region. Success is not proof of a subscribed tier or remaining plan quota. Local estimated usage belongs to separately granted log sources and is not converted into an authoritative account allowance.
- **Local Plan Credits:** a text-only estimate computed from authorized source logs and a separately configured, unverified tier preference. Logs may include wallet/non-plan calls. Incomplete scans or missing model prices make the estimate unavailable. This estimate and tier are not published as account quota, local HTTP account data, tray progress, or quota alerts.
- **No fallback:** a wallet-key refusal does not trigger Plan-key probing or the other region. Step Code session directories are independently granted even when its credential is used for account queries.

## 23. CommandCode (`commandcode`, experimental)

- **Setup and permission:** paste a key only in CommandCode’s local API Key field in Platform management. The family and account grants start off and remain separate from saving the key. No region selection or additional-account discovery is provided.
- **Credential source:** only `%APPDATA%\PanePrivate\commandcode.json` on Windows. No environment-key discovery, CLI `auth.json` read, OAuth flow, or CLI credential write.
- **Default manual:** **Auto-refresh quotas** starts off. Use the card’s **↻**, global **Refresh**, or **Ctrl+R**, or explicitly enable auto-refresh for the already-authorized account. With auto-refresh off, startup, popover, interval, and settings quota refreshes skip CommandCode before key/identity reads; saved snapshots retain their original success time. Saving a key alone neither grants access nor enables auto-refresh.
- **Requests:** Bearer-authenticated GETs to `https://api.commandcode.ai/alpha/billing/credits` and `/alpha/billing/subscriptions` only. No model/inference request, automatic redirect, arbitrary base URL, or alternate credential/endpoint probing.
- **Displays:** supported 5-hour/weekly credit windows and reset times, monthly credits remaining, and separate purchased/free credits. Unknown values stay unknown; credits are not displayed as USD. See the [Chinese setup and limits guide](commandcode-provider.md) for the balance-only monthly display and experimental API limits.
- **Status:** these billing endpoints are undocumented; synthetic fixtures do not verify a real key, account quota, subscription, or Windows-native behavior. No new local-log source is added.

## Local statistics, prices, and account attribution

The 11 independently grantable sources are Claude, Codex, OpenCode, Pi/oh-my-pi, Step Code, Grok, Devin, MiniMax, Hermes, Kimi Code, and Qwen Code. Only the corresponding selected directory roots are authority. A directory’s Found / Not found / Unavailable status is separate from its tool’s On/Off permission, and Found does not prove that usable records exist. One unavailable root does not disable other valid roots or sources. Previous private-version grants remain enabled Custom with all their roots retained and no default expansion. See the [directory guide](private-build.md#2-首次启动先选择需要的访问).

A log may describe a model served by a different vendor. The display can group such records by model or route, but the grant remains attached to the tool that wrote the log. It does not silently enable that vendor, read its credential, or assign the record to a discovered account. Local statistics do not drive account-specific weekly-capacity claims.

Price synchronization is manual only and atomically replaces one validated local bundle. Local scans use the saved bundle and embedded rates; first run can use embedded rates without downloading anything. Legacy split catalog files are ignored until a manual sync. Unknown models keep their measured tokens and an unpriced indication, including when grouped into “Other”. “API-equivalent cost” is not actual subscription billing, and missing logs or unknown prices can make totals incomplete.

## Refresh, stale data, and recovery

Applicable OAuth refresh is part of account permission. Coordination for Claude, Codex, Kimi Code, and Grok rereads and serializes a credential source within this process, compares it before owner-restricted replacement, and avoids restoring an old backup. An external update detected before replacement wins; an uncertain rotation or save failure may require fresh sign-in rather than repeated refresh attempts.

These safeguards do not lock the official CLI or provide a cross-process transaction. Native Windows behavior, final compare-to-replace races, restart loss of in-process guards, and opaque-token identity checks remain limitations. Cursor uses a source-bound in-memory cache; Antigravity's access cache is source-bound; MiniMax mcode stays read-only. See [privacy](privacy.md#oauth-refresh-safety-and-limits) and [security](../SECURITY.md).

A last-good quota may remain visible after a failed refresh, explicitly marked stale. Its last-success time must not be interpreted as a new successful query. Disabling/revoking an account removes it from current publication; a permission or mode change can temporarily clear the view. Do not confuse unavailable, stale, or unpriced information with zero usage.
