import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import ts from 'typescript';
async function load(file){const js=ts.transpileModule(readFileSync(new URL(file,import.meta.url),'utf8'),{compilerOptions:{module:ts.ModuleKind.ESNext,target:ts.ScriptTarget.ES2020}}).outputText;return import(`data:text/javascript;base64,${Buffer.from(js).toString('base64')}`);}
const {createLogSourceCatalog,renderLogSourceSettings,createLogSourceHandlers}=await load('../src/log-source-settings.ts');
const {t,setActiveLocale}=await load('../src/i18n.ts');
const esc=s=>String(s).replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const sources=[{source:'claude',enabled:true,mode:'default',customDirectories:['/saved/A','/saved/B'],directories:[{path:'/home/test/.claude/projects',status:'notFound'}]},{source:'codex',enabled:false,mode:'custom',customDirectories:['/custom/C'],directories:[{path:'/custom/C',status:'found'}]}];
test('source rendering keeps ON/not-found distinct from OFF and displays selected effective paths',()=>{
 for(const locale of ['en','zh','ru']) {setActiveLocale(locale);const html=renderLogSourceSettings({entries:sources,loading:false,error:null},[['claude','Claude Code'],['codex','Codex']],t,esc);
 assert.match(html,/<input[^>]*data-scan-enabled="claude"[^>]*checked/);
 assert.doesNotMatch(html,/<input[^>]*data-scan-enabled="codex"[^>]*checked/);
 assert.ok(html.includes('/home/test/.claude/projects'));assert.ok(html.includes(t('settings.scanNotFound')));
 assert.ok(html.includes('/saved/A\n/saved/B'));assert.match(html,/data-scan-mode="claude"/);assert.match(html,/data-scan-retry/);
 assert.doesNotMatch(html,/>settings\.scan\w+</);
 }
});
test('source handler sends only explicit mode/toggle edits and custom save replaces all paths',async()=>{
 const calls=[];const handlers=createLogSourceHandlers(()=>sources,async(command,input)=>calls.push({command,input:typeof input==='function'?input():input}));
 await handlers.change({dataset:{scanEnabled:'claude'},checked:false});
 await handlers.change({dataset:{scanMode:'codex'},value:'default'});
 await handlers.submit({dataset:{scanCustom:'claude'},querySelector:()=>({value:' /chosen/A\n /chosen/B\n/chosen/A\n'})});
 assert.deepEqual(calls,[{command:'configure_scan_source',input:{source:'claude',enabled:false,mode:'default'}},{command:'configure_scan_source',input:{source:'codex',enabled:false,mode:'default'}},{command:'configure_scan_source',input:{source:'claude',enabled:true,mode:'custom',directories:['/chosen/A','/chosen/B']}}]);
});
test('unknown controls are ignored and saving an OFF source does not silently enable it',async()=>{
 const calls=[];const handlers=createLogSourceHandlers(()=>sources,async(command,input)=>calls.push(typeof input==='function'?input():input));
 await handlers.change({dataset:{scanEnabled:'unknown'},checked:true});
 await handlers.submit({dataset:{scanCustom:'codex'},querySelector:()=>({value:'/other'})});
 assert.deepEqual(calls,[{source:'codex',enabled:false,mode:'custom',directories:['/other']}]);
});
test('source catalog coalesces identical reads, fences reset races, and retains an explicit retry error',async()=>{
 let epoch=0;const deferred=[];const catalog=createLogSourceCatalog(()=>new Promise((resolve,reject)=>deferred.push({resolve,reject})),()=>{},()=>epoch);
 const first=catalog.load();assert.equal(catalog.load(),first);await Promise.resolve();assert.equal(deferred.length,1);
 epoch++;const afterReset=catalog.load();await Promise.resolve();assert.equal(deferred.length,2);
 deferred[1].resolve([sources[1]]);await afterReset;deferred[0].resolve(sources);await first;
 assert.deepEqual(catalog.state.entries,[sources[1]]);
 const failure=catalog.load();await Promise.resolve();deferred[2].reject(new Error('metadata unavailable'));await failure;
 assert.match(catalog.state.error,/metadata unavailable/);assert.equal(catalog.state.loading,false);
 const html=renderLogSourceSettings(catalog.state,[['codex','Codex']],t,esc);assert.match(html,/role="alert"/);assert.match(html,/data-scan-retry/);
});

test('source projection treats config absence as OFF regardless of stale metadata, and keeps OFF controls available without metadata',async()=>{
 const {projectScanSources}=await load('../src/log-source-settings.ts');assert.equal(typeof projectScanSources,'function');
 const result=projectScanSources(['claude','codex'],{scanSources:{},scanRoots:{}},sources);
 assert.deepEqual(result.map(s=>({source:s.source,enabled:s.enabled,mode:s.mode,customDirectories:s.customDirectories})),[{source:'claude',enabled:false,mode:'default',customDirectories:[]},{source:'codex',enabled:false,mode:'default',customDirectories:[]}]);
 assert.deepEqual(result[0].directories,[],'old ON source locations are not an OFF default preview');
 const accepted={enabled:true,mode:'custom',customDirectories:['/new/one','/new/two'],defaultDirectories:[]};
 const fresh=projectScanSources(['claude'],{scanSources:{claude:accepted},scanRoots:{claude:accepted.customDirectories}},sources);
 assert.deepEqual(fresh[0].directories.map(d=>d.path),accepted.customDirectories);assert.ok(fresh[0].directories.every(d=>d.status==='unverified'));
 assert.equal(projectScanSources(['claude'],{scanSources:{claude:accepted},scanRoots:{}},[])[0].enabled,true,'accepted intent is available without metadata');
 const legacy=projectScanSources(['claude'],{scanRoots:{claude:['/legacy']}},[])[0];assert.equal(legacy.enabled,true);assert.equal(legacy.mode,'custom');assert.deepEqual(legacy.customDirectories,['/legacy']);
});

test('clearing metadata fences pending reads and removes old directory state synchronously',async()=>{
 let finish;const catalog=createLogSourceCatalog(()=>new Promise(resolve=>{finish=resolve;}),()=>{},()=>0);
 const pending=catalog.load();await Promise.resolve();assert.equal(typeof catalog.clear,'function');catalog.clear();finish(sources);await pending;
 assert.deepEqual(catalog.state.entries,[]);assert.equal(catalog.state.loading,false);
});
