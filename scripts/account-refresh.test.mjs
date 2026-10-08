import test from 'node:test';
import assert from 'node:assert/strict';
import {boot,settle} from './app-boot-harness.mjs';
const deferred=()=>{let resolve,reject;const promise=new Promise((yes,no)=>{resolve=yes;reject=no;});return {promise,resolve,reject};};
const policy={version:1,enabledFamilies:['claude','qwen','kimi','moonshot'],enabledAccounts:['claude','claude@extra','qwen','kimi','moonshot'],regions:{},accountBindings:{'claude@extra':{family:'claude',directory:'/synthetic/extra',name:'Extra'}},scanSources:{},scanRoots:{}};
const metric=(value)=>({label:'Weekly',kind:'progress',used_percent:value,detail:null,value:null,resets_at:null,period_ms:null});
const snapshot=(id,patch={})=>({id,name:id,plan:null,status:'ok',error:null,metrics:[metric(10)],stale:false,warning:null,fetched_at:1700000000123,attempt_failed:false,...patch});
const originals=[snapshot('claude'),snapshot('claude@extra',{name:'Extra'}),snapshot('qwen',{stale:true,warning:'Synthetic last error',attempt_failed:true})];
const bootCards=(options={})=>boot({initialConfig:{accessPolicy:policy},snapshots:originals,...options});
const card=(h,id)=>h.select(`[data-provider="${id}"]`);
const refresh=(h,id)=>h.click(`[data-account-refresh="${id}"]`);
const scopes=h=>h.usage().map(call=>[call.args.reason,call.args.scope?.kind==='account'?call.args.scope.accountId:'all']);

test('account card click sends exact usage-only scope and preserves unchanged cards, timestamps, errors and footer',async()=>{
 const h=await bootCards();try{
  const other=card(h,'qwen');const old=other.outerHTML;const footer=h.select('#status').textContent;
  const scans=h.calls.filter(c=>c.command==='fetch_spend').length;
  h.snapshots([snapshot('claude',{metrics:[metric(37)],fetched_at:1800000000000}),...originals.slice(1)]);
  await refresh(h,'claude');
  assert.deepEqual(scopes(h),[['automatic','all'],['userRefresh','claude']]);
  assert.equal(h.calls.filter(c=>c.command==='fetch_spend').length,scans);
  assert.equal(h.calls.filter(c=>c.command==='sync_prices').length,0);
  assert.equal(card(h,'qwen'),other,'unrelated card DOM is untouched');assert.equal(other.outerHTML,old);
  assert.equal(card(h,'claude@extra').querySelector('.provider-name').textContent,'Extra');
  assert.match(card(h,'claude').innerHTML,/37%/);assert.equal(h.select('#status').textContent,footer);
  assert.deepEqual(h.errors,[]);
 }finally{h.close();}
});

test('separate queued targets never broaden into global refresh and duplicate targets coalesce',async()=>{
 const held=deferred();const h=await bootCards();try{
  h.delayNextUsage(held.promise);await refresh(h,'claude');
  assert.equal(h.select('[data-account-refresh="claude"]').disabled,true);
  assert.equal(h.select('[data-account-refresh="qwen"]').disabled,false);
  await refresh(h,'claude');await refresh(h,'qwen');await refresh(h,'qwen');await refresh(h,'claude@extra');
  assert.equal(h.select('[data-account-refresh="qwen"]').getAttribute('aria-busy'),'true');
  held.resolve();await settle();
  assert.deepEqual(scopes(h),[['automatic','all'],['userRefresh','claude'],['userRefresh','qwen'],['userRefresh','claude@extra']]);
  assert.equal(h.calls.filter(c=>c.command==='fetch_spend').length,1);
  for(const button of h.w.document.querySelectorAll('[data-account-refresh]'))assert.equal(button.disabled,false);
 }finally{held.resolve();await settle();h.close();}
});

