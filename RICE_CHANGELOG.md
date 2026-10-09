# rice monitor changelog

This fork maintains its own version sequence. Earlier upstream history and
attribution remain in CHANGELOG.md and LICENSE.

## 0.0.1 — 2026-10-09

### Changed
- Product, tray, window, Start Menu shortcut, and installer name: rice monitor.
- Start a separate version sequence at 0.0.1.
- Remove launch-at-login registration, its Settings control, and the Reset All path that enabled it.
- Preserve existing private settings, permissions, keys, caches, and the application identifier.
- Require removal of the old Pane Private installation before installing the renamed product; close any running private-build instance first.

### Verification limits
- Automated tests and packaging do not verify native installation, Windows sign-in behavior, or real account responses.
