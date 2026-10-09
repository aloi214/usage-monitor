# rice monitor 0.0.1：名称、版本与 Windows 启动

## 本次变化

- 产品显示名称统一为 **rice monitor**，版本从 **0.0.1** 开始。npm/Cargo 包名与主程序是 `rice-monitor` / `rice-monitor.exe`；窗口、托盘、开始菜单快捷方式、安装器与 Actions 产物采用新名称。
- 不注册随 Windows 登录启动。不保留开机启动开关，恢复默认设置也不会开启它；旧配置中的 `autostart: true` 不再授予启动能力。
- `RICE_CHANGELOG.md` 是本产品的变化记录；`CHANGELOG.md` 保留上游历史，MIT 许可与作者署名不变。
- 保留 `local.pane.private` 应用标识，以及 `%APPDATA%\PanePrivate` 中的配置、权限、密钥、缓存。这里的旧名字是兼容标识，不要求用户改名目录，不会导入上游 Pane/OpenUsage 的数据。
- 不恢复自动更新、遥测或任何新账户权限。

## 从旧 Pane Private 安装版更新

Tauri 的 NSIS 卸载登记使用产品名。直接改名会变成另一个安装身份，因此本安装器在检测到旧 **Pane Private** 的卸载登记时停止，并提示先卸载旧程序；不会执行登记中的卸载命令，也不会自动删除旧程序、配置或快捷方式。

1. 退出旧 Pane Private 托盘程序与开发进程。正在运行的私有版与 rice monitor 共享单实例标识；安装器会要求先退出这个实例，不会按 `pane.exe` 文件名批量结束进程。
2. 在 Windows **设置 → 应用 → 已安装的应用**中卸载 **Pane Private**。保留用户数据，勿选择删除应用数据；不要清空 `%APPDATA%\PanePrivate` 或 Tauri 标识对应的 WebView 数据。
3. 再运行 rice monitor 安装器，完成后手动启动。旧配置、权限与密钥继续使用，无需重新复制或开放权限。
4. 用 Windows **设置 → 应用 → 启动**核对没有旧 Pane Private 启动项。若安装来源是自定义便携路径或自己创建的快捷方式，按下节说明处理。

检测覆盖当前用户与全局的旧卸载登记及适用的 32/64 位视图。访问失败也会停止，避免把无法检查当成没有旧版本。静默/被动安装会非零退出，不等待交互弹窗。此保护发生在复制主程序、写本产品登记和创建快捷方式之前，但 Tauri 的 WebView2 处理和安装目录准备可能先发生；它不是保证零系统副作用的预检查。后续同名 rice monitor 更新时，Tauri 的维护流程还可能在此钩子前运行旧 rice monitor 卸载器；该钩子不替代 Tauri 原有的更新流程。

全新安装没有旧登记或运行实例时可正常继续。后续 rice monitor 更新使用稳定的新产品名。源码/便携版用户只需退出旧进程，使用完整新源码重建或替换程序，保留数据目录；不要继续启动旧版，否则旧版仍可能恢复自己的自动启动设置。

## 旧启动项的处理范围

每次手动启动发布版时，只检查当前用户 Run 键中名称恰为 **Pane Private** 的旧值。对本地存在的程序，验证 Windows 版本资源中的产品名为 **Pane Private** 或 **rice monitor**；对卸载后已不存在的程序，只认可当前程序、同目录旧 `pane.exe`、`%LOCALAPPDATA%\Pane Private\pane.exe`，或旧私有版安装目录登记中明确记载的 `pane.exe` 路径。已存在但产品名不符的程序优先保留，不以目录或文件名推断归属。清理前会再次核对原值，读取或删除失败不会声称成功。

不探测 UNC、映射网络盘或经过符号链接/联接点的程序路径。带未知参数、格式异常、不可核实的自定义旧路径会保留并记录提示，应由用户在 Windows 启动设置中检查。没有旧值时不写入任何启动配置。旧 NSIS 的 `Software\Pane\Pane Private` 默认安装目录值通常在卸载后仍保留；如果这项证明也已删除，自定义位置的残留项不会被猜测清除。

不会枚举或删除其他应用的启动项，不处理上游 **Pane** 的值，也不修改系统范围的 Run、计划任务、启动文件夹或用户自己建立的启动快捷方式。仅有 StartupApproved 状态不能启动应用，因此不需要改动该状态。开发构建不修改 Windows 启动配置。

移除旧安装版后，指向已删除程序的残留 Run 值不能再启动程序；自定义便携旧版若仍保留，应在 Windows 启动设置中关闭该旧项，停止使用旧版本。运行中的旧版不会因设置变化自动退出。

## 验证边界与发布

自动化检查覆盖产品/版本一致性、初始和本地化界面、重置流程、托盘 UTF-16 容量、旧启动项路径判定及安装保护。GitHub Windows CI 会执行 Node/Rust 测试和 NSIS 打包；请以当前 PR 的具体运行结果为准。Linux 检查及打包成功均不能代替 Windows 原生的首次安装、旧版迁移、注销再登录、卸载、托盘操作与真实账户验收。

仓库已有一个指向旧源码的 `v0.0.1` 和没有安装包的公开 Release。本改名 PR 不移动该标签、不覆盖该 Release、不合并主分支，也不直接发布。重新使用同一个版本标签需要维护者另行决定并明确批准。详见[发布流程](github-releases.md)。

参考：[Tauri 产品名](https://v2.tauri.app/reference/config/#productname)、[NSIS 安装钩子](https://v2.tauri.app/reference/config/#installerhooks)、[本项目锁定 CLI 2.11.4 的安装模板](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.11.4/crates/tauri-bundler/src/bundle/windows/nsis/installer.nsi)。
