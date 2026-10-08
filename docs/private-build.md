# Pane Private：构建与使用

本指南面向源码分支，优先使用中文，界面英文标签写在括号中。上游基线为 `a55578c`，应用名称为 **Pane Private**，Tauri 标识为 `local.pane.private`。请使用此私有分支的完整源码；克隆上游或安装上游发布包不能获得这些修改。

<a id="integration-status"></a>
## 实现与验证边界

最新变化是[按平台控制自动刷新配额](provider-auto-refresh.md)。[CommandCode GOAT 实验性接入](commandcode-provider.md)、[单账号额度刷新](scoped-quota-refresh.md)与[Windows 发布包体积配置](release-package.md)继续适用；[CommandCode 验证](commandcode-validation.md)、[单账号刷新验证](scoped-refresh-validation.md)和[Claude 默认手动查询说明](manual-claude-query.md)中的数字只属于各自旧版本，不覆盖按平台自动刷新。

设置功能与通用更新步骤见[设置更新](settings-update.md)。[设置版验证](settings-validation.md)和[更早交付验证](validation.md)仅对应各自标明的旧代码提交。

- 源码已实现独立配置、移除遥测与自动更新、移除 One/New API/Sub2API、默认关闭账户访问，以及独立本地日志目录授权。
- 同时已实现精确区域/连接模式及受限请求通道、手动公开价格同步、日志片段哈希缓存和 OAuth 刷新协调。下文描述此分支的使用方式，不代表每个原生环境或服务商账户均已实测。
- 使用者已反馈上一版在 Windows 11 完成开发版构建并显示界面。此反馈不覆盖后续统一设置、单账号刷新、打包配置、CommandCode 及按平台自动刷新；这些变化的浏览器渲染/原生交互仍待验证，安装包、托盘交互、真实账户联网、CLI 同时运行及完整网络流量审计也未完成。
- 前端构建、合成界面测试和 Windows 交叉目标类型检查各自只能验证一部分，不能替代本次版本的真实渲染、原生运行与安装验收。
- 不应从上游版本号推断本分支已发布、已签名或已验证。`CHANGELOG.md` 是上游历史。

## 1. 构建条件

完整桌面应用面向 Windows。准备 Node.js 20+、npm、Rust stable 的 MSVC 工具链、Visual Studio C++ Build Tools（含 Windows SDK）及 WebView2 Runtime。源码中的 Windows API 依赖意味着仅在 Linux 构建前端不能证明桌面应用可运行。

在此分支仓库根目录执行：

```powershell
npm ci
npm run build
npm run tauri dev
```

- `npm ci` 从锁文件安装前端依赖。本次新增用于前端测试的 `jsdom` 26.x（兼容 Node.js 20），更新旧源码后也要重新执行；首次安装依赖和 Rust 编译可能需要联网下载软件包。
- `npm run build` 只检查 TypeScript 并生成前端资源，不生成 Windows 应用。
- `npm run tauri dev` 编译并启动桌面开发版。单独运行 `npm run dev` 只是 Vite 页面，不能代替原生权限、文件和托盘功能验证。
- 修改 Rust 后需重新编译并重启应用。

需要尝试生成本机安装包时：

```powershell
npm run tauri build
```

当前默认仅生成 NSIS，安装包通常位于 `src-tauri/target/release/bundle/nsis/`；Cargo target 目录配置或显式编译目标会改变路径。release 已采用体积优先优化、完整 LTO、单代码生成单元及符号裁剪，保留 panic unwind。它不会缩小开发缓存，且可能增加编译时间和内存需求。LZMA 与在线下载 WebView2 原先就是默认值；缺少 WebView2 的目标电脑仍需要联网安装。详见[打包配置及测量方法](release-package.md)。**这是构建方法，不是已成功产生或验证安装包的声明，也没有最终体积保证。** 本分支不提供上游自动更新，也不要求绕过 Windows 安全警告。

可先运行仓库已有的合成测试：

```powershell
cargo test --manifest-path tests/core-harness/Cargo.toml
cargo test --manifest-path tests/scan-harness/Cargo.toml
```

