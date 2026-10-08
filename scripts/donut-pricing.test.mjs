import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import ts from 'typescript';
const read = (path) => readFileSync(new URL(path, import.meta.url), 'utf8');
const compile = (source) => ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2020 } }).outputText;
const i18n = await import(`data:text/javascript;base64,${Buffer.from(compile(read('../src/i18n.ts'))).toString('base64')}`);
const ast = ts.createSourceFile('main.ts', read('../src/main.ts'), ts.ScriptTarget.Latest, true);
const names = ['donutEntries', 'localSpendName', 'othersFoldUsd', 'spendVal', 'fmtRate', 'spendCenter', 'fmtSpendVal', 'legendHtml', 'othersBreakdown', 'nextSpendMetric', 'escapeHtml', 'donutGeometry'];
const production = ast.statements.filter((node) => ts.isFunctionDeclaration(node) && names.includes(node.name?.text)).map((node) => node.getText(ast)).join('\n');
function row(id, cost, tokens, unpriced = 0) {
  return { id, name: id, sources: ['claude'], unpriced, today: { cost, tokens, models: [] } };
}
function harness(locale, rows) {
  i18n.setActiveLocale(locale);
  const ctx = { ...i18n, config: { spendMetric: 'cost' }, lastSpend: rows, spendVisible: () => true,
    OTHERS_ID: '__others__', TAU: Math.PI * 2, DONUT_PAD: 2.2 / 37, DONUT_MIN: 0.07,
    fmtMoney: (n) => `$${n.toFixed(2)}`, fmtTokens: String, spendColor: () => '#000' };
  vm.createContext(ctx);
  vm.runInContext(compile(read('../src/manual-pricing.ts').replaceAll('export ', '') + '\n' + production), ctx);
  return ctx;
}

