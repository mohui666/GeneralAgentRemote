# Android development instructions

These rules apply to everything under `android/` in addition to the repository-root `AGENTS.md`.

## Real-device verification

For Android behavior changes, prefer the debug-build AI-native command surface over coordinate-only UI automation:

```powershell
.\ai-native.ps1 install
.\ai-native.ps1 launch
.\ai-native.ps1 smoke
```

Then use `status`, `projects`, and `conversations` to discover the live scoped IDs. Never guess a project or conversation by display title. Use `select-project`, `select-conversation`, `new`, `draft`, `send`, `steer`, `interrupt`, and `retry` to exercise the real `RemoteViewModel`, WebSocket transport, Host, and Provider path.

For visual evidence, use:

```powershell
.\ai-native.ps1 screenshot -Out .\device-screen.png
.\ai-native.ps1 ui-dump -Out .\device-ui.xml
.\ai-native.ps1 logs
```

When validating a send, record `status` before and after the action. Confirm that a new pending command is accepted, the pending count returns to zero, and the selected timeline advances. Preserve screenshots, UI XML, and relevant `AgentRemoteNative` log lines for failures.

Use `adb reverse tcp:7437 tcp:7437` when a USB-connected device must reach a Host bound to the computer's loopback interface.

## Debug command safety

- Keep `NativeDebugCommandReceiver` and the real `NativeDebugBridge` implementation under `src/debug`; neither may appear in release APKs.
- The `src/release` bridge must remain a no-op build hook so shared Activity code cannot expose commands in production.
- Keep the exported receiver protected by `android.permission.DUMP`.
- The command bridge must act on the live ViewModel and return structured JSON; do not create a parallel fake app state.
- Commands that mutate state must verify that the requested transition or queue operation actually happened, so automation cannot report false success.
- Do not add arbitrary shell execution, unrestricted file reads, credentials, device tokens, prompt contents, or attachment bytes to debug output.

## Codex conversation storage

Do not create an Android-side or project-local Codex session directory. Codex conversation listing, creation, reading, and resumption must continue through the official `codex app-server`, which owns its normal provider session store and inherits the Host's Codex environment. Agent Remote may persist scoped metadata, command idempotency state, and an offline client cache only.

## Performance checks

Android chat changes must be checked for:

- no blocking disk or JSON work on every composer keystroke;
- no repeated animated scroll on every streaming token/event;
- no whole-project or whole-timeline recomputation in tight render/event loops when an incremental update is available;
- no send button lock caused by unrelated background synchronization;
- no successful UI acknowledgement before a command is actually queued on a usable WebSocket;
- no duplicate send after reconnect; replay must keep the original command ID.
