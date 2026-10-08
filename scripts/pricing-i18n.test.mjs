import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import ts from 'typescript';
const read = (path) => readFileSync(new URL(path, import.meta.url), 'utf8');
const compile = (source) => ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2020 } }).outputText;
const load = async (path) => import(`data:text/javascript;base64,${Buffer.from(compile(read(path))).toString('base64')}`);
const i18n = await load('../src/i18n.ts');
const pricing = await load('../src/manual-pricing.ts');
const ast = ts.createSourceFile('main.ts', read('../src/main.ts'), ts.ScriptTarget.Latest, true);
const production = ast.statements.filter((node) => ts.isFunctionDeclaration(node) ? ['applyLocale', 'syncPrices', 'renderPriceSuccess', 'spendCenter'].includes(node.name?.text) : ts.isVariableStatement(node) && node.declarationList.declarations.some((d) => d.name.getText(ast) === 'priceSyncStatus')).map((node) => node.getText(ast)).join('\n');
function harness() {
  i18n.setActiveLocale('en');
  const elements = Object.fromEntries(['#sync-prices', '#price-last-success', '#price-sync-status', '#status'].map((id) => [id, { textContent: '', disabled: false, attrs: {}, setAttribute(key, value) { this.attrs[key] = value; } }]));
  let resolve, reject;
  const calls = [];
  const ctx = { ...i18n, ...pricing, Promise, Date,
    config: { locale: 'en', spendMetric: 'cost' }, lastSpend: [], lastSnapshots: [],
    priceSync: null, lastPriceSync: null, priceEpoch: 0, accessEpoch: 0, spendLoaded: false,
    document: { querySelector: (id) => elements[id] }, spendVisible: () => true,
    fmtMoney: (v) => `$${v.toFixed(2)}`, fmtRate: (v) => `$${v.toFixed(2)}`, fmtTokens: String,
    invoke: (command) => {
      calls.push(command);
      if (command === 'sync_prices') return new Promise((yes, no) => { resolve = yes; reject = no; });
      if (command === 'pricing_status') return Promise.resolve({ last_success_ms: 0, catalog_stamp: 'builtin' });
      throw new Error(`Unexpected ${command}`);
    },
  };
  for (const name of ['applyStaticI18n', 'applyWidgetState', 'applyAppearance', 'renderIfVisible', 'populatePinnedOptions', 'renderProviderAccess', 'renderScanSources', 'renderBuildInfo']) ctx[name] = () => {};
  vm.createContext(ctx); vm.runInContext(compile(production), ctx);
  return { ctx, calls, elements, resolve: (unpriced = 0) => resolve({ pricing: { last_success_ms: 1791349200000, catalog_stamp: 'synthetic-new' }, spend: { revision: 7, rows: [{ id: 'source:claude', sources: ['claude'], scan_revision: 7, unpriced }] } }), reject: () => reject(new Error('synthetic <offline>')) };
}

test('pricing settings bind all static controls and help to English/Chinese with Russian fallback', () => {
  const html = read('../index.html');
  const section = html.slice(html.lastIndexOf('<div class="acc-group">', html.indexOf('id="sync-prices"')), html.indexOf('id="price-last-success"'));
  const nodes = [...section.matchAll(/data-i18n="([^"]+)"/g)].map((match) => ({ dataset: { i18n: match[1] }, textContent: '' }));
  assert.equal(nodes.length, 3);
  globalThis.document = { documentElement: {}, querySelectorAll: (selector) => selector === '[data-i18n]' ? nodes : [] };
  try {
    for (const locale of ['en', 'zh', 'ru']) {
      i18n.setActiveLocale(locale); i18n.applyStaticI18n();
      assert.equal(nodes[0].textContent, locale === 'zh' ? '模型价格' : 'Model prices');
      assert.match(nodes[1].textContent, locale === 'zh' ? /仅在点击.*下载/ : /only when you press/);
      assert.match(nodes[1].textContent, locale === 'zh' ? /Cursor.*保持不变/ : /Cursor charges stay unchanged/);
      assert.equal(nodes[2].textContent, locale === 'zh' ? '同步最新价格目录' : 'Sync latest price catalog');
    }
  } finally { delete globalThis.document; }
});

