# Pane 自维护版实施计划

本文保留实施前的设计与验收清单，勾选框不代表最终验证状态。实际使用与构建说明见 [private-build.md](private-build.md)，交付验证结果见 [validation.md](validation.md)。


**Goal:** 在现有 Pane 中落实最小访问范围、可控刷新、无遥测和无上游自动更新，保留其他平台与本地统计，移除 One/New API、Sub2API。

**Architecture:** 现有适配器继续处理平台协议；后端访问策略统一管理启用与扫描来源；刷新工具集中处理锁、版本核对和安全写回。价格数据只通过手动同步更新。

**Tech Stack:** Tauri 2、Rust 2021、TypeScript、Vite、Node 测试；新增 SHA-256 依赖仅用于缓存指纹与文件版本摘要。

**Spec:** `docs/private-build-design.md`（2026-10-07 修订，移除自定义中转服务）。

## Global Constraints

- 在助手自己的云端工作目录实施，不另开 Codex 任务，不访问真实账号和项目日志。
- 保留其他平台、面板、统计；One/New API、Sub2API 的代码入口及请求能力一起移除。
- 不创建远程 fork、不推送、不触发发布；所有测试使用合成日志和假令牌。
- 价格表仅手动同步；原始头尾日志片段不进入持久化缓存。
- 不把进程内锁或比较后替换宣传为彻底解决跨进程 OAuth 竞争。
- 规划时 Linux 环境尚未安装 Rust；依赖安装和独立工作副本纳入执行准备。Windows 原生构建不可用时明确保留未验证状态。

## Review Focus

1. 旧配置和重置设置不能静默授权：任务 2 验证缺失授权字段时全关。
2. 查询中途禁用不得重新发布迟到结果：任务 2 验证授权版本变化。
3. 同一服务由多个 CLI 产生用量时不能误归账号或重复计数：任务 3 使用混合来源夹具。
4. 服务端已轮换令牌但本地写失败不能回滚或盲重试：任务 5 验证明确失败状态。
5. 价格同步部分成功或缓存迁移不能产生半新半旧重复合计：任务 6 验证校验后替换与重新计价。

## 执行准备与交付边界

- 在独立本地工作副本/分支执行，保留审查基线与设计提交。
- 安装依赖前检查清单与安装脚本；不启动真实 Pane，不读取主目录认证文件。
- 运行 `npm ci --ignore-scripts`、`npm run build`、现有 Node 测试，记录基线结果。
- Rust 工具链从官方来源安装到任务目录。原生 crate 的 Windows 依赖无法在当前环境执行时，为纯逻辑模块提供引用同一生产代码的独立 Rust 测试 harness，不用复制算法替代测试。
- 全量 Windows `cargo test` / Tauri build 和真实 CLI/Desktop 共存验证需要另一个获准环境；不会用源码正则测试代替原生运行验证。

### Task 1：删除遥测、更新与自定义中转入口

**Files:** `src-tauri/src/lib.rs`、`src-tauri/src/telemetry.rs`、`src-tauri/src/providers/mod.rs`、`src-tauri/src/providers/onenewapi/`、`src-tauri/src/providers/sub2api/`、`src/main.ts`、`src/sub2api-display.ts`、`index.html`、`src-tauri/Cargo.toml`、`src-tauri/Cargo.lock`、`src-tauri/tauri.conf.json`、`src-tauri/capabilities/default.json`、`install.ps1`、`.github/workflows/`。

**Interfaces:** 删除 `check_update`、`install_update`、更新定时器、遥测调用及 One/New API/Sub2API 的命令注册；不保留可被旧前端直接调用的后门。

- [ ] 先增加 `scripts/security-surface.test.mjs`：断言可执行源码、配置及命令注册不含被移除能力的入口；历史 changelog 和设计说明不在扫描范围。运行 `node --test scripts/security-surface.test.mjs` 确认基线失败。
- [ ] 删除对应模块、UI、监听、依赖和配置，清理旧配置的展示路由但不删除用户磁盘凭据。
- [ ] 将发布配置改为仅手动构建产物；不依赖上游签名密钥，不生成上游更新清单，不向 winget 或上游仓库发送请求。安装脚本取消下载上游发布包。
- [ ] 运行结构测试、`npm run build` 和剩余 Node 测试；检查其他平台显示和设置引用无缺失，提交本任务。

