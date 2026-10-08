import test from 'node:test';
import assert from 'node:assert/strict';
import {boot,settle} from './app-boot-harness.mjs';
const deferred=()=>{let resolve,reject;const promise=new Promise((yes,no)=>{resolve=yes;reject=no;});return {promise,resolve,reject};};
const policy={version:1,enabledFamilies:['claude','commandcode','qwen','codex'],enabledAccounts:['claude','claude@extra','commandcode','qwen','codex'],regions:{},accountBindings:{'claude@extra':{family:'claude',directory:'/synthetic/extra',name:'Extra'}},scanSources:{},scanRoots:{}};
const snapshot=(id,patch={})=>({id,name:id,plan:null,status:'ok',error:null,metrics:[{label:'Weekly',kind:'progress',used_percent:25,detail:null,value:null,resets_at:null,period_ms:null}],stale:false,warning:null,fetched_at:1700000000123,attempt_failed:false,...patch});
const cards=['claude','claude@extra','commandcode','qwen','codex'].map(id=>snapshot(id));
const bootCards=options=>boot({initialConfig:{accessPolicy:policy},snapshots:cards,...options});
const toggle=id=>`[data-provider-auto-refresh="${id}"]`;
const scopes=h=>h.usage().map(c=>[c.args.reason,c.args.scope?.accountId??'all']);

test('all 23 platform families retain separate grants; 22 remote families get one expanded auto-refresh control',async()=>{
 const h=await bootCards();try{
  assert.equal(h.w.document.querySelectorAll('[data-access-family]').length,23);
  assert.equal(h.w.document.querySelectorAll('[data-provider-auto-refresh]').length,22);
  for(const input of h.w.document.querySelectorAll('[data-provider-auto-refresh]')){
   const family=input.dataset.providerAutoRefresh;
   assert.equal(input.checked,!['claude','commandcode'].includes(family),family);
   assert.equal(input.disabled,false,'preference is independent of authorization');
   assert.equal(input.closest('.acc-body').id,`platform-${family}`);
  }
  assert.equal(h.w.document.querySelector(toggle('hermes')),null);
  assert.equal(h.w.document.querySelector(toggle('claude@extra')),null);
  assert.deepEqual(h.errors,[]);
 }finally{h.close();}
});

test('auto-refresh saves independently, does not query on enable, and survives reload',async()=>{
 const h=await bootCards();let saved;try{
  const baseline=h.calls.length;const grants=h.backend().accessPolicy;
  await h.change(toggle('claude'),true);await h.change(toggle('qwen'),false);
  assert.equal(h.backend().providerAutoRefresh.claude,true);assert.equal(h.backend().providerAutoRefresh.qwen,false);
  assert.deepEqual(h.backend().accessPolicy,grants);
  const mutations=h.calls.slice(baseline).filter(c=>!['sync_tray_surfaces'].includes(c.command));
  assert.ok(mutations.length>=2);assert.ok(mutations.every(c=>c.command==='set_config'));
  const autoWrites=mutations.filter(c=>c.args.patch.providerAutoRefresh);
  assert.ok(autoWrites.every(c=>Object.keys(c.args.patch).length===1));
  assert.deepEqual(autoWrites.map(c=>Object.keys(c.args.patch.providerAutoRefresh)),[['claude'],['qwen']]);
  assert.equal(h.select('[data-access-account="claude@extra"]').checked,true);
  saved=h.backend();
 }finally{h.close();}
 const reload=await boot({initialConfig:saved,snapshots:cards});try{
  assert.equal(reload.select(toggle('claude')).checked,true);assert.equal(reload.select(toggle('qwen')).checked,false);
 }finally{reload.close();}
});

test('every off family uses saved/manual guidance and enabling removes manual-only claims for extra accounts too',async()=>{
 const saved=cards.map(s=>({...s,stale:true}));
 const h=await bootCards({snapshots:saved});try{
  await h.change(toggle('qwen'),false);await h.change(toggle('claude'),true);
  for(const locale of ['en','zh','ru']){
   await h.change('#locale',locale);
   const qwen=h.select('[data-provider="qwen"]');
   assert.match(qwen.querySelector('.manual-query').title,/2023/);assert.match(qwen.textContent,/Ctrl\+R/);
   assert.equal(qwen.querySelector('.stale'),null);assert.match(qwen.textContent,/75%/);
   for(const id of ['claude','claude@extra'])assert.equal(h.select(`[data-provider="${id}"]`).querySelector('.manual-query'),null);
   assert.equal(h.select('[data-account-refresh="qwen"]').disabled,false);
   assert.doesNotMatch(h.select('#refresh-interval-help').textContent,/manual only|仅手动查询/);
  }
  h.snapshots([snapshot('qwen',{stale:true,attempt_failed:true,warning:'HTTP 429 synthetic cooldown'})]);await h.click('[data-account-refresh="qwen"]');
  assert.match(h.select('[data-provider="qwen"] .stale').title,/Ctrl\+R/);
  assert.doesNotMatch(h.select('[data-provider="qwen"] .stale').title,/automatically|会自动/);
  await h.click('#refresh');await h.key();assert.deepEqual(scopes(h).slice(-3),[['userRefresh','qwen'],['userRefresh','all'],['userRefresh','all']]);
 }finally{h.close();}
});

