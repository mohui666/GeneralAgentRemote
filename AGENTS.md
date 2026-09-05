# GeneralAgentRemote — Repository Instructions

## 本机开发环境（2026-09-05 起）

- 主工作区为 `C:\Users\mohui666\Documents\ChatGPT\GeneralAgentRemote`，使用 Windows PowerShell、Windows Rust 和 Android SDK；日常开发、构建和 Host 启动不再依赖 WSL。
- 原 WSL 目录与 Windows 旧副本保留；不要在旧目录继续修改本项目。
- Host 由 Windows 启动文件夹中的 `GeneralAgentRemote Host.vbs` 在用户登录时启动；脚本为 `%LOCALAPPDATA%\GeneralAgentRemote\start-host.ps1`，数据为同目录下的 `data`，凭据仅保存在该运行目录。
- 本机监听 `127.0.0.1:7437`，连接既有公网 Relay。WSL 的 `agent-remote-host.service` 已停止并禁用。
- Android 使用 `cargo xtask android` 或 Windows 的 `android\gradlew.bat`；下方 WSL 互操作教程仅作历史参考，不是本机默认流程。

## Astra 协作约定

- 以用户当前目标和本轮明确约束为准。任务要求实施时，完成实际修改与必要验证，不停在计划、建议或“是否继续”。普通实现选择自行决定；只有缺失信息会实质改变结果或操作超出授权时才询问，并先完成不依赖答案的工作。
- 用户指令优先于本地 skills 的工作流建议。只读取与当前任务直接相关的文件和技能；不因关键词命中就串联整套技能、生成流程工件或增加审批。
- 保留既有业务规则、数据所有权、用户改动和明确的工具限制。只改当前目标需要的内容，不顺手重构、升级依赖、搬目录或扩展产品范围。

## 拒绝过度防御性编程

- 直接使用已有输入、文件、依赖和运行环境，不重复做环境、权限、目录或文件存在性预检查。
- 不为假想故障添加重复参数验证、大量极端输入分支、宽泛 `try/catch`、默认值兜底、静默失败或伪造成功。契约不满足时暴露具体错误。
- 不主动新增重试、退避、熔断、降级、备用实现、兼容层、自动备份、回滚、迁移或恢复机制。
- 不主动添加 SHA、MD5、签名、文件哈希、完整性校验、CI/CD、发布门禁、安全扫描、许可证审计、复杂日志、监控、遥测或诊断框架。
- 不为未来需求预建插件系统、通用框架或抽象层，不为小改动铺设大量单元测试、回归测试、故障注入或性能基准。
- 只在缺少检查会立即阻止核心功能、造成明显数据损坏或掩盖真实错误时保留最小必要检查。现有鉴权、真实业务校验和数据保护功能继续遵守其契约；本规则不授权删除这些功能。
- 例外必须来自用户明确要求，或与本次改动直接相关的既有产品契约。旧文档中泛化的“每次全量检查”“必须先审批”“自动完善”不构成额外任务。

## 验证与交付

- 选择能证明本次行为的最小验证：文档或提示词改动检查内容和 diff；代码改动运行相关构建、现有定向测试或核心流程冒烟。低影响、可逆改动不新增仅复述实现的测试。
- 必要检查通过即交付；只有新改动、失败或具体未解决疑点才扩大或重复验证。不要为了收尾重跑无关全量测试、打包、实机流程或基准。
- 错误如实报告。区分实际运行通过、静态检查、未运行与真实环境验证；历史测试数量不能当作本次证据。
- 仅在任务需要时使用子代理；不强制委派、切换模型或修改推理档位，遵守当前会话设置与工具权限。
- 按当前授权和项目约定执行 Git 操作，只提交本任务文件；不要为清空工作区而夹带其他改动，不强推或丢弃用户内容。没有远端时报告，不擅自创建远端。
- 用简明中文交代实际修改、验证结果和已知问题。只有需求、接口或已验证事实改变时同步相关文档，不追加与交付无关的报告。

## 1. Product definition

GeneralAgentRemote is a **remote messaging client for coding agents**. Its primary flow is:

