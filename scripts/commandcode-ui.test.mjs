import test from 'node:test';
import assert from 'node:assert/strict';
import {boot,settle} from './app-boot-harness.mjs';
const deferred=()=>{let resolve,reject;const promise=new Promise((yes,no)=>{resolve=yes;reject=no;});return {promise,resolve,reject};};
const policy={version:1,enabledFamilies:['commandcode','claude','qwen'],enabledAccounts:['commandcode','claude','qwen'],regions:{},accountBindings:{},scanSources:{},scanRoots:{}};
const snapshot=(patch={})=>({id:'commandcode',name:'CommandCode',plan:null,status:'manual',error:null,metrics:[],stale:false,warning:null,fetched_at:null,attempt_failed:false,...patch});
const metric=(label,patch={})=>({label,kind:'text',used_percent:null,detail:null,value:null,resets_at:null,period_ms:null,...patch});
const bootCards=(options={})=>boot({initialConfig:{accessPolicy:policy},snapshots:[snapshot(),snapshot({id:'claude',name:'Claude'})],...options});
const card=h=>h.select('[data-provider="commandcode"]');
const scopes=h=>h.usage().map(call=>[call.args.reason,call.args.scope?.accountId??'all']);

test('CommandCode joins all existing platforms as experimental, default off, with one blank masked local key field',async()=>{
 const h=await boot();try{
  assert.equal(h.w.document.querySelectorAll('[data-access-family]').length,23);
  const section=h.select('[data-cust-provider="commandcode"]');
  assert.match(section.textContent,/Experimental/);assert.match(section.textContent,/GOAT/);
  assert.match(section.textContent,/manual/i);assert.match(section.textContent,/not.*verified|unverified/i);
  assert.equal(h.select('[data-access-family="commandcode"]').checked,false);
  assert.equal(h.select('[data-access-account="commandcode"]').checked,false);
  assert.equal(h.select('[data-access-account="commandcode"]').disabled,true);
  const input=h.select('#key-commandcode');assert.equal(input.type,'password');assert.equal(input.autocomplete,'off');assert.equal(input.value,'');
  assert.equal(h.w.document.querySelectorAll('#key-commandcode').length,1);
  assert.equal(section.querySelector('[data-access-discover]'),null);
  assert.equal(section.querySelector('[data-provider-mode]'),null);
  assert.equal(h.calls.filter(c=>/key|credential|discover_provider/.test(c.command)).length,0);
  assert.deepEqual(h.errors,[]);
 }finally{h.close();}
});

test('saving and clearing the CommandCode key never grants account access and delegated handlers survive repaint',async()=>{
 const h=await boot();try{
  await h.change('[data-access-family="commandcode"]',true);
  h.select('#key-commandcode').value='synthetic-commandcode-key';await h.click('[data-save="commandcode"]');
  assert.equal(h.select('#key-commandcode').value,'');assert.deepEqual(h.backend().accessPolicy.enabledAccounts,[]);
  await h.change('#locale','zh');await h.click('#customize-btn');await h.click('[data-customize-close]');await h.click('#customize-btn');
  await h.click('[data-save="commandcode"]');
  assert.deepEqual(h.calls.filter(c=>c.command==='set_api_key').map(c=>c.args),[{provider:'commandcode',key:'synthetic-commandcode-key'},{provider:'commandcode',key:''}]);
  assert.equal(h.calls.filter(c=>c.command==='set_provider_account').length,0);
  await h.change('[data-access-account="commandcode"]',true);
  assert.deepEqual(h.backend().accessPolicy.enabledAccounts,['commandcode']);
  assert.ok(h.usage().every(c=>c.args.reason==='automatic'));
  assert.doesNotMatch(h.w.document.body.outerHTML,/synthetic-commandcode-key/);
 }finally{h.close();}
});

