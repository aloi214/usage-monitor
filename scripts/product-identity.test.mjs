import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {test} from 'node:test';
import {boot} from './app-boot-harness.mjs';
const root=fileURLToPath(new URL('..',import.meta.url));
const read=path=>readFileSync(new URL(`../${path}`,import.meta.url),'utf8');

test('rice monitor starts its version sequence at 0.0.1 with matching release manifests',()=>{
 const config=JSON.parse(read('src-tauri/tauri.conf.json'));
 assert.equal(config.productName,'rice monitor');
 assert.equal(config.app.windows[0].title,'rice monitor');
 assert.equal(config.version,'0.0.1');
 assert.equal(config.bundle.publisher,'rice monitor');
 assert.equal(JSON.parse(read('package.json')).name,'rice-monitor');
 assert.match(read('src-tauri/Cargo.toml'),/^name = "rice-monitor"$/m);
 const check=spawnSync(process.execPath,['scripts/check-release-version.mjs','v0.0.1'],{cwd:root,encoding:'utf8'});
 assert.equal(check.status,0,check.stderr);
});

test('rename preserves private storage and upstream license attribution',()=>{
 assert.equal(JSON.parse(read('src-tauri/tauri.conf.json')).identifier,'local.pane.private');
 assert.match(read('src-tauri/src/access_policy.rs'),/base\.join\("PanePrivate"\)/);
 assert.match(read('LICENSE'),/Jazii/);
 assert.match(read('README.md'),/https:\/\/github.com\/ItsJazii\/pane/);
});

test('visible app name is rice monitor across initial boot and localized widget labels',async()=>{
 const h=await boot();try {
  assert.equal(h.w.document.title,'rice monitor');
  assert.equal(h.select('#app-logo').title,'rice monitor');
  assert.equal(h.select('#app-logo img').alt,'rice monitor');
  assert.equal(h.select('.widget-title').textContent,'rice monitor');
  assert.ok(h.select('#widget-logo img') instanceof h.w.HTMLImageElement);
  assert.doesNotMatch(h.select('#widget-logo').textContent,/\[object Object\]/);
  for (const locale of ['en','zh','ru']) {
   await h.change('#locale',locale);
   assert.match(h.select('[data-i18n-title="settings.widgetModeTip"]').title,/rice monitor/);
  }
  assert.deepEqual(h.errors,[]);
 } finally {h.close();}
});

test('Windows workflows name rice monitor installer artifacts and release title',()=>{
 for(const name of ['build','pr','release']) {
  const workflow=read(`.github/workflows/${name}.yml`);
  assert.match(workflow,/name: rice-monitor-/);
  assert.doesNotMatch(workflow,/pane-private/);
 }
 assert.match(read('scripts/publish-release.mjs'),/name: `rice monitor \$\{tag\}`/);
});

test('renamed app shows its own 0.0.1 changelog when upgrading from the upstream version',async()=>{
 const h=await boot({initialConfig:{lastSeenVersion:'0.4.57'}});try {
  await h.click('#changelog-btn');
  assert.match(h.select('#whatsnew-body').textContent,/v0\.0\.1/);
  assert.match(h.select('#whatsnew-body').textContent,/rice monitor/);
  assert.doesNotMatch(h.select('#whatsnew-body').textContent,/v0\.4\.57/);
  assert.equal(h.backend().lastSeenVersion,'0.0.1');
 } finally {h.close();}
});

test('Windows builds inspect the actual executable and installer branding before upload',()=>{
 for(const name of ['build','pr','release']) {
  const workflow=read(`.github/workflows/${name}.yml`);
  assert.match(workflow,/run: \.\/scripts\/verify-windows-package\.ps1/);
  assert.ok(workflow.indexOf('verify-windows-package.ps1')>workflow.indexOf('run: npm run tauri build'));
  assert.ok(workflow.indexOf('verify-windows-package.ps1')<workflow.indexOf('uses: actions/upload-artifact'));
 }
});