test('legacy settings, malformed entries and malformed whole maps match fail-safe defaults',async()=>{
 for(const value of [undefined,{qwen:false,claude:true,'codex@extra':true,unknown:true,commandcode:'true',codex:0},null,[],true,'enabled']){
  const config={accessPolicy:policy};if(value!==undefined)config.providerAutoRefresh=value;
  const h=await boot({initialConfig:config});try{
   const malformed=value!==undefined&&(value===null||typeof value!=='object'||Array.isArray(value));
   assert.equal(h.select(toggle('qwen')).checked,value===undefined?true:false);
   assert.equal(h.select(toggle('claude')).checked,!malformed&&value?.claude===true);
   assert.equal(h.select(toggle('codex')).checked,value===undefined?true:false);
   assert.equal(h.select(toggle('commandcode')).checked,false);
   assert.deepEqual(h.errors,[]);
  }finally{h.close();}
 }
});

test('rapid family flips serialize each intended family without stale replies undoing later edits',async()=>{
 const h=await bootCards();const held=deferred();try{
  h.configWrite({delay:held.promise});await h.change(toggle('claude'),true);
  await h.change(toggle('qwen'),false);await h.change(toggle('claude'),false);await h.change(toggle('commandcode'),true);
  assert.equal(h.select(toggle('claude')).checked,false);assert.equal(h.select(toggle('commandcode')).checked,true);
  held.resolve();await settle();
  const map=h.backend().providerAutoRefresh;assert.equal(map.claude,false);assert.equal(map.qwen,false);assert.equal(map.commandcode,true);
  assert.equal(h.select(toggle('claude')).checked,false);assert.equal(h.select(toggle('qwen')).checked,false);
  assert.equal(h.usage().length,1);assert.deepEqual(h.errors,[]);
 }finally{held.resolve();await settle();h.close();}
});

for(const commit of [false,true])test(`a failed setting reply reloads the canonical ${commit?'committed':'unchanged'} map without unrelated retries`,async()=>{
 const h=await bootCards();try{
  h.configWrite({fail:true,commit});await h.change(toggle('claude'),true);
  assert.equal(h.select(toggle('claude')).checked,commit);
  assert.match(h.select('#status').textContent,/Synthetic config write failure/);
  assert.equal(h.calls.filter(c=>c.command==='get_config').length,2);
  await h.change('#locale','zh');
  const lastWrite=h.calls.filter(c=>c.command==='set_config').at(-1);
  assert.equal(Object.hasOwn(lastWrite.args.patch,'providerAutoRefresh'),false);
  assert.equal(h.usage().length,1);assert.deepEqual(h.errors,[]);
 }finally{h.close();}
});

test('pending setting saves revalidate queued automatic work without dropping separate manual scopes',async()=>{
 const h=await bootCards();const usage=deferred();const write=deferred();try{
  h.delayNextUsage(usage.promise);await h.click('[data-account-refresh="claude"]');
  await h.change('#stepfun-plan','1600');await h.click('[data-account-refresh="qwen"]');
  h.configWrite({delay:write.promise});await h.change(toggle('qwen'),false);
  usage.resolve();await settle();assert.deepEqual(scopes(h),[['automatic','all'],['userRefresh','claude']]);
  write.resolve();await settle();
  assert.deepEqual(scopes(h),[['automatic','all'],['userRefresh','claude'],['automatic','all'],['userRefresh','qwen']]);
  assert.equal(h.backend().providerAutoRefresh.qwen,false);assert.deepEqual(h.errors,[]);
 }finally{usage.resolve();write.resolve();await settle();h.close();}
});

test('manual requests stay available while a preference save is pending and never acquire account permission',async()=>{
 const h=await bootCards();const held=deferred();try{
  h.configWrite({delay:held.promise});await h.change(toggle('claude'),true);
  await h.click('[data-account-refresh="claude@extra"]');await h.key();
  assert.deepEqual(scopes(h).slice(-2),[['userRefresh','claude@extra'],['userRefresh','all']]);
  assert.deepEqual(h.backend().accessPolicy,policy);
 }finally{held.resolve();await settle();h.close();}
});