test('card click queued during automatic global work does not confer global manual intent',async()=>{
 const held=deferred();const h=await bootCards({initialConfig:{accessPolicy:policy,layout:{providerOrder:[],providers:{}}},initialUsageDelay:held.promise});try{
  await refresh(h,'claude@extra');await refresh(h,'qwen');held.resolve();await settle();
  assert.deepEqual(scopes(h),[['automatic','all'],['userRefresh','claude@extra'],['userRefresh','qwen']]);
  assert.equal(h.calls.filter(c=>c.command==='fetch_spend').length,1);
 }finally{held.resolve();await settle();h.close();}
});

test('global and settings refreshes retain their own reason and spend intent behind a card refresh',async()=>{
 const held=deferred();const h=await bootCards();try{
  h.delayNextUsage(held.promise);await refresh(h,'claude');await refresh(h,'qwen');await h.click('#refresh');
  held.resolve();await settle();
  assert.deepEqual(scopes(h),[['automatic','all'],['userRefresh','claude'],['userRefresh','all'],['userRefresh','qwen']]);
  const held2=deferred();h.delayNextUsage(held2.promise);await refresh(h,'claude');await h.change('#stepfun-plan','1600');held2.resolve();await settle();
  assert.deepEqual(scopes(h).slice(-2),[['userRefresh','claude'],['automatic','all']]);
  assert.equal(h.calls.filter(c=>c.command==='fetch_spend').length,2);
 }finally{held.resolve();await settle();h.close();}
});

test('card refresh never advances the global automatic-refresh throttle',async()=>{
 const h=await bootCards();try{
  h.advance(50_000);await refresh(h,'claude');h.advance(11_000);await h.tick(5*60_000);
  assert.deepEqual(scopes(h),[['automatic','all'],['userRefresh','claude'],['automatic','all']]);
 }finally{h.close();}
});

test('account refresh buttons and scoped busy state survive locale repaint and preserve card focus and scroll',async()=>{
 const h=await bootCards();const held=deferred();try{
  h.delayNextUsage(held.promise);await refresh(h,'claude@extra');
  for(const locale of ['en','zh','ru']){
   await h.change('#locale',locale);
   const button=h.select('[data-account-refresh="claude@extra"]');assert.equal(button.disabled,true);assert.equal(button.getAttribute('aria-busy'),'true');
   assert.match(button.title,locale==='zh'?/此账号|该账号/:/this account/);
  }
  held.resolve();await settle();
  const button=h.select('[data-account-refresh="claude@extra"]');button.focus();h.select('#providers').scrollTop=87;
  await refresh(h,'claude@extra');assert.equal(h.w.document.activeElement,h.select('[data-account-refresh="claude@extra"]'));assert.equal(h.select('#providers').scrollTop,87);
  assert.match(button.getAttribute('aria-label'),/Extra/);
 }finally{held.resolve();await settle();h.close();}
});

test('rejected scoped call reports the target error without overwriting other cards or a global Updated timestamp',async()=>{
 const h=await bootCards();const held=deferred();try{
  const footer=h.select('#status').textContent;const other=card(h,'qwen');const old=other.outerHTML;
  h.delayNextUsage(held.promise);await refresh(h,'claude');held.reject(new Error('Synthetic quota request failed'));await settle();
  assert.match(card(h,'claude').textContent,/Synthetic quota request failed/);assert.equal(card(h,'qwen'),other);assert.equal(other.outerHTML,old);
  assert.equal(h.select('#status').textContent,footer);assert.equal(h.select('[data-account-refresh="claude"]').disabled,false);
  await refresh(h,'claude');assert.doesNotMatch(card(h,'claude').textContent,/Synthetic quota request failed/);
 }finally{held.resolve();await settle();h.close();}
});

test('scope queues are cancelled when access mutations begin, and pending mutation clicks require a new click',async()=>{
 const h=await bootCards();const usage=deferred();const reply=deferred();try{
  h.delayNextUsage(usage.promise);await refresh(h,'claude');await refresh(h,'qwen');
  h.delayNextAccessReply('set_provider_account',reply.promise);await h.change('[data-access-account="claude@extra"]',false);
  await refresh(h,'qwen');assert.match(h.select('#status').textContent,/Wait.*finish.*click/i);
  usage.resolve();await settle();assert.deepEqual(scopes(h),[['automatic','all'],['userRefresh','claude']]);
  reply.resolve();await settle();assert.ok(scopes(h).slice(2).every(([reason])=>reason==='automatic'));
  await refresh(h,'qwen');assert.deepEqual(scopes(h).at(-1),['userRefresh','qwen']);
 }finally{usage.resolve();reply.resolve();await settle();h.close();}
});

