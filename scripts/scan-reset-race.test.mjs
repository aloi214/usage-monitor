import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import ts from 'typescript';
const source = readFileSync(new URL('../src/main.ts', import.meta.url), 'utf8');
const ast = ts.createSourceFile('main.ts', source, ts.ScriptTarget.Latest, true);
const production = ast.statements.filter((node) => ts.isFunctionDeclaration(node) && ['changeProviderAccess', 'resetAllSettings'].includes(node.name?.text)).map((node) => node.getText(ast)).join('\n');
const js = ts.transpileModule(production, { compilerOptions: { target: ts.ScriptTarget.ES2020 } }).outputText;
const visibilityJs = ts.transpileModule(readFileSync(new URL('../src/scan-consent.ts', import.meta.url), 'utf8'), { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2020 } }).outputText;
const { acceptSpendBatch } = await import(`data:text/javascript;base64,${Buffer.from(visibilityJs).toString('base64')}`);
const localeJs = ts.transpileModule(readFileSync(new URL('../src/i18n.ts', import.meta.url), 'utf8'), { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2020 } }).outputText;
const { t, setActiveLocale, displayScanError } = await import(`data:text/javascript;base64,${Buffer.from(localeJs).toString('base64')}`);
const clone = (value) => JSON.parse(JSON.stringify(value));
function harness({ resetFails = false, locale = 'en' } = {}) {
  setActiveLocale(locale);
  let backend = { scanRoots: {}, scanEpochs: {}, enabledAccounts: [], enabledFamilies: [] };
  let releaseFirst;
  let sourceCalls = 0;
  const order = [];
  const patches = [];
  const status = { textContent: '', classList: { remove() {} } };
  const ctx = {
    Promise, console, scanSourceCatalog: { load: async () => {}, clear: () => {} }, accessSaveQueue: Promise.resolve(), accessEpoch: 0,
    pendingAccessWrites: 0, refreshQueuedManualEpoch: null, cancelQueuedAccountRefreshes: () => {},
    config: { accessPolicy: clone(backend), disabled: [] }, lastSnapshots: [], lastSpend: [],
    lastUsageRevision: null, spendLoaded: false, spendTab: 'today',
    appConfirm: async () => true, t, displayScanError, accountAuthorized: () => false,
    document: { querySelector: () => status, body: { classList: { remove() {} } } },
    patchConfig: async (patch) => { patches.push(clone(patch)); },
    forceUsageRefreshAttempt: async () => {}, saveProviderAutoRefresh: async () => {},
    invoke: async (command, args) => {
      order.push(command);
      if (command === 'set_scan_source') {
        sourceCalls += 1;
        if (args.directories.length) {
          backend.scanRoots[args.source] = [...args.directories];
          backend.scanEpochs[args.source] = `synthetic-grant-${sourceCalls}`;
        } else {
          delete backend.scanRoots[args.source]; delete backend.scanEpochs[args.source];
        }
        const response = { accessPolicy: clone(backend) };
        if (sourceCalls === 1) return new Promise((resolve) => { releaseFirst = () => resolve(response); });
        return response;
      }
      if (command === 'reset_provider_access') {
        if (resetFails) throw new Error('synthetic persistence failure');
        backend = { scanRoots: {}, scanEpochs: {}, enabledAccounts: [], enabledFamilies: [] };
        return { accessPolicy: clone(backend) };
      }
      return {};
    },
  };
  for (const name of ['renderScanSources', 'renderProviderAccess', 'renderAll', 'requestTraySync', 'applyLocale', 'syncSettingsControls', 'scheduleAutoRefresh', 'applyAppearance', 'applyGlass', 'applyReduceMotion', 'applyWidgetState']) ctx[name] = () => {};
  vm.createContext(ctx); vm.runInContext(js, ctx);
  return { ctx, order, patches, status, backend: () => clone(backend), release: () => releaseFirst() };
}
for (const nextEdit of ['add', 'remove']) {
  test(`confirmed reset remains last after deferred source response and queued ${nextEdit}`, async () => {
    const h = harness(); const { ctx } = h;
    const requestEpoch = ctx.accessEpoch;
    const firstDirs = nextEdit === 'add' ? ['/synthetic/A'] : ['/synthetic/A', '/synthetic/B'];
    const first = ctx.changeProviderAccess('set_scan_source', { source: 'claude', directories: firstDirs });
    const second = ctx.changeProviderAccess('set_scan_source', () => ({ source: 'claude', directories: nextEdit === 'add' ? [...(ctx.config.accessPolicy.scanRoots.claude ?? []), '/synthetic/B'] : (ctx.config.accessPolicy.scanRoots.claude ?? []).filter((dir) => dir !== '/synthetic/A') }));
    await Promise.resolve();
    const reset = ctx.resetAllSettings();
    await Promise.resolve(); await Promise.resolve();
    h.release(); await Promise.all([first, second, reset]);
    assert.deepEqual(h.backend().scanRoots, {});
    assert.deepEqual(h.backend().scanEpochs, {});
    assert.deepEqual(clone(ctx.config.accessPolicy.scanRoots), {});
    assert.deepEqual(clone(ctx.config.accessPolicy.scanEpochs), {});
    assert.deepEqual(h.order.filter((command) => ['set_scan_source', 'reset_provider_access'].includes(command)), ['set_scan_source', 'set_scan_source', 'reset_provider_access']);
    assert.deepEqual(clone(ctx.lastSpend), []);
    assert.equal(acceptSpendBatch(requestEpoch, ctx.accessEpoch, 10, 9), false);
  });
}
for (const locale of ['en', 'zh', 'ru']) test(`failed queued reset preserves accepted grants, reports failure in ${locale}, and leaves queue usable`, async () => {
  const h = harness({ resetFails: true, locale }); const { ctx } = h;
  const first = ctx.changeProviderAccess('set_scan_source', { source: 'claude', directories: ['/synthetic/A'] });
  const second = ctx.changeProviderAccess('set_scan_source', () => ({ source: 'claude', directories: [...(ctx.config.accessPolicy.scanRoots.claude ?? []), '/synthetic/B'] }));
  await Promise.resolve(); const reset = ctx.resetAllSettings();
  await Promise.resolve(); await Promise.resolve(); h.release(); await Promise.all([first, second, reset]);
  assert.deepEqual(h.order, ['set_scan_source', 'set_scan_source', 'reset_provider_access']);
  assert.deepEqual(h.backend().scanRoots, { claude: ['/synthetic/A', '/synthetic/B'] });
  assert.deepEqual(clone(ctx.config.accessPolicy), h.backend());
  assert.equal(ctx.accessEpoch, 2, 'failed reset must not claim accepted revocation');
  assert.equal(h.patches.length, 0, 'display/default reset must stop after failed revocation');
  assert.equal(h.status.textContent, t('footer.accessResetFailed', { err: 'Error: synthetic persistence failure' }));
  await ctx.changeProviderAccess('set_scan_source', () => ({ source: 'claude', directories: [...(ctx.config.accessPolicy.scanRoots.claude ?? []), '/synthetic/C'] }));
  assert.deepEqual(h.backend().scanRoots.claude, ['/synthetic/A', '/synthetic/B', '/synthetic/C']);
});