test('all locales explain shared interval, independent grants, cached/manual behavior and already-sent requests',async()=>{
 const h=await boot();try{
  for(const [locale,label,interval,grants,sent] of [
   ['en',/Auto-refresh quotas/,/global refresh interval/,/does not authorize/,/already sent/],
   ['zh',/自动刷新配额/,/全局刷新间隔/,/不会授权/,/已发出/],
   ['ru',/Автообновление квот/,/общий интервал/,/не разрешает/,/уже отправлены/],
  ]){
   await h.change('#locale',locale);const row=h.select(toggle('qwen')).closest('.provider-auto-refresh');
   assert.match(row.textContent,label);assert.match(row.textContent,interval);assert.match(row.textContent,grants);assert.match(row.textContent,sent);assert.match(row.textContent,/Ctrl\+R/);
  }
 }finally{h.close();}
});

test('unverifiable failed save clearly pauses automatic remote queries while manual refresh and later saves recover',async()=>{
 const h=await bootCards();try{
  h.configWrite({fail:true,commit:true});h.failConfigRead();await h.change(toggle('claude'),true);
  assert.equal(h.select(toggle('claude')).checked,false);
  assert.match(h.select('#status').textContent,/Automatic quota queries are paused/);
  h.advance(61000);await h.tick(5*60000);assert.equal(h.usage().length,1);
  await h.click('[data-account-refresh="claude"]');assert.deepEqual(scopes(h).at(-1),['userRefresh','claude']);
  await h.change(toggle('claude'),true);assert.equal(h.select(toggle('claude')).checked,true);
  assert.doesNotMatch(h.select('#status').textContent,/paused|failure/);
  h.advance(61000);await h.tick(5*60000);assert.deepEqual(scopes(h).at(-1),['automatic','all']);
 }finally{h.close();}
});

test('enabling a manual placeholder shows waiting for refresh without claiming an error or querying immediately',async()=>{
 const h=await bootCards({snapshots:[snapshot('claude',{status:'manual',metrics:[],fetched_at:null})]});try{
  await h.change(toggle('claude'),true);
  assert.equal(h.usage().length,1);assert.equal(h.select('[data-provider="claude"]').querySelector('.manual-query'),null);
  assert.match(h.select('[data-provider="claude"] .placeholder').textContent,/Waiting for refresh/);
  assert.match(h.select('[data-cust-provider="claude"] .platform-account-head .platform-status').textContent,/Waiting for refresh/);
  assert.equal(h.select('[data-account-refresh="claude"]').disabled,false);
 }finally{h.close();}
});

test('authorized cached Cursor CSV survives automatic skip while local scans update, but revocation removes it',async()=>{
 const cursorPolicy={...policy,enabledFamilies:[...policy.enabledFamilies,'cursor'],enabledAccounts:[...policy.enabledAccounts,'cursor']};
 const spendWindow=(cost)=>({cost,tokens:100,models:[]});
 const cursor={id:'cursor',name:'Cursor',sources:[],today:spendWindow(7),yesterday:spendWindow(3),last30:spendWindow(9),trend:[0,1],unpriced:0,unpriced_models:[],month_cost:9};
 const local={id:'local:claude',name:'Claude logs',sources:['claude'],scan_revision:1,local_stats:[],today:spendWindow(1),yesterday:spendWindow(0),last30:spendWindow(1),trend:[],unpriced:0,unpriced_models:[],month_cost:1};
 const h=await bootCards({initialConfig:{accessPolicy:cursorPolicy},snapshots:[...cards,snapshot('cursor')],spendRows:[cursor,local],initialSources:{claude:{enabled:true,mode:'custom',customDirectories:['/synthetic/logs'],defaultDirectories:[]}}});try{
  assert.equal(h.calls.find(c=>c.command==='fetch_spend').args.reason,'automatic');
  await h.change(toggle('cursor'),false);await h.click('[data-caret="cursor"]');
  const before=h.select('[data-provider="cursor"]').textContent;
  assert.match(before,/7\.00/);
  h.spend([{...local,today:spendWindow(2),last30:spendWindow(2)}],true);
  h.advance(61000);await h.tick(5*60000);
  assert.match(h.select('[data-provider="cursor"]').textContent,/7\.00/);
  assert.match(h.select('[data-local-source="local:claude"]').textContent,/2\.00/);
  await h.click('#refresh');assert.equal(h.calls.filter(c=>c.command==='fetch_spend').at(-1).args.reason,'userRefresh');
  await h.change('[data-access-account="cursor"]',false);
  assert.equal(h.w.document.querySelector('[data-provider="cursor"]'),null);
  assert.doesNotMatch(h.select('#providers').textContent,/7\.00/);
 }finally{h.close();}
});

