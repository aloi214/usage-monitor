import test from 'node:test';
import assert from 'node:assert/strict';
import {boot,settle} from './app-boot-harness.mjs';

test('whole module boot renders every required settings control with no duplicate IDs or credential requests',async()=>{
 const h=await boot();try{
  assert.deepEqual(h.errors,[]);assert.equal(h.w.document.querySelectorAll('[data-access-family]').length,23);
  assert.equal(h.w.document.querySelectorAll('[data-scan-enabled]').length,11);
  const ids=[...h.w.document.querySelectorAll('[id]')].map(el=>el.id);assert.equal(new Set(ids).size,ids.length);
  for(const id of ['key-qwen','key-minimax','stepfun-plan','scan-sources','settings-btn','customize-btn','locale','reset-all-settings'])h.select('#'+id);
  assert.equal(h.calls.filter(call=>/set_api_key|discover_provider|set_provider/.test(call.command)).length,0);
  assert.equal(h.w.document.querySelectorAll('#settings [data-access-family],#settings [data-save]').length,0);
 }finally{h.close();}
});

test('dynamic key/region/tier handlers survive family toggle, locale repaint and panel reopening',async()=>{
 const h=await boot();try{
  await h.click('#customize-btn');
  await h.change('[data-access-family="qwen"]',true);
  assert.match(h.select('[data-cust-provider="qwen"]').textContent,/No account authorized/);
  assert.equal(h.select('[data-cust-expand="qwen"]').getAttribute('aria-expanded'),'true');
  await h.change('[data-provider-mode="qwen"]','china:bearer');
  h.select('#key-qwen').value='synthetic-key';await h.click('[data-save="qwen"]');
  assert.equal(h.select('#key-qwen').value,'');
  assert.equal(h.backend().accessPolicy.enabledAccounts.length,0,'key and region saves never enable accounts');
  await h.change('#locale','zh');await h.click('[data-customize-close]');await h.click('#customize-btn');
  h.select('#key-qwen').value='second-synthetic-key';await h.click('[data-save="qwen"]');
  await h.change('#stepfun-plan','1600');
  assert.equal(h.backend().stepfunPlanCredits,1600);
  assert.deepEqual(h.calls.filter(call=>call.command==='set_api_key').map(call=>call.args.key),['synthetic-key','second-synthetic-key']);
  assert.deepEqual(h.errors,[]);
 }finally{h.close();}
});

test('family ON expands immediately before delayed backend response without granting any accounts',async()=>{
 const h=await boot();try{
  let release;h.delayFamily(new Promise(resolve=>{release=resolve;}));
  await h.click('#customize-btn');
  const input=h.select('[data-access-family="qwen"]');input.checked=true;input.dispatchEvent(new h.w.Event('change',{bubbles:true}));
  assert.equal(h.select('[data-cust-expand="qwen"]').getAttribute('aria-expanded'),'true');
  assert.equal(h.backend().accessPolicy.enabledAccounts.length,0);release();await settle();
 }finally{h.close();}
});

test('local source OFF/default/custom transitions preserve paths and failed saves roll back to saved authority',async()=>{
 const h=await boot();try{
  await h.change('[data-scan-enabled="claude"]',true);
  assert.equal(h.backend().accessPolicy.scanSources.claude.mode,'default');
  await h.change('[data-scan-mode="claude"]','custom');
  const form=h.select('[data-scan-custom="claude"]');h.select('#scan-paths-claude').value='/custom/A\n/custom/B';
  form.dispatchEvent(new h.w.Event('submit',{bubbles:true,cancelable:true}));await settle();
  assert.deepEqual(h.backend().accessPolicy.scanRoots.claude,['/custom/A','/custom/B']);
  await h.change('[data-scan-enabled="claude"]',false);assert.equal(h.backend().accessPolicy.scanRoots.claude,undefined);
  await h.change('[data-scan-enabled="claude"]',true);assert.deepEqual(h.backend().accessPolicy.scanRoots.claude,['/custom/A','/custom/B']);
  h.failSave();h.select('#scan-paths-claude').value='/bad/path';h.select('[data-scan-custom="claude"]').dispatchEvent(new h.w.Event('submit',{bubbles:true,cancelable:true}));await settle();
  assert.deepEqual(h.backend().accessPolicy.scanRoots.claude,['/custom/A','/custom/B']);
  assert.match(h.select('#status').textContent,/Log directory is unavailable/);
  assert.equal(h.select('#scan-paths-claude').value,'/bad/path','failed draft remains available to correct');
  assert.match(h.select('[data-scan-source="claude"]').textContent,/\/custom\/A/);
  await h.change('[data-scan-mode="claude"]','default');assert.deepEqual(h.backend().accessPolicy.scanRoots.claude,['/synthetic/claude/logs']);
  assert.deepEqual(h.errors,[]);
 }finally{h.close();}
});