test('CommandCode first-use and saved manual states localize, keep timestamps, and do not invent failures',async()=>{
 const h=await bootCards();try{
  for(const locale of ['en','zh','ru']){
   await h.change('#locale',locale);
   assert.match(card(h).textContent,locale==='zh'?/手动查询/:/Manual query/);
   assert.match(card(h).textContent,locale==='zh'?/实验性/:/Experimental/);
   assert.equal(card(h).querySelector('.placeholder'),null);
   assert.match(h.select('[data-cust-provider="commandcode"] .platform-account-head .platform-status').textContent,locale==='zh'?/等待手动查询/:/Awaiting manual query/);
   assert.match(h.select('#refresh').title,/CommandCode/);assert.match(h.select('#refresh-interval-help').textContent,/Ctrl\+R/);
  }
  h.snapshots([snapshot({status:'ok',stale:true,fetched_at:1700000000123,metrics:[metric('Monthly credits remaining',{value:'48 credits'})]})]);await h.click('#refresh');
  for(const locale of ['en','zh','ru']){
   await h.change('#locale',locale);const notice=card(h).querySelector('.manual-query');
   assert.match(notice.textContent,locale==='zh'?/已保存结果/:/Saved result/);
   assert.match(notice.title,/2023/);assert.match(notice.title,locale==='zh'?/尚未重新验证/:/have not been reverified/);
   assert.equal(card(h).querySelector('.stale'),null);
   assert.match(h.select('[data-cust-provider="commandcode"] .platform-account-head .platform-status').textContent,locale==='zh'?/已保存结果/:/Saved result/);
  }
 }finally{h.close();}
});

test('credit windows, monthly remainder and separate extras retain honest units and missing resets',async()=>{
 const reset=Date.now()+2*3600000;
 const metrics=[metric('5-hour',{kind:'progress',used_percent:50,detail:'7 of 14 credits used',resets_at:reset,period_ms:5*3600000}),metric('Weekly',{kind:'progress',used_percent:20,detail:'7 of 35 credits used'}),metric('Monthly credits remaining',{value:'49 credits'}),metric('Purchased credits',{value:'12 credits'}),metric('Free credits',{value:'3 credits'}),metric('Subscription period ends',{value:'2026-11-01T00:00:00Z'})];
 const h=await bootCards({snapshots:[snapshot({status:'ok',plan:'individual-goat',metrics})]});try{
  if(card(h).querySelector('[data-caret]'))await h.click('[data-caret="commandcode"]');
  assert.match(card(h).textContent,/7 of 14 credits used/);assert.match(card(h).textContent,/49 credits/);assert.match(card(h).textContent,/12 credits/);assert.match(card(h).textContent,/3 credits/);
  assert.doesNotMatch(card(h).textContent,/inferred|Monthly credits used/);assert.doesNotMatch(card(h).textContent,/\$|USD/);
  assert.match(card(h).querySelector('[data-row="5-hour"]').textContent,/Resets/i);
  assert.doesNotMatch(card(h).querySelector('[data-row="Weekly"]').textContent,/Resets|1970|Not started/i);
  assert.equal(card(h).querySelector('[data-row="Monthly credits used (inferred)"]'),null);
  await h.change('#locale','zh');assert.doesNotMatch(card(h).textContent,/推算/);assert.match(card(h).textContent,/月度剩余/);assert.match(card(h).textContent,/已购/);assert.match(card(h).textContent,/免费/);
  h.snapshots([snapshot({status:'ok',metrics:[metric('Monthly credits remaining',{value:'49 credits'})]})]);await h.click('#refresh');
  assert.equal(card(h).querySelector('[data-row="Monthly credits used (inferred)"]'),null);
  assert.equal(card(h).querySelector('[data-row="5-hour"]'),null);
 }finally{h.close();}
});

test('CommandCode quota is dispatched manually only by card, global Refresh or Ctrl+R, retaining exact card scope',async()=>{
 const h=await bootCards();try{
  h.advance(61000);await h.tick(5*60000);h.advance(61000);await h.emit('popover-shown');
  await h.change('#stepfun-plan','1600');await h.change('#locale','zh');
  assert.ok(h.usage().every(c=>c.args.reason==='automatic'));
  const scans=h.calls.filter(c=>c.command==='fetch_spend').length;
  await h.click('[data-account-refresh="commandcode"]');assert.deepEqual(scopes(h).at(-1),['userRefresh','commandcode']);
  assert.equal(h.calls.filter(c=>c.command==='fetch_spend').length,scans);
  await h.click('#refresh');await h.key();assert.deepEqual(scopes(h).slice(-2),[['userRefresh','all'],['userRefresh','all']]);
  assert.equal(h.calls.filter(c=>c.command==='sync_prices').length,0);
 }finally{h.close();}
});

