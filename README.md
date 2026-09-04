# Agent Remote Messenger

> 在 Android 或浏览器里继续电脑上的 **Codex 和 ACP coding-agent** 会话。Agent、项目文件、凭据和状态始终留在 Host。

它不是远程桌面或网页版 IDE，而是一条专注于消息、进度、审批和结果的轻量远程通道。

## 核心能力

- 原生 Android 与响应式 Web 客户端
- 扫码配对；Android Keystore 加密保存多 Host 凭证
- 按 Host + Provider 动态选择已授权项目，并同步已有远程会话和历史
- 保存最近 Host、Agent、项目、会话、模型、effort、权限和草稿；冷启动自动恢复
- 离线显示缓存，采用有上限的退避重连，也可手动停止或立即重试
- 首次发送时才创建新会话；客户端命令与 Provider 历史使用稳定 ID 去重
- 模型、effort、权限模式和附件限制全部来自当前 Provider capability
- 实时显示消息；计划、工具、命令、文件变化、审批和错误归并为可折叠活动
- 支持追加指令、停止任务、局域网直连和公网 Relay

```text
Android / Browser ── Direct WebSocket ───────────────┐
                                                     ▼
Android / Browser ── HTTPS Relay ◀── outbound ── Host ── Codex app-server
                                                   ├── Grok ACP
                                                   ├── Claude Code ACP bridge
                                                   ├── Gemini / Copilot / OpenCode ACP
                                                   ├── Cursor / Cline / Goose ACP
                                                   ├── JetBrains Junie ACP
                                                   ├── Qwen / Kimi / Kiro ACP
                                                   ├── Mistral Vibe / Qoder ACP
                                                   ├── Auggie / Factory Droid / Devin ACP
                                                   ├── CodeBuddy / GLM Agent / Kilo ACP
                                                   ├── Amp ACP bridge
                                                   └── authorized projects
```

Host 是唯一状态中心，远程客户端只能选择你提前授权的项目。

## Android APK

```powershell
cargo xtask android
adb install -r dist\android\agent-remote-debug.apk
```

APK 输出到 `dist/android/agent-remote-debug.apk`。完整说明见 [Android 构建与 USB 测试](docs/android.md)。

## 三步开始

### 1. 构建

构建只需要 Rust stable 和 `wasm32-unknown-unknown`。要实际开始远程对话，还需至少一个已经全局安装并完成本机认证的 Provider。Host 内置 Codex app-server 适配器，并提供 21 个 ACP profile：Grok、Claude Code、Gemini CLI、GitHub Copilot CLI、OpenCode、Cursor Agent、Cline、Goose、JetBrains Junie、Qwen Code、Kimi CLI、Kiro CLI、Mistral Vibe、Qoder CLI、Augment Auggie、Factory Droid、Devin、Tencent CodeBuddy、GLM Agent、Kilo Code 和 Amp。连同 Codex 共 **22 个内置 Provider**。各 Provider 相互独立，缺少某个命令不会阻止其他 Provider 工作。

Qoder 已通过正式的 `qoder --acp` profile 接入。ZCode 3.11.2 也已在 WSLg 临时环境完成 GUI 登录和免费模型真实回合，但截至本次核查没有公开的 ACP、app-server、SDK 或 API 可供 Host 调用，因此不列为内置 Provider。`glm-acp-agent` 是独立的 GLM ACP 适配器，不是对 ZCode 桌面程序的封装。

Host 默认从 `PATH` 启动 Provider；也可用文档列出的 `AGENT_REMOTE_*_BIN` 环境变量指定已安装的可执行文件。Host 不会下载、安装、更新或登录任何 CLI。Claude Code profile 要求 `claude-agent-acp` 已经全局安装；Host 不会在运行时调用 `npx`。Provider 凭据继续由各 CLI 保存在 Host 电脑上，不进入 Host 数据库、Relay 或远程客户端。

```powershell
rustup target add wasm32-unknown-unknown --toolchain stable
cargo xtask build
```

### 2. 授权项目并启动 Host

```powershell
dist\bin\agent-remote-host.exe project add C:\path\to\project
dist\bin\agent-remote-host.exe serve --web-root dist\web
```

省略 `--provider` 时启用全部内置 profile；已有项目可用 `project set-providers` 精确选择。

### 3. 配对

在另一个终端运行：

```powershell
dist\bin\agent-remote-host.exe pair
```