test('failed catalog startup has a working delegated retry after locale repaint and leaves other settings usable',async()=>{
 const h=await boot({failModes:true});try{
  assert.match(h.select('#provider-access').textContent,/synthetic mode failure/);
  await h.change('#locale','zh');await h.click('#customize-btn');await h.click('[data-provider-modes-retry]');
  h.select('[data-provider-mode="qwen"]');assert.doesNotMatch(h.select('#provider-access').textContent,/synthetic mode failure/);
  assert.equal(h.calls.filter(call=>call.command==='get_provider_modes').length,2);assert.deepEqual(h.errors,[]);
 }finally{h.close();}
});


test('custom location editing is optional and does not mutate authority before Save',async()=>{
 const h=await boot();try{
  assert.equal(h.select('[data-scan-custom="claude"]').hidden,true);
  const before=h.calls.filter(c=>c.command==='configure_scan_source').length;
  await h.change('[data-scan-mode="claude"]','custom');
  assert.equal(h.select('[data-scan-custom="claude"]').hidden,false);
  assert.equal(h.calls.filter(c=>c.command==='configure_scan_source').length,before);
  assert.match(h.select('[data-scan-source="claude"]').textContent,/Off · remembered locations/);
 }finally{h.close();}
});

test('delayed key save clears the current repainted field, while retaining a newer draft',async()=>{
 const h=await boot();try{
  let release;h.delayKey(new Promise(resolve=>{release=resolve;}));
  h.select('#key-qwen').value='submitted-synthetic-key';h.select('[data-save="qwen"]').click();await settle();
  await h.change('#locale','zh');assert.equal(h.select('#key-qwen').value,'submitted-synthetic-key');
  release();await settle();assert.equal(h.select('#key-qwen').value,'');
  let releaseAgain;h.delayKey(new Promise(resolve=>{releaseAgain=resolve;}));
  h.select('#key-qwen').value='first-draft';h.select('[data-save="qwen"]').click();await settle();
  await h.change('#locale','en');h.select('#key-qwen').value='newer-draft';
  releaseAgain();await settle();assert.equal(h.select('#key-qwen').value,'newer-draft');
 }finally{h.close();}
});

test('reset cancel leaves source access untouched; confirmed reset follows rapid edits and removes all grants',async()=>{
 const h=await boot();try{
  await h.change('[data-scan-enabled="claude"]',true);
  await h.click('#reset-all-settings');await h.click('#confirm-cancel');
  assert.equal(h.backend().accessPolicy.scanSources.claude.enabled,true);
  assert.equal(h.calls.filter(c=>c.command==='reset_provider_access').length,0);
  let release;h.delayScan(new Promise(resolve=>{release=resolve;}));
  const checkbox=h.select('[data-scan-enabled="claude"]');
  checkbox.checked=false;checkbox.dispatchEvent(new h.w.Event('change',{bubbles:true}));
  checkbox.checked=true;checkbox.dispatchEvent(new h.w.Event('change',{bubbles:true}));await settle();
  await h.click('#reset-all-settings');await h.click('#confirm-ok');
  release();await settle();
  assert.deepEqual(h.backend().accessPolicy.scanRoots,{});assert.deepEqual(h.backend().accessPolicy.scanSources,{});
  assert.equal(h.select('[data-scan-enabled="claude"]').checked,false);
  assert.deepEqual(h.calls.filter(c=>['configure_scan_source','reset_provider_access'].includes(c.command)).map(c=>c.command),['configure_scan_source','configure_scan_source','configure_scan_source','reset_provider_access']);
  assert.equal(h.w.document.querySelector('#confirm-overlay'),null);assert.deepEqual(h.errors,[]);
 }finally{h.close();}
});

test('collapsed provider settings are excluded from focus until expanded and return to that state on collapse',async()=>{
 const h=await boot();try{
  const body=h.select('#platform-qwen');assert.equal(body.hasAttribute('inert'),true);
  await h.click('[data-cust-expand="qwen"]');assert.equal(body.hasAttribute('inert'),false);
  await h.click('[data-cust-expand="qwen"]');assert.equal(body.hasAttribute('inert'),true);
 }finally{h.close();}
});


for(const failure of ['rejected','delayed']) test(`reset is immediately authoritative with ${failure} metadata; saving afterward cannot restore a grant`,async()=>{
 const h=await boot();let release;try{
  await h.change('[data-scan-enabled="claude"]',true);await h.change('[data-scan-mode="claude"]','custom');
  h.select('#scan-paths-claude').value='/synthetic/old';h.select('[data-scan-custom="claude"]').dispatchEvent(new h.w.Event('submit',{bubbles:true,cancelable:true}));await settle();
  if(failure==='rejected')h.failCatalog(true);
  else h.delayNextCatalog(new Promise(resolve=>{release=resolve;}));
  await h.click('#reset-all-settings');await h.click('#confirm-ok');
  assert.deepEqual(h.backend().accessPolicy.scanSources,{});
  assert.equal(h.select('[data-scan-enabled="claude"]').checked,false);
  assert.equal(h.select('[data-scan-mode="claude"]').value,'default');
  assert.equal(h.select('#scan-paths-claude').value,'');
  assert.equal(h.select('[data-scan-custom="claude"]').hidden,true);
  await h.change('[data-scan-mode="claude"]','custom');h.select('#scan-paths-claude').value='/synthetic/new';
  h.select('[data-scan-custom="claude"]').dispatchEvent(new h.w.Event('submit',{bubbles:true,cancelable:true}));await settle();
  assert.equal(h.calls.filter(c=>c.command==='configure_scan_source').at(-1).args.enabled,false);
  assert.deepEqual(h.backend().accessPolicy.scanRoots,{});
  release?.();await settle();assert.equal(h.select('[data-scan-enabled="claude"]').checked,false);
 }finally{release?.();await settle();h.close();}
});

