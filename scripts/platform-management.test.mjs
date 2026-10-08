import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
const source = readFileSync(new URL('../src/main.ts', import.meta.url), 'utf8');
const ast = ts.createSourceFile('main.ts', source, ts.ScriptTarget.Latest, true);
const extract = (names) => ast.statements.filter(n => ts.isFunctionDeclaration(n) && names.includes(n.name?.text)).map(n => n.getText(ast)).join('\n');
const compile = text => ts.transpileModule(text, { compilerOptions: { target: ts.ScriptTarget.ES2020 } }).outputText;
async function load(path) {
 const code=ts.transpileModule(readFileSync(new URL(path,import.meta.url),'utf8'),{compilerOptions:{module:ts.ModuleKind.ESNext,target:ts.ScriptTarget.ES2020}}).outputText;
 return import(`data:text/javascript;base64,${Buffer.from(code).toString('base64')}`);
}
const modes=await load('../src/provider-modes.ts');
const {t,setActiveLocale}=await load('../src/i18n.ts');
const providerDecl=ast.statements.find(n=>ts.isVariableStatement(n)&&n.declarationList.declarations.some(d=>d.name.getText(ast)==='ALL_PROVIDERS'));
const providers=new Function(compile(providerDecl.getText(ast))+'; return ALL_PROVIDERS')();
const escapeHtml=s=>String(s).replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const emptyLayout=()=>({metricOrder:[],onDemand:[],starred:[],hidden:[]});
function harness(locale='en') {
 setActiveLocale(locale);
 const calls=[]; const layouts={qwen:{metricOrder:['Tokens'],onDemand:[],starred:[],hidden:[]}};
 const config={accessPolicy:{enabledFamilies:['qwen'],enabledAccounts:['qwen'],accountBindings:{'claude@a':{family:'claude',name:'Work <account>',directory:'/synthetic/claude'}},regions:{}},layout:{providerOrder:['qwen','claude@a','claude'],providers:layouts},disabled:[],stepfunPlanCredits:1600};
 const env={...modes,config,ALL_PROVIDERS:providers,t,escapeHtml,lastSnapshots:[],custExpanded:new Set(['qwen']),
  providerFamily:id=>config.accessPolicy.accountBindings[id]?.family??id,
  liveProviderLayout:id=>layouts[id]??emptyLayout(),isStarrable:()=>false,displayMetricLabel:s=>s,
  providerModeCatalog:{state:{entries:[{id:'qwen',family:'qwen',choices:[{value:'china:bearer',label:'China'}],allowLocalOrigin:false}],loading:false,error:null}},
  providerModeHandlers:{change:async()=>{}},
  changeProviderAccess:async(command,input)=>calls.push({command,input}),providerLayout:id=>layouts[id],saveLayout:()=>calls.push('saveLayout'),
  patchConfig:async input=>calls.push({patch:input}),refresh:async()=>{},
 };
 const api=new Function(...Object.keys(env),compile(extract(['normalizeProviderAutoRefresh','renderProviderAutoRefresh','isManualQuotaProvider','experimentalProviderBadge','accountAuthorized','renderCustomize','renderProviderAccountSettings','handleCustomizeChange']))+'; return {renderCustomize,handleCustomizeChange}')(...Object.values(env));
 return {api,config,calls,layouts};
}
test('management contains all 23 families once, with accounts and metrics inside their family',()=>{
 const {api}=harness();const html=api.renderCustomize();
 assert.equal((html.match(/data-access-family=/g)??[]).length,23);
 assert.equal((html.match(/data-cust-expand=/g)??[]).length,23);
 assert.match(html,/data-access-account="claude@a"/);
 assert.match(html,/Work &lt;account&gt;/);
 assert.match(html,/data-provider-mode="qwen"/);
 assert.match(html,/data-visible="qwen\|Tokens"/);
 assert.doesNotMatch(html,/data-enable=/);
 assert.match(html,/aria-expanded="true"/);
});
test('family off shows remembered account state but disables its query checkbox',()=>{
 const {api,config}=harness();config.accessPolicy.enabledFamilies=[];
 const html=api.renderCustomize();
 assert.match(html,/<input[^>]*data-access-account="qwen"[^>]*disabled/);
 assert.ok(html.includes(t('platform.noAccounts')) || html.includes(t('platform.off')));
});
test('query and visibility controls use separate production handlers, with no implicit account grants',()=>{
 const {api,calls,layouts}=harness();
 api.handleCustomizeChange({dataset:{accessFamily:'qwen'},checked:true});
 api.handleCustomizeChange({dataset:{accessAccount:'qwen'},checked:false});
 api.handleCustomizeChange({dataset:{visible:'qwen|Tokens'},checked:false});
 assert.deepEqual(calls,[{command:'set_provider_family',input:{family:'qwen',enabled:true}},{command:'set_provider_account',input:{account:'qwen',enabled:false}},'saveLayout']);
 assert.deepEqual(layouts.qwen.hidden,['Tokens']);
});
test('each configured key and Step Plan tier live in provider management and general settings has no duplicate permissions',()=>{
 const {api}=harness();const html=api.renderCustomize();
 for(const id of ['openrouter','zai','commandcode','stepfun','minimax','deepseek','kimi','moonshot','elevenlabs','codebuff','kilo','aihubmix','qwen']) assert.match(html,new RegExp(`id="key-${id}"`));
 assert.match(html,/id="stepfun-plan"/);
 assert.match(html,/<option value="1600" selected>/);
 const document=readFileSync(new URL('../index.html',import.meta.url),'utf8');
 const settings=document.slice(document.indexOf('<aside id="settings"'));
 assert.doesNotMatch(settings,/id="provider-access"|id="key-|id="stepfun-plan"/);
});
test('all management copy resolves in English, Chinese and Russian fallback',()=>{
 for(const locale of ['en','zh','ru']) {
  const {api}=harness(locale);const html=api.renderCustomize();
  assert.ok(html.includes(t('platform.title')));
  assert.ok(html.includes(t('platform.displayOnly')));
  assert.doesNotMatch(html,/>platform\.[a-zA-Z]+</);
  assert.notEqual(t('sidebar.customize'),locale==='en'?'Customize':locale==='zh'?'自定义':'Настроить');
 }
});

test('dragging a provider moves its whole account group, while extra-account order remains adjustable',()=>{
 const reorder=new Function(compile(extract(['moveProviderOrder']))+';return typeof moveProviderOrder==="function"?moveProviderOrder:null')();
 assert.ok(reorder,'provider-group reorder helper is present');
 const original=['claude@a','qwen','claude','codex','claude@b'];
 assert.deepEqual(reorder(original,'qwen','claude'),['qwen','claude@a','claude','codex','claude@b']);
 assert.deepEqual(reorder(original,'claude','codex'),['qwen','claude@a','claude','claude@b','codex']);
 assert.deepEqual(reorder(original,'claude@b','claude@a'),['claude@b','claude@a','qwen','claude','codex']);
});

test('reset copy describes revoked access and the actual private key store',()=>{
 for (const locale of ['en','zh','ru']) {
  setActiveLocale(locale);
  assert.ok(t('settings.apiKeysNote').includes('PanePrivate'));
  assert.doesNotMatch(t('settings.resetBody'),/re-detected|重新检测|найдены заново/);
 }
});