test('unknown, unauthorized, disabled and forged local-source targets cannot dispatch account refresh',async()=>{
 const h=await bootCards({initialConfig:{accessPolicy:policy,disabled:['qwen']}});try{
  const before=h.usage().length;
  for(const id of ['unknown','claude@missing','qwen','source:claude']){
   const button=h.w.document.createElement('button');button.dataset.accountRefresh=id;h.select('#providers').append(button);button.click();await settle();button.remove();
  }
  assert.equal(h.usage().length,before);
  assert.equal(h.w.document.querySelector('[data-provider="qwen"]'),null);
 }finally{h.close();}
});

test('local-log cards do not get quota buttons even when their source shares an account family',async()=>{
 const window={cost:1,tokens:100,models:[]};
 const h=await bootCards({initialSources:{claude:{enabled:true,mode:'custom',customDirectories:['/synthetic/logs'],defaultDirectories:[]}},spendRows:[{id:'local:claude',name:'Claude logs',sources:['claude'],scan_revision:1,local_stats:[],today:window,yesterday:window,last30:window,trend:[],unpriced:0,unpriced_models:[],month_cost:1}]});try{
  const local=h.select('[data-local-source="local:claude"]');assert.equal(local.querySelector('[data-account-refresh]'),null);
  h.select('[data-account-refresh="claude"]');
 }finally{h.close();}
});

test('scoped publication remains authoritative for removals and safety corrections',async()=>{
 const h=await bootCards();try{
  h.snapshots([snapshot('claude',{metrics:[metric(32)]}),snapshot('qwen',{status:'error',error:'Access changed',metrics:[]})]);
  await refresh(h,'claude');assert.equal(h.w.document.querySelector('[data-provider="claude@extra"]'),null);assert.match(card(h,'qwen').textContent,/Access changed/);
 }finally{h.close();}
});

test('Kimi plan refresh labels saved wallet rows with their own historical time and warning',async()=>{
 const wallet={label:'Balance',kind:'text',value:'$7.00',detail:null,used_percent:null,resets_at:null,period_ms:null};
 const h=await bootCards({snapshots:[snapshot('kimi',{metrics:[metric(10),wallet]})]});try{
  h.snapshots([snapshot('kimi',{metrics:[metric(23),wallet],fetched_at:1800000000000,wallet_history:{fetched_at:1700000000123,attempted_at:1700000050123,warning:'Synthetic wallet warning'}})]);
  await refresh(h,'kimi');
  if(!card(h,'kimi').querySelector('[data-row="Balance"]'))await h.click('[data-caret="kimi"]');
  for(const locale of ['en','zh','ru']){
   await h.change('#locale',locale);const saved=card(h,'kimi').querySelector('[data-wallet-history]');assert.ok(saved);
   assert.match(saved.textContent,locale==='zh'?/已保存.*钱包/:/Saved Moonshot wallet/);
   assert.match(saved.title,/2023/);assert.match(saved.title,/Synthetic wallet warning/);
   assert.match(saved.textContent,locale==='zh'?/全局刷新/:/global Refresh/);
   assert.match(card(h,'kimi').querySelector('[data-row="Balance"]').textContent,locale==='zh'?/已保存/:/Saved/);
   assert.match(card(h,'kimi').textContent,/\$7\.00/);
  }
 }finally{h.close();}
});

