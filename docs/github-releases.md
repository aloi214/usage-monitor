# GitHub 自动检查与 Windows 发布

本仓库的构建目标是 **Windows x64 / NSIS `*-setup.exe`**。工作流不会发布 macOS/Linux 安装包，不会恢复上游自动更新，也不需要填写 PAT、签名私钥或第三方服务密钥。

## 1. PR：自动检查和试打包

向 `main` 新建、重新打开 PR，或向 PR 分支推送新提交后，`PR Windows checks` 会：

1. 在 GitHub 标准 `windows-latest` runner 保留源码 LF 换行，安装锁定的 npm 依赖。
2. 运行 Node 测试、TypeScript/Vite 生产构建和 Rust 单元测试。标记为 `ignored` 的真实联网探测不会执行。
3. 使用 Cargo 锁文件生成 NSIS 安装包。
4. 仅上传安装包，名称为 `rice-monitor-pr-<PR编号>-<合并测试提交SHA>`，保留 7 天。后续提交会取消同一 PR 的过时构建。

进入 PR 的 **Checks → Test and build Windows installer → Details**，或仓库 [Actions](https://github.com/aloi214/usage-monitor/actions)，打开成功的运行，在 **Artifacts** 下载并解压 ZIP。下载 Actions 产物通常需要登录 GitHub。产物用于试用和验收，不会自动创建 Release。PR 构建使用 GitHub 的临时合并提交；SHA 不一定等于 PR 分支的最新提交。

第三方贡献的 PR 可能需要仓库维护者批准运行。如果某个已有 PR 没有新触发事件，不代表检查已通过；先确认它实际有成功的运行。可使用下文的手动工作流选择该分支打包。

PR 的令牌只有 `contents: read`，检出后不保留 Git 凭据，不使用仓库 secrets，也不使用 `pull_request_target`。仅上传指定的安装包，不上传源码工作区、环境变量、账户信息、日志或调试目录。应用代码仍会作为构建/测试的一部分执行，因此应正常审查外部 PR。

## 2. 保留手动打包

原有 `Manual Windows build`（`.github/workflows/build.yml`）保持手动触发；仅补充检出前保留 LF 换行，避免 Windows 自动转换换行导致现有源码结构测试误报。在 **Actions → Manual Windows build → Run workflow** 选择分支；成功后下载 `rice-monitor-installer`。这也可用于尚未合并的修复分支，不会创建 Release。

## 3. 正式版本：创建标签后自动发布

先把本工作流 PR 以及需要发布的功能/修复合并到 `main`。发布标签必须指向已在 `main` 历史中的提交。**合并 PR 本身不会发布版本。**

每次发版先同步以下版本号并提交合并：

- `package.json` 的 `version`
- `package-lock.json` 的顶层 `version` 与 `packages[""].version`
- `src-tauri/tauri.conf.json` 的 `version`
- `src-tauri/Cargo.toml` 的 `[package].version`
- `src-tauri/Cargo.lock` 中 `rice-monitor` 包的 `version`

rice monitor 的源码版本从 `0.0.1` 开始，可在本地检查：

```powershell
node scripts/check-release-version.mjs v0.0.1
```

所有版本一致、审查和 Windows 验收完成后，由维护者明确选择要发布的提交和未使用的标签。**仓库已经存在一个指向旧源码、没有安装包的 `v0.0.1` 标签和公开 Release；本 PR 不移动标签、不删除或覆盖该 Release。重新使用 `v0.0.1` 需要维护者另行明确批准并处理旧发布，不能直接重新运行旧构建。**

后续使用新版本号时，先同步全部版本字段，再合并至 `main`，最后由维护者推送匹配的新标签。**推送标签会自动公开发布安装包，不要用真实版本标签做试运行。** 标签格式只接受 `vMAJOR.MINOR.PATCH`，不接受 `-rc`、构建后缀或前导零；错误格式、版本不一致或标签不在 `main` 上都会在安装依赖和打包前失败。

`Release Windows installer` 会重新测试和构建标签指向的源码，生成 `SHA256SUMS`，再把安装包和校验文件交给独立发布 job。发布 job 不检出或执行应用代码，仅它拥有临时 `GITHUB_TOKEN` 的 `contents: write` 权限。安装包上传期间 Release 保持草稿，上传完成才公开发布。若仓库或组织策略禁止 Actions 发布，运行会明确失败，需要维护者处理该策略；不要用额外 PAT 绕过。

发布成功后，从本仓库的 [Releases](https://github.com/aloi214/usage-monitor/releases) 下载 `*-setup.exe`。GitHub 自动附带的 Source code ZIP/TAR 是源码，不是安装包。无需先在 Releases 页面手动发布空版本，也不要选用上游项目的安装包。

### 失败与重跑

- 测试/构建失败：不会进入发布 job。检查失败日志，修复后用一个新版本/新标签发布；不要悄悄移动已发布标签。
- 上传或公开发布失败：可能留下未公开的 Release 草稿。流程刻意不覆盖任何现有 Release。维护者应先检查草稿和资产，决定手动完成发布，或删除这个失败草稿（保留标签）后重新运行失败的发布 job。
- 已公开版本不会被工作流重跑覆盖。要替换已发布内容，应审查修复并发布新的版本号。

## 4. 安装与安全边界

- 当前没有 Windows Authenticode 签名证书。安装器可能显示 **Unknown Publisher / 未知发布者** 或 SmartScreen 警告；`publisher: rice monitor` 文字不是数字签名。不要关闭系统安全设置。
- `SHA256SUMS` 只用于核对下载完整性，不证明发布者身份。可在 PowerShell 使用 `Get-FileHash '.\rice monitor_*.exe' -Algorithm SHA256`，与下载的 `SHA256SUMS` 比较。
- 缺少 WebView2 时，安装过程需要联网下载运行时；这不是完整离线安装包。
- 自动构建成功不等于实际安装、启动、托盘、账户登录/额度刷新已验收。仍需在 Windows 上测试，特别是装有与未装 WebView2 的环境。合成测试不会验证真实账户返回。
- 本次改名 PR 不创建或移动版本标签、不替换公开 Release。新旧产品安装身份与数据保留步骤见[改名与升级](rice-monitor-0.0.1.md)。

工作流使用标准 GitHub 托管 runner，无大型付费 runner。新增 PR/Release 工作流的临时 Artifacts 保留 7 天；手动工作流沿用仓库默认保留期。已发布 Release 的附件不随构建产物期限过期。

参考：[GitHub 工作流事件](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows)、[GITHUB_TOKEN 权限](https://docs.github.com/en/actions/tutorials/authenticate-with-github_token)、[下载构建产物](https://docs.github.com/en/actions/managing-workflow-runs-and-deployments/managing-workflow-runs/downloading-workflow-artifacts)、[GitHub CLI Release](https://cli.github.com/manual/gh_release_create)。