test('metadata started before reset cannot restore old locations, intent or drafts when it completes later',async()=>{
 const h=await boot();let release;try{
  await h.change('[data-scan-enabled="claude"]',true);await h.change('[data-scan-mode="claude"]','custom');
  h.select('#scan-paths-claude').value='/synthetic/old';h.select('[data-scan-custom="claude"]').dispatchEvent(new h.w.Event('submit',{bubbles:true,cancelable:true}));await settle();
  h.delayNextCatalog(new Promise(resolve=>{release=resolve;}));await h.click('[data-scan-retry]');
  await h.click('#reset-all-settings');await h.click('#confirm-ok');release();await settle();
  assert.equal(h.select('[data-scan-enabled="claude"]').checked,false);assert.equal(h.select('[data-scan-mode="claude"]').value,'default');
  assert.equal(h.select('#scan-paths-claude').value,'');assert.doesNotMatch(h.select('[data-scan-source="claude"]').textContent,/synthetic\/old/);
 }finally{release?.();await settle();h.close();}
});

test('provider origin/key drafts and caret focus survive refresh, locale and metadata repaints',async()=>{
 const h=await boot();try{
  await h.click('#customize-btn');await h.click('[data-cust-expand="ollama"]');await h.click('[data-cust-expand="qwen"]');
  for(const selector of ['[data-local-origin]','#key-qwen']) {
   const input=h.select(selector);input.value=selector==='#key-qwen'?'synthetic-key':'http://127.0.0.1:12345';input.focus();input.setSelectionRange(4,9);
   const expected=input.value;
   for(const repaint of [()=>h.click('#refresh'),()=>h.change('#locale','zh'),()=>h.change('[data-provider-mode="qwen"]','china:bearer')]) {
    await repaint();const current=h.select(selector);
    assert.equal(current.value,expected);assert.equal(h.w.document.activeElement,current);assert.equal(current.selectionStart,4);assert.equal(current.selectionEnd,9);
   }
  }
 }finally{h.close();}
});


test('initial metadata failure retains all source OFF controls and accepted paths; later failures never show stale selections',async()=>{
 const h=await boot({failCatalogInitially:true,initialSources:{claude:{enabled:true,mode:'custom',customDirectories:['/accepted/A'],defaultDirectories:['/synthetic/claude/logs']}}});try{
  assert.equal(h.w.document.querySelectorAll('[data-scan-enabled]').length,11);
  assert.equal(h.select('[data-scan-enabled="claude"]').checked,true);assert.match(h.select('[data-scan-source="claude"] .scan-path').textContent,/accepted\/A/);
  await h.change('[data-scan-enabled="claude"]',false);assert.equal(h.select('[data-scan-enabled="claude"]').checked,false);
  await h.change('[data-scan-mode="claude"]','default');
  const paths=[...h.w.document.querySelectorAll('[data-scan-source="claude"] .scan-path')].map(el=>el.textContent).join('');
  assert.match(paths,/synthetic\/claude\/logs/);assert.doesNotMatch(paths,/accepted\/A/);assert.match(paths,/Not checked/);
  assert.deepEqual(h.backend().accessPolicy.scanRoots,{});
 }finally{h.close();}
});

test('saved origin becomes clean, presets update it, and a newer draft survives a delayed origin save',async()=>{
 const h=await boot();let release;try{
  await h.click('#customize-btn');await h.click('[data-cust-expand="ollama"]');
  h.select('[data-local-origin]').value='http://localhost:12345/';h.select('[data-provider-local="ollama"]').dispatchEvent(new h.w.Event('submit',{bubbles:true,cancelable:true}));await settle();
  assert.equal(h.select('[data-local-origin]').value,'http://localhost:12345');assert.equal(h.select('[data-local-origin]').defaultValue,'http://localhost:12345');
  await h.change('[data-provider-mode="ollama"]','local:http://127.0.0.1:11434');assert.equal(h.select('[data-local-origin]').value,'http://127.0.0.1:11434');
  h.delayMode(new Promise(resolve=>{release=resolve;}));h.select('[data-local-origin]').value='http://localhost:23456';h.select('[data-provider-local="ollama"]').dispatchEvent(new h.w.Event('submit',{bubbles:true,cancelable:true}));await settle();
  h.select('[data-local-origin]').value='http://localhost:34567';h.select('[data-local-origin]').focus();h.select('[data-local-origin]').setSelectionRange(8,12);
  release();await settle();assert.equal(h.backend().accessPolicy.regions.ollama,'local:http://localhost:23456');assert.equal(h.select('[data-local-origin]').value,'http://localhost:34567');
  assert.equal(h.w.document.activeElement,h.select('[data-local-origin]'));assert.equal(h.select('[data-local-origin]').selectionStart,8);
 }finally{release?.();await settle();h.close();}
});

