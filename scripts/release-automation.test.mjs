import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { test } from 'node:test';

const normalizeNewlines = (text) => text.replace(/\r\n/g, '\n');
const read = (path) => normalizeNewlines(readFileSync(new URL(`../${path}`, import.meta.url), 'utf8'));
const checker = new URL('./check-release-version.mjs', import.meta.url);
const fixture = (overrides = {}) => ({
  'package.json': JSON.stringify({ version: '1.2.3' }),
  'package-lock.json': JSON.stringify({ version: '1.2.3', packages: { '': { version: '1.2.3' } } }),
  'src-tauri/tauri.conf.json': JSON.stringify({ version: '1.2.3' }),
  'src-tauri/Cargo.toml': '[package]\nname = "rice-monitor"\nversion = "1.2.3"\n',
  'src-tauri/Cargo.lock': 'version = 4\n\n[[package]]\nname = "another"\nversion = "9.0.0"\n\n[[package]]\nname = "rice-monitor"\nversion = "1.2.3"\n',
  ...overrides,
});
function check(tag, files = fixture()) {
  const cwd = mkdtempSync(join(tmpdir(), 'pane-release-test-'));
  try {
    mkdirSync(join(cwd, 'src-tauri'));
    for (const [path, content] of Object.entries(files)) writeFileSync(join(cwd, path), content);
    return spawnSync(process.execPath, [fileURLToPath(checker), ...(tag === undefined ? [] : [tag])], { cwd, encoding: 'utf8' });
  } finally {
    rmSync(cwd, { recursive: true, force: true });
  }
}

test('workflow checks normalize Git for Windows CRLF checkouts', () => {
  for (const file of ['pr.yml', 'release.yml']) {
    const lf = read(`.github/workflows/${file}`);
    const crlf = lf.replace(/\n/g, '\r\n');
    assert.ok(crlf.includes('\r\n'));
    assert.equal(normalizeNewlines(crlf), lf);
  }
});

test('release version checker accepts CRLF manifests from Windows checkouts', () => {
  const files = Object.fromEntries(Object.entries(fixture()).map(([path, contents]) => [path, contents.replace(/\n/g, '\r\n')]));
  const result = check('v1.2.3', files);
  assert.equal(result.status, 0, result.stderr);
});

test('release version checker accepts matching stable version across all manifests', () => {
  const result = check('v1.2.3');
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /1\.2\.3/);
});

test('release version checker rejects missing, malformed and prerelease tags', () => {
  for (const tag of [undefined, '', '1.2.3', 'v01.2.3', 'v1.2', 'v1.2.3-rc.1', 'v1.2.3+build', 'v1.2.3\n', 'v1.2.3;echo unsafe']) {
    const result = check(tag);
    assert.equal(result.status, 1, `must reject ${JSON.stringify(tag)}`);
    assert.match(result.stderr, /vMAJOR\.MINOR\.PATCH/);
  }
});

test('release version checker rejects a mismatch in any manifest or lockfile', () => {
  const files = fixture();
  for (const [path, content] of Object.entries(files)) {
    const result = check('v1.2.3', fixture({ [path]: content.replace('1.2.3', '1.2.4') }));
    assert.equal(result.status, 1, `must reject version drift in ${path}`);
    assert.ok(result.stderr.includes(path), result.stderr);
  }
  const result = check('v1.2.3', fixture({ 'package-lock.json': JSON.stringify({ version: '1.2.3', packages: { '': { version: '1.2.4' } } }) }));
  assert.equal(result.status, 1);
  assert.match(result.stderr, /package-lock\.json/);
});

test('release version checker fails closed on missing or invalid version data', () => {
  const missing = fixture();
  delete missing['package.json'];
  assert.equal(check('v1.2.3', missing).status, 1);
  for (const content of ['{}', '{ invalid json']) {
    assert.equal(check('v1.2.3', fixture({ 'package.json': content })).status, 1);
  }
  assert.equal(check('v1.2.3', fixture({ 'src-tauri/Cargo.lock': '[[package]]\nname = "another"\nversion = "1.2.3"\n' })).status, 1);
});