还应运行网络、刷新、价格及前端测试，见仓库 `tests/` 和 `scripts/`。针对要使用的精确源码版本检查实际结果；部分测试通过不等于全应用验证通过。

## 2. 首次启动：先选择需要的访问

配置默认独立保存到 `%APPDATA%\PanePrivate`，不会迁移上游 `%APPDATA%\Pane` 或 `OpenUsage`。不要复制旧配置来代替明确授权。

### 只看本地日志

1. 打开**设置 → 本地日志来源（Local log sources）**，找到写入日志的工具。每个工具都有独立开关，首次使用均为关闭。
2. 标准安装可选**默认位置（Default location）**，核对显示的实际目录后打开该工具。它只授权该工具已知的日志位置，不搜索整个磁盘，也不读凭据来发现账户。
3. 日志在别处时，在**自定义日志目录 · 每行一个**中输入完整绝对路径，点击**保存自定义位置（Save custom locations）**。选择“自定义位置”只是开始编辑，保存成功才生效；自定义路径替换全部当前读取位置，不与默认位置叠加。保存不会自动打开关闭的来源。
4. 查看路径旁的**已找到 / 未找到 / 不可用（Found / Not found / Unavailable）**。目录“已找到”只表示目录存在，不保证其中有支持的日志或有效记录。“已开启”且“未找到”不会自动关闭；默认目录日后在同一获准位置创建后可以读取。**重新检查位置（Recheck locations）**只检查目录状态，不授权账户或读取日志内容。
5. 统计保留 **Local logs** 和账户归属未经核实的提示。按模型或路由分组不代表已确认付费账户。要停止读取，请关闭对应工具；隐藏指标或关闭账户查询不会撤销日志授权。

无效或保存失败的自定义路径**不会回退默认位置，也不会撤销原先有效的读取授权**；错误提示期间，原设置仍生效。如需立即停止旧位置的读取，先关闭该工具。关闭会撤销其全部读取授权，但记住位置方便再次选择。某个路径不可用不会禁用其他有效路径或工具。

旧版已授权的目录会保留为**已开启 + 自定义位置**，包括原有多个目录；不会自动添入默认目录。需要移除某个自定义目录时，在列表中删除该行后保存；关闭全部读取用工具开关即可。

选择最小必要目录，不要选择整个用户目录或磁盘根目录。不同日志源不能使用相互重叠的目录。文件系统根目录、非绝对路径、不可用的新自定义目录会被拒绝；默认位置可在尚未创建时明确启用。已获准路径被重新检查，环境变量改变或目录链接改指向别处不会扩大授权；这不是对所有恶意文件系统竞态的完整防护。

下表是常见 Windows 默认位置；界面显示的是本机实际解析的完整目录。已支持的工具目录环境变量可替代相应基础路径，只接受有效的绝对路径。自定义位置不使用这些默认候选。

