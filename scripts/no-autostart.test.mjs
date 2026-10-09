import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { boot } from './app-boot-harness.mjs';
const read = path => readFileSync(new URL(`../${path}`, import.meta.url), 'utf8');

test('settings boot has no startup control or autostart commands, including legacy saved opt-in', async () => {
  const h = await boot({ initialConfig: { autostart: true } });
  try {
    assert.equal(h.w.document.querySelector('#autostart'), null);
    assert.deepEqual(h.calls.filter(call => /autostart/.test(call.command)), []);
    assert.deepEqual(h.errors, []);
  } finally { h.close(); }
});

test('confirmed reset never requests startup registration', async () => {
  const h = await boot({ initialConfig: { autostart: true } });
  try {
    await h.click('#reset-all-settings');
    await h.click('#confirm-ok');
    assert.equal(h.calls.filter(call => call.command === 'reset_provider_access').length, 1);
    assert.deepEqual(h.calls.filter(call => call.command === 'set_autostart'), []);
    assert.deepEqual(h.errors, []);
  } finally { h.close(); }
});

test('backend has no autostart plugin, enable command or saved opt-in authority', () => {
  assert.doesNotMatch(read('src-tauri/Cargo.toml'), /tauri-plugin-autostart/);
  assert.doesNotMatch(read('src-tauri/src/lib.rs'), /tauri_plugin_autostart|fn (?:get|set)_autostart|\.autolaunch\(\)|"autostart"/);
});
