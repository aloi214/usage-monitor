import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { prepareReleaseAssets } from './release-assets.mjs';
import { createGitHubApi, publishRelease } from './publish-release.mjs';

const tag = 'v1.2.3';
const commit = 'a'.repeat(40);
const name = 'rice.monitor_1.2.3_x64-setup.exe';
const bytes = Buffer.from('verified installer');
const hash = (data) => createHash('sha256').update(data).digest('hex');
const sums = (data = bytes) => Buffer.from(`${hash(data)}  ${name}\n`);
const marker = `<!-- rice-monitor-release:${tag}:${commit} -->`;
const label = (data = bytes) => `rice-monitor:${commit}:${hash(data)}`;

function fixture(t, contents = bytes) {
  const directory = mkdtempSync(join(tmpdir(), 'rice-release-'));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  writeFileSync(join(directory, name), contents);
  writeFileSync(join(directory, 'SHA256SUMS'), sums(contents));
  return directory;
}
function release(overrides = {}) {
  return { id: 7, tag_name: tag, target_commitish: commit, draft: false, prerelease: true, name: 'Custom title', body: 'User notes', ...overrides };
}
function asset(id, assetName, data, overrides = {}) {
  return { id, name: assetName, state: 'uploaded', size: data.length, digest: `sha256:${hash(data)}`, data, ...overrides };
}
// A stateful in-memory GitHub boundary: exercise real publisher decisions without mutating GitHub.
function github(releases = [], assets = [], options = {}) {
  const state = { releases: structuredClone(releases), assets: assets.map((a) => ({ ...a })), mutations: [], reads: [] };
  const api = async (path, { method = 'GET', body, binary = false } = {}) => {
    options.before?.(state, path, method);
    if (method === 'GET') state.reads.push(path);
    else state.mutations.push({ path, method, body });
    if (options.fail?.(path, method)) throw new Error('Simulated network failure');
    if (path === `commits/${encodeURIComponent(`refs/tags/${tag}`)}`) return { sha: options.sha ?? commit };
    if (path.startsWith('releases?')) {
      const page = Number(new URLSearchParams(path.split('?')[1]).get('page'));
      return structuredClone(state.releases.slice((page - 1) * 100, page * 100));
    }
    if (/^releases\/\d+\/assets\?per_page/.test(path)) {
      const page = Number(new URLSearchParams(path.split('?')[1]).get('page'));
      return state.assets.slice((page - 1) * 100, page * 100).map(({ data, ...a }) => a);
    }
    if (/^releases\/assets\/\d+$/.test(path) && binary) {
      const found = state.assets.find((a) => a.id === Number(path.split('/').at(-1)));
      return Buffer.from(found.data);
    }
    if (path === 'releases' && method === 'POST') {
      const created = release({ ...body, id: 19 });
      state.releases.push(created);
      return { ...created };
    }
    if (/^releases\/\d+\/assets\?name=/.test(path) && method === 'POST') {
      const query = new URLSearchParams(path.split('?')[1]);
      assert.ok(!state.assets.some((a) => a.name === query.get('name')), 'never clobber assets');
      const created = asset(100 + state.assets.length, query.get('name'), Buffer.from(body), { label: query.get('label') });
      state.assets.push(created);
      return created;
    }
    if (/^releases\/\d+$/.test(path)) {
      const found = state.releases.find((r) => r.id === Number(path.split('/').at(-1)));
      if (method === 'PATCH') Object.assign(found, body);
      return { ...found };
    }
    throw new Error(`Unexpected API request ${method} ${path}`);
  };
  return { state, api };
}
const run = (directory, remote, overrides = {}) => publishRelease({ directory, tag, commit, api: remote.api, ...overrides });

// Removing pre-hash normalization must break this test.
test('normalizes the spaced NSIS filename before writing the checksum manifest', (t) => {
  const directory = fixture(t);
  rmSync(join(directory, name));
  writeFileSync(join(directory, 'rice monitor_1.2.3_x64-setup.exe'), bytes);
  prepareReleaseAssets(directory, tag);
  assert.deepEqual(readdirSync(directory).sort(), ['SHA256SUMS', name].sort());
  assert.equal(readFileSync(join(directory, 'SHA256SUMS'), 'utf8'), sums().toString());
});