| 日志源 | 已知默认位置（环境变量未指定时） | 支持的目录变量 / 自定义内容 |
|---|---|---|
| Claude Code | `%USERPROFILE%\.claude\projects` | `CLAUDE_CONFIG_DIR` 下的 `projects`；自定义选 projects 日志树 |
| Codex | `%USERPROFILE%\.codex\sessions` 和 `archived_sessions` | `CODEX_HOME` 下的两棵日志树；自定义可选 Codex home 或 rollout sessions 树 |
| OpenCode | `%USERPROFILE%\.local\share\opencode` | `XDG_DATA_HOME` 下的 `opencode`；包含 `opencode.db` |
| Pi / oh-my-pi | `%USERPROFILE%\.pi\agent\sessions` 和 `.omp\agent\sessions` | Pi 优先 `PI_CODING_AGENT_SESSION_DIR`，否则 `PI_CODING_AGENT_DIR` 下的 `sessions` |
| Step Code | `%USERPROFILE%\.stepcode\agent\sessions` | `STEP_CODING_AGENT_DIR` 下的 `sessions`，独立于账户权限 |
| Grok CLI | `%USERPROFILE%\.grok\logs` | `GROK_HOME` 下的 `logs`；包含 `unified.jsonl` |
| Devin CLI | `%APPDATA%\devin\cli` | 无 `APPDATA` 时用 `XDG_DATA_HOME` 或 `.local\share` 下的 `devin\cli`；包含 `sessions.db` |
| MiniMax | `%USERPROFILE%\.minimax` | 仅读取固定位置的 `sqlite.db`、`runtime-state.sqlite` 或 `v2\sqlite\runtime-state.sqlite`；自定义可选数据库所在目录 |
| Hermes | `%LOCALAPPDATA%\hermes` | 包含 `state.db`；缺少 `LOCALAPPDATA` 时须明确选择自定义目录 |
| Kimi Code | `%USERPROFILE%\.kimi-code\sessions` 和 `.kimi\sessions` | 分别支持 `KIMI_CODE_HOME`、`KIMI_SHARE_DIR` 下的 `sessions`；包含 `wire.jsonl` |
| Qwen Code | `%USERPROFILE%\.qwen\usage` | 包含 token-usage JSONL 的 usage 目录 |

数据库来源只打开所支持的固定数据库及安全的辅助文件，不会因为基础目录内还有配置文件就读取凭据。Cursor CSV 是例外：它是需要 Cursor 账户授权的远程用量下载，不是本地日志目录读取。自动下载同时遵守 Cursor 的自动刷新开关；关闭时跳过凭据读取和下载，可保留适用的已授权历史统计。需要更新可用全局刷新 / Ctrl+R，不影响其他获准的本地扫描。CSV 中已报告的费用保留其来源；手动价格同步后的本地重新计价不会下载 Cursor CSV，也不会覆盖其已报告费用。

### 查询账户额度和调整显示

1. 点击侧栏 **☰ 平台管理（Platform management）**。这是平台相关设置的唯一入口，不再到常规设置寻找重复的授权或密钥区块。
2. 找到平台，在标题行打开**查询（Queries）**。它会展开平台设置，但不会自动授权默认或其他账户；尚未选择时显示**请选择账户（No account authorized）**。
3. 在展开的账户区选择所需的 **Region / connection mode / 区域 / 连接模式**，需要时保存 API Key，再单独勾选**允许此账号的查询和登录刷新（Allow this account’s queries and login refresh）**。保存密钥或选择模式不授予访问权限。获准的账户可能读取该服务商现有 CLI/编辑器凭据、查询接口，并在适用时刷新凭据。
4. Claude、Codex、OpenCode 的额外账户要输入准确的账户目录并点击**识别目录（Identify directory）**，再单独启用识别出的账户。此操作不会遍历整个 home，也不授权日志读取。
5. 在对应账户的**指标与布局（Metrics & layout）**中调整显隐、顺序或托盘加星。这些仅改变显示，不会停止查询。要撤销单个账户，关掉它的账户开关；要撤销整个平台全部账户的查询和登录刷新，关掉平台标题行的“查询”。再次打开平台仍需重新选择账户。本地日志授权独立管理。

区域、连接方式、适用的 API Key 和 StepFun 套餐档位都放在对应平台内。常规设置仍提供外观、刷新、代理、本地日志来源和模型价格等全局选项。新控件提供中英文，俄文缺少新译文时回退英文。

**单账号额度刷新**：点击账户卡片标题行的 **↻ 仅刷新此账号**，只查询该绑定账户，不查询默认/同平台其他账户，也不触发全局日志扫描或价格下载。纯日志卡片没有额度刷新按钮。后台已有的独立刷新仍可继续。其他卡片保留原数据和时间，撤权清理除外；Kimi Code 卡片只更新套餐，合并的钱包保留“已保存”标记和独立时间，钱包要用全局刷新更新。详见[单账号刷新说明](scoped-quota-refresh.md)。