test('CommandCode failures retain saved numbers with manual retry and key-specific authentication guidance',async()=>{
 const h=await bootCards();try{
  for(const warning of ['HTTP 401 synthetic authentication failure','HTTP 429 synthetic cooldown','HTTP 503 synthetic outage','Synthetic schema failure','network connect failed']){
   h.snapshots([snapshot({status:'ok',stale:true,attempt_failed:true,warning,metrics:[metric('Monthly credits remaining',{value:'49 credits'})],fetched_at:1700000000123})]);await h.click('#refresh');
   for(const locale of ['en','zh','ru']){
    await h.change('#locale',locale);const help=card(h).querySelector('.stale').title;
    assert.ok(help.startsWith(warning+'.'));assert.match(help,/Ctrl\+R/);
    assert.doesNotMatch(help,/automatically|on its own|by itself|会自动|自己重试/);
    if(warning.includes('401')){assert.match(help,/CommandCode/);assert.match(help,/API|密钥/);assert.doesNotMatch(help,/`claude`/);}
   }
  }
 }finally{h.close();}
});

for(const key of ['replacement-synthetic-key',''])test(`CommandCode ${key?'replacement':'clear'} fences queued manual scopes and pending clicks, then requires a fresh click`,async()=>{
 const h=await bootCards();const usage=deferred();const save=deferred();try{
  h.delayNextUsage(usage.promise);await h.click('[data-account-refresh="claude"]');
  await h.click('[data-account-refresh="commandcode"]');await h.click('#refresh');
  h.delayKey(save.promise);h.select('#key-commandcode').value=key;await h.click('[data-save="commandcode"]');
  await h.click('[data-account-refresh="commandcode"]');await h.key();
  assert.match(h.select('#status').textContent,/saving.*finish.*Refresh/i);
  usage.resolve();await settle();assert.deepEqual(scopes(h).filter(([reason])=>reason==='userRefresh'),[['userRefresh','claude']]);
  h.snapshots([snapshot()]);save.resolve();await settle();
  assert.deepEqual(scopes(h).filter(([reason])=>reason==='userRefresh'),[['userRefresh','claude']]);
  assert.equal(h.select('#key-commandcode').value,'');
  await h.click('[data-account-refresh="commandcode"]');assert.deepEqual(scopes(h).at(-1),['userRefresh','commandcode']);
 }finally{usage.resolve();save.resolve();await settle();h.close();}
});

test('failed CommandCode key save hides uncertain old data, keeps newer draft and releases the manual gate',async()=>{
 const h=await bootCards({snapshots:[snapshot({status:'ok',metrics:[metric('Monthly credits remaining',{value:'49 credits'})]})]});const save=deferred();try{
  h.delayKey(save.promise);h.select('#key-commandcode').value='rejected-synthetic-key';await h.click('[data-save="commandcode"]');
  await h.change('#locale','en');h.select('#key-commandcode').value='newer-synthetic-draft';await h.click('#refresh');
  assert.ok(h.usage().every(c=>c.args.reason==='automatic'));
  save.reject(new Error('Synthetic key store failed'));await settle();
  assert.match(h.select('#status').textContent,/Synthetic key store failed/);assert.doesNotMatch(h.select('#status').textContent,/Key saved/);
  assert.equal(h.select('#key-commandcode').value,'newer-synthetic-draft');
  assert.equal(h.w.document.querySelector('[data-provider="commandcode"]'),null);
  assert.match(h.select('#status').textContent,/may have changed/);
  await h.click('#refresh');assert.deepEqual(scopes(h).at(-1),['userRefresh','all']);
 }finally{save.resolve();await settle();h.close();}
});