test('untargeted Kimi wallet safety removal replaces same-ID DOM immediately',async()=>{
 const wallet={label:'Balance',kind:'text',value:'$7.00',detail:null,used_percent:null,resets_at:null,period_ms:null};
 const kimi=snapshot('kimi',{metrics:[metric(10),wallet],wallet_history:{fetched_at:1700000000123}});
 const h=await bootCards({snapshots:[...originals,kimi]});try{
  const before=card(h,'kimi');assert.ok(before.querySelector('[data-wallet-history]'));
  h.snapshots([...originals,snapshot('kimi',{metrics:[metric(10)]})]);await refresh(h,'claude');
  assert.notEqual(card(h,'kimi'),before);assert.equal(card(h,'kimi').querySelector('[data-wallet-history]'),null);assert.doesNotMatch(card(h,'kimi').textContent,/\$7\.00/);
 }finally{h.close();}
});

test('late cached response cannot restore cards after an authoritative empty scoped publication',async()=>{
 const cached=deferred();const h=await bootCards({initialConfig:{accessPolicy:policy,layout:{providerOrder:[],providers:{}}},initialCachedDelay:cached.promise});try{
  h.snapshots([]);await refresh(h,'claude');assert.equal(h.w.document.querySelector('[data-provider]'),null);
  cached.resolve();await settle();assert.equal(h.w.document.querySelector('[data-provider]'),null);
 }finally{cached.resolve();await settle();h.close();}
});

test('a late full spend scan cannot overwrite a later scoped quota publication',async()=>{
 const spend=deferred();const h=await bootCards({initialSpendDelay:spend.promise});try{
  h.snapshots([snapshot('claude',{metrics:[metric(67)]}),...originals.slice(1)]);await refresh(h,'claude');
  spend.resolve();await settle();assert.match(card(h,'claude').innerHTML,/67%/);
  assert.deepEqual(scopes(h),[['automatic','all'],['userRefresh','claude']]);
 }finally{spend.resolve();await settle();h.close();}
});

test('scoped results preserve provider and log-source drafts, caret and settings focus',async()=>{
 const h=await bootCards();try{
  await h.click('#customize-btn');await h.click('[data-cust-expand="qwen"]');
  const key=h.select('#key-qwen');key.value='synthetic-unsaved-draft';key.focus();key.setSelectionRange(4,8);
  await refresh(h,'claude');assert.equal(h.select('#key-qwen').value,key.value);assert.equal(h.w.document.activeElement,h.select('#key-qwen'));assert.equal(h.select('#key-qwen').selectionStart,4);
  await h.click('[data-customize-close]');await h.click('#settings-btn');await h.change('[data-scan-mode="claude"]','custom');
  const logs=h.select('#scan-paths-claude');logs.value='/synthetic/unsaved';logs.focus();logs.setSelectionRange(3,10);
  await refresh(h,'claude');assert.equal(h.w.document.activeElement,logs);assert.equal(logs.value,'/synthetic/unsaved');assert.equal(logs.selectionStart,3);
 }finally{h.close();}
});

test('collapsed widget reads the updated real card and labels saved wallet metrics',async()=>{
 const api={label:'API',kind:'progress',used_percent:7,value:null,detail:null,resets_at:null,period_ms:null};
 const h=await bootCards({initialConfig:{accessPolicy:policy,widgetMode:true,widgetCollapsed:true},snapshots:[snapshot('kimi',{metrics:[api]})]});try{
  h.snapshots([snapshot('kimi',{metrics:[{...api,used_percent:31}],wallet_history:{fetched_at:1700000000123}})]);await refresh(h,'kimi');await settle();
  assert.match(h.select('#widget-ticker .ticker-title').textContent,/Saved/);assert.equal(h.select('#widget-ticker .fill').style.width,'31%');
  assert.equal(h.w.document.body.classList.contains('widget-collapsed'),true);assert.deepEqual(h.errors,[]);
 }finally{h.close();}
});

test('global successful refresh clears an earlier card transport error',async()=>{
 const h=await bootCards();const held=deferred();try{
  h.delayNextUsage(held.promise);await refresh(h,'claude');held.reject(new Error('Synthetic connection error'));await settle();
  assert.match(card(h,'claude').textContent,/Synthetic connection error/);
  await h.click('#refresh');assert.doesNotMatch(card(h,'claude').textContent,/Synthetic connection error/);
 }finally{held.resolve();await settle();h.close();}
});