test('pull requests test and build Windows installers with read-only access', () => {
  const workflow = read('.github/workflows/pr.yml');
  assert.match(workflow, /pull_request:\n\s+branches: \[main\]/);
  assert.match(workflow, /permissions:\n\s+contents: read/);
  assert.match(workflow, /persist-credentials: false/);
  assert.match(workflow, /runs-on: windows-latest/);
  assert.match(workflow, /cancel-in-progress: true/);
  assert.match(workflow, /run: npm test/);
  assert.match(workflow, /run: cargo test --locked --manifest-path src-tauri\/Cargo\.toml --lib/);
  assert.match(workflow, /run: npm run tauri build -- --bundles nsis -- --locked/);
  assert.match(workflow, /path: src-tauri\/target\/release\/bundle\/nsis\/\*-setup\.exe/);
  assert.match(workflow, /retention-days: 7/);
  assert.match(workflow, /if-no-files-found: error/);
  assert.doesNotMatch(workflow, /contents: write|secrets\.|pull_request_target|gh release/);
});

test('release build validates the stable tag and main ancestry before building', () => {
  const workflow = read('.github/workflows/release.yml');
  const build = workflow.split(/^  publish:/m)[0];
  assert.match(build, /push:\n\s+tags:\n\s+- 'v\*'/);
  assert.doesNotMatch(build, /pull_request|workflow_dispatch|contents: write|secrets\./);
  assert.match(build, /fetch-depth: 0/);
  assert.match(build, /persist-credentials: false/);
  assert.match(build, /run: node scripts\/check-release-version\.mjs/);
  assert.match(build, /git merge-base --is-ancestor HEAD origin\/main/);
  assert.ok(build.indexOf('check-release-version.mjs') < build.indexOf('run: npm ci'));
  assert.match(build, /run: npm test/);
  assert.match(build, /run: cargo test --locked --manifest-path src-tauri\/Cargo\.toml --lib/);
  assert.match(build, /run: npm run tauri build -- --bundles nsis -- --locked/);
  assert.match(build, /Get-FileHash .* -Algorithm SHA256/);
  assert.match(build, /SHA256SUMS/);
  assert.match(build, /retention-days: 7/);
});

test('release publisher only uploads verified build outputs with job-scoped write access', () => {
  const workflow = read('.github/workflows/release.yml');
  const publish = workflow.split(/^  publish:/m)[1];
  assert.ok(publish);
  assert.match(publish, /needs: build/);
  assert.match(publish, /permissions:\n\s+contents: write/);
  assert.match(publish, /uses: actions\/download-artifact@[a-f0-9]{40}/);
  assert.match(publish, /sha256sum --check SHA256SUMS/);
  assert.match(publish, /GH_TOKEN: \$\{\{ github\.token \}\}/);
  assert.match(publish, /gh release create/);
  assert.match(publish, /--draft --verify-tag/);
  assert.match(publish, /gh release edit .* --draft=false/);
  assert.doesNotMatch(publish, /actions\/checkout|npm |cargo |\.exe\s*$/m);
  assert.match(workflow, /cancel-in-progress: false/);
  for (const file of ['pr.yml', 'release.yml']) {
    for (const action of read(`.github/workflows/${file}`).matchAll(/uses: (\S+)/g)) {
      assert.match(action[1], /^(actions\/(?:checkout|setup-node|upload-artifact|download-artifact)|dtolnay\/rust-toolchain)@[a-f0-9]{40}$/);
    }
  }
});

test('all Windows builds retain LF before checkout for existing source-contract tests', () => {
  for (const file of ['build.yml', 'pr.yml', 'release.yml']) {
    const workflow = read(`.github/workflows/${file}`);
    const config = workflow.indexOf('run: git config --global core.autocrlf false');
    assert.ok(config >= 0, `${file} must retain LF`);
    assert.ok(config < workflow.indexOf('uses: actions/checkout@'), `${file} must configure line endings before checkout`);
  }
});
