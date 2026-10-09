# rice monitor

[English](README.md) · **简体中文** · [Русский](README.ru.md)

这是基于 [Pane for Windows](https://github.com/ItsJazii/pane) 上游提交 `a55578c` 的隐私强化源码分支。应用使用 Tauri、Rust 和 TypeScript，在 Windows 托盘中显示 AI 账户额度，以及另行授权的本地用量估算。

> 产品现名为 **rice monitor**，自己的版本从 **0.0.1** 开始。上游安装脚本、winget 包和安装包不包含此分支修改。前身源码已通过 Windows CI 测试与 NSIS 打包；这不代表本次改名版本、原生安装、托盘交互或真实账户已经验收。请查看当前 PR 检查和[改名与升级说明](docs/rice-monitor-0.0.1.md)。

## 从这里开始

- **[GitHub 自动检查与 Windows 发布](docs/github-releases.md)**：PR 试打包、手动分支构建与版本标签发布

最新变化：[按平台控制自动刷新配额](docs/provider-auto-refresh.md) · [CommandCode GOAT 实验性接入](docs/commandcode-provider.md) · [单账号额度刷新](docs/scoped-quota-refresh.md) · [Windows 发布包体积配置](docs/release-package.md)。[Claude 手动查询说明](docs/manual-claude-query.md)保留旧版本验证记录。

- **[私有版构建与使用指南](docs/private-build.md)**：构建条件、源码构建、首次授权与验证边界
- **[设置更新与升级步骤](docs/settings-update.md)**：统一平台管理、日志位置模式及保留配置的更新方法
- [隐私说明](docs/privacy.md)：账户访问、本地日志、存储与联网行为
- [服务商说明](docs/providers.md)：全部 23 个服务商：保留原有 22 个，新增实验性 CommandCode
- [本地 HTTP API](docs/local-http-api.md)：供本机工具读取的快照
- [安全说明](SECURITY.md)：安全边界与问题报告

## 私有分支的变化

- 不随 Windows 登录自动启动，移除开机启动开关和重置时开启路径。只清理能核实为本应用的旧启动项；[升级说明](docs/rice-monitor-0.0.1.md)列出处理范围与限制。

- 移除上游遥测和自动更新。更新需手动安装本仓库经过审阅的 Release（如已有发布），或审阅源码后重新构建。
- 账户权限默认关闭。**平台管理**统一收纳账户权限、区域/连接模式、密钥及指标布局。平台标题行的**查询**开关只允许进一步选择账户，不会自动授权；关闭会撤销此平台全部账户查询和登录刷新。指标显隐只改变显示。
- **自动刷新配额**按平台独立设置，同平台额外账户共用一个开关和全局刷新间隔。Claude、CommandCode 默认关闭，其余支持账户查询的平台默认开启；开启不会授权账户，权限仍默认关闭。关闭后保留适用的历史结果，卡片刷新和全局刷新 / Ctrl+R 仍可手动查询已授权账户。Hermes 只有本地日志来源，不提供此开关。
- **本地日志来源**默认关闭，每种工具独立开关，可选择已知**默认位置**或保存准确的**自定义位置**。未找到目录不等于关闭；无效自定义路径保留原授权，不回退。账户查询关闭时仍可使用本地统计，不会为日志归属而搜集登录凭据。
- 移除 One/New API 和 Sub2API，其余 22 个服务商保留，包括纯本地的 Hermes 和本机 Ollama；新增实验性 CommandCode 后合计 23 个。
- Windows 配置使用独立的 `%APPDATA%\PanePrivate`，不会导入上游 Pane/OpenUsage 的配置、密钥或缓存。
- 源码已实现精确区域/凭据模式、手动同步公开价格表、日志片段哈希缓存和 OAuth 刷新协调。[验证边界](docs/private-build.md#integration-status)见指南；合成测试和交叉目标检查不能当作 Windows 原生验证。

账户额度与日志估算是两类数据。本地花费不是账单、账户余额，也不能证明由哪个账户付费。缺少价格的模型仍统计实测 Token，不把未知价格当成免费。

## 致谢与许可

保留上游 [MIT 许可](LICENSE)及完整署名：

- © 2026 Jazii，[Pane for Windows](https://github.com/ItsJazii/pane)
- © 2025 Robin Ebers，[OpenUsage for macOS](https://github.com/robinebers/openusage)，原始概念与服务商研究
- © 2025 Peter Steinberger，[CodexBar](https://github.com/steipete/CodexBar)，许可中列明的服务商研究

另感谢 [Tauri](https://tauri.app/)、[LiteLLM](https://github.com/BerriAI/litellm)、[models.dev](https://models.dev/)、[prasen.dev](https://www.prasen.dev/) 和 [shadcn/ui](https://ui.shadcn.com/) 为上游提供的组件、数据和视觉技术。服务商名称及标志归其所有者所有；本分支不代表上述服务商或上游项目。

`RICE_CHANGELOG.md` 记录 rice monitor 的变化；`CHANGELOG.md` 为上游历史记录，不是 rice monitor 的发布或安全验证记录。
