import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
const spend = readFileSync(new URL('../src-tauri/src/spend.rs', import.meta.url), 'utf8').split('#[cfg(test)]\nmod tests')[0];
test('scan discovery never reads credentials or unselected default homes', () => {
  assert.doesNotMatch(spend, /discover_extra_accounts|discover_keyless_dirs|home_account_id|default_identity|has_credentials|extra_ledger_homes|dirs::home_dir|orca_codex_homes/);
});
test('collection requires explicit source policy and never downloads prices', () => {
  assert.match(spend, /pub fn collect\(policy: &ScanPolicy/);
  assert.doesNotMatch(spend, /pricing::ensure_fresh\(\)/);
});
const lib = readFileSync(new URL('../src-tauri/src/lib.rs', import.meta.url), 'utf8');
const main = readFileSync(new URL('../src/main.ts', import.meta.url), 'utf8');
const html = readFileSync(new URL('../index.html', import.meta.url), 'utf8');
const oc = readFileSync(new URL('../src-tauri/src/providers/opencode.rs', import.meta.url), 'utf8');
const fn = (text, name, next) => text.slice(text.indexOf(name), text.indexOf(next, text.indexOf(name)));
test('OpenCode query failure never opens local usage ledgers', () => {
  const body = fn(oc, 'async fn fetch(', 'async fn fetch_official(');
  assert.doesNotMatch(body, /local_windows_snapshot|read_messages|earliest_go_anchor/);
});
test('source grants use dedicated backend mutation and guarded collection', () => {
  assert.match(lib, /async fn set_scan_source\(/);
  assert.match(lib, /pub\(crate\) fn local_scan_policy\(/);
  assert.match(lib, /scan_sources::runtime_policy\(&policy\)/);
  assert.match(lib, /spend::collect\(&scan_policy, None\)/);
  assert.match(lib, /spend::invalidate_published\(\)/);
});
test('source-only UI renders local details without query-account gating', () => {
  assert.match(html, /id="scan-sources"/);
  assert.match(main, /function renderLocalSpendCards\(/);
  assert.match(main, /local_stats/);
  assert.match(main, /renderTotalSpend\(\) \+ renderLocalSpendCards\(\)/);
  assert.doesNotMatch(fn(main, 'function donutEntries(', 'function spendVal('), /filter\(\(s\) => !isCardDisabled/);
});
test('spend refresh rejects cross-consent replies and renders without account cards', () => {
  assert.match(main, /requestAccessEpoch !== accessEpoch/);
  const body = fn(main,'  const spend = await spendPromise;','function scheduleAutoRefresh(');
  assert.match(body, /requestAccessEpoch !== accessEpoch/);
  assert.doesNotMatch(body, /!customizeOpen && lastSnapshots.length/);
});