### Task 2：后端权威启用策略

**Files:** 新增 `src-tauri/src/access_policy.rs`、修改 `src-tauri/src/lib.rs`、`src/main.ts`、`index.html`。

**Interfaces:** `AccessPolicy { version: u32, enabled_families: BTreeSet<String>, enabled_accounts: BTreeSet<String>, scan_roots: BTreeMap<String, Vec<PathBuf>>, regions: BTreeMap<String, String> }`；`from_config(&Value) -> AccessPolicy`、`allows_discovery(&str) -> bool`、`allows_account(&str) -> bool`。版本字段为配置格式版本；另由运行时递增授权 revision 取消旧结果发布。

- [ ] 写 Rust 测试 `legacy_config_denies_access`、`family_enable_does_not_enable_discovered_accounts`、`frontend_filter_cannot_grant_access`、`disabled_during_refresh_drops_result`；断言未授权时假读取器与假请求器调用次数为零，先运行得到预期失败。
- [ ] 实现策略解析和持久化；未知版本、未知账号、格式损坏均拒绝访问。配置迁移展示确认入口，不从旧 disabled 数组反推授权。
- [ ] 在 `fetch_usage`、`cached_usage`、账号发现、身份缓存戳、刷新、手动操作、额度兑换之前检查策略；后续请求及结果发布再次核对 revision。
- [ ] 前端提供平台和账号开关；保存 key、恢复默认设置和启动探测不自动开启账号。设置页在不读凭据的条件下展示平台清单。
- [ ] 运行策略测试、Node 测试、前端构建，并检查全部保留适配器均有入口覆盖，提交本任务。

### Task 3：统计来源与认证读取分离

**Files:** `src-tauri/src/spend.rs`、`src-tauri/src/lib.rs`、`src/main.ts`、必要的 `providers/claude.rs`、`codex.rs`、`opencode.rs`、`kimi.rs`；新增 `src-tauri/src/scan_policy.rs`。

**Interfaces:** `ScanPolicy { roots: BTreeMap<String, Vec<PathBuf>> }` 由任务 2 的 scan_roots 投影；`allows_path(&self, source: &str, path: &Path) -> bool`。统计采集显式接收 ScanPolicy，不通过全局凭据发现寻找账号。

- [ ] 写 `disabled_source_is_not_opened`、`scan_never_reads_auth_json`、`symlink_escape_is_rejected`、`mixed_sources_keep_tokens_once`；夹具放在测试临时目录，先确认失败。
- [ ] 将扫描根与日志归属传入采集函数，去除 Claude/Codex/OpenCode 统计发现里的认证文件读取及 Kimi `has_credentials()` 判断。
- [ ] 对每次遍历路径规范化并检查根目录；拒绝越界链接，遍历期间发现路径变化则跳过并报告，不声称解决所有文件系统竞争。
- [ ] 无法证明账号归属时按日志来源展示，不用猜测的账号信息合并。关闭查询不影响独立获准的本地扫描。
- [ ] 跑统计夹具及现有 spend 测试，验证聚合与多来源路由保持正确；提交本任务。

### Task 4：目的域名、区域与重定向

**Files:** 新增 `src-tauri/src/network_policy.rs`；修改 `providers/mod.rs`、全部保留适配器、`src/main.ts` 和设置配置。

**Interfaces:** `ProviderEndpoint { provider: String, region: String, origin: String }`；`validate_destination(endpoint: &ProviderEndpoint, url: &reqwest::Url) -> Result<(), String>`。每个请求在添加凭据之前完成校验。

- [ ] 写 `credential_redirect_is_not_followed`、`wrong_region_does_not_probe_sibling`、`embedded_credentials_rejected`、`unconfigured_host_rejected`、`loopback_requires_explicit_configuration`；假 HTTP 服务确认拒绝目标无请求。
- [ ] 全部凭据 HTTP 客户端统一不跟随跳转。记录每个平台 quota/auth/API 的固定来源，包含必要的多官方域名但禁止任意配置绕过。
- [ ] GLM/Z.ai、MiniMax、Moonshot、Qwen、StepFun 改为明确区域与凭据类型；未选择时提示配置，不试探其他区域或请求头。
- [ ] 保留明确配置的本机平台回环例外，覆盖 IPv4/IPv6/主机名及端口规范化；删除自定义中转的任意 origin 功能。
- [ ] 执行网络策略测试和适配器 mock 测试，人工核对所有 HTTP 构造点均受覆盖；提交本任务。

