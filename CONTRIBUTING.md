# Contributing to rice monitor

This repository is a privacy-focused fork of Pane for Windows. Use the [private build guide](docs/private-build.md), not upstream install scripts or release binaries. The [MIT license](LICENSE) and upstream attribution must remain intact.

## Changes

- Keep changes focused and describe the behavior, authorization boundary, and tests.
- New credential access requires an explicit backend account permission. Saving a key or selecting a network mode must not silently enable an account.
- Local log access must remain independently authorized by source and exact directory. Do not read credentials merely to classify logs or assign costs to accounts.
- Document each retained provider's credential sources, destinations, and failure behavior in [docs/providers.md](docs/providers.md).
- Do not restore telemetry, upstream auto-updates, background price downloads, broad credential discovery, One/New API, or Sub2API as a side effect of merging upstream changes.
- Regional failures must not probe other regions, ports, or authentication headers. Preserve the selected provider/account/mode boundary and test refusal paths.
- Use synthetic credentials and logs in tests. Do not commit private data or raw diagnostic dumps.
- Explain any new dependency and the reason it is needed.

## Validation

Run the frontend build and relevant portable test harnesses, then check the integrated result. A successful Linux test or Windows cross-target check is not evidence of a working Windows installer, native filesystem security, or real CLI coexistence. State which checks passed, failed, or were not run.

Security findings belong in a private report under [SECURITY.md](SECURITY.md), not a public issue with secrets or exploitable detail. No public contribution, release, support, or response-time commitment is implied by this source-only fork.
