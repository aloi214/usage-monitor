import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import ts from 'typescript';

const read = (path) => readFileSync(new URL(path, import.meta.url), 'utf8');
const compile = (source) => ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2020 } }).outputText;
const i18n = await import(`data:text/javascript;base64,${Buffer.from(compile(read('../src/i18n.ts'))).toString('base64')}`);
const {renderLogSourceSettings}=await import(`data:text/javascript;base64,${Buffer.from(compile(read('../src/log-source-settings.ts'))).toString('base64')}`);
const ast = ts.createSourceFile('main.ts', read('../src/main.ts'), ts.ScriptTarget.Latest, true);
const names = ['normalizeProviderAutoRefresh','isManualQuotaProvider','experimentalProviderBadge','preserveSettingsFocus','renderScanSources', 'localSpendName', 'renderLocalSpendCards', 'donutEntries', 'othersFoldUsd', 'spendVal', 'escapeHtml', 'changeProviderAccess', 'renderCard', 'accountRefreshLabel', 'renderWalletHistory'];
const production = ast.statements.filter((node) => ts.isFunctionDeclaration(node) ? names.includes(node.name?.text) : ts.isVariableStatement(node) && node.declarationList.declarations.some((d) => ['SCAN_SOURCES', 'OTHERS_ID', 'ALL_PROVIDERS'].includes(d.name.getText(ast)))).map((node) => node.getText(ast)).join('\n');
function harness(locale, invoke = async () => { throw 'Log directory is unavailable'; }) {
  i18n.setActiveLocale(locale);
  const root = { innerHTML: '', querySelectorAll:()=>[] }, status = { textContent: '' };
  const entries=['claude','codex','opencode','pi','stepcode','grok','devin','minimax','hermes','kimi','qwen'].map(source=>({source,enabled:source==='claude',mode:'default',customDirectories:[],directories:source==='claude'?[{path:'/synthetic/\"<&日志',status:'found'}]:[]}));
  const ctx = { ...i18n, Promise, invoke,renderLogSourceSettings,scanSourceCatalog:{state:{entries,loading:false,error:null},load:async()=>{}},currentScanSources:()=>entries, config: { accessPolicy: { scanRoots: { claude: ['/synthetic/"<&日志'] } }, spendMetric: 'cost' }, accessSaveQueue: Promise.resolve(),
    document: { querySelector: (selector) => selector === '#scan-sources' ? root : status },
    pendingAccessWrites: 0, refreshQueuedManualEpoch: null, cancelQueuedAccountRefreshes() {},
    canRefreshAccount: () => false, accountRefreshBusy: () => false, accountRefreshErrors: new Map(),
    lastSpend: [], lastSnapshots: [], spendVisible: () => true, SPEND_KEYS: [], renderTrend: () => '',
    renderProviderAccess() {}, renderAll() {}, providerFamily: (id) => id, PROVIDER_ICONS: {}, PROVIDER_LINKS: {},
  };
  vm.createContext(ctx); vm.runInContext(compile(production), ctx);
  return { ctx, root, status };
}

test('actual source controls translate Chinese on/off, actions, accessibility labels and all tool hints', () => {
  const { ctx, root } = harness('zh'); ctx.renderScanSources();
  for (const text of ['已开启', '已关闭', '保存自定义位置', '日志目录的完整绝对路径', '读取Claude Code的本地日志', '选择包含 opencode.db 的目录', '选择包含 token-usage JSONL 文件的 usage 目录']) assert.ok(root.innerHTML.includes(text), text);
  assert.equal((root.innerHTML.match(/data-scan-custom=/g) ?? []).length, 11);
  assert.doesNotMatch(root.innerHTML, /Choose |Allow directory|Turn off|Exact absolute|>On<|>Off</);
  assert.ok(root.innerHTML.includes('/synthetic/&quot;&lt;&amp;日志'));
  for (const locale of ['en', 'ru', 'zh']) {
    i18n.setActiveLocale(locale); ctx.renderScanSources();
    assert.ok(root.innerHTML.includes(i18n.t('settings.scanSaveCustom')));
    assert.doesNotMatch(root.innerHTML, /settings\.scan/);
  }
});

