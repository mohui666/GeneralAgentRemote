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