**自动刷新配额**：展开平台，在账户设置上方单独选择是否自动查询。Claude、CommandCode 默认关闭，其余支持账户查询的平台默认开启；同平台额外账户继承此开关，共用全局刷新间隔。Hermes 只有独立本地日志来源，没有额度自动刷新开关。开启不授予任何账户权限，所有新配置的账户授权仍默认关闭。

关闭自动刷新后，启动、打开浮窗、定时刷新、保存设置和额度重置后的自动补查跳过该平台，不为这些自动路径读取凭据、身份或发现额外账户。卡片刷新仍只查询该账户，全局刷新 / **Ctrl+R** 仍查询全部已授权账户，并遵守限流冷却。关闭不取消已经发出的请求；后续排队或新触发的自动刷新使用最新开关。显式识别目录、兑换重置额度及其登录刷新保留原有权限规则；独立本地日志统计不受这个额度开关影响。完整说明见[按平台自动刷新](provider-auto-refresh.md)。

自动刷新关闭且尚无结果时，账户卡片显示手动查询提示；有结果时保留上次成功的数据和时间，鼠标悬停提示可查看。跳过自动查询时显示的是已保存结果，账户信息尚未重新验证，可能对应此前的 CLI 登录；真实查询失败仍显示其错误，不会冒充新的成功。关闭自动刷新不删除历史；关闭授权及相关账户配置变更继续使缓存失效。

**CommandCode（实验性）**：在平台管理中保存本地 API Key，再单独授权账户，默认使用卡片刷新或全局刷新 / Ctrl+R，需要时可单独打开自动刷新。不会读取 CommandCode CLI 登录或环境变量，也不会为验证密钥发起模型请求；真实账户额度尚未核验。完整配置和 credits 口径见[专门说明](commandcode-provider.md)。

重置设置会撤销访问权限，但“重置布局”只重置显示。撤销权限也不等同于安全擦除全部历史缓存，详见[隐私说明](privacy.md)。

## 3. 精确选择区域与连接方式

选择模式本身不授予账户权限；区域不符、密钥错误或接口拒绝时会报错，不会拿同一个密钥轮流试其他地区或认证头。

- **Z.ai、Kimi API**：International 或 China，再选 API key。
- **MiniMax**：International 或 China，再选 API key 或 MiniMax Code sign-in。登录模式只读取所选地区的有效 mcode 登录，Pane 不刷新或改写 mcode 凭据；失败不自动切换到密钥模式。
- **Qwen Code**：选区域，再准确选 Bearer key、x-api-key 或 x-dashscope-api-key。不会探测其他认证头。
- **StepFun**：选区域，再选 Wallet API key 或 Step Plan key。钱包查余额，Plan 模式只核对选定服务的模型接口；本地估算不是服务商报告的套餐剩余额度。
- **Antigravity**：Google Cloud Code（云端模式）或 Local Antigravity process（本机进程模式）。本机模式需要显式选择，仅使用识别到的 Antigravity 进程对应的 loopback 端口；失败不回退云端。云端模式不触发本机进程扫描。
- **Ollama**：显式选择 `http://127.0.0.1:11434`、`http://[::1]:11434` 或 `http://localhost:11434`；也可填写准确的 http(s) loopback origin/端口。不能填写路径、查询参数、用户名密码、局域网或远程主机。它不会枚举其他端口或自动切换地址。

区域/连接方式变更会清除相关旧展示数据并要求按新配置重新获取。连接失败时先核对所选模式，勿依赖上游的跨区或认证方式自动回退。

## 4. 价格和花费

在 **Settings → Model prices → Sync latest price catalog** 手动同步。只有点击该按钮才请求公开目录；启动、定时刷新、扫描日志、发现未知模型都不下载价格。同步状态显示结果和最近成功时间。

目录来自 LiteLLM、models.dev 和 OpenUsage 的公开补充表，不发送账户凭据、日志、模型查询列表或用量。下载仍会向服务端暴露通常的网络连接信息。离线/首次启动使用有效的本地目录整包及内置价格；没有整包时仍可用内置价格，其余模型保持未知。旧的分散目录文件不会自动拼装或导入，需手动同步生成新整包。