test('header refresh button remains keyboard focusable and pointer events never arm card or widget dragging',async()=>{
 const h=await bootCards({initialConfig:{accessPolicy:policy,widgetMode:true}});try{
  const button=h.select('[data-account-refresh="claude"]');button.focus();assert.equal(h.w.document.activeElement,button);assert.equal(button.type,'button');
  const enter=new h.w.KeyboardEvent('keydown',{key:'Enter',bubbles:true,cancelable:true});button.dispatchEvent(enter);assert.equal(enter.defaultPrevented,false,'native button keyboard activation is not swallowed');
  button.dispatchEvent(new h.w.MouseEvent('mousedown',{button:0,bubbles:true,cancelable:true}));
  button.dispatchEvent(new h.w.MouseEvent('pointerdown',{button:0,bubbles:true,cancelable:true}));
  button.dispatchEvent(new h.w.MouseEvent('mousemove',{clientX:35,clientY:35,bubbles:true}));
  button.dispatchEvent(new h.w.MouseEvent('mouseup',{button:0,bubbles:true}));button.click();await settle();
  assert.equal(card(h,'claude').draggable,false);assert.equal(h.w.document.body.classList.contains('row-dragging'),false);
  assert.equal(h.calls.filter(call=>call.command==='widget_start_drag').length,0);assert.deepEqual(scopes(h).at(-1),['userRefresh','claude']);
 }finally{h.close();}
});

test('authorized extra OpenCode account receives and dispatches an exact-account quota button',async()=>{
 const id='opencode@synthetic';
 const accessPolicy={...policy,enabledFamilies:['opencode'],enabledAccounts:[id],accountBindings:{[id]:{family:'opencode',directory:'/synthetic/opencode',name:'Extra OpenCode'}}};
 const h=await bootCards({initialConfig:{accessPolicy},snapshots:[snapshot(id,{name:'Extra OpenCode'})]});try{
  await refresh(h,id);assert.deepEqual(scopes(h),[['automatic','all'],['userRefresh',id]]);
  assert.equal(h.calls.filter(call=>call.command==='fetch_spend').length,1);
 }finally{h.close();}
});

for(const binding of [undefined,{family:'claude',directory:'/synthetic/wrong',name:'Wrong binding'}])test(`extra OpenCode with ${binding?'mismatched':'missing'} policy binding has no quota action`,async()=>{
 const id='opencode@unbound';
 const accessPolicy={...policy,enabledFamilies:['opencode'],enabledAccounts:[id],accountBindings:binding?{[id]:binding}:{}};
 const h=await bootCards({initialConfig:{accessPolicy},snapshots:[snapshot(id)]});try{
  assert.equal(card(h,id).querySelector('[data-account-refresh]'),null);
  const button=h.w.document.createElement('button');button.dataset.accountRefresh=id;card(h,id).append(button);button.click();await settle();
  assert.deepEqual(scopes(h),[['automatic','all']]);
 }finally{h.close();}
});

test('mixed-freshness Kimi badge explains saved wallet without inventing a failed plan refresh',async()=>{
 const wallet={label:'Balance',kind:'text',value:'$7.00',detail:null,used_percent:null,resets_at:null,period_ms:null};
 const history={source_account_id:'moonshot',fetched_at:1700000000123,warning:'Moonshot API wallet: Synthetic wallet failure'};
 const h=await bootCards({snapshots:[snapshot('kimi',{metrics:[metric(23),wallet],stale:true,attempt_failed:false,wallet_history:history})]});try{
  for(const locale of ['en','zh','ru']){
   await h.change('#locale',locale);const badge=card(h,'kimi').querySelector('.provider-head .stale');assert.ok(badge);
   assert.match(badge.title,locale==='zh'?/钱包/:/wallet/i);assert.doesNotMatch(badge.title,/last refresh failed|last good numbers|上次刷新失败|Последнее обновление/i);
  }
  h.snapshots([snapshot('kimi',{metrics:[metric(23),wallet],stale:true,attempt_failed:true,warning:'Synthetic plan refresh failed',wallet_history:history})]);await refresh(h,'kimi');
  assert.match(card(h,'kimi').querySelector('.provider-head .stale').title,/Synthetic plan refresh failed/);
  assert.match(card(h,'kimi').querySelector('[data-wallet-history]').title,/Synthetic wallet failure/);
 }finally{h.close();}
});

