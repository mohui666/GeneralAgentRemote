# Android app

The Android client is a native Kotlin and Jetpack Compose application. Codex, Grok, project files, and the Host database remain on the computer. The phone speaks the same binary CBOR application protocol as the web client over either a Direct WebSocket or the public Relay.

## Build and install

Open `android/` in Android Studio, or point `android/local.properties` at an installed Android SDK and build from the repository root:

```powershell
cargo xtask android
adb install -r dist\android\agent-remote-debug.apk
```

`cargo xtask android` runs the Android protocol unit tests, builds the debug APK, and copies it to `dist/android/agent-remote-debug.apk`. `cargo xtask android --release` creates an unsigned release APK for signing by the distributor.

The checked-in project uses Gradle Wrapper 9.4.1, Android Gradle Plugin 9.2.0, compile/target SDK 37, and minimum Android 8.0 (API 26).

## AI/CLI device test driver

`cargo xtask android-device` is a computer-side development driver for an explicitly connected Android device. It is not part of the Host/Relay protocol and does not expose a terminal or unrestricted device control in the product.

Start with:

```powershell
cargo xtask android-device doctor --json
cargo xtask android-device prepare --port 7437 --json
```

When multiple devices are connected, add `--serial <id>` to every command. `prepare` runs the normal debug build and Android unit tests, installs the APK with state retained, configures `adb reverse`, and launches the app. Add `--fresh` to uninstall first and deliberately clear app credentials/cache.

The stable command surface is:

```text
cargo xtask android-device doctor [--serial <id>] [--json]
cargo xtask android-device prepare [--serial <id>] [--fresh] [--port 7437] [--json]
cargo xtask android-device inspect [--serial <id>] [--output <dir>] [--json]
cargo xtask android-device ui dump [--serial <id>] [--json]
cargo xtask android-device ui click --id <stable-id> [--serial <id>] [--json]
cargo xtask android-device ui text --id <stable-id> --value <text> [--serial <id>] [--json]
cargo xtask android-device ui wait --id <stable-id> --state <visible|gone|enabled> [--timeout <sec>] [--serial <id>] [--json]
cargo xtask android-device scenario --name <project-tree|send|reconnect|layout|send-latency> [--mode <mock|real>] [--serial <id>] [--json]
cargo xtask android-device logs [--serial <id>] [--duration <sec>] [--output <dir>] [--json]
cargo xtask android-device capture [--serial <id>] [--output <dir>] [--json]
```

`ui click`, `ui text`, and `ui wait` match only exact Compose test tags exposed as UIAutomator resource IDs (or the same accessibility description); they never locate controls by translated title text. Supported application IDs are `gar.drawer.open`, `gar.project.<projectId>`, `gar.project.<projectId>.toggle`, `gar.conversation.<conversationId>`, `gar.composer.input`, `gar.composer.send`, `gar.send.retry`, and `gar.connection.status`.

`inspect`, `logs`, and `capture` default to `dist/android-device/`; `--output` is always a directory. Named scenarios write fixed, replaceable evidence under `dist/android-device/scenarios/<name>/`. The layout scenario captures portrait and landscape and restores the device's original rotation settings. The reconnect scenario force-stops and relaunches only this app. Each send scenario opens the drawer when necessary, starts a new lazy conversation, submits one explicit test message, and requires the correlated `click → local_pending → websocket_write → host_received → provider_received → first_provider_event` trace. `mock` expects the already-running Host to use a test Provider setup; `real` expects a deliberately configured real Provider. The driver never silently substitutes one for the other.

With `--json`, stdout contains one JSON result object. A successful command exits `0`; argument, adb, locator, scenario, or artifact failures emit `{ "ok": false, ... }` and exit `1`. Send trace output contains only IDs, stages, and elapsed milliseconds—not prompt text, tokens, credentials, or attachment contents. Set the `ADB` environment variable to select a particular adb executable; otherwise the driver uses `adb` from `PATH`.

## USB test loop

USB port reversal lets a connected phone reach a Host bound only to the computer's loopback interface. Enable USB debugging, accept the computer's RSA prompt, then run:

```powershell
adb devices -l
adb reverse tcp:7437 tcp:7437

dist\bin\agent-remote-host.exe project add C:\path\to\project --provider codex
dist\bin\agent-remote-host.exe pair --base-url http://127.0.0.1:7437
dist\bin\agent-remote-host.exe serve --listen 127.0.0.1:7437 --web-root dist\web
```

On the phone, open Agent Remote and either scan the terminal QR code or paste the complete pairing URL. The pair token expires after ten minutes and can be used once. After pairing, the app clears it and stores only the issued device credential encrypted with Android Keystore.

`adb reverse` is a USB-only development bridge. For normal use on the same trusted LAN, generate a pairing URL with the computer's LAN address and listen explicitly:

```powershell
dist\bin\agent-remote-host.exe pair --base-url http://192.168.1.25:7437
dist\bin\agent-remote-host.exe serve --listen 0.0.0.0:7437 --dev-insecure --web-root dist\web
```

For use outside the LAN, configure the HTTPS/WSS Relay described in [public-relay.md](public-relay.md), then generate the Android-compatible link with:

```powershell
dist\bin\agent-remote-host.exe pair --relay --base-url https://relay.example.com
```

## Expected phone behavior

- A successful first pair immediately opens the Host snapshot and saves that Host in the connection screen.
- Killing and reopening the app authenticates with the saved credential, restores the cached Host/Provider/project/conversation/settings/draft state, and does not reuse the pair token.
- A dropped connection preserves the visible cached conversation, reports offline status, then reconnects with bounded backoff and fetches a fresh snapshot. The drawer can stop automatic retries or request an immediate retry.
- Provider and project selection is scoped to the active Host. Switching Provider refreshes only its authorized projects and syncs the selected project's remote sessions.
- A new conversation is created on its first send. One unacknowledged send is retained across a reconnect and replayed with the same command ID.
- Revoking the phone from `agent-remote-host device revoke <device-id>` makes the next authentication fail and removes the invalid local credential.
- The QR scanner is opened only when the user taps **扫码** and does not require an app-level camera permission.
