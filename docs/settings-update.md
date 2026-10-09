# rice monitor：设置更新与升级步骤

当前产品已改名为 rice monitor 0.0.1。旧 Pane Private 安装版请先按[改名升级说明](rice-monitor-0.0.1.md)处理；不要直接并排安装并继续运行旧版本。

本文记录此前的设置更新，并补充当前入口说明；后续新增的[按平台自动刷新](provider-auto-refresh.md)、[单账号额度刷新](scoped-quota-refresh.md)和[Windows 打包配置](release-package.md)见各自说明。下方测试数字仅属于原设置更新版本。

本次更新整理平台设置，并简化本地日志来源的授权。使用这个分支的完整新源码；上游 Pane 安装包不包含这些修改。已有的 rice monitor 用户配置和缓存无需删除。

## 改了什么

### 一个「平台管理」入口

点击侧栏 **☰ 平台管理（Platform management）**，展开相应平台，即可管理账户、区域/连接方式、API Key 和**指标与布局（Metrics & layout）**。常规设置不再重复放置平台授权或密钥表单。

- 标题行的**查询（Queries）**开关控制平台级权限。打开后展开设置，仍需逐一勾选账户；未授权任何账户时会显示**请选择账户（No account authorized）**。
- 账户开关是**允许此账号的查询和登录刷新（Allow this account’s queries and login refresh）**。保存密钥或选择区域/连接方式不会自动打开它。
- 关闭平台查询会撤销该平台全部账户的查询和登录刷新权限。重新打开后，要重新选择账户。
- Claude、Codex、OpenCode 可通过**识别目录（Identify directory）**添加明确目录里的账户，再单独授权。不会搜索整个用户目录，也不会顺带授权日志。
- 展开平台后，账户设置上方另有**自动刷新配额**，控制此平台所有已授权账户（包括额外账户）的自动额度查询，沿用全局刷新间隔。Claude、CommandCode 默认关闭，其他支持账户查询的平台默认开启；不会自动授权账户。关闭后保留已保存结果，仍可用卡片刷新或全局刷新 / Ctrl+R；已发出的请求可能仍会完成。Hermes 只有本地统计来源，没有此开关。详见[按平台自动刷新](provider-auto-refresh.md)。
- 指标显隐、顺序、托盘加星和重置布局只改变显示。关闭自动刷新只停止自动查询；要撤销账户读取和登录刷新权限，使用账户或平台查询开关。

### 本地日志来源：每种工具独立开关

打开**设置 → 本地日志来源（Local log sources）**。11 种日志来源各有开关及**位置模式（Location mode）**，与平台账户查询互不授权。

- **默认位置（Default location）**：显示该工具已知的实际日志路径，打开来源才授权读取。不扫描整个磁盘，不读取凭据来发现账户。目录尚未创建也可明确启用；之后只能读取同一获准位置。
- **自定义位置（Custom locations）**：每行输入一个完整绝对目录，点击**保存自定义位置（Save custom locations）**。仅选择此模式或编辑文本尚未生效。保存成功后替换该来源全部选定位置，包括默认位置；不会自动把关闭的来源打开。
- **保存无效路径或保存失败**：显示错误，保留之前的设置和读取授权，绝不自动回退默认目录。若想在修正路径前停止旧目录读取，先关闭这个来源。
- **已开启但未找到目录**与**已关闭**是不同状态。路径会显示**已找到 / 未找到 / 不可用 / 未检查（Found / Not found / Unavailable / Not checked）**；已找到只说明目录存在，不保证有支持的日志或有效记录。已授权路径在运行时暂时不可用，不会关闭其他有效路径或来源；新建或更改授权时仍会校验整组选定路径，失败则保留旧设置。
- **重新检查位置（Recheck locations）**只检查目录状态，不读取日志内容或授予账户权限。
- **关闭**撤销该工具所有日志读取，但记住选过的位置。重新启用仍只使用保存的位置，不会因环境变量或目录链接变化偷偷扩大授权。
- **保留旧授权**：旧版已选的多个日志目录迁移为“已开启 + 自定义位置”，不增添默认目录。要移除某个自定义目录，删掉对应行后保存；要停止全部读取，关闭该工具。