test('turning off reset-trigger auto-refresh clears its timer without changing quota cache',async()=>{
 const reset=Date.now()+10_000;
 const quota=snapshot('qwen',{metrics:[{...cards[0].metrics[0],resets_at:reset,period_ms:7*86400000}]});
 const h=await bootCards({initialConfig:{accessPolicy:policy,notifyReset:true},snapshots:[quota]});try{
  await h.change(toggle('qwen'),false);
  await assert.rejects(()=>h.timeout(ms=>ms>30_000&&ms<=40_000),/Timeout near/);
  assert.match(h.select('[data-provider="qwen"]').textContent,/75%/);
  assert.equal(h.usage().length,1);
 }finally{h.close();}
});

test('Kimi auto-off retains an independently refreshed Moonshot card without retiming the saved Kimi plan',async()=>{
 const accessPolicy={...policy,enabledFamilies:['kimi','moonshot'],enabledAccounts:['kimi','moonshot']};
 const wallet=value=>({label:'Balance',kind:'text',used_percent:null,detail:null,value,resets_at:null,period_ms:null});
 const kimi=snapshot('kimi',{stale:true,metrics:[cards[0].metrics[0],wallet('$7.00')],wallet_history:{fetched_at:1700000000123}});
 const moonshot=snapshot('moonshot',{fetched_at:1800000000000,metrics:[wallet('$9.00')]});
 const h=await boot({initialConfig:{accessPolicy,providerAutoRefresh:{kimi:false,moonshot:true}},snapshots:[kimi,moonshot]});try{
  assert.match(h.select('[data-provider="moonshot"]').textContent,/9\.00/);
  assert.match(h.select('[data-provider="kimi"] .manual-query').title,/2023/);
  assert.match(h.select('[data-provider="kimi"] [data-wallet-history]').title,/2023/);
  assert.equal(h.select('[data-provider="moonshot"]').querySelector('.manual-query'),null);
 }finally{h.close();}
});

for(const fail of [false,true])test(`reset revokes access and ${fail?'reports failed preference reset without inventing saved defaults':'restores compatible automatic-refresh defaults'}`,async()=>{
 const h=await bootCards();try{
  await h.change(toggle('claude'),true);await h.change(toggle('qwen'),false);
  if(fail)h.configWrite({fail:true,autoOnly:true});
  await h.click('#reset-all-settings');await h.click('#confirm-ok');
  assert.deepEqual(h.backend().accessPolicy.enabledAccounts,[]);assert.deepEqual(h.backend().accessPolicy.enabledFamilies,[]);
  assert.equal(h.select(toggle('claude')).checked,fail);assert.equal(h.select(toggle('qwen')).checked,!fail);
  if(fail)assert.match(h.select('#status').textContent,/Synthetic config write failure/);
  assert.ok(h.usage().every(c=>c.args.reason==='automatic'));
  assert.deepEqual(h.errors,[]);
 }finally{h.close();}
});

for(const failedFamily of ['claude','qwen'])test(`failed ${failedFamily} change is not retried by a separately queued family preference`,async()=>{
 const h=await bootCards();const held=deferred();try{
  const other=failedFamily==='claude'?'qwen':'claude';
  const enabled=id=>id==='claude';
  h.configWrite({fail:true,delay:held.promise,autoOnly:true});
  await h.change(toggle(failedFamily),enabled(failedFamily));
  await h.change(toggle(other),enabled(other));
  held.resolve();await settle();
  assert.equal(h.backend().providerAutoRefresh[failedFamily],!enabled(failedFamily));
  assert.equal(h.backend().providerAutoRefresh[other],enabled(other));
  assert.equal(h.select(toggle(failedFamily)).checked,!enabled(failedFamily));
  assert.doesNotMatch(h.select('#status').textContent,/failure|not saved/);
  assert.equal(h.usage().length,1);
 }finally{held.resolve();await settle();h.close();}
});

test('an old Cursor replay marker cannot restore remote spend after a newer scoped publication',async()=>{
 const accessPolicy={...policy,enabledFamilies:[...policy.enabledFamilies,'cursor'],enabledAccounts:[...policy.enabledAccounts,'cursor']};
 const window={cost:7,tokens:100,models:[]};
 const cursor={id:'cursor',name:'Cursor',sources:[],today:window,yesterday:window,last30:window,trend:[],unpriced:0,unpriced_models:[],month_cost:7};
 const h=await bootCards({initialConfig:{accessPolicy},snapshots:[...cards,snapshot('cursor')],spendRows:[cursor]});const held=deferred();try{
  await h.change(toggle('cursor'),false);h.spend([],true);h.delayNextSpend(held.promise);
  h.advance(61000);await h.tick(5*60000);
  h.revision(2);h.snapshots(cards);await h.click('[data-account-refresh="qwen"]');
  held.resolve();await settle();
  assert.equal(h.w.document.querySelector('[data-provider="cursor"]'),null);
  assert.doesNotMatch(h.select('#providers').textContent,/\$7\.00/);
 }finally{held.resolve();await settle();h.close();}
});
