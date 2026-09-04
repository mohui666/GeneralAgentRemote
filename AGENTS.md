# GeneralAgentRemote — Repository Instructions

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

Before editing:

1. inspect the repository layout, manifests, generated schemas, existing adapters, state stores, and relevant tests;
2. trace the current end-to-end data flow before proposing a replacement;
3. use existing naming, error, serialization, and state-management conventions;
4. identify whether a change requires Host, Relay, client, persistence, and provider-adapter updates.

During implementation:

- implement a real end-to-end path, not a visual mock;
- do not rewrite the entire project for a localized feature;
- keep protocol changes typed, versioned, and backward-compatible where practical;
- add migrations for persisted schema changes;
- make network and send operations idempotent;
- preserve drafts and cached data on recoverable failures;
- add focused tests for state transitions, deduplication, reconnection, and provider capability differences;
- do not add production dependencies unless they solve a concrete need and fit the current stack;
- do not stop after producing a plan when the task asks for implementation.

When the user corrects the same repository assumption more than once, update this file with the durable rule rather than relying on conversation memory.

## 11. Verification

Discover and use the repository's actual commands from its manifests and scripts. Do not invent script names.

For affected Rust workspaces, normally run the applicable equivalents of:

- formatting check;
- Clippy/static analysis with warnings treated as errors where the project supports it;
- relevant unit/integration tests;
- workspace build/check.

For the client, normally run the repository-defined equivalents of:

- formatting/lint;
- type checking;
- unit/component tests;
- production build.

Also verify the relevant manual flows:

- cold-start automatic connection;
- offline startup and later recovery;
- switching Host/Agent without state leakage;
- project restoration and invalid-project fallback;
- remote conversation discovery and history resume;
- reconnection without duplicate messages/subscriptions;
- one-click new conversation without duplicate remote creation;
- automatic title generation and manual-rename lock;
- attachment success, rejection, retry, and draft preservation;
- compact desktop sidebar and mobile drawer;
- permission/model/effort capability handling.

## 12. Definition of done

A task is complete only when:

- the requested behavior works through the real Host/Relay/client/provider path;
- no fake models, projects, conversations, or activity events were introduced;
- persistence and reconnection behavior are defined;
- relevant tests/checks pass, or exact pre-existing blockers are reported;
- the diff is reviewed for duplicate listeners, state leakage, security regressions, and mobile layout regressions;
- the final report lists implemented behavior, principal files changed, protocol/data-model changes, commands run, results, and remaining provider limitations.

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