for (const locale of ['en', 'zh']) {
  const unknown = locale === 'zh' ? '未知' : 'Unknown';
  const partial = locale === 'zh' ? '未知' : 'unknown';
  test(`individual legend keeps partial costs and unknown rates explicit in ${locale}`, () => {
    const ctx = harness(locale, [row('mixed', 1, 2_000_000, 1)]);
    ctx.config.spendMetric = 'mtok';
    let entries = ctx.donutEntries('today');
    assert.equal(ctx.spendCenter(entries).primary, unknown);
    assert.ok(ctx.legendHtml(entries).includes(`<span class="legend-val">${unknown}</span>`));
    assert.doesNotMatch(ctx.legendHtml(entries), /\$0\.50\/MTok/);
    assert.match(ctx.legendHtml(entries), /data-pid="mixed"/);
    ctx.config.spendMetric = 'cost'; entries = ctx.donutEntries('today');
    assert.ok(ctx.legendHtml(entries).includes(`$1.00 + ${partial}`));
    assert.equal(entries[0].w.cost, 1, 'the known subtotal must be retained');
    assert.equal(ctx.nextSpendMetric(false), 'mtok');
    ctx.config.spendMetric = 'mtok'; assert.equal(ctx.nextSpendMetric(false), 'tokens');
    ctx.config.spendMetric = 'tokens'; entries = ctx.donutEntries('today');
    assert.match(ctx.legendHtml(entries), /legend-val">2000000</);
    assert.equal(ctx.spendCenter(entries).primary, '2000000');
    assert.equal(ctx.nextSpendMetric(true), 'mtok');
  });

  test(`Others preserves mixed, wholly unknown and known contributions in ${locale}`, () => {
    const ctx = harness(locale, [row('large', 20, 1_000_000), row('mixed', 1, 2_000_000, 1), row('unpriced', 0, 3_000_000, 1), row('known-small', 2, 1_000_000)]);
    let entries = ctx.donutEntries('today');
    let others = entries.find((entry) => entry.s.id === '__others__');
    assert.equal(others.w.cost, 3);
    assert.equal(others.w.tokens, 6_000_000, 'unknown-cost tokens cannot be discarded when folding');
    assert.equal(others.parts.length, 3);
    let breakdown = ctx.othersBreakdown(others);
    assert.ok(breakdown.includes(`mixed · ${locale === 'zh' ? '本地日志' : 'local logs'}  $1.00 + ${partial}`));
    assert.ok(breakdown.includes(`unpriced · ${locale === 'zh' ? '本地日志' : 'local logs'}  ${unknown}`));
    assert.ok(breakdown.includes('known-small') && breakdown.includes('$2.00'));
    assert.ok(breakdown.includes(i18n.t('pricing.knownCostsOnly')), 'the folding threshold refers only to known costs');
    assert.ok(ctx.legendHtml(entries).includes(`$3.00 + ${partial}`));
    assert.match(ctx.legendHtml(entries), /data-pid="__others__"/);
    ctx.config.spendMetric = 'mtok'; entries = ctx.donutEntries('today');
    others = entries.find((entry) => entry.s.id === '__others__');
    breakdown = ctx.othersBreakdown(others);
    assert.ok(ctx.legendHtml([others]).includes(`<span class="legend-val">${unknown}</span>`));
    assert.ok(breakdown.includes('$2.00/MTok'), 'known parts retain their exact rates');
    assert.doesNotMatch(breakdown, /\$0\.50\/MTok|unpriced[^\n]*\$0\.00\/MTok/);
    assert.ok(ctx.legendHtml(entries).includes('$20.00/MTok'));
    ctx.config.spendMetric = 'tokens'; entries = ctx.donutEntries('today');
    others = entries.find((entry) => entry.s.id === '__others__');
    assert.match(ctx.legendHtml([others]), /legend-val">6000000</);
    assert.equal(ctx.spendCenter(entries).primary, '7000000');
    assert.match(ctx.othersBreakdown(others), /unpriced[^\n]*3000000/);
  });

  test(`all-unknown rows remain visible and known-free tokens remain exact in ${locale}`, () => {
    const free = row('known-free', 0, 1_000_000);
    const ctx = harness(locale, [row('unpriced', 0, 3_000_000, 1), free]);
    for (const metric of ['cost', 'mtok']) {
      ctx.config.spendMetric = metric;
      const entries = ctx.donutEntries('today');
      assert.ok(entries.some((entry) => entry.s.id === 'unpriced'));
      assert.ok(ctx.legendHtml(entries).includes(`<span class="legend-val">${unknown}</span>`));
      assert.equal(ctx.fmtSpendVal(free.today, 0), metric === 'cost' ? '$0.00' : '$0.00/MTok');
    }
    ctx.config.spendMetric = 'tokens';
    const entries = ctx.donutEntries('today');
    assert.equal(entries.length, 2);
    assert.equal(ctx.spendCenter(entries).primary, '4000000');
    assert.match(ctx.legendHtml(entries), /legend-val">1000000</);
    assert.match(ctx.legendHtml(entries), /legend-val">3000000</);
  });

  test(`unknown rates have no sector while their legend and exact token geometry remain in ${locale}`, () => {
    for (const mixedCost of [10, 1]) {
      const ctx = harness(locale, [row('known', 20, 1_000_000), row('mixed', mixedCost, 2_000_000, 1)]);
      ctx.config.spendMetric = 'mtok';
      let entries = ctx.donutEntries('today');
      const mixedId = mixedCost === 1 ? '__others__' : 'mixed';
      let geometry = ctx.donutGeometry(entries);
      assert.equal(geometry.total, 20, 'unknown rates must not enter angular weights');
      assert.deepEqual([...geometry.geo.keys()], ['known']);
      assert.equal(geometry.geo.get('known').a1, Math.PI * 2);
      assert.ok(ctx.legendHtml(entries).includes(`data-pid="${mixedId}"`));
      assert.ok(ctx.legendHtml(entries).includes(`<span class="legend-val">${unknown}</span>`));
      ctx.config.spendMetric = 'tokens'; entries = ctx.donutEntries('today'); geometry = ctx.donutGeometry(entries);
      assert.equal(geometry.total, 3_000_000);
      assert.equal(geometry.geo.size, 2);
      assert.ok(geometry.geo.has(mixedId));
      ctx.config.spendMetric = 'cost'; entries = ctx.donutEntries('today'); geometry = ctx.donutGeometry(entries);
      assert.equal(geometry.total, 20 + mixedCost);
      assert.ok(geometry.geo.has(mixedId), 'known-cost subtotal remains representable');
    }
    const ctx = harness(locale, [row('mixed', 10, 2_000_000, 1)]);
    ctx.config.spendMetric = 'mtok';
    const entries = ctx.donutEntries('today');
    assert.equal(ctx.donutGeometry(entries).geo.size, 0);
    assert.equal(ctx.donutGeometry(entries).total, 0);
    assert.ok(ctx.legendHtml(entries).includes(unknown));
  });
}
