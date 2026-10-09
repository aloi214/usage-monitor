# Windows 发布包：以体积为优先

当前源码为 Windows 发布构建设置了较小体积优先的 Cargo 配置：

| 配置 | 当前值 | 影响 |
|---|---|---|
| 优化级别 | `opt-level = "s"` | 以体积为优化目标，运行速度仍需实测 |
| 链接时优化 | `lto = true` | 完整 LTO，可能增加编译时间及内存占用 |
| 代码生成单元 | `codegen-units = 1` | 配合跨代码优化，可能降低并行编译效率 |
| 符号裁剪 | `strip = "symbols"` | 减少成品符号信息，也减少二进制调试细节 |
| panic 行为 | `panic = "unwind"` | 保留既有栈展开行为 |

这些设置针对 **release 成品**，不会缩小或清理已有的 `target/debug`、增量编译缓存或 `node_modules`。源码目录和开发缓存的占用不能等同于安装包大小；`npm run tauri dev` 也不使用此 release 优化配置。

## 安装包形式

- 默认只生成 **NSIS `*-setup.exe`**，不再同时生成 MSI。少生成一种安装格式会减少产物数量，不等于单个 NSIS 安装包因此变小。
- **LZMA 压缩**和 WebView2 **`downloadBootstrapper`** 已在配置中明确写出。它们原先就是 Tauri 默认值，本次不能把它们算作新增的体积收益。
- 不捆绑离线/固定版本 WebView2 Runtime、额外资源包或旁加载可执行程序。目标电脑缺少 WebView2 时，安装过程需要联网下载并安装运行时；这不是离线安装包。
- 仍不生成上游自动更新产物。

## Windows 构建

需要 Windows MSVC 工具链、Node.js/npm、Visual Studio C++ Build Tools 等[构建条件](private-build.md#1-构建条件)。在此私有源码根目录运行：

```powershell
npm ci
npm test
npm run tauri build
Get-Item .\src-tauri\target\release\pane.exe,
  .\src-tauri\target\release\bundle\nsis\*-setup.exe |
  Select-Object Name, Length
Get-FileHash .\src-tauri\target\release\bundle\nsis\*-setup.exe -Algorithm SHA256
```

`npm run tauri build` 会先运行前端生产构建，并按当前默认配置生成 NSIS。需要显式指定格式时可用 `npm run tauri build -- --bundles nsis`。

NSIS 输出通常在 `src-tauri/target/release/bundle/nsis/`。上述路径假设使用默认 Cargo target 目录；设置 `CARGO_TARGET_DIR`、Cargo `target-dir` 或显式目标 triple 会改变位置。GitHub Actions 使用同样的 NSIS 配置，缺少安装包产物时会报错。原有手动工作流仍可选择分支运行；新增的 PR 自动检查与版本标签发布流程见 [GitHub 自动检查与 Windows 发布](github-releases.md)。合并 PR 不会自动创建 Release。

## 还没有测量或验证的部分

**尚未完成此配置的 Windows MSVC/NSIS 安装包构建和原生安装验收，也没有可信的前后体积对比，不承诺减少多少 MB 或百分比。** 结构测试、schema 校验、前端构建、Cargo metadata 和交叉编译检查不能证明最终安装包大小、启动性能或 Windows 运行结果。

公平比较时，用相同应用源码、锁文件、Windows 工具链、NSIS 版本、架构及签名设置构建两次：一次使用原 release 默认配置，一次使用本次自定义配置。两次都保持 NSIS-only、同样的 WebView2 模式和压缩方式；使用独立干净的 target 目录，记录主程序和安装包字节数及 SHA-256。不要在对比中同时变更功能代码。

之后还需分别在已安装 WebView2，以及未安装但可以联网的 Windows 环境中验证安装、启动、托盘和刷新响应。单账号刷新使用方式见[额度刷新说明](scoped-quota-refresh.md)。

参考：[Tauri 应用体积](https://v2.tauri.app/concept/size/)、[Tauri Windows 安装包](https://v2.tauri.app/distribute/windows-installer/)、[Cargo profiles](https://doc.rust-lang.org/cargo/reference/profiles.html)。
