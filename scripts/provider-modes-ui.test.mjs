import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
async function load(relative) {
  const source = readFileSync(new URL(relative, import.meta.url), 'utf8');
  const js = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2020 } }).outputText;
  return import(`data:text/javascript;base64,${Buffer.from(js).toString('base64')}`);
}
const { createProviderModeHandlers, renderProviderMode, localModeSelection, dropModeSnapshots } = await load('../src/provider-modes.ts');
const {t, setActiveLocale} = await load('../src/i18n.ts');
const catalog = [
  {id:'qwen',family:'qwen',choices:[{value:'china:bearer',label:'China · Bearer key'},{value:'international:x_api_key',label:'International · x-api-key'}],selected:null,defaultSelection:null,allowLocalOrigin:false},
  {id:'ollama',family:'ollama',choices:[{value:'local:http://127.0.0.1:11434',label:'Local IPv4 · port 11434'}],selected:null,defaultSelection:null,allowLocalOrigin:true},
  {id:'antigravity',family:'antigravity',choices:[{value:'cloud',label:'Google Cloud Code'},{value:'local_process',label:'Local Antigravity process'}],selected:null,defaultSelection:'cloud',allowLocalOrigin:false},
];
const esc = value => value.replace(/[&<>"']/g, ch => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[ch]));
const form = (id, value) => ({dataset:{providerLocal:id},querySelector:()=>({value})});
function setup() {
 const calls=[], errors=[];
 const handlers=createProviderModeHandlers(()=>catalog, async (command,input)=>{calls.push({command,input});}, key=>errors.push(key));
 return {handlers,calls,errors};
}

test('actual select handler uses the typed catalog and exact account under synthetic IPC, without a grant', async()=>{
 const {handlers,calls}=setup();
 await handlers.change({dataset:{providerMode:'qwen'},value:'international:x_api_key'});
 assert.deepEqual(calls,[{command:'set_provider_region',input:{id:'qwen',region:'international:x_api_key'}}]);
});

test('actual select handler rejects unknown accounts, missing or foreign modes', async()=>{
 const {handlers,calls,errors}=setup();
 for (const [id,value] of [['qwen',''],['qwen','china:api_key'],['qwen@spoofed','china:bearer'],['qwen','https://attacker.example']]) await handlers.change({dataset:{providerMode:id},value});
 assert.equal(calls.length,0); assert.equal(errors.length,4);
});

test('actual local form handler only configures Ollama exact loopback origins', async()=>{
 const {handlers,calls,errors}=setup();
 await handlers.submit(form('ollama','http://[::1]:12345'));
 await handlers.submit(form('qwen','http://127.0.0.1:12345'));
 await handlers.submit(form('ollama','http://192.168.1.3:11434'));
 assert.deepEqual(calls,[{command:'set_provider_region',input:{id:'ollama',region:'local:http://[::1]:12345'}}]);
 assert.equal(errors.length,2);
});

test('local origin validation rejects credentials, nonloopback, paths, query and fragment',()=>{
 for(const origin of ['http://token@localhost:11434','http://localhost:11434/api','http://localhost:11434?q=x','http://localhost:11434#x','http://localhost.evil:11434','http://0.0.0.0:11434','ftp://127.0.0.1:11434','http://127.0.0.1:0']) assert.throws(()=>localModeSelection(origin));
 assert.equal(localModeSelection(' http://localhost:12001/ '),'local:http://localhost:12001');
});

test('disabled accounts have selectable modes and missing-selection guidance in English and Chinese',()=>{
 for (const locale of ['en','zh']) {
  setActiveLocale(locale);
  const html=renderProviderMode(catalog[0],undefined,t,esc);
  assert.match(html,/data-provider-mode="qwen"/);
  assert.doesNotMatch(html,/<select[^>]*disabled/);
  assert.ok(html.includes(t('settings.modeMissing')));
  assert.ok(html.includes(t('settings.modeNoGrant')));
  assert.ok(html.includes(t('mode.option.china:bearer')));
  assert.doesNotMatch(html,/settings\.mode|mode\.option/);
 }
});

test('local custom control does not create a general remote service editor; Antigravity defaults to cloud',()=>{
 setActiveLocale('en');
 assert.match(renderProviderMode(catalog[1], 'local:http://localhost:12500',t,esc),/value="http:\/\/localhost:12500"/);
 const antigravity=renderProviderMode(catalog[2],undefined,t,esc);
 assert.match(antigravity,/<option value="cloud" selected>/);
 assert.doesNotMatch(antigravity,/data-provider-local|input/);
});

test('mode change drops affected old snapshots including folded Moonshot wallet',()=>{
 const snapshots=[{id:'moonshot'},{id:'kimi'},{id:'qwen'}];
 assert.deepEqual(dropModeSnapshots(snapshots,'moonshot'),[{id:'qwen'}]);
 assert.deepEqual(dropModeSnapshots(snapshots,'qwen'),[{id:'moonshot'},{id:'kimi'}]);
});

test('unsupported persisted selection never looks like an implicitly selected regional mode',()=>{
 setActiveLocale('zh');
 const html=renderProviderMode(catalog[0],'china:legacy_cookie',t,esc);
 assert.match(html,/<option value="" disabled selected>/);
 assert.ok(html.includes(t('settings.modeMissing')));
 const local=renderProviderMode(catalog[1],'local:http://remote.invalid:11434',t,esc);
 assert.match(local,/<option value="" disabled selected>/);
 assert.doesNotMatch(local,/remote\.invalid/);
});

test('actual settings startup preserves catalog failure through locale repaint and explicit retry restores controls', async()=>{
 const modes=await load('../src/provider-modes.ts');
 const {acceptPriceEpoch}=await load('../src/manual-pricing.ts');
 const source=readFileSync(new URL('../src/main.ts',import.meta.url),'utf8');
 const ast=ts.createSourceFile('main.ts',source,ts.ScriptTarget.Latest,true,ts.ScriptKind.TS);
 const declarations=ast.statements.filter(node=>ts.isVariableStatement(node) && node.declarationList.declarations.some(declaration=>['providerModeCatalog','providerModeHandlers'].includes(declaration.name.getText(ast))));
 const functions=ast.statements.filter(node=>ts.isFunctionDeclaration(node) && ['normalizeProviderAutoRefresh','renderProviderAutoRefresh','isManualQuotaProvider','experimentalProviderBadge','preserveSettingsFocus','renderProviderAccess','renderCustomize','renderProviderAccountSettings','applyLocale'].includes(node.name?.text));
 const init=ast.statements.find(node=>ts.isFunctionDeclaration(node) && node.name?.text==='initSettings');
 // Execute the exact production catalog setup, event registration and locale
 // repaint without unrelated autostart/key/widget settings side effects.
 const initStartup=source.slice(init.getStart(ast),source.indexOf('  // First launch timestamp',init.getStart(ast)))+'\n  applyLocale();\n}';
 const compiled=ts.transpileModule([...declarations,...functions].map(node=>node.getText(ast)).join('\n')+'\n'+initStartup,{compilerOptions:{target:ts.ScriptTarget.ES2020,module:ts.ModuleKind.ESNext}}).outputText;
 const listeners={};
 const root={innerHTML:'',querySelectorAll:()=>[],addEventListener:(event,callback)=>{listeners[event]=callback;}};
 const status={textContent:''};
 const config={locale:'zh',accessPolicy:{enabledFamilies:[],enabledAccounts:[],regions:{},accountBindings:{}}};
 const calls=[];
 let reads=0;
 const environment={
  ...modes,t,config,
  scanSourceCatalog:{load:async()=>{}},custExpanded:new Set(),providerFamily:id=>id,
  liveProviderLayout:()=>({metricOrder:[],onDemand:[],hidden:[],starred:[]}),isStarrable:()=>false,displayMetricLabel:value=>value,
  document:{querySelector:id=>id==='#status'?status:id==='#sync-prices'?{addEventListener(){}}:root},
  invoke:async(command)=>{calls.push(command);if(command==='get_config')return config;if(command==='pricing_status')return {last_success_ms:0,catalog_stamp:'builtin'};if(command==='get_provider_modes'){if(reads++===0)throw new Error('temporary <failure>');return catalog;}throw new Error('unexpected IPC');},
  ALL_PROVIDERS:[['qwen','Qwen']],escapeHtml:esc,accountAuthorized:()=>false,changeProviderAccess:()=>assert.fail('retry must not change access'),
  normalizeLocalePref:value=>value,resolveLocale:value=>value,setActiveLocale,
  applyStaticI18n:()=>{},applyWidgetState:async()=>{},applyAppearance:()=>{},
  lastSnapshots:[],lastSpend:[],populatePinnedOptions:()=>{},renderScanSources:()=>{},renderBuildInfo:()=>{},
  confirmedProviderAutoRefresh:{},priceEpoch:0,priceSync:null,lastPriceSync:null,acceptPriceEpoch,renderPriceSuccess:()=>{},
 };
 const api=new Function(...Object.keys(environment),compiled+'\nreturn {initSettings,applyLocale,renderProviderAccess,retry:()=>providerModeCatalog.load()};')(...Object.values(environment));
 await api.initSettings();
 await new Promise(resolve=>setImmediate(resolve));
 assert.equal(status.textContent,t('footer.starting'),'the ordinary status gets repainted');
 assert.ok(root.innerHTML.includes(t('settings.modeLoadFailed',{err:'Error: temporary &lt;failure&gt;'})),'catalog failure must remain in provider settings after startup repaint');
 assert.match(root.innerHTML,/data-provider-modes-retry/);
 assert.doesNotMatch(root.innerHTML,/data-provider-mode="qwen"/);
 config.locale='en'; api.applyLocale();
 assert.ok(root.innerHTML.includes(t('settings.modeRetry')));
 assert.equal(reads,1,'locale repaint must not automatically retry');
 await api.retry();
 await new Promise(resolve=>setImmediate(resolve));
 assert.equal(reads,2);
 assert.match(root.innerHTML,/data-provider-mode="qwen"/);
 assert.doesNotMatch(root.innerHTML,/temporary|data-provider-modes-retry/);
 assert.deepEqual(calls,['get_config','pricing_status','get_provider_modes','get_provider_modes']);
});

test('catalog retries coalesce while pending and stop after failure until another explicit request', async()=>{
 const {createProviderModeCatalog,renderProviderModeCatalogStatus}=await load('../src/provider-modes.ts');
 let calls=0, finish;
 const state=createProviderModeCatalog(()=>{calls++;return new Promise((resolve,reject)=>{finish={resolve,reject};});},()=>{});
 const first=state.load(); const duplicate=state.load();
 assert.equal(first,duplicate);
 await Promise.resolve();
 assert.equal(calls,1); assert.equal(state.state.loading,true);
 finish.reject(new Error('offline')); await first;
 assert.equal(state.state.loading,false);
 for (const locale of ['zh','en']) {
  setActiveLocale(locale);
  assert.match(renderProviderModeCatalogStatus(state.state,t,esc),/role="alert"/);
 }
 await new Promise(resolve=>setImmediate(resolve));
 assert.equal(calls,1,'failure and repaints cannot create background retries');
 const retry=state.load(); await Promise.resolve();
 assert.equal(calls,2);
 assert.match(renderProviderModeCatalogStatus(state.state,t,esc),/data-provider-modes-retry disabled/);
 finish.resolve(catalog); await retry;
 assert.deepEqual(state.state.entries,catalog);
 assert.equal(renderProviderModeCatalogStatus(state.state,t,esc),'');
});
