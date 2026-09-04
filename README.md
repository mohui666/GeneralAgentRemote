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

构建只需要 Rust stable 和 `wasm32-unknown-unknown`。要实际开始远程对话，还需至少一个已经全局安装并完成本机认证的 Provider。Host 内置 Codex app-server 适配器，并提供 Grok、Claude Code、Gemini CLI、GitHub Copilot CLI、OpenCode、Cursor Agent、Cline、Goose 和 JetBrains Junie 的 ACP profile。各 Provider 相互独立，缺少某个命令不会阻止其他 Provider 工作。

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
```

- `cargo xtask test` 覆盖 Rust、Web 生产编译与确定性的 Codex/ACP mock 协议链路。
- `cargo xtask android --release` 运行 Android 协议单测并生成未签名 release APK。
- `provider-smoke` 在临时授权目录中对每个已安装且已认证的 Provider 运行真实回合。只有 `PASS` 表示该 Provider 完成了真实回合；缺少命令、登录、额度或付费条件会得到 `SKIP`，`SKIP` 不算验证，协议错误得到 `FAIL`。
- 自动化构建不能替代真机或真实 Provider 验收，两类结果应分别记录。

### 真实 Provider 实测

实测日期：**2026-09-04**。`✅` 只表示通过 GeneralAgentRemote Host 适配器完成真实模型回合，并返回精确标记 `AGENT_REMOTE_SMOKE_OK`。

| Provider | Host 启动方式 | 真实回合结果 |
|---|---|---|
| OpenAI Codex 0.150.1 | `codex app-server --stdio` | ✅ PASS |
| OpenCode 1.18.25 | `opencode acp` | ✅ PASS |
| Grok Build 1.0.13 | `grok --no-auto-update agent stdio` | ⚠️ Windows CLI 从 WSL Host 启动时拒绝 WSL 路径；同系统环境待测 |
| Claude Code | `claude-agent-acp` | ⏭ SKIP：测试机未安装 |
| Gemini CLI | `gemini --acp` | ⏭ SKIP：测试机未安装 |
| GitHub Copilot CLI | `copilot --acp --stdio --no-auto-update` | ⏭ SKIP：测试机未安装 |
| Cursor Agent | `agent acp` | ⏭ SKIP：测试机未安装 |
| Cline | `cline --acp` | ⏭ SKIP：测试机未安装 |
| Goose | `goose acp` | ⏭ SKIP：测试机未安装 |
| JetBrains Junie | `junie --acp=true` | ⏭ SKIP：测试机未安装 |

需要由 Codex/AI 在电脑端驱动一台已明确连接的 Android 设备时，使用 `cargo xtask android-device doctor|prepare|inspect|ui|scenario|logs|capture`。该工具只通过本地 adb 和稳定 accessibility ID 执行开发测试，不会把远程终端能力加入产品；完整命令见 [Android 文档](docs/android.md#aicli-device-test-driver)。

## 安全边界

- 公网部署应使用 HTTPS/WSS；`--dev-insecure` 只适合受信任的开发网络。
- Relay 不持久化项目、消息或附件，但它是受信任传输端点；v0.1 不宣称应用层端到端加密。
- Provider 功能按实际 ACP 握手动态开放。缺少会话列表、历史恢复、模型、effort、权限模式或附件能力时，对应控件会隐藏或禁用；例如 Gemini CLI 当前不提供 ACP `session/list`。
- 当前客户端只持久化并重放一条未确认发送，不是通用离线任务队列。
- v0.1 不包含远程终端、文件浏览器、Git/代码编辑器、推送通知或云端 Agent。

## 文档

[Host 设置](docs/setup.md) · [Android](docs/android.md) · [公网 Relay](docs/public-relay.md) · [架构](docs/architecture.md) · [Provider 兼容性](docs/provider-compatibility.md)