用 Android 应用扫描终端二维码，或在浏览器打开输出链接。配对链接十分钟过期且只能使用一次。

USB 调试时，配对前先运行：

```powershell
adb reverse tcp:7437 tcp:7437
```

## 验证命令

```powershell
cargo xtask test
cargo xtask android
cargo xtask provider-smoke
cargo xtask provider-smoke --provider opencode
```

- `cargo xtask test` 覆盖 Rust、Web 生产编译与确定性的 Codex/ACP mock 协议链路。
- `cargo xtask android --release` 运行 Android 协议单测并生成未签名 release APK。
- `provider-smoke` 在临时授权目录中对已安装且已认证的 Provider 运行真实回合；可重复使用 `--provider` 只测已确认有免费额度的项目。只有 `PASS` 表示该 Provider 完成了真实回合；缺少命令、登录、额度或付费条件会得到 `SKIP`，`SKIP` 不算验证，协议错误得到 `FAIL`。
- 自动化构建不能替代真机或真实 Provider 验收，两类结果应分别记录。

### Provider 分层实测

证据截至 **2026-09-05**。三列不能互相替代：安装检查只证明命令可执行；ACP `initialize` 只证明该协议进程能在登录前完成握手；真实回合才会请求模型并要求返回精确标记 `AGENT_REMOTE_SMOKE_OK`。`✅` 仅用于已有证据。

| Provider | Host 启动方式 | 临时环境安装/版本 | 协议占位 | 真实模型回合 |
|---|---|---|---|---|
| OpenAI Codex | `codex app-server --stdio` | ✅ 0.150.1 | ✅ app-server | ✅ PASS |
| Grok Build | `grok --no-auto-update agent stdio` | ✅ 1.0.13 | ✅ ACP `initialize` | ✅ PASS（Windows 同系统链路） |
| Claude Code | `claude-agent-acp` | ⏭ 按要求未安装 | ⏭ 未测 | ⏭ 未测 |
| Gemini CLI | `gemini --acp` | ⏭ 按要求未安装 | ⏭ 未测 | ⏭ 未测 |
| GitHub Copilot CLI | `copilot --acp --stdio --no-auto-update` | ✅ 1.0.82 | ✅ ACP `initialize` | ⏭ 按要求不再授权；停止于占位 |
| OpenCode | `opencode acp` | ✅ 1.18.27 | ✅ ACP `initialize` | ✅ PASS（1.18.25） |
| Cursor Agent | `agent acp` | ✅ 2026.09.02-c22c1a3 | ✅ ACP `initialize` | ⏭ 按要求不再授权；停止于占位 |
| Cline | `cline --acp` | ✅ 3.0.61 | ✅ ACP `initialize` | ⏭ 按要求不再授权；停止于占位 |
| Goose | `goose acp` | ✅ 1.49.0 | ✅ ACP `initialize` | ⏭ 按要求不再授权；停止于占位 |
| JetBrains Junie | `junie --acp=true` | ✅ 26.8.31（3013.5） | ✅ ACP `initialize` | ⏭ 按要求不再授权；停止于占位 |
| Qwen Code | `qwen --acp` | ✅ 0.23.0 | ✅ ACP `initialize` | ⏭ 按要求不再授权；停止于占位 |
| Kimi CLI | `kimi acp` | ✅ 0.41.0 | ✅ ACP `initialize` | ⏭ 按要求不再授权；停止于占位 |
| Kiro CLI | `kiro-cli acp` | ✅ 2.21.0 | ⏭ 登录前拒绝，未完成 `initialize` | ⏭ 按要求不再授权；停止于占位 |
| Mistral Vibe | `vibe-acp` | ✅ 2.25.0 | ✅ ACP `initialize` | ⏭ 按要求不再授权；停止于占位 |
| **Qoder CLI** | `qoder --acp` | ✅ 1.1.43 | ✅ ACP `initialize` | ⏭ 按要求不再授权；停止于占位 |
| Augment Auggie | `auggie --acp` | ✅ 0.36.0 | ✅ ACP `initialize` | ⏭ 按要求不再授权；停止于占位 |
| Factory Droid | `droid exec --output-format acp-daemon` | ✅ 0.212.0 | ✅ ACP `initialize` | ⏭ 按要求不再授权；停止于占位 |
| Devin | `devin acp` | ✅ 3000.6.14 | ✅ ACP `initialize` | ⏭ 按要求不再授权；停止于占位 |
| Tencent CodeBuddy | `codebuddy --acp` | ✅ 2.143.1 | ✅ ACP `initialize` | ⏭ 按要求不再授权；停止于占位 |
| GLM Agent | `glm-acp-agent` | ✅ 包 1.8.0 | ✅ ACP `initialize` | ⏭ 按要求不再授权；停止于占位 |
| Kilo Code | `kilo acp` | ✅ 7.5.9 | ✅ ACP `initialize` | ⏭ 按要求不再授权；停止于占位 |
| Amp | `amp-acp` | ✅ bridge 0.9.0 | ✅ ACP `initialize` | ⏭ 按要求不再授权；停止于占位 |
| ZCode 3.11.2（非内置 Provider） | 无公开 Host 协议 | ✅ WSLg AppImage 启动 | ⏭ 无 ACP/app-server/SDK/API | ✅ GUI PASS（免费 GLM-5.3-Flash，非 Host） |

