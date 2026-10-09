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

所有版本一致、审查和 Windows 验收完成后，由维护者明确选择要发布的提交和新标签。已公开的 `v0.0.1` 不应通过重建覆盖；后续发布先同步版本并合并至 `main`，再推送匹配的新标签。**推送标签会自动公开发布新版本，不要用真实版本标签做试运行。** 标签只接受 `vMAJOR.MINOR.PATCH`；错误格式、版本不一致或提交不在 `main` 历史上都会在打包前失败。

`Release Windows installer` 会重新测试和构建标签指向的源码。打包检查通过后，先把 `rice monitor_<版本>_x64-setup.exe` 重命名为 GitHub 上传时使用的 `rice.monitor_<版本>_x64-setup.exe`，再生成 `SHA256SUMS`。因此本地、Actions 产物、Release 附件和校验清单使用同一个文件名。

独立发布 job 仅检出触发提交 SHA 中的两个无依赖发布脚本，不安装依赖、不执行应用代码或产物内的脚本、不保留 Git 凭据。只有该 job 拥有临时 `GITHUB_TOKEN` 的 `contents: write`；不需要 PAT。它验证产物校验和、版本文件名及远端 `refs/tags/<标签>` 的实际提交，分页查找所有同标签 Release，并始终按 **Release ID / Asset ID** 操作。若仓库策略禁止发布，明确失败，不用额外凭据绕过。

- **没有现有 Release：** 创建带工作流标记的草稿，上传并重新下载验证两个附件，最后才按 ID 公开。
- **唯一现有 Release：** 只补充缺失的本版本安装包和 `SHA256SUMS`；保留标题、说明、预发布状态、其他附件和当前公开/草稿状态。用户创建的草稿不会被自动公开；已经公开的空 Release 补附件时也不会被改回草稿。
- **已有完整附件：** 校验下载内容与清单及源提交绑定，验证成功即不写入。相同提交重新打包可能产生不同字节，不能据此覆盖已验证的原安装包。
- **有多个同标签 Release（包括草稿）：** 停止并列出 ID，不猜测目标、不创建新的重复草稿、不删除或编辑已有项。

自动上传的安装包通过附件 label 记录提交 SHA 与摘要；已有安装包也可通过 Release 的完整 `target_commitish` SHA 与本次标签提交匹配。该绑定用于重跑检查，不是数字签名或发布者身份认证。未能确认来源、文件名冲突或摘要不符时必须人工检查。触发器仍仅为标签 push；仅编辑现有 Release 不会触发构建。

发布成功后，从本仓库的 [Releases](https://github.com/aloi214/usage-monitor/releases) 下载 `*-setup.exe`。GitHub 自动附带的 Source code ZIP/TAR 是源码，不是安装包。无需先在 Releases 页面手动发布空版本，也不要选用上游项目的安装包。

### 失败与重跑

- 测试/构建失败：不会进入发布 job。修复后使用新版本/新标签；不要移动已发布标签。
- 上传中断或响应丢失：可以重跑失败的发布 job；它会重新发现唯一 Release 的 ID，验证已有附件，仅补缺失项。不会在同一次运行中盲目重试创建或上传。
- 仅有安装包：来源和实际摘要通过检查后，为原安装包补校验清单，即使本次重建字节不同也不替换原文件。仅有校验清单：只有本地产物符合该清单时才补安装包，否则停止，需找回原构建产物或发布新版本。
- 工作流自己创建的草稿可以在完整验证后继续公开。用户创建/接管的草稿保持私有；要接管工作流草稿，可移除说明中的 `rice-monitor-release` HTML 标记。
- 附件处于未完成的 `starter` 状态、摘要冲突、同名/旧式空格文件名冲突、缺少来源绑定、标签提交变化或多个同标签 Release：流程停止并说明原因。维护者须先审查对应 ID；脚本从不删除附件、草稿或标签，也没有 `--clobber` 恢复路径。
- 已公开的空/部分 Release 在补附件期间仍然公开；失败时可能仍不完整。检查日志后按以上策略重跑，工作流不会偷偷改变其可见性。
- 发布前会重新检查两项附件和标签/Release 身份。请勿在运行期间并发编辑同一 Release；GitHub 不提供跨标签、附件和公开操作的原子事务。

## 4. 安装与安全边界

- 当前没有 Windows Authenticode 签名证书。安装器可能显示 **Unknown Publisher / 未知发布者** 或 SmartScreen 警告；`publisher: rice monitor` 文字不是数字签名。不要关闭系统安全设置。
- `SHA256SUMS` 只用于核对下载完整性，不证明发布者身份。可在 PowerShell 使用 `Get-FileHash '.\rice.monitor_*.exe' -Algorithm SHA256`，与下载的 `SHA256SUMS` 比较。
- 缺少 WebView2 时，安装过程需要联网下载运行时；这不是完整离线安装包。
- 自动构建成功不等于实际安装、启动、托盘、账户登录/额度刷新已验收。仍需在 Windows 上测试，特别是装有与未装 WebView2 的环境。合成测试不会验证真实账户返回。
- 本次流程修复不创建或移动真实版本标签、不修改现有公开 Release 或遗留草稿。新旧产品安装身份与数据保留步骤见[改名与升级](rice-monitor-0.0.1.md)。

工作流使用标准 GitHub 托管 runner，无大型付费 runner。新增 PR/Release 工作流的临时 Artifacts 保留 7 天；手动工作流沿用仓库默认保留期。已发布 Release 的附件不随构建产物期限过期。

参考：[GitHub 工作流事件](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows)、[GITHUB_TOKEN 权限](https://docs.github.com/en/actions/tutorials/authenticate-with-github_token)、[下载构建产物](https://docs.github.com/en/actions/managing-workflow-runs-and-deployments/managing-workflow-runs/downloading-workflow-artifacts)、[GitHub CLI Release](https://cli.github.com/manual/gh_release_create)。