test('static source help uses the production locale application and Russian fallback', () => {
  const html = read('../index.html');
  const section = html.slice(html.lastIndexOf('<div class="acc-group">', html.indexOf('id="scan-sources"')), html.indexOf('id="scan-sources"'));
  const nodes = [...section.matchAll(/data-i18n="([^"]+)"/g)].map((match) => ({ dataset: { i18n: match[1] }, textContent: '' }));
  assert.equal(nodes.length, 3, 'source heading and both help paragraphs must be translated');
  globalThis.document = { documentElement: {}, querySelectorAll: (selector) => selector === '[data-i18n]' ? nodes : [] };
  try {
    for (const locale of ['en', 'zh', 'ru']) {
      i18n.setActiveLocale(locale); i18n.applyStaticI18n();
      for (const node of nodes) assert.equal(node.textContent, i18n.t(node.dataset.i18n));
      assert.match(nodes[2].textContent, locale === 'zh' ? /Cursor.*单独授权.*远程导出/ : /Cursor.*separately authorized remote export/);
      assert.match(nodes[2].textContent, locale === 'zh' ? /账号归属未经核实/ : /unverified account ownership/);
    }
  } finally { delete globalThis.document; }
});

test('actual source-only cards and donut retain localized ownership caveats and details', () => {
  const { ctx } = harness('zh');
  ctx.lastSpend = [{ id: 'source:claude', name: 'Claude logs', sources: ['pi'], today: { cost: 10, tokens: 25 }, local_stats: [
    { label: 'Requests today', value: '3' }, { label: 'Recent models', value: '<synthetic-model>' },
    { label: 'Go session (local estimate)', value: '$1.25 / assumed $12 limit' },
    { label: 'Plan attribution', value: 'Account unverified; authorized logs may include non-plan usage.' },
  ] }];
  const html = ctx.renderLocalSpendCards();
  for (const text of ['Claude 模型用量', '本地日志', '已扫描工具：Pi / oh-my-pi。账号归属未经核实。', '今日请求', '最近使用的模型', 'Go 会话（本地估算）', '$1.25 / 假定上限 $12', '账号未经核实；授权日志可能包含套餐外用量。', '&lt;synthetic-model&gt;']) assert.ok(html.includes(text), text);
  assert.equal(ctx.donutEntries('today')[0].s.name, 'Claude 模型用量 · 本地日志');
  i18n.setActiveLocale('ru');
  assert.match(ctx.renderLocalSpendCards(), /Scanned tools: Pi \/ oh-my-pi\. Account ownership unverified\./);
  assert.equal(ctx.localSpendName({ id: 'source:codex' }), 'Codex-model usage');
});

test('source validation failures use localized status and preserve unknown diagnostics', async () => {
  const { ctx, status } = harness('zh');
  await ctx.changeProviderAccess('set_scan_source', { source: 'claude', directories: ['/synthetic/missing'] });
  assert.equal(status.textContent, '本地日志授权：日志目录不可用');
  ctx.invoke = async () => { throw 'synthetic I/O detail'; };
  await ctx.changeProviderAccess('set_scan_source', {});
  assert.equal(status.textContent, '本地日志授权：synthetic I/O detail');
  i18n.setActiveLocale('ru');
  await ctx.changeProviderAccess('set_scan_source', {});
  assert.equal(status.textContent, 'Local log access: synthetic I/O detail');
});

test('all known grant validation errors and reset failure status have Chinese translations', () => {
  i18n.setActiveLocale('zh');
  for (const text of ['Unknown log source', 'Too many log directories', 'Unknown log source or too many directories', 'Choose an absolute log directory', 'Log directory is unavailable', 'Choose a log directory, not a filesystem root', 'Different log sources must use separate directories', 'Cannot create a new source-grant generation', 'A saved log directory now resolves elsewhere; remove it and explicitly select its new location']) {
    assert.equal(typeof i18n.displayScanError, 'function');
    assert.match(i18n.displayScanError(text), /[\u4e00-\u9fff]/, text);
  }
  assert.equal(i18n.t('footer.accessResetFailed', { err: 'synthetic failure' }), '无法撤销账号及日志访问权限：synthetic failure');
});

test('query cards translate source-only fallback guidance while preserving remote diagnostics', () => {
  const { ctx } = harness('zh');
  const examples = [
    ['hermes', 'Hermes has local statistics only. Enable its directory under Local log sources to view models and sessions separately.', 'Hermes 仅提供本地统计'],
    ['qwen', 'Server quota is unavailable. Qwen request counts and tokens are shown separately when its directory is enabled under Local log sources.', '服务端额度不可用'],
    ['opencode', 'Usage API unavailable (synthetic <503>). Local OpenCode estimates are shown separately when its directory is enabled under Local log sources.', '用量 API 不可用（synthetic &lt;503&gt;）'],
  ];
  for (const [id, error, expected] of examples) {
    const html = ctx.renderCard({ id, name: id, status: 'error', error, metrics: [] });
    assert.ok(html.includes(expected));
    assert.ok(html.includes('本地日志来源'));
  }
  assert.ok(ctx.renderCard({ id: 'qwen', name: 'Qwen', status: 'error', error: 'synthetic remote error', metrics: [] }).includes('synthetic remote error'));
});