test('creates one draft, verifies both uploaded bytes, then publishes that exact release ID', async (t) => {
  const remote = github();
  const result = await run(fixture(t), remote);
  assert.equal(result.id, 19);
  assert.equal(remote.state.releases[0].draft, false);
  assert.deepEqual(remote.state.assets.map((a) => a.name), [name, 'SHA256SUMS']);
  assert.equal(remote.state.assets[0].label, label());
  assert.deepEqual(remote.state.mutations.at(-1), { path: 'releases/19', method: 'PATCH', body: { draft: false } });
  assert.ok(remote.state.reads.includes('releases/assets/100'));
  assert.ok(remote.state.reads.includes('releases/assets/101'));
});

test('a published same-tag release is a verified no-op even when a rebuild differs', async (t) => {
  const existing = release();
  const remote = github([existing], [asset(1, name, bytes), asset(2, 'SHA256SUMS', sums())]);
  await run(fixture(t, Buffer.from('different same-commit rebuild')), remote);
  assert.deepEqual(remote.state.mutations, []);
  assert.deepEqual(remote.state.releases, [existing]);
});

for (const draft of [true, false]) {
  test(`fills a unique empty user-created ${draft ? 'draft' : 'public prerelease'} without changing its metadata or visibility`, async (t) => {
    const existing = release({ target_commitish: 'main', draft });
    const unrelated = asset(1, 'user-notes.txt', Buffer.from('leave me alone'));
    const remote = github([existing], [unrelated]);
    await run(fixture(t), remote);
    assert.deepEqual(remote.state.releases, [existing]);
    assert.deepEqual(remote.state.assets[0], unrelated);
    assert.equal(remote.state.mutations.length, 2);
    assert.ok(remote.state.mutations.every((m) => m.method === 'POST' && m.path.startsWith('releases/7/assets?')));
    await run(fixture(t, Buffer.from('another rebuild')), remote);
    assert.equal(remote.state.mutations.length, 2, 'rerun uses installer provenance label');
  });
}

test('duplicate public/draft tags fail before any write, even across release list pages', async (t) => {
  const others = Array.from({ length: 99 }, (_, i) => release({ id: 100 + i, tag_name: `other-${i}` }));
  const remote = github([release(), ...others, release({ id: 8, draft: true })]);
  await assert.rejects(run(fixture(t), remote), /ambiguous.*7.*8/i);
  assert.deepEqual(remote.state.mutations, []);
});

test('resumes a workflow-owned partial draft using the original installer despite differing rebuilt bytes', async (t) => {
  const remote = github([release({ draft: true, body: `notes\n${marker}` })], [asset(1, name, bytes, { label: label() })]);
  await run(fixture(t, Buffer.from('rebuilt')), remote);
  assert.equal(remote.state.releases[0].draft, false);
  assert.equal(remote.state.assets[1].data.toString(), sums().toString());
  assert.equal(remote.state.mutations.filter((m) => m.method === 'POST').length, 1);
});

test('an interrupted upload can resume without creating another release', async (t) => {
  let fail = true;
  const remote = github([], [], { fail: (path, method) => fail && method === 'POST' && path.includes('name=SHA256SUMS') });
  const directory = fixture(t);
  await assert.rejects(run(directory, remote), /Simulated network failure/);
  assert.equal(remote.state.releases[0].draft, true);
  assert.equal(remote.state.assets.length, 1);
  fail = false;
  await run(directory, remote);
  assert.equal(remote.state.releases.length, 1);
  assert.equal(remote.state.releases[0].draft, false);
});

test('a checksum-only draft resumes only with matching local installer bytes', async (t) => {
  const remote = github([release({ draft: true, body: marker })], [asset(1, 'SHA256SUMS', sums())]);
  await run(fixture(t), remote);
  assert.equal(remote.state.releases[0].draft, false);
  const conflict = github([release({ draft: true, body: marker })], [asset(1, 'SHA256SUMS', sums(Buffer.from('other')))]);
  await assert.rejects(run(fixture(t), conflict), /checksum.*conflict/i);
  assert.deepEqual(conflict.state.mutations, []);
});