`connect host → choose agent → choose project → choose or create conversation → send text/images/files → observe replies and execution activity`

The product is deliberately smaller than an IDE. Preserve a chat-first experience.

### In scope

- Connect to a local or remote Host through the existing Relay architecture.
- Select a Host, Agent, authorized project, and conversation.
- Send and receive text, images, and supported files.
- Resume persisted remote conversations.
- Show streamed replies, approvals, tool activity, command results, file-change activity, completion, cancellation, and errors.
- Support Codex and Grok through provider adapters.

### Out of scope

Do not turn the product into a full IDE. Do not add a general-purpose:

- code editor;
- terminal emulator;
- repository/file-tree workbench;
- debugger;
- Git GUI;
- plugin marketplace;
- unrestricted remote filesystem browser.

## 2. Architecture invariants

Keep the existing architecture unless the user explicitly requests a migration:

- **Rust Host** owns local Agent processes, project authorization, filesystem access, credentials, and provider adapters.
- **Rust Relay** transports authenticated messages and events. It must not become the source of truth for local Agent state.
- **Rust/WASM client** provides the responsive chat UI and local cache.
- **Codex** integrates through the official `codex app-server` protocol.
- **Grok** integrates through the repository's native ACP/stdio adapter.
- The Host initiates the outbound Relay connection.
- The client must not connect directly to or expose a local Agent port.
- Never implement provider integration by scraping terminal text or screenshots.

Keep provider-specific protocol details inside adapters. Shared UI and domain code must use typed, provider-neutral models.

## 3. Identity and state rules

Never identify projects or conversations by title alone.

Use stable scoped identities equivalent to:

- `ConnectionScope = connectionId + hostId`
- `AgentScope = ConnectionScope + agentId`
- `ProjectScope = AgentScope + remoteProjectId`, or a Host-normalized authorized project identifier
- `ConversationScope = ProjectScope + providerConversationId`

Additional rules:

- A project belongs to one Host and one Agent scope; there is no unscoped global project list.
- A conversation belongs to exactly one project scope.
- Normalize paths on the Host, not independently in each client.
- Use stable `clientMessageId`, `remoteMessageId`, and provider event identifiers for idempotency.
- Preserve mappings between optimistic local messages and provider-confirmed messages.
- Store schema versions and migrate persisted client data safely.
- Never silently merge data from different Hosts, Agents, projects, or conversations.

## 4. Provider capability contract

The UI must be capability-driven. Do not pretend all providers support the same operations.

The shared adapter contract should expose typed equivalents of:

- project discovery or authorized-project listing;
- conversation listing, reading, resuming, and pagination;
- conversation rename, pin/archive/delete when supported;
- model discovery;
- reasoning-effort discovery;
- permission-profile discovery;
- attachment types and size limits;
- incremental synchronization or full-history fallback;
- streamed messages and activity events;
- approval requests and responses.

Unsupported capabilities must be hidden or disabled with a clear explanation. Do not add fake data or hard-code model names, effort levels, projects, or conversations.

Generate or consume provider schemas from the installed provider version when the repository already supports schema generation. Do not spread untyped JSON values through business logic.

## 5. Required UX contract

### Compact sidebar

Desktop defaults:

- approximately `190px` wide;
- no more than approximately `208px`;
- collapsed width approximately `46px`;
- persist width and collapsed state.

The sidebar contains, in order:

1. compact connection status;
2. Host/Agent switcher;
3. projects as stable scoped, collapsed-by-default tree nodes;
4. one-click **New conversation** action;
5. project and conversation search that preserves the tree hierarchy;
6. conversations inside their owning project's expanded region, newest first;
7. settings and collapse control.

The project tree must default to only the current project expanded and allow explicit per-project expansion. Persist expansion by Host + Provider + stable project ID, and automatically expand the owning project when a conversation is selected or matched by search. Do not render every project's conversations while collapsed, trigger synchronization merely by expanding/collapsing, or leave all projects permanently expanded. On mobile, use a dismissible drawer and close it after project or conversation selection.

### Project selection

