import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
const main = readFileSync(new URL('../src/main.ts', import.meta.url), 'utf8');
const lib = readFileSync(new URL('../src-tauri/src/lib.rs', import.meta.url), 'utf8');
const html = readFileSync(new URL('../index.html', import.meta.url), 'utf8');
const between = (text, from, to) => text.slice(text.indexOf(from), text.indexOf(to, text.indexOf(from)));
test('saving a key has no account enable or disabled-filter mutation', () => {
  const fn = between(main, 'async function saveApiKey(', '\n}\n');
  assert.doesNotMatch(fn, /set_provider_account|set_provider_family|disabled:|markProviderEnablePending|recentlyKeyed\.set/);
});
test('settings contain explicit family, account and directory consent controls', () => {
  assert.match(html, /id="provider-access"/);
  assert.match(main, /set_provider_family/);
  assert.match(main, /set_provider_account/);
  assert.match(main, /discover_provider_account/);
  assert.match(main, /data-access-directory/);
});
test('reset settings revokes access instead of rearming startup discovery', () => {
  const fn = between(main, 'async function resetAllSettings()', 'function syncSettingsControls');
  assert.match(fn, /reset_provider_access/);
});
test('general settings patch cannot write accessPolicy', () => {
  const keys = between(lib, 'const CONFIG_KEYS:', 'static CONFIG_WRITE:');
  assert.doesNotMatch(keys, /"accessPolicy"/);
});
test('all retained provider futures run inside backend account authorization', () => {
  const fn = between(lib, 'async fn fetch_usage(', 'fn cached_usage(');
  assert.match(fn, /runtime\.run_account_at_checked\(&id, &disabled, access_revision, check, fut\)/);
  assert.match(fn, /is_current\(access_revision\)/);
  assert.match(fn, /let disabled = query_disabled\.clone\(\)/);
  assert.match(fn, /automatic\.then\(/);
});
test('extra-account discovery uses only individually enabled remembered directories', () => {
  for (const family of ['claude', 'codex', 'opencode']) {
    const source = readFileSync(new URL(`../src-tauri/src/providers/${family}.rs`, import.meta.url), 'utf8');
    const fn = between(source, 'pub(crate) fn discover_extra_accounts_for', '\n}\n');
    assert.match(fn, /account_bindings/);
    assert.match(fn, /policy\.allows_account\(id\)/);
    assert.doesNotMatch(fn, /account_scan_roots|extra_data_dirs|default_identity/);
  }
});
test('private config path neither imports nor renames upstream state', () => {
  const source = readFileSync(new URL('../src-tauri/src/providers/mod.rs', import.meta.url), 'utf8');
  const fn = between(source, 'pub fn config_dir()', '\n}\n');
  assert.match(fn, /private_config_dir/);
  assert.doesNotMatch(fn, /std::fs::(?:rename|copy)|join\("Pane"\)|join\("OpenUsage"\)/);
});