Codex 和 OpenCode 的 Host 真实回合勾选来自 2026-09-04 的既有记录；OpenCode 当时使用 1.18.25，本次隔离环境安装的是 1.18.27。Grok 1.0.13 于 2026-09-05 由当前源码构建的 Windows Host 直接调用已登录的 Windows Grok，在 Windows 临时项目中返回精确标记。ZCode 同日通过自身 GUI 使用免费 GLM-5.3-Flash 返回精确标记，但没有可供 GeneralAgentRemote Host 调用的公共协议。其余已安装 Provider 只执行版本检查和一次 ACP `initialize`；没有发送 `authenticate`、`session/new`、`session/prompt` 或模型请求。完整边界见 [Provider 兼容性](docs/provider-compatibility.md#verification-status-and-real-smoke-policy)。

### Android 16 模拟器实测

实测环境：**截至 2026-09-05，Pixel 9 AVD `Pixel_9_API_36_1`，Android 16 / SDK 36，1080×2424 @ 420 dpi**。以下勾选均来自安装后的 Android 应用、隔离 Host 和真实 Provider 链路。

| 场景 | 实测结果 |
|---|---|
| 构建、保留数据安装、启动与前台 Activity | ✅ PASS |
| 协议 v5 配对、在线与历史恢复 | ✅ PASS |
| 4 个授权项目与 Codex/Grok 切换器 | ✅ PASS |
| 一次性链接配对并进入在线状态 | ✅ PASS |
| 真实 Codex 发送与六阶段关联链 | ✅ PASS：首个 Provider 事件 17.264 秒 |
| 第二次真实发送延迟 | ✅ PASS：首个 Provider 事件 6.494 秒 |
| 项目树展开/折叠 | ✅ PASS：展开态显示 9 条会话 |
| 强制停止后自动重连 | ✅ PASS：2.947 秒恢复认证在线 |
| 竖屏与横屏布局 | ✅ PASS：逐图确认内容、输入框和控制项完整可见 |
| 8 秒应用日志 | ✅ PASS：无崩溃、ANR 或协议错误 |

本机启动 Android 16 AVD、跨 WSL 调用 Windows ADB、配对和场景命令已经写入 [`AGENTS.md`](AGENTS.md#13-android-16-emulator-test-tutorial)。

需要由 Codex/AI 在电脑端驱动一台已明确连接的 Android 设备时，使用 `cargo xtask android-device doctor|prepare|inspect|ui|scenario|logs|capture`。该工具只通过本地 adb 和稳定 accessibility ID 执行开发测试，不会把远程终端能力加入产品；完整命令见 [Android 文档](docs/android.md#aicli-device-test-driver)。

## 安全边界

- 公网部署应使用 HTTPS/WSS；`--dev-insecure` 只适合受信任的开发网络。
- Relay 不持久化项目、消息或附件，但它是受信任传输端点；v0.1 不宣称应用层端到端加密。
- Provider 功能按实际 ACP 握手动态开放。缺少会话列表、历史恢复、模型、effort、权限模式或附件能力时，对应控件会隐藏或禁用；例如 Gemini CLI 当前不提供 ACP `session/list`。
- 当前客户端只持久化并重放一条未确认发送，不是通用离线任务队列。
- v0.1 不包含远程终端、文件浏览器、Git/代码编辑器、推送通知或云端 Agent。

## 文档

[Host 设置](docs/setup.md) · [Android](docs/android.md) · [公网 Relay](docs/public-relay.md) · [架构](docs/architecture.md) · [Provider 兼容性](docs/provider-compatibility.md)
