# 智谱与 Kimi 额度窗口修复

本次基于 `9fa6f657c3c3c67968529b24413f4bf68bc4722c`，只修复额度响应解析和对应的旧界面配置迁移。账户授权、自动/手动刷新、代理、钱包查询以及打包配置不变。

## 智谱 / Z.ai

旧实现仅按 `TOKENS_LIMIT` 命名，忽略周期字段；前端按名字去重并取第一项，导致同时返回的 5 小时和每周额度合并显示。

修复后：

- 按 `unit=3, number=5` 识别 **5 Hours**，按 `unit=6, number=1` 识别 **Weekly**，保留各自的 `nextResetTime` 毫秒时间。
- 以响应中的百分比为准，合法的零值不会丢失；没有百分比时，只有真实返回的用量与上限同时存在才计算比例。
- 不按数组顺序猜周期。不认识的周期保留原始 `unit/number` 标签，不伪造 5 小时或每周额度。
- 已识别窗口优先保留，避免旧的五项显示上限把它们截掉。

该额度端点没有可依赖的公开官方 JSON 契约。周期和字段含义参考 [CodexBar 实现](https://github.com/steipete/CodexBar/blob/f8b75cf2a3f701496ad3e945a2de4cf385c302e5/Sources/CodexBarCore/Resources/Plugins/zai.js)及其 [BigModel CN 回归样例](https://github.com/steipete/CodexBar/blob/f8b75cf2a3f701496ad3e945a2de4cf385c302e5/Tests/CodexBarTests/ZaiProviderTests.swift)，不是对某个具体账户响应的实测。既有 `TIME_LIMIT` 搜索额度的语义不在本次修复范围内。

## Kimi Code

增加官方新版 `usages` 比例池解析：

- `limit_5h` → Session
- `limit_7d` → Weekly
- `limit_month_total` → Monthly

`used_ratio` 是 0–1 已用比例；界面由此展示已用/剩余百分比，保留 `reset_time` 的有效重置时间。没有返回的窗口不会凭空出现，非法比例不按零处理。

Monthly 是 Kimi 与 Code 的共享总池；`limit_month_code` 是消费分项，不作为独立总额度重复显示。接口未提供月周期起点，因此月额度不假定 30 天，也不生成没有依据的周期进度。它不代表绝对 token 或调用次数上限。

保留旧套餐的五小时和周额度响应兼容。新版比例结构优先，不把新版响应中的无明确周期旧字段误标成 Weekly。依据：[官方结构](https://github.com/MoonshotAI/kimi-code/blob/main/packages/oauth/src/managed-usage.ts)、[官方测试](https://github.com/MoonshotAI/kimi-code/blob/main/packages/oauth/test/managed-usage.test.ts)。

## 旧卡片配置

- 旧智谱 `TOKENS_LIMIT` / `token_limit` 布局映射到实际返回的窗口。隐藏和展开偏好保留；单项星标/置顶优先映射到 5 Hours，否则使用实际存在的 token 窗口。
- 若接口仍真实返回未识别的裸 `TOKENS_LIMIT`，它继续作为实际行保留，不擅自当成旧别名删除。
- Kimi 只有 Monthly、没有真实 Weekly 时，旧 Weekly 的布局与选择迁移到 Monthly；若两者均存在则分别保留。
- 无需清空平台配置或重新填写密钥。

## 验证

基于本地冻结产品修订 `588298e22d7b775e2c26d209a80ac5c0f9ecfa90`，独立验证结果如下：

- Rust：**561 通过、0 失败、2 个既有真实联网探测忽略**。核心 41、扫描 231、网络 98、凭据刷新 141、查询发布 50。
- 前端 Node：**198 项通过**；TypeScript/Vite 生产构建通过。
- 完整应用和测试的 Windows GNU `cargo check --tests` 通过，使用实际交叉编译工具链，不是 Windows API 替身。
- Tauri schema、release TOML、依赖 metadata、版本一致性、差异检查通过；最终产品文件与冻结版本逐文件一致。
- 独立复查另外验证了 5 组解析边界样例、13 项完整模块 DOM 用例，以及实际 Z.ai/Kimi 模块测试；没有剩余阻断问题。

仍有既有未使用代码等警告；整文件 rustfmt 检查在本次改动前后均有历史差异，未为此重排无关代码。所有用量样例为合成测试或公开协议样例，没有读取用户凭据、查询真实账号或运行真实联网探测。模拟 DOM 与 Windows GNU 类型检查不能代替 Windows/WebView2 实机、真实账户及安装包验收。