完整路径清单、不同工具的日志格式和权限边界见[构建与使用指南](private-build.md#2-首次启动先选择需要的访问)及[隐私说明](privacy.md)。新控件提供中英文，俄文缺少新译文时使用英文回退。MIT 许可及上游署名保持不变。

## Windows 11 更新步骤

1. **退出旧版本。** 在运行 `npm run tauri dev` 的终端按 `Ctrl+C`。如果托盘里仍有 rice monitor，再从托盘退出应用，避免新旧进程同时运行。
2. **放入完整新源码。** 可以解压到一个新的源码文件夹，或更新现有源码目录。保留 `%APPDATA%\PanePrivate` 中的用户配置与缓存，不要为更新清空 AppData，也不要导入或覆盖上游 Pane/OpenUsage 的配置。无需复制旧的 `node_modules`、`dist` 或编译产物。
3. **在新源码根目录重新安装依赖并构建前端。** 本次测试新增 `jsdom` 26.x 开发依赖，兼容 Node.js 20；使用随源码提供的锁文件，不要跳过 `npm ci`。

```powershell
npm ci
npm run build
```

4. **继续原来的开发版方式，或自行构建 NSIS 安装包。** 两者都需要指南列明的 Windows Rust/MSVC、C++ Build Tools 和 WebView2 条件。

开发版：

```powershell
npm run tauri dev
```

NSIS 安装包：

```powershell
npm run tauri -- build --bundles nsis
```

NSIS 产物通常位于 `src-tauri\target\release\bundle\nsis\`；配置 Cargo target 目录或显式指定编译目标会改变位置。这是构建命令，不表示本次安装包已经构建、安装或签名验证。不要绕过 Windows 安全警告。

5. **检查保存的设置。** 平台授权、已选模式和显示布局应保留；已有日志目录应显示为自定义位置。首次新配置的账户权限和日志来源仍全部关闭；自动刷新偏好与权限分开，Claude、CommandCode 默认关闭，其余支持账户查询的平台默认开启。核对实际路径、来源开关和账户勾选，别把“未找到”当成已关闭。选择默认位置会改用它显示的默认路径，保存自定义位置会替换当前路径，因此只在需要改动时操作。

使用旧版本回滚前保留私有配置副本；不要假定旧版能理解本次新增的位置模式。配置及缓存可能包含私密数据，不要把它们放入共享源码包或公开问题报告。

## 验证范围

- 使用者已反馈上一版源码在 Windows 11 成功构建开发版并显示界面。这只覆盖当时的版本与操作。
- 本次自动化验证为 480 项 Rust 测试、97 项 Node 测试、前端构建及 Windows GNU 完整类型检查通过，详见[本次验证记录](settings-validation.md)。浏览器实际渲染、Windows 原生设置交互和安装包仍待验证。
- [上一版验证记录](validation.md)中的代码提交和测试数量属于上一版，不是本次更新的结果。构建和测试时应核对实际代码版本。
- 不读取真实凭据或日志的合成测试，不能替代真实账户联网、CLI 共存、系统权限、目录联接点或托盘行为验收。

<a id="english-update-steps"></a>
## English update steps

The side-bar **Platform management** panel now holds accounts, exact connection/region modes, keys, and metrics/layout. The platform’s **Queries** switch alone grants no account; enable each account separately. Turning it off revokes that platform’s account queries and refreshes. Hiding or reordering metrics only changes the display. Each account-query family also has a separate **Auto-refresh quotas** switch above its account settings, shared by extra accounts and the global interval. Claude and CommandCode default off; other account-query families default on without granting any account. Off keeps saved results and authorized manual refresh available; already-sent requests may finish. Hermes has no quota-auto switch. See [per-provider auto-refresh](provider-auto-refresh.md).

**Settings → Local log sources** has an independent per-tool switch and **Default location / Custom locations** choice. Defaults are finite known log paths. Save Custom replaces all selected locations and preserves On/Off; invalid or failed saves retain the previous grant without fallback. Off revokes reads but remembers locations. Found / Not found / Unavailable / Not checked describes directory availability, not permission or completeness. Existing grants remain enabled Custom with all prior roots preserved and no added defaults.

To update on Windows:

1. Stop the old `npm run tauri dev` with `Ctrl+C`, and quit any remaining rice monitor tray process.
2. Extract the complete updated source into a source folder. Keep the existing `%APPDATA%\PanePrivate` configuration and caches; do not clear AppData or import upstream state.
3. Run `npm ci` and `npm run build`. The new test dependency is Node.js-20-compatible `jsdom` 26.x.
4. Run `npm run tauri dev` again, or build NSIS with `npm run tauri -- build --bundles nsis` using the Windows prerequisites.
5. Confirm your platform/account selections and existing custom log paths. A missing directory is not the same as an off source.

The earlier Windows 11 development build/render was user-reported. This settings revision still requires its own native/rendered acceptance, and the installer is unverified. The old validation record does not establish test results for this revision.
