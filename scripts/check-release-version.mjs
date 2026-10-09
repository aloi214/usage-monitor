import { readFileSync } from 'node:fs';

// No dependency install or shell evaluation is needed to reject an invalid tag.
const read = (path) => readFileSync(path, 'utf8');
const json = (path) => JSON.parse(read(path));
const tomlValue = (section, key) => section?.match(new RegExp(`^${key}\\s*=\\s*"([^"]+)"\\s*(?:#.*)?$`, 'm'))?.[1];

try {
  const tag = process.argv[2] ?? process.env.RELEASE_TAG;
  const match = typeof tag === 'string' && tag.match(/^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/);
  if (!match || match[0] !== tag) throw new Error('Release tag must be vMAJOR.MINOR.PATCH (stable versions only).');
  const version = tag.slice(1);
  const lock = json('package-lock.json');
  const cargoPackage = read('src-tauri/Cargo.toml').split(/^\[package\][ \t]*\r?$/m)[1]?.split(/^\[/m)[0];
  const cargoLockPackage = read('src-tauri/Cargo.lock').split(/^\[\[package\]\][ \t]*\r?$/m).find((section) => tomlValue(section, 'name') === 'pane');
  const versions = [
    ['package.json', json('package.json').version],
    ['package-lock.json', lock.version],
    ['package-lock.json packages[""]', lock.packages?.['']?.version],
    ['src-tauri/tauri.conf.json', json('src-tauri/tauri.conf.json').version],
    ['src-tauri/Cargo.toml', tomlValue(cargoPackage, 'version')],
    ['src-tauri/Cargo.lock (pane)', tomlValue(cargoLockPackage, 'version')],
  ];
  for (const [source, actual] of versions) {
    if (actual !== version) throw new Error(`${source}: expected ${version}, found ${actual ?? 'no version'}.`);
  }
  console.log(`Release version ${version} matches all manifests and lockfiles.`);
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
