# Pane Private

**English** · [简体中文](README.zh-CN.md) · [Русский](README.ru.md)

A privacy-focused fork of [Pane for Windows](https://github.com/ItsJazii/pane), based on upstream commit `a55578c`. The desktop app uses Tauri, Rust, and TypeScript to show AI account limits and separately authorized local usage estimates.

> This repository describes **Pane Private**, not an upstream Pane release. Upstream installers, winget packages, install scripts, and release downloads do not contain these changes. The earlier source version was reported to build and render in Windows 11 development mode. The later settings, exact-account refresh, CommandCode, and per-provider auto-refresh changes still need native acceptance testing; no private Windows installer or size reduction is verified here.

## Start here

- **[GitHub Windows builds and releases / 自动检查与发布](docs/github-releases.md)**: PR test installers, manual branch builds, and version-tagged Releases

- **[Per-provider quota auto-refresh / 按平台自动刷新](docs/provider-auto-refresh.md)**: independent scheduling switches, shared interval, and saved/manual results
- **[CommandCode GOAT (experimental) / 配置与限制](docs/commandcode-provider.md)**: a separately authorized local key and default-manual billing queries with optional auto-refresh; live account support is unverified
- **[Exact-account quota refresh / 单账号额度刷新](docs/scoped-quota-refresh.md)**: per-card queries, global Refresh, and saved Kimi wallet data
- **[Size-focused Windows packaging / Windows 发布包体积配置](docs/release-package.md)**: release settings, NSIS build steps, and measurement limits
- **[Private build and use guide / 私有版构建与使用](docs/private-build.md)**: prerequisites, source builds, first-run consent, and verification limits
- **[Settings update / 设置更新](docs/settings-update.md)**: the unified platform controls, log-location modes, and safe source-update steps
- [Privacy](docs/privacy.md): account access, local logs, storage, network behavior, and limitations
- [Providers](docs/providers.md): 23 provider families: 22 retained providers plus experimental CommandCode
- [Local HTTP API](docs/local-http-api.md): read-only snapshots for local tools
- [Security](SECURITY.md): boundaries and responsible reporting

## Private-fork behavior

- Upstream telemetry and automatic updates are removed. Updates are installed manually from this repository’s reviewed Releases, when available, or by rebuilding reviewed source.
- Account permissions start off. **Platform management** is the single place for provider/account permissions, region/connection modes, keys, and metrics/layout. A platform’s **Queries** switch permits individual account choices; it does not authorize them. Turning it off revokes that platform’s account queries and refreshes. Metric visibility only changes the display.
- **Auto-refresh quotas** is separate from account permission and shared by each family’s extra accounts. Claude and CommandCode default off; other account-query providers default on, while all account grants still start off. Off retains applicable saved results and permits authorized card/global manual refresh. Hermes is local-only and has no quota-auto switch.
- **Local log sources** start off and have independent per-tool switches. Choose known **Default location** paths or save exact **Custom locations**. Missing locations stay distinct from Off; invalid custom saves retain the prior grant without falling back. Local statistics need no account permission or credential discovery.
- One/New API and Sub2API are removed. The other 22 provider families are retained, including local-only Hermes and local Ollama support; experimental CommandCode brings the total to 23.
- The private build uses `%APPDATA%\PanePrivate` on Windows and does not import upstream Pane/OpenUsage settings, keys, or caches.
- Exact regional/key modes, manual-only public price synchronization, hashed log-fragment fingerprints, and coordinated OAuth refresh are implemented in this source. See the [build guide's verification limits](docs/private-build.md#integration-status); synthetic checks and cross-target checks do not prove native Windows behavior.

Account quotas and log-derived spend answer different questions. A log estimate is not an invoice, an account balance, or proof of which account paid. Unpriced models retain measured token counts; unknown prices are not treated as free.

## Credits and license

Pane Private retains the upstream [MIT license](LICENSE) and attribution:

- © 2026 Jazii, [Pane for Windows](https://github.com/ItsJazii/pane)
- © 2025 Robin Ebers, [OpenUsage for macOS](https://github.com/robinebers/openusage), original concept and provider research
- © 2025 Peter Steinberger, [CodexBar](https://github.com/steipete/CodexBar), provider research identified in the license

Thanks also to [Tauri](https://tauri.app/), [LiteLLM](https://github.com/BerriAI/litellm), [models.dev](https://models.dev/), [prasen.dev](https://www.prasen.dev/), and [shadcn/ui](https://ui.shadcn.com/) for upstream components, data, and visual techniques. Provider names and logos belong to their owners; this fork is not endorsed by those providers or the upstream projects.

`CHANGELOG.md` records upstream history; it is not a release or security-verification record for Pane Private.