for (const invalid of [
  [asset(1, name, bytes), asset(2, 'SHA256SUMS', sums(Buffer.from('corrupt')))],
  [asset(1, name, bytes, { digest: `sha256:${'0'.repeat(64)}` })],
  [asset(1, name, bytes, { state: 'starter' })],
  [asset(1, name, bytes), asset(2, 'SHA256SUMS', Buffer.from(`${hash(bytes)}  rice monitor_1.2.3_x64-setup.exe\n`))],
]) {
  test(`conflicting or incomplete existing assets fail without overwriting (${invalid.at(-1).id}, ${invalid[0].state}, ${invalid.length})`, async (t) => {
    const remote = github([release({ draft: true, body: marker })], invalid);
    await assert.rejects(run(fixture(t), remote), /checksum|digest|uploaded/i);
    assert.deepEqual(remote.state.mutations, []);
    assert.equal(remote.state.releases[0].draft, true);
  });
}

test('rejects a pre-existing installer without commit provenance', async (t) => {
  const remote = github([release({ target_commitish: 'main' })], [asset(1, name, bytes), asset(2, 'SHA256SUMS', sums())]);
  await assert.rejects(run(fixture(t), remote), /provenance/i);
  assert.deepEqual(remote.state.mutations, []);
});

test('tag SHA drift and invalid local filenames/checksums fail before writes', async (t) => {
  const drift = github([], [], { sha: 'b'.repeat(40) });
  await assert.rejects(run(fixture(t), drift), /tag.*commit/i);
  assert.deepEqual(drift.state.mutations, []);
  const directory = fixture(t);
  writeFileSync(join(directory, 'SHA256SUMS'), sums(Buffer.from('bad')));
  const remote = github();
  await assert.rejects(run(directory, remote), /checksum/i);
  assert.deepEqual(remote.state.mutations, []);
  await assert.rejects(run(fixture(t), remote, { tag: 'v1.2.4' }), /installer|filename/i);
  await assert.rejects(run(fixture(t), remote, { tag: 'v1.2.3\n' }), /tag/i);
});

test('failed verification of uploaded bytes never publishes the draft', async (t) => {
  const remote = github([], [], { fail: (path, method) => method === 'GET' && path.startsWith('releases/assets/') });
  await assert.rejects(run(fixture(t), remote), /Simulated network failure/);
  assert.equal(remote.state.releases[0].draft, true);
  assert.ok(!remote.state.mutations.some((m) => m.method === 'PATCH'));
});

test('GitHub transport uses fixed API/upload hosts, JSON bodies and binary asset downloads', async () => {
  const calls = [];
  const api = createGitHubApi('owner/repo', 'test-token', async (url, init) => {
    calls.push({ url, ...init });
    return new Response(init.headers.Accept === 'application/octet-stream' ? bytes : JSON.stringify({ id: 9 }));
  });
  await api('releases', { method: 'POST', body: { draft: true } });
  await api('releases/9/assets?name=installer.exe', { method: 'POST', body: bytes, upload: true });
  assert.deepEqual(await api('releases/assets/1', { binary: true }), bytes);
  assert.equal(calls[0].url, 'https://api.github.com/repos/owner/repo/releases');
  assert.equal(calls[0].body, '{"draft":true}');
  assert.equal(calls[1].url, 'https://uploads.github.com/repos/owner/repo/releases/9/assets?name=installer.exe');
  assert.deepEqual(calls[1].body, bytes);
  assert.equal(calls[2].headers.Accept, 'application/octet-stream');
});


test('asset pagination finds already-published installer and checksum after unrelated files', async (t) => {
  const unrelated = Array.from({ length: 100 }, (_, i) => asset(1000 + i, `notes-${i}`, Buffer.from('note')));
  const remote = github([release()], [...unrelated, asset(1, name, bytes), asset(2, 'SHA256SUMS', sums())]);
  await run(fixture(t), remote);
  assert.deepEqual(remote.state.mutations, []);
  assert.deepEqual(remote.state.assets.slice(0, 100), unrelated);
});

test('legacy spaced installer aliases fail rather than uploading a second installer', async (t) => {
  const remote = github([release()], [asset(1, name.replace('rice.monitor', 'rice monitor'), bytes)]);
  await assert.rejects(run(fixture(t), remote), /unexpected installer/i);
  assert.deepEqual(remote.state.mutations, []);
});

test('a release targeting another full commit SHA fails before uploading', async (t) => {
  const remote = github([release({ target_commitish: 'b'.repeat(40) })]);
  await assert.rejects(run(fixture(t), remote), /release.*commit/i);
  assert.deepEqual(remote.state.mutations, []);
});