test('rapid CommandCode key edits are serialized, preserve later draft across repaint and keep manual gate until both finish',async()=>{
 const h=await bootCards();const first=deferred();const second=deferred();try{
  h.delayKey(first.promise);h.select('#key-commandcode').value='first-synthetic-key';await h.click('[data-save="commandcode"]');
  h.delayKey(second.promise);h.select('#key-commandcode').value='second-synthetic-key';await h.click('[data-save="commandcode"]');
  assert.equal(h.calls.filter(c=>c.command==='set_api_key').length,1,'second write waits its turn');
  await h.change('#locale','zh');h.select('#key-commandcode').value='unsubmitted-synthetic-key';
  first.resolve();await settle();await h.key();assert.ok(h.usage().every(c=>c.args.reason==='automatic'));
  assert.deepEqual(h.calls.filter(c=>c.command==='set_api_key').map(c=>c.args.key),['first-synthetic-key','second-synthetic-key']);
  second.resolve();await settle();assert.equal(h.select('#key-commandcode').value,'unsubmitted-synthetic-key');
  await h.key();assert.deepEqual(scopes(h).at(-1),['userRefresh','all']);
 }finally{first.resolve();second.resolve();await settle();h.close();}
});

test('successful partial CommandCode data shows its warning without claiming a failed refresh',async()=>{
 const h=await bootCards({snapshots:[snapshot({status:'ok',warning:'5-hour reset time unknown; Subscription metadata unavailable',metrics:[metric('5-hour',{kind:'progress',used_percent:25,detail:'3.5 of 14 credits used'}),metric('Weekly',{value:'Unknown'}),metric('Monthly credits remaining',{value:'49 credits'})]})]});try{
  const notice=card(h).querySelector('[data-quota-warning]');assert.ok(notice);
  assert.match(notice.textContent,/reset time unknown.*Subscription metadata unavailable/);
  assert.equal(card(h).querySelector('.stale'),null);
  assert.doesNotMatch(card(h).textContent,/Last refresh failed|Saved result/);
  await h.change('#locale','zh');if(card(h).querySelector('[data-caret]'))await h.click('[data-caret="commandcode"]');assert.match(card(h).querySelector('[data-row="Weekly"]').textContent,/未知/);
  h.snapshots([snapshot({status:'ok',metrics:[metric('Monthly credits remaining',{value:'49 credits'})]})]);await h.click('[data-account-refresh="commandcode"]');
  assert.equal(card(h).querySelector('[data-quota-warning]'),null);
 }finally{h.close();}
});


test('partially committed key cleanup failure fences late old quota responses and never reports unchanged credentials',async()=>{
 const h=await bootCards({snapshots:[snapshot({status:'ok',metrics:[metric('Monthly credits remaining',{value:'49 credits'})]})]});const usage=deferred();const key=deferred();try{
  h.delayNextUsage(usage.promise);await h.click('[data-account-refresh="commandcode"]');await h.click('#refresh');
  h.delayKey(key.promise);h.select('#key-commandcode').value='partially-saved-synthetic-key';await h.click('[data-save="commandcode"]');
  key.reject(new Error("the key change is saved, but clearing the previous key's cached data failed: synthetic failure"));await settle();
  assert.match(h.select('#status').textContent,/may have changed/);assert.equal(h.w.document.querySelector('[data-provider="commandcode"]'),null);
  h.snapshots([snapshot()]);usage.resolve();await settle();
  assert.doesNotMatch(h.select('#providers').textContent,/49 credits/);
  assert.equal(h.usage().filter(c=>c.args.reason==='userRefresh').length,1,'old queued click never replays against potentially changed key');
  await h.click('#refresh');assert.deepEqual(scopes(h).at(-1),['userRefresh','all']);
 }finally{usage.resolve();key.resolve();await settle();h.close();}
});

test('CommandCode experimental metadata appears once in a compact card header',async()=>{
 const h=await bootCards({snapshots:[snapshot({status:'ok',plan:'GOAT · Experimental'})]});try{
  assert.equal((card(h).querySelector('.provider-head').textContent.match(/Experimental/g)??[]).length,1);
  assert.equal(card(h).querySelector('.plan').textContent,'GOAT');
  h.snapshots([snapshot({status:'ok',plan:'Experimental'})]);await h.click('#refresh');assert.equal(card(h).querySelector('.plan'),null);
 }finally{h.close();}
});
