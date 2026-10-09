import { pathToFileURL } from 'node:url';
import { checksum, digest, readReleaseAssets } from './release-assets.mjs';

// No dependencies, application imports, shell interpolation, or asset deletion.
export function createGitHubApi(repository, token, fetcher = fetch) {
  if (!/^[\w.-]+\/[\w.-]+$/.test(repository ?? '') || !token) throw new Error('GitHub repository and token are required.');
  return async (path, { method = 'GET', body, binary = false, upload = false } = {}) => {
    let response = await fetcher(`https://${upload ? 'uploads' : 'api'}.github.com/repos/${repository}/${path}`, {
      method,
      redirect: 'manual',
      headers: {
        Authorization: `Bearer ${token}`,
        Accept: binary ? 'application/octet-stream' : 'application/vnd.github+json',
        'X-GitHub-Api-Version': '2022-11-28',
        ...(body ? { 'Content-Type': upload ? 'application/octet-stream' : 'application/json' } : {}),
      },
      body: body ? (upload ? body : JSON.stringify(body)) : undefined,
      signal: AbortSignal.timeout(60_000),
    });
    if (binary && response.status === 302) {
      const location = new URL(response.headers.get('location'));
      if (location.protocol !== 'https:' || location.username || location.password ||
          !['release-assets.githubusercontent.com', 'objects.githubusercontent.com'].includes(location.hostname)) {
        throw new Error('Refusing unexpected asset download redirect.');
      }
      // GitHub returns a signed URL. Never forward the repository token to it.
      response = await fetcher(location.href, {
        headers: { Accept: 'application/octet-stream' }, redirect: 'error', signal: AbortSignal.timeout(60_000),
      });
    }
    if (!response.ok) throw new Error(`GitHub ${method} ${path}: HTTP ${response.status}. Inspect the release before retrying; no assets were deleted.`);
    return binary ? Buffer.from(await response.arrayBuffer()) : response.json();
  };
}
async function list(api, path) {
  const all = [];
  for (let page = 1; ; page++) {
    const batch = await api(`${path}?per_page=100&page=${page}`);
    all.push(...batch);
    if (batch.length < 100) return all;
  }
}
async function download(api, asset) {
  if (asset.state !== 'uploaded') throw new Error(`Asset ${asset.name} is not fully uploaded; inspect asset ID ${asset.id}.`);
  const bytes = await api(`releases/assets/${asset.id}`, { binary: true });
  if (bytes.length !== asset.size || (asset.digest && asset.digest !== `sha256:${digest(bytes)}`)) {
    throw new Error(`Asset digest or size conflict for ${asset.name} (ID ${asset.id}).`);
  }
  return bytes;
}