test('source textarea draft and caret survive metadata recheck and locale repaint',async()=>{
 const h=await boot();try{
  await h.change('[data-scan-mode="claude"]','custom');const input=h.select('#scan-paths-claude');input.value='/unsaved/one\n/unsaved/two';input.focus();input.setSelectionRange(5,10);
  await h.click('[data-scan-retry]');await h.change('#locale','zh');
  assert.equal(h.select('#scan-paths-claude').value,input.value);assert.equal(h.w.document.activeElement,h.select('#scan-paths-claude'));assert.equal(h.select('#scan-paths-claude').selectionStart,5);
 }finally{h.close();}
});

test('provider drag ghost is decorative with no duplicate IDs or focusable form controls, and dragend removes it',async()=>{
 const h=await boot();try{
  await h.click('#customize-btn');await h.click('[data-cust-expand="qwen"]');
  const block=h.select('[data-cust-provider="qwen"]');block.dispatchEvent(new h.w.Event('dragstart',{bubbles:true,cancelable:true}));
  const ghost=h.select('.drag-ghost');const ids=[...h.w.document.querySelectorAll('[id]')].map(el=>el.id);assert.equal(new Set(ids).size,ids.length);
  assert.equal(ghost.hasAttribute('inert'),true);assert.equal(ghost.getAttribute('aria-hidden'),'true');
  assert.equal(ghost.querySelectorAll('[id],[for],[aria-controls],[aria-labelledby],[aria-describedby],[name]').length,0);
  for(const field of ghost.querySelectorAll('input,textarea,select,button')){assert.equal(field.disabled,true);assert.equal(field.tabIndex,-1);}
  assert.equal(h.select('#key-qwen').disabled,false,'real controls are unchanged');
  block.dispatchEvent(new h.w.Event('dragend',{bubbles:true}));assert.equal(h.w.document.querySelector('.drag-ghost'),null);
 }finally{h.close();}
});


for(const blocked of ['source','modes','both','rejected']) test(`accepted settings remain interactive while initial catalogs are ${blocked}`,async()=>{
 let release;const held=new Promise(resolve=>{release=resolve;});
 const h=await boot({initialSources:{claude:{enabled:true,mode:'custom',customDirectories:['/accepted/initial'],defaultDirectories:[]}},
  initialCatalogDelay:['source','both'].includes(blocked)?held:undefined,initialModeDelay:['modes','both'].includes(blocked)?held:undefined,failCatalogInitially:blocked==='rejected'});
 try{
  assert.equal(h.select('[data-scan-enabled="claude"]').checked,true);
  assert.equal(h.calls.filter(c=>['configure_scan_source','set_provider_family','set_provider_account','reset_provider_access'].includes(c.command)).length,0,'catalog loading cannot make permission writes');
  await h.change('[data-scan-enabled="claude"]',false);
  assert.equal(h.calls.filter(c=>c.command==='configure_scan_source').length,1);assert.equal(h.backend().accessPolicy.scanSources.claude.enabled,false);
  await h.change('#interval','9');assert.equal(h.backend().refreshMinutes,9);
  await h.click('#reset-all-settings');h.select('#confirm-overlay');await h.click('#confirm-ok');
  assert.deepEqual(h.backend().accessPolicy.scanSources,{});assert.deepEqual(h.backend().accessPolicy.scanRoots,{});
  release();await settle();assert.equal(h.select('[data-scan-enabled="claude"]').checked,false);assert.deepEqual(h.errors,[]);
 }finally{release();await settle();h.close();}
});

const usagePolicy={version:1,enabledFamilies:['claude','qwen'],enabledAccounts:['claude','qwen'],regions:{},accountBindings:{},scanSources:{},scanRoots:{}};
const claudeSnapshot=(patch={})=>({id:'claude',name:'Claude',plan:null,status:'manual',error:null,metrics:[],stale:false,warning:null,fetched_at:null,attempt_failed:false,...patch});
const deferred=()=>{let resolve,reject;const promise=new Promise((done,fail)=>{resolve=done;reject=fail;});return {promise,resolve,reject};};

test('startup, interval and popover refreshes explicitly remain automatic',async()=>{
 const h=await boot();try{
  assert.deepEqual(h.usage().map(call=>call.args.reason),['automatic']);
  h.advance(61_000);await h.tick(5*60_000);
  h.advance(61_000);await h.emit('popover-shown');
  assert.deepEqual(h.usage().map(call=>call.args.reason),['automatic','automatic','automatic']);
 }finally{h.close();}
});

test('only the global Refresh click and Ctrl+R request userRefresh',async()=>{
 const h=await boot();try{
  assert.match(h.select('#refresh').title,/all.*Claude/i);
  await h.click('#refresh');await h.key();
  assert.deepEqual(h.usage().map(call=>call.args.reason),['automatic','userRefresh','userRefresh']);
 }finally{h.close();}
});

