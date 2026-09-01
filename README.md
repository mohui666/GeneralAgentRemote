# Agent Remote Messenger

> 在 Android 或浏览器里继续电脑上的 **Codex / Grok** 会话。Agent、项目文件和状态始终留在 Host。

它不是远程桌面或网页版 IDE，而是一条专注于消息、进度、审批和结果的轻量远程通道。

## 核心能力

- 原生 Android 与响应式 Web 客户端
- 扫码配对；Android Keystore 加密保存多 Host 凭证
- 动态选择项目、Provider、模型、effort 和已有 Agent 会话
- 实时显示消息、计划、工具、命令、文件变化、审批和图片
- 支持追加指令、停止任务、断线重连、局域网直连和公网 Relay

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

## 验证

```powershell
cargo xtask test
cargo xtask android
cargo xtask provider-smoke
```

- Rust/Web 与 mock 链路：43 项测试通过
- Android 协议：3/3 通过
- PJV110、Android 16 真机：配对、Codex 消息、冷启动恢复、断线重连通过
- `provider-smoke` 只有 `PASS` 才表示本机真实 Provider 可用；`SKIP` 不算验证

## 安全边界

- 公网部署应使用 HTTPS/WSS；`--dev-insecure` 只适合受信任的开发网络。
- Relay 不持久化项目、消息或附件，但它是受信任传输端点；v0.1 不宣称应用层端到端加密。
- v0.1 不包含远程终端、文件浏览器、Git/代码编辑器、离线队列、推送通知或云端 Agent。

## 文档

[Host 设置](docs/setup.md) · [Android](docs/android.md) · [公网 Relay](docs/public-relay.md) · [架构](docs/architecture.md) · [Provider 兼容性](docs/provider-compatibility.md)