test('estimated costs localize unknown values while keeping known zero and known subtotal', () => {
  for (const locale of ['en', 'zh', 'ru']) {
    i18n.setActiveLocale(locale);
    assert.equal(pricing.estimatedCostText('$0.00', 0, 1, i18n.t), locale === 'zh' ? '未知' : 'Unknown');
    assert.equal(pricing.estimatedCostText('$3.00', 3, 1, i18n.t), locale === 'zh' ? '$3.00 + 未知' : '$3.00 + unknown');
    assert.equal(pricing.estimatedCostText('$0.00', 0, 0, i18n.t), '$0.00');
  }
});

for (const missing of [0, 1]) test(`locale repaint preserves in-flight sync and completed ${missing ? 'missing-price' : 'success'} result`, async () => {
  const h = harness(), { ctx, elements } = h;
  ctx.config.locale = 'zh'; ctx.applyLocale();
  assert.equal(elements['#price-last-success'].textContent, '正在加载本地价格状态…');
  const first = ctx.syncPrices();
  assert.equal(ctx.syncPrices(), first, 'repeated click must coalesce');
  assert.equal(elements['#sync-prices'].disabled, true);
  assert.match(elements['#price-sync-status'].textContent, /正在下载价格目录/);
  ctx.config.locale = 'en'; ctx.applyLocale();
  assert.match(elements['#price-sync-status'].textContent, /Downloading price catalogs/);
  assert.deepEqual(h.calls, ['sync_prices'], 'repainting must not request another sync');
  h.resolve(missing); await first;
  assert.match(elements['#price-sync-status'].textContent, missing ? /Some models still have unknown prices/ : /Local estimates recalculated/);
  const savedStamp = ctx.lastPriceSync.last_success_ms;
  ctx.config.locale = 'zh'; ctx.applyLocale();
  assert.equal(elements['#price-sync-status'].textContent, missing ? '价格已同步。部分模型的价格仍然未知。' : '价格已同步。本地费用估算已重新计算。');
  assert.equal(elements['#price-last-success'].textContent, `上次同步成功：${new Date(savedStamp).toLocaleString('zh-CN')}`);
  assert.equal(elements['#sync-prices'].attrs['aria-busy'], 'false');
  assert.equal(elements['#sync-prices'].disabled, false);
  assert.equal(ctx.priceEpoch, 1);
  assert.equal(ctx.lastSpend[0].unpriced, missing);
  ctx.config.locale = 'ru'; ctx.applyLocale();
  assert.match(elements['#price-sync-status'].textContent, /Prices synchronized/);
  assert.equal(ctx.lastPriceSync.last_success_ms, savedStamp);
});

test('locale changes preserve sync failure and last verified local price status', async () => {
  const h = harness(), { ctx, elements } = h;
  const first = ctx.syncPrices(); h.reject(); await first;
  assert.equal(elements['#price-sync-status'].textContent, 'Price sync failed: Error: synthetic <offline>');
  for (const locale of ['zh', 'en', 'ru']) {
    ctx.config.locale = locale; ctx.applyLocale();
    assert.equal(elements['#price-sync-status'].textContent, locale === 'zh' ? '价格同步失败：Error: synthetic <offline>' : 'Price sync failed: Error: synthetic <offline>');
    assert.match(elements['#price-last-success'].textContent, locale === 'zh' ? /尚未成功手动同步/ : /No successful manual sync yet/);
  }
  assert.deepEqual(h.calls, ['sync_prices', 'pricing_status']);
});

test('actual donut unknown cost and average-cost messages translate without fabricated zero', () => {
  const { ctx } = harness(); ctx.config.locale = 'zh'; ctx.applyLocale();
  ctx.lastSpend = [{ unpriced: 1 }];
  const entries = [{ w: { cost: 0, tokens: 12 } }];
  assert.equal(ctx.spendCenter(entries).primary, '未知');
  assert.equal(ctx.spendCenter(entries).sub, '仅含已知费用');
  assert.equal(ctx.spendCenter(entries).exact, '部分模型价格不可用，总费用未知');
  ctx.config.spendMetric = 'mtok';
  assert.equal(ctx.spendCenter(entries).sub, '缺少价格');
  assert.equal(ctx.spendCenter(entries).exact, '部分模型价格不可用，平均费用未知');
  ctx.lastSpend = []; ctx.config.spendMetric = 'cost';
  assert.equal(ctx.spendCenter(entries).primary, '$0.00');
});