test('settings, account changes, key saves and reset do not request Claude manual queries',async()=>{
 const h=await boot({initialConfig:{accessPolicy:usagePolicy}});try{
  await h.change('#stepfun-plan','1600');
  h.select('#key-qwen').value='synthetic';await h.click('[data-save="qwen"]');
  await h.change('[data-provider-mode="qwen"]','china:bearer');
  await h.change('[data-access-account="claude"]',false);
  await h.change('[data-access-account="claude"]',true);
  await h.change('[data-access-family="claude"]',false);
  await h.change('[data-scan-enabled="claude"]',true);
  await h.click('#reset-all-settings');await h.click('#confirm-ok');
  assert.ok(h.usage().length>=7,'settings exercise automatic refresh paths');
  assert.ok(h.usage().every(call=>call.args.reason==='automatic'));
  assert.deepEqual(h.errors,[]);
 }finally{h.close();}
});

test('manual clicks during automatic work coalesce into one queued manual query',async()=>{
 const held=deferred();const h=await boot({initialUsageDelay:held.promise});try{
  await h.click('#refresh');await h.key();await h.click('#refresh');
  assert.equal(h.usage().length,1);
  held.resolve();await settle();
  assert.deepEqual(h.usage().map(call=>call.args.reason),['automatic','userRefresh']);
  assert.equal(h.select('#refresh').getAttribute('aria-busy'),'false');
 }finally{held.resolve();await settle();h.close();}
});

test('automatic work queued behind a manual query never inherits manual intent',async()=>{
 const h=await boot();const held=deferred();try{
  h.delayNextUsage(held.promise);await h.click('#refresh');
  await h.change('#stepfun-plan','1600');
  held.resolve();await settle();
  assert.deepEqual(h.usage().map(call=>call.args.reason),['automatic','userRefresh','automatic']);
 }finally{held.resolve();await settle();h.close();}
});

test('repeated manual clicks coalesce and a following automatic queue gets a clean reason',async()=>{
 const h=await boot();const first=deferred();const queued=deferred();try{
  h.delayNextUsage(first.promise);await h.click('#refresh');
  await h.click('#refresh');await h.click('#refresh');await h.key();
  h.delayNextUsage(queued.promise);first.resolve();await settle();
  assert.equal(h.usage().length,3,'one active manual call and only one queued manual call');
  await h.change('#stepfun-plan','1600');queued.resolve();await settle();
  assert.deepEqual(h.usage().map(call=>call.args.reason),['automatic','userRefresh','userRefresh','automatic']);
 }finally{first.resolve();queued.resolve();await settle();h.close();}
});

test('revocation while pending discards stale data and cancels a queued old-scope manual click',async()=>{
 const h=await boot({initialConfig:{accessPolicy:usagePolicy},snapshots:[claudeSnapshot()]});const held=deferred();try{
  h.delayNextUsage(held.promise);await h.click('#refresh');await h.click('#refresh');
  await h.change('[data-access-account="claude"]',false);
  assert.equal(h.w.document.querySelector('[data-provider="claude"]'),null);
  held.resolve();await settle();
  assert.deepEqual(h.usage().map(call=>call.args.reason),['automatic','userRefresh','automatic']);
  assert.equal(h.w.document.querySelector('[data-provider="claude"]'),null,'late result cannot restore revoked card');
 }finally{held.resolve();await settle();h.close();}
});

for(const clickAfterChange of [false,true]) test(`access change ${clickAfterChange?'keeps a later':'cancels an earlier'} pending manual intent`,async()=>{
 const h=await boot({initialConfig:{accessPolicy:{...usagePolicy,enabledAccounts:['qwen']}}});const held=deferred();try{
  h.delayNextUsage(held.promise);await h.click('#refresh');
  if(!clickAfterChange)await h.click('#refresh');
  await h.change('[data-access-account="claude"]',true);
  if(clickAfterChange)await h.click('#refresh');
  held.resolve();await settle();
  assert.deepEqual(h.usage().map(call=>call.args.reason),['automatic','userRefresh',clickAfterChange?'userRefresh':'automatic']);
 }finally{held.resolve();await settle();h.close();}
});

test('first-use Claude manual state is neutral and localized with Russian fallback',async()=>{
 const h=await boot({initialConfig:{accessPolicy:usagePolicy},snapshots:[claudeSnapshot()]});try{
  for(const locale of ['en','zh','ru']){
   await h.change('#locale',locale);
   const card=h.select('[data-provider="claude"]');
   assert.match(card.textContent,locale==='zh'?/手动查询/:/Manual query/);
   assert.match(card.textContent,locale==='zh'?/刷新.*Ctrl\+R/:/Refresh.*Ctrl\+R/);
   assert.doesNotMatch(card.textContent,/Not connected|notConnected|未连接|Не подключ/);
   assert.equal(card.querySelector('.stale'),null);
   assert.match(h.select('[data-cust-provider="claude"] .platform-account-head .platform-status').textContent,locale==='zh'?/等待手动查询/:/Awaiting manual query/);
   assert.match(h.select('#refresh').title,/Claude/);
  }
 }finally{h.close();}
});

