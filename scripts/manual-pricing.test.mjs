import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
const read = path => readFileSync(new URL(`../${path}`, import.meta.url), 'utf8');
const pricing = read('src-tauri/src/pricing.rs').split('#[cfg(test)]')[0];
const lib = read('src-tauri/src/lib.rs');
const main = read('src/main.ts');
test('startup_and_spend_do_not_download_prices', () => {
  assert.doesNotMatch(pricing, /pub fn ensure_fresh|REFRESH_MS|RETRY_MS|UNPRICED_HINT/);
  assert.match(pricing, /pub async fn sync_catalogs/);
  assert.doesNotMatch(read('src-tauri/src/spend.rs'), /pricing::sync_catalogs/);
  assert.equal((lib.match(/pricing::sync_catalogs\(\)/g) || []).length, 1);
});
test('manual sync has explicit UI and local-only backend recomputation', () => {
  assert.match(read('index.html'), /id="sync-prices"/);
  assert.match(main, /"sync_prices"/);
  const command = lib.slice(lib.indexOf('async fn sync_prices('), lib.indexOf('/// The key a provider', lib.indexOf('async fn sync_prices(')));
  assert.match(command, /spend::collect/);
  assert.doesNotMatch(command, /fetch_usage_csv|fetch_usage\(|fetch_spend\(/);
  assert.match(command, /commit_if_current/);
});

import ts from 'typescript';
const load = async path => import(`data:text/javascript;base64,${Buffer.from(ts.transpileModule(read(path), { compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 } }).outputText).toString('base64')}`);
const { t, setActiveLocale } = await load('src/i18n.ts');
test('manual repricing preserves Cursor charges and rejects stale access/pricing epochs', async () => {
  setActiveLocale('en');
  const { mergeRepricedLocal, acceptPriceEpoch, estimatedCostText } = await load('src/manual-pricing.ts');
  const cursor = { id:'cursor',sources:[],cost:12.34,tokens:1234 };
  const local = { id:'source:claude',sources:['claude'],scan_revision:7,cost:1,tokens:100 };
  const updated = { ...local,cost:2 };
  const rows = mergeRepricedLocal([cursor,local],[updated],7,row=>true);
  assert.equal(rows[0],cursor);
  assert.deepEqual(rows[1],updated);
  assert.equal(rows[1].tokens,local.tokens);
  assert.deepEqual(mergeRepricedLocal([cursor,local],[{...updated,scan_revision:6}],7,row=>true),[cursor]);
  assert.equal(acceptPriceEpoch(1,2),false);
  assert.equal(acceptPriceEpoch(2,2),true);
  assert.equal(estimatedCostText('$0.00',0,1,t),'Unknown');
  assert.equal(estimatedCostText('$3.00',3,1,t),'$3.00 + unknown');
  assert.equal(estimatedCostText('$0.00',0,0,t),'$0.00');
});

import vm from 'node:vm';
for (const locale of ['en', 'zh', 'ru']) test(`actual model tooltip renders aggregated unknown rates as unknown in ${locale}`, () => {
  setActiveLocale(locale);
  const start=main.indexOf('function showModelTip(');
  const end=main.indexOf('// ---------------------------------------------------------------------------',start);
  const source=read('src/manual-pricing.ts').replaceAll('export ','')+'\n'+main.slice(start,end);
  const js=ts.transpileModule(source,{compilerOptions:{target:ts.ScriptTarget.ES2022}}).outputText;
  const tip={innerHTML:'',style:{},offsetHeight:50,offsetWidth:50};
  const models=[{model:'Other',cost:0,tokens:700,unpriced:true},{model:'known-free',cost:0,tokens:20,unpriced:false}];
  const context={lastSpend:[{id:'source:claude',unpriced_models:['original-a','original-b'],today:{cost:0,tokens:720,models}}],document:{querySelector:()=>tip},window:{innerWidth:600,innerHeight:500},fmtMoney:v=>'$'+v.toFixed(2),fmtTokens:String,t,escapeHtml:String};
  vm.createContext(context);vm.runInContext(js,context);
  context.showModelTip({dataset:{spend:'source:claude|today'},getBoundingClientRect:()=>({bottom:10,left:10})});
  assert.ok(tip.innerHTML.includes(`Other</span><span>${locale === 'zh' ? '未知' : 'Unknown'}`));
  assert.ok(tip.innerHTML.includes(locale === 'zh' ? '价格未知' : 'Unknown price'));
  assert.match(tip.innerHTML,/known-free<\/span><span>\$0\.00/);
});