test('periodic repaint preserves idle refresh focus and restores focused in-flight action without stealing later focus',async()=>{
 const h=await bootCards();const held=deferred();try{
  h.select('[data-account-refresh="claude"]').focus();h.select('#providers').scrollTop=61;
  await h.tick(30_000);assert.equal(h.w.document.activeElement,h.select('[data-account-refresh="claude"]'));assert.equal(h.select('#providers').scrollTop,61);
  h.delayNextUsage(held.promise);await refresh(h,'claude');await h.tick(30_000);held.resolve();await settle();
  assert.equal(h.w.document.activeElement,h.select('[data-account-refresh="claude"]'));
  const held2=deferred();h.delayNextUsage(held2.promise);await refresh(h,'claude');h.select('#settings-btn').focus();await h.tick(30_000);held2.resolve();await settle();
  assert.equal(h.w.document.activeElement,h.select('#settings-btn'));
 }finally{held.resolve();await settle();h.close();}
});

test('first wallet failure without saved values remains visible after successful Kimi-only refresh',async()=>{
 const warning='Moonshot API wallet: Synthetic first wallet request failed';
 const history={source_account_id:'moonshot',fetched_at:null,attempted_at:1700000050123,warning};
 const h=await bootCards({snapshots:[snapshot('kimi',{metrics:[metric(10)]})]});try{
  h.snapshots([snapshot('kimi',{metrics:[metric(42)],fetched_at:1800000000000,wallet_history:history})]);await refresh(h,'kimi');
  for(const locale of ['en','zh','ru']){
   await h.change('#locale',locale);const current=card(h,'kimi');const saved=current.querySelector('[data-wallet-history]');assert.ok(saved);
   assert.match(saved.textContent,locale==='zh'?/Moonshot 钱包查询记录/:/Moonshot wallet query history/);
   assert.match(saved.textContent,locale==='zh'?/上次查询成功时间未知/:/last successful query time unknown/);
   assert.match(saved.textContent,locale==='zh'?/上次查询尝试/:/last query attempt/);
   assert.match(saved.textContent,/2023/);assert.doesNotMatch(saved.textContent,/2027/,'plan success clock must not become a wallet clock');
   assert.ok(saved.textContent.includes(warning));assert.ok(saved.title.includes(warning));
   assert.match(saved.textContent,locale==='zh'?/全局刷新/:/global Refresh/);
   assert.match(saved.textContent,locale==='zh'?/仅查询 Kimi 套餐/:/Kimi plan only/);
   assert.equal(current.querySelector('[data-row="Balance"], [data-row="API"], [data-row="Cash"], [data-row="Vouchers"], [data-row="Credits used"]'),null);
   assert.equal(current.querySelector('.provider-head .stale'),null,'successful plan query is not a failed plan query');
   assert.match(current.querySelector('[data-row="Weekly"]').innerHTML,/42%/);
  }
  assert.deepEqual(scopes(h),[['automatic','all'],['userRefresh','kimi']]);
  assert.equal(h.calls.filter(call=>call.command==='fetch_spend').length,1);assert.deepEqual(h.errors,[]);
 }finally{h.close();}
});

test('wallet history without known attempt or success time never fabricates a date',async()=>{
 const history={source_account_id:'moonshot',fetched_at:null,attempted_at:null,warning:'Synthetic wallet error with unknown clock'};
 const h=await bootCards({snapshots:[snapshot('kimi',{wallet_history:history})]});try{
  const saved=card(h,'kimi').querySelector('[data-wallet-history]');assert.ok(saved);
  assert.match(saved.textContent,/last successful query time unknown/);assert.doesNotMatch(saved.textContent,/last query attempt|2023|2027|1970/);
  assert.ok(saved.textContent.includes(history.warning));
 }finally{h.close();}
});