test('refresh help explains shared enabled-platform interval and manual rate-limit cooldowns',async()=>{
 const h=await boot({initialConfig:{accessPolicy:usagePolicy},snapshots:[claudeSnapshot()]});try{
  for(const locale of ['en','zh','ru']){
   await h.change('#locale',locale);
   const intervalHelp=h.select('#refresh-interval-help');
   assert.equal(h.select('#interval').getAttribute('aria-describedby'),intervalHelp.id);
   assert.match(intervalHelp.textContent,locale==='zh'?/本地统计.*共用/:locale==='ru'?/Общий интервал.*локальной статистики/:/Shared.*local statistics/);
   assert.match(intervalHelp.textContent,/Ctrl\+R/);
   assert.match(h.select('[data-provider="claude"] .manual-query').textContent,locale==='zh'?/限流冷却/:/Rate-limit cooldowns still apply/);
  }
 }finally{h.close();}
});

test('cached Claude cards retain metrics and show historical timestamp without a failed-refresh warning',async()=>{
 const timestamp=1_700_000_000_123;
 const saved=claudeSnapshot({id:'claude@synthetic',name:'Saved Claude account',status:'ok',stale:true,fetched_at:timestamp,metrics:[{label:'Session',kind:'text',used_percent:null,detail:null,value:'Saved usage',resets_at:null,period_ms:null}]});
 const policy={...usagePolicy,enabledAccounts:['claude@synthetic'],accountBindings:{'claude@synthetic':{family:'claude',name:'Saved account',directory:'/synthetic/account'}}};
 const h=await boot({initialConfig:{accessPolicy:policy},snapshots:[saved]});try{
  for(const locale of ['en','zh','ru']){
   await h.change('#locale',locale);const card=h.select('[data-provider="claude@synthetic"]');
   assert.match(card.textContent,/Saved usage/);
   assert.match(card.textContent,locale==='zh'?/手动查询.*已保存结果/:/Manual query.*Saved result/);
   assert.match(h.select('[data-cust-provider="claude@synthetic"] .platform-status').textContent,locale==='zh'?/已保存结果/:/Saved result/);
   const notice=card.querySelector('.manual-query');assert.ok(notice);
   assert.match(notice.title,locale==='zh'?/上次查询成功/:/Last successful query/);
   assert.ok(notice.title.includes(new Date(timestamp).toLocaleString(locale==='zh'?'zh-CN':locale==='ru'?'ru-RU':'en-US')));
   assert.match(notice.title,locale==='zh'?/账户信息尚未重新验证/:/account details have not been reverified/);
   assert.doesNotMatch(card.innerHTML,/last refresh failed|Last refresh failed|上次刷新失败/);
   assert.equal(card.querySelector('.stale'),null);
  }
 }finally{h.close();}
});

test('fresh Claude cards still show manual notice and genuine stale failures retain their warning',async()=>{
 const h=await boot({initialConfig:{accessPolicy:usagePolicy},snapshots:[claudeSnapshot()]});try{
  h.snapshots([claudeSnapshot({status:'ok',fetched_at:1_700_000_000_123})]);await h.click('#refresh');
  assert.match(h.select('[data-provider="claude"] .manual-query').textContent,/Manual query/);
  assert.doesNotMatch(h.select('[data-provider="claude"]').textContent,/Saved result/);
  h.snapshots([claudeSnapshot({status:'ok',stale:true,attempt_failed:true,warning:'HTTP 429 synthetic failure',fetched_at:1_700_000_000_123})]);
  await h.click('#refresh');
  for(const locale of ['en','zh','ru']){
   await h.change('#locale',locale);const help=h.select('[data-provider="claude"] .stale').title;
   assert.match(help,/HTTP 429 synthetic failure/);
   assert.match(help,locale==='zh'?/刷新.*Ctrl\+R/:/Refresh.*Ctrl\+R/);
   assert.match(help,locale==='zh'?/冷却/:/cooldown/);
   assert.doesNotMatch(help,/automatically|nothing to do|on its own|by itself|автоматически|сам повтор|会自动|自己重试/);
  }
 }finally{h.close();}
});

test('all Claude failure categories require a manual retry, including extra accounts',async()=>{
 const id='claude@synthetic';const policy={...usagePolicy,enabledAccounts:[id],accountBindings:{[id]:{family:'claude',name:'Saved account',directory:'/synthetic/account'}}};
 const h=await boot({initialConfig:{accessPolicy:policy}});try{
  for(const warning of ['HTTP 503 synthetic outage','Synthetic failure','network connect failed','invalid_grant synthetic sign-in failure','Run `claude` to sign in']){
   h.snapshots([claudeSnapshot({id,status:'ok',stale:true,attempt_failed:true,warning,fetched_at:1_700_000_000_123})]);await h.click('#refresh');
   for(const locale of ['en','zh','ru']){
    await h.change('#locale',locale);const help=h.select(`[data-provider="${id}"] .stale`).title;
    assert.ok(help.startsWith(warning+'.'));
    assert.match(help,locale==='zh'?/刷新.*Ctrl\+R/:/Refresh.*Ctrl\+R/);
    assert.doesNotMatch(help,/automatically|nothing to do|on its own|by itself|автоматически|сам повтор|会自动|自己重试/);
    if(warning.startsWith('invalid_grant'))assert.match(help,/`claude`/,'extra accounts retain Claude-specific sign-in guidance');
   }
  }
 }finally{h.close();}
});

