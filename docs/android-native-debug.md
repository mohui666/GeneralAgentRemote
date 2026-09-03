# Android AI-native device testing

The Android debug build exposes a small, deterministic command surface for Codex or another coding agent to test the real app on a connected phone. It uses an explicit ADB broadcast instead of screen-coordinate automation for stateful actions, while screenshots and `uiautomator` remain available for visual assertions.

The receiver and real command bridge exist only in `src/debug`, are omitted from release APKs, and require Android's `DUMP` permission. Release builds compile against a no-op hook only. Normal third-party apps cannot use the command surface; `adb shell` can.

## Fast path on Windows

From `android/`:

```powershell
.\ai-native.ps1 install
.\ai-native.ps1 launch
.\ai-native.ps1 status
.\ai-native.ps1 projects
.\ai-native.ps1 conversations
```

A practical end-to-end send test is:

```powershell
.\ai-native.ps1 select-project -Id <project-uuid> -Provider codex
.\ai-native.ps1 new
.\ai-native.ps1 send -Text "Reply with exactly: DEVICE_OK"
.\ai-native.ps1 status
.\ai-native.ps1 screenshot -Out .\device-result.png
.\ai-native.ps1 ui-dump -Out .\device-ui.xml
.\ai-native.ps1 logs
```

Use `bash ./ai-native.sh ...` on macOS/Linux. The shell script may not retain an executable bit when downloaded as an archive, so invoking it through `bash` is portable.

Stateful wrapper commands print exactly one fresh JSON result. They delete the previous result, invoke the debug receiver, and read the new result through `adb shell run-as`. A rejected command, malformed/stale result, or command-name mismatch produces a non-zero script exit.

## Commands

| Command | Purpose |
|---|---|
| `status` | Connection, selection, draft, pending-command, approval, and active timeline summary |
| `dump` | State plus project and conversation inventories |
| `projects` | Stable project IDs, short paths, providers, and conversation counts |
| `conversations` | Conversations for the selected project, or `-ProjectId <uuid>` |
| `select-project` | Select and synchronize a project |
| `select-conversation` | Select a conversation and automatically align its Provider/project scope |
| `new` / `list` | Enter new-conversation mode or return to the conversation list |
| `draft` | Replace the current composer text without tapping the screen |
| `send` / `steer` | Send a normal message or steer a running Agent turn; success requires a newly queued command |
| `interrupt` | Stop the selected running turn; success requires a newly queued command |
| `retry` / `disconnect` | Exercise connection recovery and intentional disconnect behavior |
| `pair` / `connect-host` | Pair with a URL or reconnect a saved Host by UUID |
| `smoke` | Launch, poll until the live bridge is ready, then collect status, projects, and conversations |
| `screenshot` / `ui-dump` / `logs` | Capture visual/accessibility evidence and structured command results |

Run `.\ai-native.ps1 help` for the concise CLI form.

## Raw ADB protocol

The wrapper calls this debug-only component:

```powershell
adb shell am broadcast `
  -a dev.agentremote.messenger.DEBUG_COMMAND `
  -n dev.agentremote.messenger/dev.agentremote.messenger.debug.NativeDebugCommandReceiver `
  --es command status
```

The receiver writes the same JSON to ordered-broadcast result data, logcat tag `AgentRemoteNative`, and the debug app's private `files/agent-remote-native-result.json`. To read the unescaped JSON manually:

```powershell
adb shell run-as dev.agentremote.messenger `
  cat files/agent-remote-native-result.json
```

Examples:

```powershell
adb shell am broadcast -a dev.agentremote.messenger.DEBUG_COMMAND `
  -n dev.agentremote.messenger/dev.agentremote.messenger.debug.NativeDebugCommandReceiver `
  --es command select_project --es id <project-uuid> --es provider codex

adb shell am broadcast -a dev.agentremote.messenger.DEBUG_COMMAND `
  -n dev.agentremote.messenger/dev.agentremote.messenger.debug.NativeDebugCommandReceiver `
  --es command send --es text "DEVICE_OK"
```

If a manual raw command returns `app_not_ready`, start `MainActivity` and poll `status` again. The wrapper's `smoke` command performs this bounded poll automatically. The bridge intentionally controls the live `RemoteViewModel`; it does not create a second fake test state.

## Recommended AI test loop

1. Build and install the debug APK.
2. Run `launch`, then poll `status` until the bridge is ready, or use `smoke` for the automatic bounded poll.
3. Read IDs from `projects` and `conversations`; never guess UUIDs or select by title.
4. Execute one state transition at a time and read `status` after it.
5. For sends, retain the JSON returned by `send`, then poll `status` until `pending_command_count` returns to zero and the selected timeline advances.
6. Capture a screenshot and UI XML for layout regressions.
7. Read `AgentRemoteNative` logcat output when a command fails.

The native commands complement, rather than replace, Compose unit/instrumentation tests. They are intended for real-device integration checks across the phone, WebSocket, Relay/Direct transport, Host, and Provider adapter.