三个来源均取得有效数据并通过验证后才原子替换整包；下载、验证或写入失败保留上次成功版本和时间。未知费用即使合并进“其他”汇总仍标为未知，不会按免费计算。价格同步不会开启本地日志或账户权限，只重新计算当前获准的本地日志结果。

本地缓存的头/尾校验只保存片段的 SHA-256 与采样长度，不保存原始片段。旧格式或损坏的缓存文件会在不读取日志的情况下失效，并尝试删除；删除失败会报告错误；哈希不是加密，删除也不是安全擦除，缓存仍可能含路径、模型名、用量汇总及解析状态。

StepFun 的 **Plan Credits** 仅是获准日志的本地估算；设置的套餐档位未经账户核验，日志也可能含非套餐请求。扫描不完整或缺少价格时显示不可用，不进入账户额度、托盘或额度告警。

## 5. OAuth 刷新与故障处理

账户授权包含适用服务商的 token 刷新。刷新协调逻辑对同一凭据文件串行刷新、写前重新读取并比较文件，避免已观察到的外部更改被覆盖；新 token 保存失败或刷新结果不确定时会阻止当前进程继续盲目重试。

这些保护**不是跨进程事务**，不持有官方 CLI 的锁；最终比较与替换之间仍有竞态，保护状态也不跨应用重启持久化。需要同时运行 CLI 的场景仍需 Windows 实测。遇到登录变化、保存失败或需重新登录提示，应先用官方工具核对登录状态，再重新授权；不要反复重启来强行重试同一旧 refresh token。

## 6. 本地 API 与更新

运行期间的只读 API 位于 `http://127.0.0.1:6736/v1/usage`。它返回当前有权限的账户快照，不触发远程刷新，不包含独立本地日志统计。没有认证：能连接该 loopback 端口的本机程序可能读取显示名称、套餐及用量。详见 [API 文档](local-http-api.md)。

此分支没有自动更新或上游遥测。更新时先退出旧开发进程和托盘应用，保留用户配置与缓存，再使用完整新源码执行 `npm ci` 并重新构建。具体步骤见[设置更新](settings-update.md)。不要用上游安装包覆盖私有版。

## English summary

Use this private source tree, install dependencies with `npm ci`, then run `npm run build` and, on a suitably configured Windows machine, `npm run tauri dev`. `npm run tauri build` defaults to NSIS-only packaging under `src-tauri/target/release/bundle/nsis/` unless the target directory/target triple changes. The size-focused release profile has no measured size promise and does not shrink development caches. Missing WebView2 is downloaded during installation; see [packaging details](release-package.md). No installer or native acceptance is implied. An account card’s refresh queries only its exact bound account; global Refresh / Ctrl+R retains its all-authorized-accounts behavior. A Kimi card refresh keeps any authorized Moonshot wallet rows as saved data; use global Refresh to update that folded wallet. See [scoped refresh](scoped-quota-refresh.md).

Open **Platform management** for provider/account permissions, exact region/connection modes, keys, and metrics/layout. A platform switch does not authorize any account; choose accounts individually. Turning it off revokes their queries and refreshes, while hiding a metric only changes the display. In **Settings → Local log sources**, independently enable each tool’s known **Default location** or save **Custom locations**. A custom save replaces the selected locations and keeps the current On/Off state. Invalid saves retain the old grant without fallback. Off revokes reads but remembers locations; a missing directory is a separate status. Existing multiple grants stay enabled Custom without added defaults. See [update steps](settings-update.md#english-update-steps). **Auto-refresh quotas** controls each account-query family separately; extra accounts inherit the family choice and the interval stays global. Claude and CommandCode default off, others default on, with all account grants still default off. Hermes has no quota-auto switch. Off preserves historical data and authorized manual refresh, without cancelling already-sent requests or disabling separately granted local logs. See [auto-refresh](provider-auto-refresh.md). The earlier Windows 11 development build/render was user-reported; the later settings, scoped-refresh, CommandCode, and auto-refresh changes still need rendered/native acceptance, and the installer remains unverified.