- Refresh projects when Host or Agent changes.
- Restore the last valid project for that Host/Agent scope.
- If it no longer exists, fall back to the most recently active available project.
- Show project name, sanitized/short path, recent activity, availability/sync state, and conversation count when known.
- Never perform an unrestricted full-disk scan to manufacture project entries.

### New conversation

Creating a conversation is one action. Reuse the selected Host, Agent, project, project defaults, and last valid permission/model/effort settings.

Prefer lazy remote creation on first send when the provider permits it, so an accidental click does not create empty remote conversations. Ensure concurrent sends cannot create duplicate remote conversations. Preserve the user's draft if creation fails.

### Composer layout

Left side:

- attachment action;
- current permission mode;
- only other essential tool controls.

Right side:

- compact `model · effort` control;
- send/cancel action.

Do not place large project, model, and effort forms in the empty-conversation screen.

### Model and effort

- Discover models and supported effort values dynamically from the active provider.
- Keep the control collapsed by default, for example `Model · High ›`.
- Use labeled discrete steps or a segmented control; do not expose an unlabeled arbitrary continuous value.
- Hide effort when the selected model does not support it.
- Distinguish current-conversation overrides from project defaults.
- If a provider cannot switch model or effort mid-conversation, state when the change will take effect.

### Permissions

- Show the effective permission mode clearly on the left side of the composer.
- Treat permissions as conversation settings with a project default, unless the provider contract requires a narrower scope.
- Never elevate permissions as a side effect of navigation, reconnection, or state restoration.
- Require explicit confirmation for materially higher-risk access.
- Send the actual setting to the provider adapter; a UI-only change is invalid.

### Attachments

- Support click, drag-and-drop, pasted images/screenshots, multiple files, progress, retry, removal, and previews.
- A type/category menu may be offered, but do not force an unnecessary extra selection before the native picker.
- Allowed types, count, and size limits come from Host/Relay/provider capabilities.
- Do not reveal the Host's absolute local paths to remote clients.
- A failed upload must not erase the text draft.
- Do not send the message until required uploads are complete.

### Activity display

Present execution activity as compact, collapsible summaries, such as:

- read 6 files;
- ran 4 commands;
- changed 3 files;
- tests passed;
- waiting for approval.

Show provider-supplied plan/reasoning summaries when available, but do not expose hidden chain-of-thought or default to raw protocol JSON. Detailed command output and raw events belong in an expanded details view or explicit debug mode.

## 6. Automatic connection and restoration

On application startup:

- render the UI immediately;
- restore the last successful connection profile;
- connect in the background;
- restore the last Host, Agent, project, conversation, permissions, model, effort, sidebar state, and cached draft where valid;
- synchronize missing remote state after connection.

Connection behavior must include:

- explicit `connecting`, `connected`, `syncing`, `reconnecting`, `offline`, and `failed` states;
- bounded exponential backoff with jitter;
- immediate retry and stop-retrying actions;
- automatic recovery after network return;
- no duplicate WebSockets, subscriptions, timers, or provider streams;
- reauthentication instead of infinite retry when credentials are invalid;
- suppression of automatic reconnect after an intentional manual disconnect during the same app session.

Cached conversations remain readable while offline.

## 7. Remote project and conversation synchronization

The remote provider state is authoritative; the client cache is an offline/performance layer.

When a project becomes active:

1. show cached conversations immediately;
2. fetch the provider's current conversation list for that project;
3. merge by stable provider IDs;
4. fetch missing or stale history as needed;
5. subscribe only to active/required streams;
6. persist the new cursor/version/watermark.

Synchronization must support, where available:

- cursor-based pagination;
- loading older history on upward scroll;
- incremental updates after reconnect;
- deduplication of repeated events;
- deterministic ordering of out-of-order events;
- continuation of in-progress streamed messages;
- retry without deleting valid cache;
- a visible manual resync action.

If a provider lacks incremental APIs, implement a controlled full-read fallback with stable-ID deduplication. Never create duplicate conversations or messages merely because the app reconnected.

## 8. Automatic conversation naming

Title precedence is:

1. user-assigned title;
2. valid provider title;
3. generated title;
4. `New conversation` fallback.

After the first meaningful user message:

- create a fast provisional title locally from the task intent;
- remove code-block, path, and boilerplate noise;
- keep Chinese titles roughly 8–20 characters and English titles roughly 3–8 words;
- optionally replace it asynchronously with a better provider-generated title when supported;
- never insert a visible title-generation message into the conversation;
- never interrupt the active Agent turn;
- stop all automatic overwrites after a manual rename;
- synchronize rename to the provider when supported, otherwise store a scoped local override;
- prevent multi-device title ping-pong with version/timestamp or explicit source precedence.

## 9. Security and privacy

- Keep provider credentials and local filesystem authority on the Host.
- Authenticate and authorize every Relay operation.
- Validate project membership on the Host for every conversation, file, and command request.
- Treat all client-supplied IDs and paths as untrusted.
- Use bounded message and upload sizes.
- Do not log tokens, cookies, credentials, private prompts, or sensitive file contents by default.
- Sanitize user-visible errors and paths without hiding actionable context.
- Destructive remote actions require explicit confirmation and provider support.

## 10. Engineering workflow

- Trace only the affected Host, Relay, client, persistence, or adapter path and implement the requested behavior end to end.
- Preserve the scoped identity, provider authority, draft preservation, and capability contracts above. Reconnection and synchronization are product behavior; they do not justify unrelated defensive infrastructure.
- Persisted data changes require the existing migration mechanism only when the schema actually changes. Avoid parallel compatibility paths.
- Reuse the repository's types, conventions, and dependencies. Update stable project guidance when the user explicitly asks or a confirmed project contract changes.

## 11. Verification

Read actual commands from the affected manifest or `cargo xtask` command. Run the smallest relevant check/test or one core-flow smoke test. Documentation changes need content and diff review only.

For a network, persistence, or send-path change, exercise that affected transition; do not repeat every startup, reconnect, attachment, mobile, and provider scenario. Real-provider results require a real provider run. UI/device claims require the corresponding rendered app or device evidence.

The Android procedure below is an optional recipe for Android work, not a checklist for every task.

## 12. Definition of done

The requested path is implemented, directly relevant validation is complete, and failures or unavailable real-provider/device checks are reported accurately. Report behavior, principal files, validation, and remaining limitations without expanding the task.

## 13. Android 16 emulator test tutorial

Use the existing local Android 16 AVD for repeatable device tests when a physical phone is unavailable. On this workstation the verified AVD is `Pixel_9_API_36_1` (`android-36.1`, Google Play x86_64, Pixel 9) and its fixed serial is `emulator-5554`.

Start it from Windows PowerShell and wait for Android to finish booting:

```powershell
$Sdk = Join-Path $env:LOCALAPPDATA "Android\Sdk"
$Emulator = Join-Path $Sdk "emulator\emulator.exe"
$Adb = Join-Path $Sdk "platform-tools\adb.exe"

& $Emulator -list-avds
Start-Process $Emulator -ArgumentList @(
  '-avd', 'Pixel_9_API_36_1',
  '-port', '5554',
  '-no-window', '-no-audio', '-no-boot-anim', '-no-snapshot-save'
)
& $Adb -s emulator-5554 wait-for-device
do {
  Start-Sleep -Seconds 2
  $Booted = (& $Adb -s emulator-5554 shell getprop sys.boot_completed).Trim()
} until ($Booted -eq '1')
& $Adb devices -l
```

The emulator belongs to the Windows ADB server and may not appear in WSL's Linux `adb`. In that case, create a temporary WSL wrapper that calls the Windows SDK tool and translates existing WSL file paths before `adb install`:

```bash
cat > /tmp/gar-adb-windows <<'PY'
#!/usr/bin/env python3
import os
import subprocess
import sys

ADB_EXE = subprocess.check_output(
    [
        "powershell.exe",
        "-NoProfile",
        "-Command",
        '[Console]::Write((Join-Path $env:LOCALAPPDATA "Android\\Sdk\\platform-tools\\adb.exe"))',
    ],
    text=True,
).strip()

def translate(value: str) -> str:
    if value.startswith("/") and os.path.exists(value):
        return subprocess.check_output(["wslpath", "-w", value], text=True).strip()
    return value

def cmd_escape(value: str) -> str:
    escaped = []
    for char in value:
        if char == "&":
            escaped.append("\\^")
        elif char in "^|<>() ":
            escaped.append("^")
        escaped.append(char)
    return "".join(escaped)

command_line = " ".join(
    cmd_escape(value) for value in [ADB_EXE, *(translate(arg) for arg in sys.argv[1:])]
)
os.chdir("/mnt/c/Windows")
os.execvp("cmd.exe", ["cmd.exe", "/d", "/s", "/c", command_line])
PY
chmod +x /tmp/gar-adb-windows
export ADB=/tmp/gar-adb-windows
$ADB devices -l
```

Run the repository driver from WSL. Use the same port for `prepare`, `adb reverse`, the pairing URL, and the Host listener. Keep the Host foreground process alive in a separate terminal; a short-lived command runner may clean up background children.

```bash
export DEVICE_SERIAL=emulator-5554
export HOST_PORT=7437
export HOST_DATA=/tmp/gar-emulator-host
mkdir -p "$HOST_DATA"

cargo xtask android-device --serial "$DEVICE_SERIAL" doctor --json
cargo xtask android-device --serial "$DEVICE_SERIAL" prepare --port "$HOST_PORT" --json
cargo xtask android-device --serial "$DEVICE_SERIAL" inspect --output dist/android-device/inspect --json

# In a separate terminal, authorize a project, create a one-use pair link, and keep serve running.
cargo run -p agent-remote-host -- --data-dir "$HOST_DATA" project add "$PWD" --provider codex
cargo run -p agent-remote-host -- --data-dir "$HOST_DATA" pair --base-url "http://127.0.0.1:$HOST_PORT"
cargo run -p agent-remote-host -- --data-dir "$HOST_DATA" serve --listen "127.0.0.1:$HOST_PORT" --web-root dist/web
```

Paste the complete pair link into the emulator app and tap **连接并配对**. Require `gar.connection.status` to show online before running real scenarios:

```bash
cargo xtask android-device --serial "$DEVICE_SERIAL" scenario --name send --mode real --json
cargo xtask android-device --serial "$DEVICE_SERIAL" scenario --name project-tree --mode real --json
cargo xtask android-device --serial "$DEVICE_SERIAL" scenario --name reconnect --mode real --json
cargo xtask android-device --serial "$DEVICE_SERIAL" scenario --name layout --mode real --json
cargo xtask android-device --serial "$DEVICE_SERIAL" capture --output dist/android-device/final --json
```

The real send scenario must report the correlated stages `click`, `local_pending`, `websocket_write`, `host_received`, `provider_received`, and `first_provider_event` for one command/message identity. Treat generated PNGs as evidence only after visually checking that the app is foreground, content is rendered, and both portrait and landscape are usable; screen bounds alone do not prove a valid render. Keep mock and real scenario results separate.

## 14. Windows and WSL interoperability on this workstation

When a command, login, deployment credential, device, or Provider appears unavailable in WSL, inspect the existing Windows environment before treating it as missing. Windows executables may be called directly through `powershell.exe`, `cmd.exe`, or their absolute `/mnt/c/Windows/...` paths. Use the platform that owns the credential and project path for a real Provider run; for example, a Windows Agent should be tested with a Windows Host build and a Windows project path instead of receiving a WSL path.

Preserve Windows originals when bringing required state into WSL. Copy credentials only when the task needs them, keep them outside the repository, restrict credential files to mode `600` and helper commands to mode `700`, and never print secret values. Prefer a small explicit environment file or tool-specific home setting over copying browser profiles or whole user directories. Verify the migrated command with a read-only operation before using it for a deployment or Provider turn.

The private ServerHub command channel is available in WSL as:

```bash
serverhub-remote "hostname"
```

Its local helper is `~/.local/bin/serverhub-remote` and its credential file is `~/.config/agent-remote/serverhub-ntfy.env`. The helper deliberately invokes Windows `curl.exe`, because that is the verified authenticated route on this workstation. Neither file belongs in Git.