test('other provider failure tooltips keep automatic retry guidance',async()=>{
 const h=await boot({initialConfig:{accessPolicy:usagePolicy}});try{
  for(const warning of ['HTTP 429 synthetic limit','HTTP 503 synthetic outage','Synthetic failure']){
   h.snapshots([claudeSnapshot({id:'qwen',name:'Qwen',status:'ok',stale:true,attempt_failed:true,warning})]);await h.click('#refresh');
   const help=h.select('[data-provider="qwen"] .stale').title;
   assert.ok(help.startsWith(warning+'.'));assert.match(help,/automatically|on its own|by itself/);
   assert.doesNotMatch(help,/query Claude/);
  }
 }finally{h.close();}
});

test('reset-moment timer refresh is automatic even though forced',async()=>{
 const reset=Date.now()+10_000;
 const h=await boot({initialConfig:{accessPolicy:usagePolicy,providerAutoRefresh:{claude:true},notifyReset:true},snapshots:[claudeSnapshot({status:'ok',metrics:[{label:'Weekly',kind:'progress',used_percent:10,detail:null,value:null,resets_at:reset,period_ms:7*24*60*60_000}]})]});try{
  await h.timeout(ms=>ms>30_000&&ms<=40_000);
  assert.deepEqual(h.usage().map(call=>call.args.reason),['automatic','automatic']);
 }finally{h.close();}
});

test('post-credit refresh stays automatic after an explicit Claude credit redemption',async()=>{
 const h=await boot({initialConfig:{accessPolicy:usagePolicy},snapshots:[claudeSnapshot({status:'ok',metrics:[{label:'Resets',kind:'resets',used_percent:null,detail:JSON.stringify([{id:'synthetic-credit',expires_at:null}]),value:'1',resets_at:null,period_ms:null}]})]});try{
  h.select('[data-resets]').dispatchEvent(new h.w.MouseEvent('mouseover',{bubbles:true}));await h.timeout(400);
  h.select('[data-rs-credit]').dispatchEvent(new h.w.MouseEvent('mouseover',{bubbles:true}));
  await h.click('[data-rs-use]');await h.click('[data-rs-go]');
  assert.equal(h.calls.filter(call=>call.command==='claude_redeem_credit').length,1);
  assert.deepEqual(h.usage().map(call=>call.args.reason),['automatic','automatic']);
  assert.deepEqual(h.errors,[]);
 }finally{h.close();}
});

test('access mutation initiation cancels old queued manual intent before an accepted grant reply arrives',async()=>{
 const h=await boot({initialConfig:{accessPolicy:{...usagePolicy,enabledAccounts:['qwen']}}});const usage=deferred();const reply=deferred();try{
  h.delayNextUsage(usage.promise);await h.click('#refresh');await h.click('#refresh');
  h.delayNextAccessReply('set_provider_account',reply.promise);await h.change('[data-access-account="claude"]',true);
  assert.ok(h.backend().accessPolicy.enabledAccounts.includes('claude'),'backend accepts grant before its reply');
  usage.resolve();await settle();
  assert.deepEqual(h.usage().map(call=>call.args.reason),['automatic','userRefresh','automatic']);
  reply.resolve();await settle();
  assert.equal(h.usage().filter(call=>call.args.reason==='userRefresh').length,1,'old queued click never replays against new scope');
  await h.click('#refresh');assert.equal(h.usage().at(-1).args.reason,'userRefresh');
 }finally{usage.resolve();reply.resolve();await settle();h.close();}
});

test('manual clicks while an access reply is pending are suppressed with localized wait-and-reclick guidance',async()=>{
 const h=await boot({initialConfig:{accessPolicy:{...usagePolicy,enabledAccounts:['qwen']}}});const reply=deferred();try{
  h.delayNextAccessReply('set_provider_account',reply.promise);await h.change('[data-access-account="claude"]',true);
  const before=h.usage().length;
  for(const locale of ['en','zh','ru']){
   await h.change('#locale',locale);await h.click('#refresh');await h.key();
   assert.equal(h.usage().length,before);
   assert.match(h.select('#status').textContent,locale==='zh'?/设置.*保存.*完成.*刷新/:/settings.*saving.*finish.*Refresh/i);
  }
  reply.resolve();await settle();
  assert.ok(h.usage().every(call=>call.args.reason==='automatic'),'pending clicks are not deferred authority');
  await h.click('#refresh');assert.equal(h.usage().at(-1).args.reason,'userRefresh');
 }finally{reply.resolve();await settle();h.close();}
});