### Task 5：刷新协调与安全写回

**Files:** 新增 `src-tauri/src/credential_refresh.rs`；修改 `providers/claude.rs`、`codex.rs`、`kimi.rs`、`grok.rs`、`cursor.rs`、`antigravity.rs`。

**Interfaces:** `CredentialVersion([u8; 32])`、`CommitOutcome::{Written, ChangedExternally}`；`credential_version(bytes: &[u8]) -> CredentialVersion`；`commit_if_unchanged(path: &Path, expected: CredentialVersion, updated: &[u8]) -> Result<CommitOutcome, String>`；异步锁 key 为规范化凭据来源路径或明确的内存账号标识。

- [ ] 写 `concurrent_refresh_calls_provider_once`、`external_rotation_wins`、`unique_temp_files_do_not_collide`、`unknown_json_fields_survive`、`remote_success_local_failure_stops_retry`、`cancelled_refresh_cannot_delete_other_temp`；使用假 token 与 barrier 控制交错，先确认失败。
- [ ] 锁内重读、去重刷新，刷新被拒绝时检查磁盘外部更新；逐适配器核对账号标识，不在错误文本输出密钥。
- [ ] 写回前版本比较，独占临时文件、安全权限、保留未知字段及受控清理；明确处理 Windows 文件替换语义，不把 Unix rename 结果当作 Windows 通过。
- [ ] 内存刷新与 Pane 自有缓存使用同一协调原则，不顺带复制凭据到新的长久存储，也不操作真实账号。
- [ ] 执行并发及失败测试，记录 compare-before-write 的不可消除时间窗口；提交本任务。

### Task 6：哈希缓存与手动价格同步

**Files:** `src-tauri/src/spend.rs`、`src-tauri/src/pricing.rs`、`src-tauri/src/lib.rs`、`src/main.ts`、`index.html`、`src-tauri/Cargo.toml`；必要时新增独立可测的 `cache_fingerprint.rs`。

**Interfaces:** `SampleFingerprint { sampled_len: usize, sha256: String }` 取代 prefix_head/prefix_tail 原文；`pricing::sync_catalogs() -> Result<PricingSyncResult, String>` 仅由显式 Tauri 同步命令调用，`PricingSyncResult { last_success_ms: i64, catalog_stamp: String }`；本地加载不联网。

- [ ] 写 `serialized_cache_has_no_fixture_text`、`append_and_rewrite_detection_preserved`、`old_cache_rebuild_replaces_totals`；确认当前原文缓存实现失败后再替换为 SHA-256 并升级格式。
- [ ] 写 `startup_and_spend_do_not_download_prices`、`sync_failure_preserves_old_catalog`、`repeated_clicks_coalesce`、`reprice_changes_cost_not_tokens`、`unknown_price_stays_unknown`，用假下载器验证调用计数及统计结果。
- [ ] 去除 `ensure_fresh()` 隐式下载职责，将“本地读取”和“远程同步”拆开；只在手动按钮命令中执行固定来源下载。先校验整批数据再替换，不发布半成功的新版本。
- [ ] 同步成功更新价格戳并触发费用重新计算，失败不更新成功时间；前端显示进行中、成功时间、失败原因、缺价状态。
- [ ] 运行全部统计、价格、前端测试，验证原始日志未更改，缓存迁移不重复计数；提交本任务。

### Task 7：集成审查与源码交付

**Files:** `docs/private-build-validation.md`、必要的安全回归测试；不添加自动部署。

- [ ] 针对最终代码运行 `npm run build`、`node --test scripts/*.test.mjs`、`cargo fmt --check`、可执行的 Rust 测试；分别记录通过、失败、受平台阻塞和从未运行。
- [ ] 独立审查全部 diff，重点检查未经授权的凭据读取、遗留更新入口、地域回退、统计丢失、迟到结果发布和失败写回。
- [ ] 修复审查确认的问题并重跑受影响检查；检查源码包不含日志、凭据、依赖目录和构建缓存。
- [ ] 交付源码包、补丁、测试记录及 Windows 构建步骤。只有 Windows 原生编译/运行实际完成后才交付为可安装版本；否则明确是待 Windows 验证的源码。
