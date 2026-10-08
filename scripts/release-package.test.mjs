import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';

const read = (path) => readFileSync(new URL(`../${path}`, import.meta.url), 'utf8');
const config = JSON.parse(read('src-tauri/tauri.conf.json'));

test('release profile prioritizes size without disabling panic unwinding', () => {
  const cargo = read('src-tauri/Cargo.toml');
  const release = cargo.match(/^\[profile\.release\]\s*\n([\s\S]*?)(?=^\[|$(?![\s\S]))/m)?.[1];
  assert.ok(release, 'an explicit release profile is required');
  assert.match(release, /^opt-level\s*=\s*"s"\s*(?:#.*)?$/m);
  assert.match(release, /^lto\s*=\s*true\s*(?:#.*)?$/m);
  assert.match(release, /^codegen-units\s*=\s*1\s*(?:#.*)?$/m);
  assert.match(release, /^strip\s*=\s*"symbols"\s*(?:#.*)?$/m);
  assert.match(release, /^panic\s*=\s*"unwind"\s*(?:#.*)?$/m);
});

test('Windows packages one NSIS installer with explicit LZMA compression', () => {
  assert.equal(config.bundle.active, true);
  assert.deepEqual(config.bundle.targets, ['nsis']);
  assert.equal(config.bundle.windows.nsis.compression, 'lzma');
  assert.equal(config.bundle.windows.nsis.installerIcon, 'icons/icon.ico');
});

test('online installer downloads missing WebView2 instead of bundling a runtime', () => {
  assert.deepEqual(config.bundle.windows.webviewInstallMode, {
    type: 'downloadBootstrapper',
    silent: true,
  });
  assert.deepEqual(config.bundle.resources ?? [], []);
  assert.deepEqual(config.bundle.externalBin ?? [], []);
  assert.equal(config.bundle.createUpdaterArtifacts, false);
});

test('manual Windows workflow checks tests and explicitly builds the NSIS artifact', () => {
  const workflow = read('.github/workflows/build.yml');
  assert.match(workflow, /^\s+- run: npm test\s*$/m);
  assert.match(workflow, /^\s+- run: npm run tauri build -- --bundles nsis\s*$/m);
  assert.match(workflow, /^\s+path: src-tauri\/target\/release\/bundle\/nsis\/\*-setup\.exe\s*$/m);
  assert.match(workflow, /^\s+if-no-files-found: error\s*$/m);
  assert.match(workflow, /^\s+workflow_dispatch:\s*$/m);
  assert.doesNotMatch(workflow, /^\s+(?:push|pull_request|schedule):/m);
});