export async function publishRelease({ directory, tag, commit, api }) {
  const local = readReleaseAssets(directory, tag);
  if (typeof commit !== 'string' || commit.length !== 40 || !/^[a-f0-9]{40}$/.test(commit)) throw new Error('Expected a full build commit SHA.');
  const marker = `<!-- rice-monitor-release:${tag}:${commit} -->`;
  const assertTag = async () => {
    // The commit endpoint peels both lightweight and annotated tags.
    if ((await api(`commits/${encodeURIComponent(`refs/tags/${tag}`)}`)).sha !== commit) throw new Error(`Tag ${tag} no longer points to build commit ${commit}.`);
  };
  const matching = async () => {
    const releases = (await list(api, 'releases')).filter((release) => release.tag_name === tag);
    if (releases.length > 1) throw new Error(`Ambiguous releases for ${tag}: IDs ${releases.map((r) => r.id).join(', ')}. Resolve manually; nothing will be overwritten.`);
    return releases[0];
  };
  await assertTag();
  let release = await matching();
  if (!release) {
    await assertTag();
    // Never retry an uncertain create in-process. A later run discovers its ID.
    release = await api('releases', { method: 'POST', body: {
      tag_name: tag, target_commitish: commit, name: `rice monitor ${tag}`, draft: true, prerelease: false,
      body: `Windows x64 NSIS installer built from commit ${commit}. This installer is unsigned; Windows may show Unknown Publisher or SmartScreen warnings. If WebView2 is missing, installation needs internet access. SHA256SUMS verifies download integrity, not publisher identity. This build does not enable automatic updates. Native installation and account behavior still need manual acceptance testing.\n\n${marker}`,
    } });
  }
  const id = release.id;
  const mayPublish = release.draft && release.body?.includes(marker);
  const assertIdentity = async () => {
    await assertTag();
    if ((await matching())?.id !== id) throw new Error(`Release identity changed for ${tag}; expected ID ${id}.`);
    const current = await api(`releases/${id}`);
    if (/^[a-f0-9]{40}$/.test(current.target_commitish) && current.target_commitish !== commit) {
      throw new Error(`Release ID ${id} targets a different commit.`);
    }
    if (current.tag_name !== tag) throw new Error(`Release ID ${id} changed its tag.`);
    return current;
  };
  release = await assertIdentity();
  const assets = await list(api, `releases/${id}/assets`);
  const legacy = assets.find((asset) => /^rice[ .]monitor_.*_x64-setup\.exe$/.test(asset.name) && asset.name !== local.name);
  if (legacy) throw new Error(`Unexpected installer asset ${legacy.name} (ID ${legacy.id}); inspect manually before retrying.`);
  const owned = (name) => {
    const found = assets.filter((asset) => asset.name === name);
    if (found.length > 1) throw new Error(`Ambiguous assets named ${name} on release ID ${id}.`);
    return found[0];
  };
  const installer = owned(local.name);
  const manifest = owned('SHA256SUMS');
  let bytes = local.bytes;
  if (installer) {
    bytes = await download(api, installer);
    const provenance = `rice-monitor:${commit}:${digest(bytes)}`;
    if ((installer.label?.startsWith('rice-monitor:') && installer.label !== provenance) ||
        (installer.label !== provenance && release.target_commitish !== commit)) {
      throw new Error(`Existing installer ${installer.id} has no matching commit provenance. Inspect manually; it will not be overwritten.`);
    }
  }
  const expected = checksum(digest(bytes), local.name);
  if (manifest && (await download(api, manifest)).toString().replace(/\r\n/g, '\n') !== expected.toString()) {
    throw new Error(`Checksum conflict on release ID ${id}. Keep original build artifacts or publish a new version; no assets were overwritten.`);
  }
  const upload = async (name, data, label) => {
    await assertIdentity();
    const query = new URLSearchParams({ name, ...(label ? { label } : {}) });
    const uploaded = await api(`releases/${id}/assets?${query}`, { method: 'POST', upload: true, body: data });
    if (uploaded.name !== name || !data.equals(await download(api, uploaded))) {
      throw new Error(`Uploaded asset verification failed for ${name}; release ID ${id} was not published.`);
    }
  };
  if (!installer) await upload(local.name, bytes, `rice-monitor:${commit}:${digest(bytes)}`);
  if (!manifest) await upload('SHA256SUMS', expected);
  // Only this workflow's marked drafts are automatically published. Existing
  // public releases and user-created drafts retain all metadata and visibility.
  const finalAssets = await list(api, `releases/${id}/assets`);
  for (const [name, expectedBytes] of [[local.name, bytes], ['SHA256SUMS', expected]]) {
    const found = finalAssets.filter((asset) => asset.name === name);
    if (found.length !== 1) throw new Error(`Missing or ambiguous final asset ${name}; release ID ${id} is incomplete.`);
    const actual = await download(api, found[0]);
    const matches = name === 'SHA256SUMS'
      ? actual.toString().replace(/\r\n/g, '\n') === expectedBytes.toString()
      : actual.equals(expectedBytes);
    if (!matches) throw new Error(`Final asset digest conflict for ${name}; release ID ${id} was not published.`);
  }
  release = await assertIdentity();
  if (mayPublish && release.draft && release.body?.includes(marker)) {
    release = await api(`releases/${id}`, { method: 'PATCH', body: { draft: false } });
  }
  return release;
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const release = await publishRelease({
      directory: process.argv[2] ?? 'release', tag: process.env.RELEASE_TAG, commit: process.env.RELEASE_COMMIT,
      api: createGitHubApi(process.env.GITHUB_REPOSITORY, process.env.GH_TOKEN),
    });
    console.log(`Verified release ID ${release.id} (${release.draft ? 'draft retained' : 'published'}).`);
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
