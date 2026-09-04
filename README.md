# Agent Remote Messenger

> 在 Android 或浏览器里继续电脑上的 **Codex / Grok** 会话。Agent、项目文件和状态始终留在 Host。

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
Android / Browser ── HTTPS Relay ◀── outbound ── Host ── Codex / Grok
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

需要 Rust stable、`wasm32-unknown-unknown`，以及已经安装并登录的 Codex 或 Grok。

```powershell
rustup target add wasm32-unknown-unknown --toolchain stable
cargo xtask build
```

### 2. 授权项目并启动 Host

```powershell
dist\bin\agent-remote-host.exe project add C:\path\to\project --provider codex
dist\bin\agent-remote-host.exe serve --web-root dist\web
```

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

- `cargo xtask test` 覆盖 Rust、Web 生产编译与 Codex/Grok mock 协议链路。
- `cargo xtask android --release` 运行 Android 协议单测并生成未签名 release APK。
- `provider-smoke` 只有 `PASS` 才表示本机真实 Provider 可用；`SKIP` 不算验证。
- 自动化构建不能替代真机或真实 Provider 验收，两类结果应分别记录。

需要由 Codex/AI 在电脑端驱动一台已明确连接的 Android 设备时，使用 `cargo xtask android-device doctor|prepare|inspect|ui|scenario|logs|capture`。该工具只通过本地 adb 和稳定 accessibility ID 执行开发测试，不会把远程终端能力加入产品；完整命令见 [Android 文档](docs/android.md#aicli-device-test-driver)。

## 安全边界

- 公网部署应使用 HTTPS/WSS；`--dev-insecure` 只适合受信任的开发网络。
- Relay 不持久化项目、消息或附件，但它是受信任传输端点；v0.1 不宣称应用层端到端加密。
- 当前客户端只持久化并重放一条未确认发送，不是通用离线任务队列。
- v0.1 不包含远程终端、文件浏览器、Git/代码编辑器、推送通知或云端 Agent。

## 文档

[Host 设置](docs/setup.md) · [Android](docs/android.md) · [公网 Relay](docs/public-relay.md) · [架构](docs/architecture.md) · [Provider 兼容性](docs/provider-compatibility.md)
