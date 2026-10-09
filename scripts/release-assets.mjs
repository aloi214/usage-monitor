import { createHash } from 'node:crypto';
import { readFileSync, readdirSync, renameSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

export const digest = (bytes) => createHash('sha256').update(bytes).digest('hex');
export const checksum = (hash, name) => Buffer.from(`${hash}  ${name}\n`);
export function installerName(tag) {
  if (typeof tag !== 'string' || !/^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(tag) || tag.includes('\n')) {
    throw new Error('Release tag must be vMAJOR.MINOR.PATCH.');
  }
  return `rice.monitor_${tag.slice(1)}_x64-setup.exe`;
}
function installer(directory) {
  const names = readdirSync(directory).filter((name) => name.endsWith('-setup.exe'));
  if (names.length !== 1) throw new Error('Expected exactly one NSIS installer.');
  return names[0];
}
export function prepareReleaseAssets(directory, tag) {
  const name = installerName(tag);
  const source = installer(directory);
  if (source !== name && source !== name.replace('rice.monitor', 'rice monitor')) {
    throw new Error(`Unexpected installer filename: ${source}; expected ${name}.`);
  }
  // GitHub normalizes spaces on upload. Use the final name before hashing.
  if (source !== name) renameSync(join(directory, source), join(directory, name));
  writeFileSync(join(directory, 'SHA256SUMS'), checksum(digest(readFileSync(join(directory, name))), name));
}
export function readReleaseAssets(directory, tag) {
  const name = installerName(tag);
  if (installer(directory) !== name) throw new Error(`Expected canonical installer filename ${name}.`);
  const bytes = readFileSync(join(directory, name));
  const hash = digest(bytes);
  const manifest = readFileSync(join(directory, 'SHA256SUMS'));
  if (!bytes.length || manifest.toString().replace(/\r\n/g, '\n') !== checksum(hash, name).toString()) {
    throw new Error('Local installer checksum verification failed.');
  }
  return { name, bytes, hash };
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    prepareReleaseAssets(process.argv[2], process.env.RELEASE_TAG);
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
