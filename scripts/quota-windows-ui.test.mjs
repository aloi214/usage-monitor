import test from 'node:test';
import assert from 'node:assert/strict';
import {boot} from './app-boot-harness.mjs';

const policy={version:1,enabledFamilies:['zai','kimi'],enabledAccounts:['zai','kimi'],regions:{},accountBindings:{},scanSources:{},scanRoots:{}};
const metric=(label,used=25,extra={})=>({label,kind:'progress',used_percent:used,value:null,detail:null,resets_at:null,period_ms:null,...extra});
const snapshot=(id,metrics)=>({id,name:id,plan:null,status:'ok',error:null,metrics,stale:false,warning:null,fetched_at:1700000000123,attempt_failed:false});
const layout=(label,extra={})=>({metricOrder:['Other',label,'Web Searches'],hidden:[],onDemand:[],starred:[label],expanded:false,...extra});
const start=(id,metrics,saved,config={})=>boot({initialConfig:{accessPolicy:policy,showUsed:true,layout:{providerOrder:[id],providers:{[id]:saved}},pinned:{provider:id,label:saved.starred[0]??saved.metricOrder[1]},...config},snapshots:[snapshot(id,metrics)]});
const rows=h=>[...h.select('#providers').querySelectorAll('[data-row]')].map(el=>el.dataset.row);

for(const alias of ['TOKENS_LIMIT','token_limit'])test(`Z.ai ${alias} expands at its saved position and retargets star/pin deterministically`,async()=>{
 const h=await start('zai',[metric('Weekly',70),metric('5 Hours',20),metric('Web Searches',9)],layout(alias));try{
  const saved=h.backend();
  assert.deepEqual(saved.layout.providers.zai.metricOrder,['Other','5 Hours','Weekly','Web Searches']);
  assert.deepEqual(saved.layout.providers.zai.starred,['5 Hours']);assert.equal(saved.pinned.label,'5 Hours');
  assert.deepEqual(rows(h),['5 Hours','Weekly','Web Searches']);
  const tray=h.calls.filter(c=>c.command==='sync_tray_surfaces').at(-1);assert.equal(tray.args.projection.pinned.label,'5 Hours');assert.deepEqual(tray.args.projection.providers.zai.starred,['5 Hours']);
  h.snapshots([snapshot('zai',[metric('5 Hours',21),metric('Weekly',71),metric('Web Searches',10)])]);await h.click('[data-account-refresh="zai"]');
  assert.deepEqual(h.backend().layout.providers.zai.metricOrder,saved.layout.providers.zai.metricOrder);assert.deepEqual(h.errors,[]);
 }finally{h.close();}
});

test('generic hidden and on-demand intent applies to both windows without disturbing unrelated preferences',async()=>{
 const h=await start('zai',[metric('5 Hours'),metric('Weekly'),metric('Web Searches')],layout('TOKENS_LIMIT',{hidden:['TOKENS_LIMIT','Missing'],onDemand:['TOKENS_LIMIT','Web Searches']}));try{
  const saved=h.backend().layout.providers.zai;
  assert.deepEqual(saved.hidden,['5 Hours','Weekly','Missing']);assert.deepEqual(saved.onDemand,['5 Hours','Weekly','Web Searches']);
  assert.ok(!rows(h).includes('5 Hours'));assert.ok(!rows(h).includes('Weekly'));
 }finally{h.close();}
});

test('explicit existing window positions and flags survive a mixed old layout without duplicates',async()=>{
 const h=await start('zai',[metric('5 Hours'),metric('Weekly')],layout('TOKENS_LIMIT',{metricOrder:['Weekly','Other','TOKENS_LIMIT','5 Hours'],hidden:['Weekly'],starred:['Weekly','TOKENS_LIMIT']}));try{
  const saved=h.backend().layout.providers.zai;assert.deepEqual(saved.metricOrder,['Weekly','Other','5 Hours']);assert.deepEqual(saved.hidden,['Weekly']);assert.deepEqual(saved.starred,['Weekly','5 Hours']);
 }finally{h.close();}
});

