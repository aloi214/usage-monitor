import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import { test } from 'node:test';

const root = new URL('../', import.meta.url);
const hookPath = new URL('src-tauri/windows/installer-hooks.nsh', root);
const config = JSON.parse(readFileSync(new URL('src-tauri/tauri.conf.json', root), 'utf8'));
const hook = () => {
  assert.ok(existsSync(hookPath), 'the rename must provide a preinstall safety guard');
  return readFileSync(hookPath, 'utf8');
};
const macro = (name) => {
  const body = hook().match(new RegExp(`^!macro ${name}(?: [^\\n]*)?\\n([\\s\\S]*?)^!macroend`, 'm'))?.[1];
  assert.ok(body, `${name} is required`);
  return body.split('\n').filter((line) => !line.trimStart().startsWith(';')).join('\n');
};

// Source contracts only: native Windows installation is a separate acceptance check.
test('every NSIS install includes the rename guard without replacing the Tauri template', () => {
  assert.equal(config.bundle.windows.nsis.installerHooks, 'windows/installer-hooks.nsh');
  assert.equal(config.bundle.windows.nsis.template, undefined);
  const body = macro('NSIS_HOOK_PREINSTALL');
  assert.doesNotMatch(body, /\$UpdateMode|\$PassiveMode|\$NoShortcutMode|\$INSTDIR/);
  assert.match(body, /Push \$0\s+Push \$1/);
  assert.match(body, /Pop \$1\s+Pop \$0/);
});

test('legacy registration detection checks only the exact key in both hives and views', () => {
  const body = macro('RiceMonitorCheckLegacyRegistration');
  assert.match(body, /RegOpenKeyExW\(p \$\{ROOT\}, w "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Pane Private", i 0, i \$\{ACCESS\}, \*p \.r1\) i\.r0/);
  const checks = [...macro('NSIS_HOOK_PREINSTALL').matchAll(/!insertmacro RiceMonitorCheckLegacyRegistration (0x[0-9A-Fa-f]+) (0x[0-9A-Fa-f]+)/g)]
    .map(([, hive, access]) => [Number(hive), Number(access)]);
  assert.deepEqual(checks, [
    [0x80000001, 0x0201], [0x80000002, 0x0201],
    [0x80000001, 0x0101], [0x80000002, 0x0101],
  ], 'open with KEY_QUERY_VALUE plus the requested WOW64 view, never write access');
  assert.match(macro('NSIS_HOOK_PREINSTALL'), /\$\{If\} \$\{RunningX64\}[\s\S]*0x0101[\s\S]*\$\{EndIf\}/);
  assert.doesNotMatch(hook(), /^\s*SetRegView\b/m, 'do not change Tauri registry-view state');
});

test('legacy registration presence and inspection errors fail closed with handles closed', () => {
  const body = macro('RiceMonitorCheckLegacyRegistration');
  assert.match(body, /\$\{If\} \$0 = 0\s+System::Call 'advapi32::RegCloseKey\(p r1\) i\.r0'\s+!insertmacro RiceMonitorAbortInstall/);
  assert.match(body, /\$\{ElseIf\} \$0 <> 2\s+!insertmacro RiceMonitorAbortInstall/);
  assert.match(body, /uninstall Pane Private/);
  assert.match(body, /Keep.*app data/i);
});

test('a running private instance blocks install by its exact shared mutex, never a process name', () => {
  const body = macro('NSIS_HOOK_PREINSTALL');
  assert.match(body, /OpenMutexW\(i 0x00100000, i 0, w "local\.pane\.private-sim"\) p\.r0 \?e'\s+Pop \$1/);
  assert.match(body, /\$\{If\} \$0 P<> 0\s+System::Call 'kernel32::CloseHandle\(p r0\) i\.r0'\s+!insertmacro RiceMonitorAbortInstall/);
  assert.match(body, /\$\{ElseIf\} \$1 <> 2\s+!insertmacro RiceMonitorAbortInstall/);
  assert.match(body, /Exit Pane Private or rice monitor/);
  assert.equal(config.identifier, 'local.pane.private');
  const cargo = readFileSync(new URL('src-tauri/Cargo.toml', root), 'utf8');
  assert.match(cargo, /^tauri-plugin-single-instance = "2"$/m, 'the shared mutex must not become version-scoped with the semver feature');
});

test('silent and passive failures never open a dialog and always return nonzero', () => {
  const body = macro('RiceMonitorAbortInstall');
  assert.match(body, /DetailPrint "\$\{MESSAGE\}"/);
  assert.match(body, /\$\{IfNot\} \$\{Silent\}\s+\$\{If\} \$PassiveMode <> 1\s+MessageBox MB_OK\|MB_ICONSTOP "\$\{MESSAGE\}"\s+\$\{EndIf\}\s+\$\{EndIf\}/);
  assert.match(body, /SetErrorLevel 2\s+Abort "\$\{MESSAGE\}"/);
});

test('the guard cannot run uninstallers, modify data or registry, or terminate other software', () => {
  const code = hook().split('\n').filter((line) => !line.trimStart().startsWith(';')).join('\n');
  assert.doesNotMatch(code, /\b(?:Exec\w*|Delete\w*|RMDir|WriteReg\w*|Rename|CopyFiles|File|KillProcess\w*|CheckIfAppIsRunning|CreateMutex\w*|TerminateProcess|RegSet\w*|RegCreate\w*)\b/i);
  const calls = [...code.matchAll(/System::Call '([^'(]+)\(/g)].map(([, api]) => api);
  assert.deepEqual(calls, [
    'advapi32::RegOpenKeyExW', 'advapi32::RegCloseKey',
    'kernel32::OpenMutexW', 'kernel32::CloseHandle',
  ]);
  assert.doesNotMatch(code, /UninstallString|\$APPDATA|\$LOCALAPPDATA|pane\.exe/);
});
