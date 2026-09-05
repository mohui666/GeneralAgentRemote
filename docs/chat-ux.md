# 聊天体验改进

本轮以远程查看进展、定位内容、处理审批和发送图片为范围，保留现有 Host / Relay / 客户端架构。

## 同类产品参考

2026-09-05 查阅了以下官方资料，依据现有代码选择可直接改善聊天主流程的功能：

| 产品与来源 | 参考点 | 本轮落地 |
| --- | --- | --- |
| [Happy](https://happy.engineering/) | 在手机和 Web 上查看、引导并审批本机运行的 Agent | 让未处理审批保持可见，直达对应卡片 |
| [HAPI 工作流程](https://hapi.run/docs/guide/how-it-works) | 远程会话、实时消息、权限处理 | 在已有聊天流中增加搜索定位和阅读提示 |
| [HAPI 通知说明](https://hapi.run/docs/guide/pwa#notifications) | 区分等待输入、完成和失败 | 修正多个审批与结束回合的状态处理；本轮提示限应用内 |
| [ACP 内容协议](https://agentclientprotocol.com/protocol/v1/content) | 根据图片输入能力发送标准图片块 | 补通已经协商但未接入发送链路的图片输入 |

## 使用方式

- **搜索当前对话**：在聊天顶部点击搜索。Web 可在应用内按 Ctrl/Cmd+F，再用 Enter / Shift+Enter 切换结果，Esc 关闭。Android 提供上一条、下一条和关闭按钮。
- **搜索范围**：Web 搜索已加载的消息、计划和活动内容；Android 搜索已加载的用户与 Agent 消息。结果定位后可继续阅读上下文。需要更早内容时先加载历史；搜索不会主动全量下载会话。
- **阅读新内容**：向上阅读时保留当前位置。收到更新后，底部入口提示新内容；点击回到最新后恢复跟随。搜索和审批跳转不会立即被自动跟随拉回底部。
- **横屏布局**：Web 压缩搜索、审批提示和输入区，保持聊天与发送可见。Android 横屏搜索时暂时收起输入区，关闭搜索后恢复原草稿和附件。
- **处理审批**：当前回合有未处理审批时显示计数入口，点击定位审批卡片。多个请求必须分别处理；已完成、失败或中断回合的未处理审批失效，不能再次提交。
- **复制代码**：Web Markdown 代码块有独立复制按钮，复制保留原始代码；整条消息复制仍保留。剪贴板需要浏览器允许访问，通常使用 HTTPS 或 localhost。
- **发送图片**：选择支持图片输入的 ACP Agent 后，附件入口按能力开放。Host 检查并托管图片，以 ACP 图片内容发送，不将本地路径传给客户端。当前支持 PNG、JPEG、WebP、GIF，最多四张，单张及合计均不超过 10 MiB。Grok 1.0.13 的握手漏报图片能力，Host 已按该精确版本修正；其他版本与 Agent 继续按协议声明判断。

## 数据与边界

没有新增生产依赖、协议字段或持久化 schema。搜索与阅读状态保存在当前 UI 中，既有草稿、会话缓存和重连流程继续使用原来的作用域与存储。失效审批复用现有 `Progress` 活动类型保存，已处理审批保留原选择。

未加入系统推送、语音、会话分享、任意文件附件或远程文件工作台。图片入口代表当前 Agent 的已协商或已按精确版本验证的能力；某个具体模型是否接受图片仍由该 Provider 决定。协议模拟测试与真实模型调用分别记录。

## 本轮验证

- `cargo xtask test`：通过，包含格式检查、Clippy、136 个 Rust 测试（含 Host / Relay 集成）和 Web 生产构建。
- `cargo clippy -p agent-remote-host --all-targets -- -D warnings`：最终审批锁改动通过静态检查。
- Web 的 wasm check 与 wasm Clippy：通过。
- Android `lintDebug testDebugUnitTest assembleDebug`：最终构建通过，51 个单测；实测发现并修正词内下划线被当作 Markdown 强调符的问题。
- `cargo test -p agent-remote-testkit`：图片测试素材修正后 12 个测试通过；两个 mock binary 已重建。原有损坏 PNG 曾让界面完成回合后显示图片错误，现已替换为可解码的 PNG。
- 新增针对性测试覆盖：ACP 图片能力协商、托管 PNG 实际内容传输、不支持时拒绝；多审批状态、结束回合失效、审批回包不覆盖终态；搜索作用域及复制代码转义。
- Chromium 实际界面：通过隔离 Host 与 mock Codex 配对、发送、审批、停止、继续发送及图片显示。验证了搜索跳转、展开活动、中文代码精确复制、流式更新时保留滚动位置；桌面、390px、320px 和 844×390 横屏均完成截图检查。横屏搜索与审批同时展开曾挤掉聊天区，已修正并以重新构建的 Web 复测。
- Codex 真实图片调用：通过。生产 Web 文件选择器上传随机生成的 PNG，经独立 Rust Host 送至实际登录的 Codex 0.150.1，动态选中的模型为 `gpt-5.6-sol`。模型准确返回只出现在图片中的 `J4KQJTLB`，以及从左到右的蓝色正方形、红色圆形、黄色三角形；没有调用工具或读取文件。Web 与 Host 持久化状态均为完成，证据见 `dist/qa/chat-ux/real-images-codex/`。这是 Codex 图片链路的证据，不能代替 ACP 验证。
- Android 16 首轮独立模拟器实装：完成配对、对话搜索、上一/下一条定位、审批入口与提交，以及横竖屏截图检查。修正横屏搜索占满阅读区的问题后，重新安装最终 APK 检查了 2424×1080 横屏、软键盘及关闭搜索后的输入区恢复。使用 mock Codex；这不是物理手机或真实模型验收。最后审批提交后入口已消失，截图时仍在运行，未确认该轮完成；Android 独立新内容到达和长历史压力未实测。

构建产物为 `dist/web` 与 `dist/android/agent-remote-debug.apk`，本地验证日志和界面截图保存于 `dist/qa/chat-ux`。真实 Codex 和 Grok 图片调用分别记录；自动化测试通过仍不代表所有 Provider 或模型都完成图片验收。OpenCode 1.18.27 在创建会话阶段返回 `Internal error: OpenCode service failure` / `service=directory`，该次未到达模型图片调用，不计为通过。

## 续轮验收

- Grok 真实图片调用通过：已安装的 Grok 1.0.13 初始化虽然声明 `image=false`，实际接受标准 ACP 图片块。官方实现遗漏图片能力声明，Host 仅对 Grok profile、`grokShell=true` 与版本 `1.0.13` 的组合修正。重建后通过生产 Web 上传另一张随机图片，真实 `grok-4.6` 准确识别 `53D7XTDC` 与三种彩色图形，Host 与界面均为完成。证据：`dist/qa/chat-ux/real-images-grok/`。
- Android 真实 Codex 回合通过：独立 Host 7467 使用真实 `gpt-5.6-sol` 返回 `ANDROID_REAL_CODEX_COMPLETED`。Host 状态为完成；修正 Markdown 下划线后重新安装最终 APK，核对原会话完整原文与“已完成”，无需重复模型请求。证据：`dist/qa/chat-ux/android-completion/real-completed.png` 与 `real-host-state.json`。
- 首轮 Android 审批后卡运行的原因已复现：Web 与 Android 共用的 mock Codex 只有一个 `active_turn` / `pending_approval`，Web 后续回合覆盖并清空了 Android 回合。按旧数据库的交错顺序重放没有 Android 完成事件，串行对照正常完成。这是测试夹具的并发缺陷；旧结果不能作为产品完成证明。证据：`dist/qa/chat-ux/shared-mock-causality.json`。
- Android 独立审批闭环通过：最终 APK 连接专用 Host 7468，在没有其他客户端复用 mock 的情况下点击 Allow once，收到命令成功、最终回复与完成状态。Host 保存审批选择 `accept`，命令 exit code 为 0；原生界面最终回复和“已完成”已截图确认。真实回合与 mock 回合均记录了同一消息身份的全部六个发送阶段。证据：`dist/qa/chat-ux/android-completion/mock-completed.png` 与 `mock-host-state.json`。
- 最后 Host 验证：94 个库测试、Host 全目标 Clippy（warnings as errors）、格式检查均通过。没有新增协议字段或持久化 schema。

## 主要文件

| 位置 | 改动 |
| --- | --- |
| `crates/web/src/ui.rs`、`crates/web/src/lib.rs`、`web/app.css` | 搜索与定位、代码块复制、阅读提示及审批入口 |
| `android/app/src/main/java/dev/agentremote/messenger/ui/RemoteApp.kt`、`TimelineNavigation.kt`、`MarkdownText.kt` | 原生搜索、滚动导航、审批入口、状态提示与标识符原文显示 |
| `crates/host/src/app.rs` | 多审批状态、选项校验、终态失效处理和并发状态保护 |
| `crates/host/src/providers/acp.rs` | 能力协商驱动的图片输入及标准 ACP 内容发送 |
| `crates/testkit/src/codex.rs`、`crates/testkit/src/grok.rs` | 修正实际界面验证发现的损坏图片测试素材 |