test('a tag moved after release discovery is rejected before the first asset upload', async (t) => {
  const options = { before: (_state, path) => { if (path.startsWith('releases/7/assets?per_page')) options.sha = 'b'.repeat(40); } };
  const remote = github([release()], [], options);
  await assert.rejects(run(fixture(t), remote), /tag.*commit/i);
  assert.deepEqual(remote.state.mutations, []);
});

test('an uncertain create response never retries creation in-process', async (t) => {
  const remote = github([], [], { fail: (path, method) => path === 'releases' && method === 'POST' });
  await assert.rejects(run(fixture(t), remote), /Simulated network failure/);
  assert.equal(remote.state.mutations.length, 1);
});

test('an upload accepted before a lost response resumes from its verified asset', async (t) => {
  const remote = github();
  const original = remote.api;
  let loseResponse = true;
  remote.api = async (path, options) => {
    const response = await original(path, options);
    if (loseResponse && options?.method === 'POST' && path.includes('assets?name=')) {
      loseResponse = false;
      throw new Error('Upload response lost');
    }
    return response;
  };
  const directory = fixture(t);
  await assert.rejects(run(directory, remote), /response lost/);
  await run(directory, remote);
  assert.equal(remote.state.releases.length, 1);
  assert.equal(remote.state.assets.length, 2);
  assert.equal(remote.state.releases[0].draft, false);
});

test('removing the workflow marker during upload keeps the user-edited draft private', async (t) => {
  const remote = github([release({ draft: true, body: marker })], [], {
    before: (state, path, method) => {
      if (method === 'POST' && path.includes('name=SHA256SUMS')) Object.assign(state.releases[0], { body: 'My new notes', name: 'My title', prerelease: true });
    },
  });
  await run(fixture(t), remote);
  assert.equal(remote.state.releases[0].draft, true);
  assert.equal(remote.state.releases[0].body, 'My new notes');
  assert.equal(remote.state.releases[0].name, 'My title');
  assert.equal(remote.state.releases[0].prerelease, true);
  assert.ok(!remote.state.mutations.some((m) => m.method === 'PATCH'));
});

test('asset removal during upload fails final verification before publishing', async (t) => {
  const remote = github([], [], {
    before: (state, path, method) => {
      if (method === 'POST' && path.includes('name=SHA256SUMS')) state.assets = state.assets.filter((a) => a.name !== name);
    },
  });
  await assert.rejects(run(fixture(t), remote), /missing|incomplete/i);
  assert.equal(remote.state.releases[0].draft, true);
});

test('binary redirects strip the token and reject untrusted locations', async () => {
  const calls = [];
  const fetcher = async (url, init) => {
    calls.push({ url, ...init });
    return calls.length === 1
      ? new Response(null, { status: 302, headers: { location: 'https://release-assets.githubusercontent.com/file' } })
      : new Response(bytes);
  };
  const api = createGitHubApi('owner/repo', 'test-token', fetcher);
  assert.deepEqual(await api('releases/assets/1', { binary: true }), bytes);
  assert.equal(calls[0].redirect, 'manual');
  assert.equal(calls[1].headers.Authorization, undefined);
  assert.equal(calls[1].redirect, 'error');
  const unsafe = createGitHubApi('owner/repo', 'test-token', async () => new Response(null, { status: 302, headers: { location: 'https://example.com/file' } }));
  await assert.rejects(unsafe('releases/assets/1', { binary: true }), /redirect/i);
});


test('a public automation release changed to draft by a maintainer is never republished', async (t) => {
  const remote = github([release({ body: marker })], [asset(1, name, bytes), asset(2, 'SHA256SUMS', sums())], {
    before: (state, path) => { if (path === 'releases/assets/1') state.releases[0].draft = true; },
  });
  await run(fixture(t), remote);
  assert.equal(remote.state.releases[0].draft, true);
  assert.deepEqual(remote.state.mutations, []);
});

test('a conflicting workflow provenance label cannot be bypassed by target_commitish', async (t) => {
  const remote = github([release()], [asset(1, name, bytes, { label: `rice-monitor:${'b'.repeat(40)}:${hash(bytes)}` }), asset(2, 'SHA256SUMS', sums())]);
  await assert.rejects(run(fixture(t), remote), /provenance/i);
  assert.deepEqual(remote.state.mutations, []);
});