test('manual dispatch remains blocked through multiple queued access writes and re-enabling',async()=>{
 const h=await boot({initialConfig:{accessPolicy:usagePolicy}});const first=deferred();const second=deferred();try{
  h.delayNextAccessReply('set_provider_account',first.promise);h.delayNextAccessReply('set_provider_account',second.promise);
  await h.change('[data-access-account="claude"]',false);await h.change('[data-access-account="claude"]',true);
  await h.click('#refresh');assert.ok(h.usage().every(call=>call.args.reason==='automatic'));
  first.resolve();await settle();assert.ok(h.backend().accessPolicy.enabledAccounts.includes('claude'));
  await h.key();assert.ok(h.usage().every(call=>call.args.reason==='automatic'),'finishing one write does not release the remaining gate');
  second.resolve();await settle();assert.ok(h.usage().every(call=>call.args.reason==='automatic'));
  await h.click('#refresh');assert.equal(h.usage().at(-1).args.reason,'userRefresh');
 }finally{first.resolve();second.resolve();await settle();h.close();}
});

test('failed access writes release the manual gate without reviving earlier clicks',async()=>{
 const h=await boot({initialConfig:{accessPolicy:usagePolicy}});const reply=deferred();try{
  h.delayNextAccessReply('set_provider_account',reply.promise);await h.change('[data-access-account="claude"]',false);
  await h.click('#refresh');assert.ok(h.usage().every(call=>call.args.reason==='automatic'));
  reply.reject(new Error('Synthetic access reply failed'));await settle();
  assert.match(h.select('#status').textContent,/Synthetic access reply failed/);
  assert.ok(h.usage().every(call=>call.args.reason==='automatic'));
  await h.click('#refresh');assert.equal(h.usage().at(-1).args.reason,'userRefresh');
 }finally{reply.resolve();await settle();h.close();}
});

test('confirmed reset invalidates queued manual intent and gates clicks until its reply settles',async()=>{
 const h=await boot({initialConfig:{accessPolicy:usagePolicy}});const usage=deferred();const reply=deferred();try{
  h.delayNextUsage(usage.promise);await h.click('#refresh');await h.click('#refresh');
  h.delayNextAccessReply('reset_provider_access',reply.promise);await h.click('#reset-all-settings');await h.click('#confirm-ok');
  assert.deepEqual(h.backend().accessPolicy.enabledAccounts,[]);
  usage.resolve();await settle();await h.click('#refresh');
  assert.equal(h.usage().filter(call=>call.args.reason==='userRefresh').length,1);
  reply.resolve();await settle();assert.equal(h.usage().filter(call=>call.args.reason==='userRefresh').length,1);
  await h.click('#refresh');assert.equal(h.usage().filter(call=>call.args.reason==='userRefresh').length,2);
 }finally{usage.resolve();reply.resolve();await settle();h.close();}
});

test('failed reset releases the manual gate, while cancelled reset never starts one',async()=>{
 const h=await boot({initialConfig:{accessPolicy:usagePolicy}});const reply=deferred();try{
  await h.click('#reset-all-settings');await h.click('#confirm-cancel');await h.click('#refresh');
  assert.equal(h.usage().at(-1).args.reason,'userRefresh');
  const before=h.usage().length;h.delayNextAccessReply('reset_provider_access',reply.promise);
  await h.click('#reset-all-settings');await h.click('#confirm-ok');await h.key();assert.equal(h.usage().length,before);
  reply.reject(new Error('Synthetic reset failed'));await settle();
  await h.click('#refresh');assert.equal(h.usage().length,before+1);assert.equal(h.usage().at(-1).args.reason,'userRefresh');
 }finally{reply.resolve();await settle();h.close();}
});

for(const command of ['set_provider_family','set_provider_region','configure_scan_source','discover_provider_account']) test(`manual dispatch is blocked while ${command} is pending`,async()=>{
 const h=await boot({initialConfig:{accessPolicy:usagePolicy}});const reply=deferred();try{
  h.delayNextAccessReply(command,reply.promise);
  if(command==='set_provider_family')await h.change('[data-access-family="claude"]',false);
  else if(command==='set_provider_region')await h.change('[data-provider-mode="qwen"]','china:bearer');
  else if(command==='configure_scan_source')await h.change('[data-scan-enabled="claude"]',true);
  else {h.select('[data-access-directory="claude"]').value='/synthetic/new-account';h.select('[data-access-discover="claude"]').dispatchEvent(new h.w.Event('submit',{bubbles:true,cancelable:true}));await settle();}
  assert.equal(h.calls.filter(call=>call.command===command).length,1);
  await h.click('#refresh');assert.ok(h.usage().every(call=>call.args.reason==='automatic'));
  reply.resolve();await settle();assert.ok(h.usage().every(call=>call.args.reason==='automatic'));
  await h.click('#refresh');assert.equal(h.usage().at(-1).args.reason,'userRefresh');
 }finally{reply.resolve();await settle();h.close();}
});
