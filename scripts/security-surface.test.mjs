import {readFileSync,existsSync} from 'node:fs';
import assert from 'node:assert/strict';
import {test} from 'node:test';
const read=p=>readFileSync(new URL('../'+p,import.meta.url),'utf8');
test('telemetry cannot be compiled or invoked',()=>{
 const lib=read('src-tauri/src/lib.rs');
 assert.doesNotMatch(lib,/mod telemetry;|telemetry::/);
 assert.equal(existsSync(new URL('../src-tauri/src/telemetry.rs',import.meta.url)),false);
});
test('upstream updater has no executable command or plugin',()=>{
 assert.doesNotMatch(read('src-tauri/src/lib.rs'),/async fn (install_update|check_update)|tauri_plugin_updater|spawn_update_checker/);
 assert.doesNotMatch(read('src/main.ts'),/invoke[^\n]*(install_update|check_update)|listen[^\n]*update-available/);
 const cfg=JSON.parse(read('src-tauri/tauri.conf.json'));
 assert.equal(cfg.plugins?.updater,undefined);
 assert.notEqual(cfg.bundle?.createUpdaterArtifacts,true);
 assert.doesNotMatch(read('src-tauri/Cargo.toml'),/tauri-plugin-updater|reqwest_updater/);
});
test('retired relay modules and commands are absent',()=>{
 assert.doesNotMatch(read('src-tauri/src/providers/mod.rs'),/pub mod (onenewapi|sub2api);/);
 assert.doesNotMatch(read('src-tauri/src/lib.rs'),/providers::(?:onenewapi|sub2api)::/);
 assert.doesNotMatch(read('src/main.ts'),/invoke[^\n]*(?:onenewapi|sub2api)_/);
 assert.doesNotMatch(read('index.html'),/id="(?:onenewapi|sub2api)[^"]*"/);
});
test('tray provider registry length matches its Rust declaration',()=>{
 const source=read('src-tauri/src/lib.rs');
 const m=source.match(/const STRIP_PROVIDER_IDS: \[&str; (\d+)\] = \[([\s\S]*?)\];/);
 assert.ok(m);
 assert.equal(Number(m[1]),(m[2].match(/"[a-z]+"/g)||[]).length);
});
test('private build does not reuse the upstream installer identity',()=>{
 const cfg=JSON.parse(read('src-tauri/tauri.conf.json'));
 assert.equal(cfg.identifier,'local.pane.private');
 assert.equal(cfg.productName,'Pane Private');
});
test('manual pinned Rust action selects a real toolchain explicitly',()=>{
 const workflow=read('.github/workflows/build.yml');
 assert.match(workflow,/uses: dtolnay\/rust-toolchain@[a-f0-9]{40}\n\s+with:\n\s+toolchain: stable/);
});