test('missing five-hour window maps old pin to first actual token window; unknown cycles remain visible without fabricated pace/reset',async()=>{
 const unknown='TOKENS_LIMIT (unit=9, number=2)';const h=await start('zai',[metric(unknown,42)],layout('token_limit'));try{
  assert.equal(h.backend().pinned.label,unknown);assert.deepEqual(rows(h),[unknown]);
  const card=h.select('[data-provider="zai"]');assert.match(card.textContent,/unit=9, number=2/);assert.equal(card.querySelector('.tick'),null);assert.equal(card.querySelector('[data-flip="reset"]'),null);
 }finally{h.close();}
});

test('empty/error-era snapshot defers generic migration until an actual token window arrives',async()=>{
 const h=await start('zai',[],layout('TOKENS_LIMIT'));try{
  assert.equal(h.backend().pinned.label,'TOKENS_LIMIT');assert.ok(h.backend().layout.providers.zai.metricOrder.includes('TOKENS_LIMIT'));
  h.snapshots([snapshot('zai',[metric('Weekly')])]);await h.click('[data-account-refresh="zai"]');assert.equal(h.backend().pinned.label,'Weekly');assert.deepEqual(rows(h),['Weekly']);
 }finally{h.close();}
});

test('Kimi monthly shared ratio replaces absent legacy Weekly selectors and preserves independent five-hour reset',async()=>{
 const reset=Date.now()+2*3600000;const h=await start('kimi',[metric('Monthly',0.56,{resets_at:Date.now()+15*86400000}),metric('Session',30,{resets_at:reset,period_ms:18000000})],layout('Weekly'));try{
  assert.equal(h.backend().pinned.label,'Monthly');assert.deepEqual(h.backend().layout.providers.kimi.starred,['Monthly']);assert.ok(!h.backend().layout.providers.kimi.metricOrder.includes('Weekly'));
  const month=h.select('[data-provider="kimi"] [data-row="Monthly"]');assert.equal(month.querySelector('.fill').style.width,'0.56%');assert.equal(month.querySelector('.tick'),null);assert.match(month.querySelector('.left-val').textContent,/1%/);
  assert.ok(h.select('[data-row="Session"] [data-flip="reset"]'));assert.deepEqual(h.errors,[]);
 }finally{h.close();}
});

test('Kimi retains a real legacy Weekly pool when Monthly is also present',async()=>{
 const h=await start('kimi',[metric('Monthly'),metric('Weekly')],layout('Weekly'));try{assert.equal(h.backend().pinned.label,'Weekly');assert.deepEqual(rows(h),['Weekly','Monthly']);}finally{h.close();}
});

test('minimal display follows migrated star and translated labels remain available',async()=>{
 const h=await start('zai',[metric('Weekly',70),metric('5 Hours',20)],layout('TOKENS_LIMIT'),{minimal:true});try{
  assert.deepEqual(rows(h),['5 Hours']);
  for(const locale of ['zh','ru','en']){await h.change('#locale',locale);assert.deepEqual(rows(h),['5 Hours']);assert.doesNotMatch(h.select('[data-provider="zai"]').textContent,/TOKENS_LIMIT|token_limit/);}
 }finally{h.close();}
});

 test('a live bare unknown TOKENS_LIMIT is a real row, not a legacy alias',async()=>{
 const h=await start('zai',[metric('TOKENS_LIMIT'),metric('5 Hours'),metric('Weekly')],layout('TOKENS_LIMIT'));try{
  assert.equal(h.backend().pinned.label,'TOKENS_LIMIT');assert.deepEqual(h.backend().layout.providers.zai.starred,['TOKENS_LIMIT']);assert.deepEqual(rows(h),['TOKENS_LIMIT','5 Hours','Weekly']);
 }finally{h.close();}
});

test('an explicit star keeps its position when the old generic star also exists',async()=>{
 const h=await start('zai',[metric('5 Hours'),metric('Weekly')],layout('TOKENS_LIMIT',{starred:['TOKENS_LIMIT','Weekly','5 Hours']}));try{
  assert.deepEqual(h.backend().layout.providers.zai.starred,['Weekly','5 Hours']);
 }finally{h.close();}
});
