# Host setup

## Build prerequisites

Install Rust stable, then add the WebAssembly standard library:

```powershell
rustup target add wasm32-unknown-unknown --toolchain stable
cargo xtask build
```

The first Web build installs the pinned Trunk 0.21.14 binary under `target/xtask-tools`. No Node runtime is used. Release files are written to `dist/bin` and `dist/web`.

Codex and Grok are optional independently. The Host discovers executables from `PATH` and does not install or update either Provider.

```powershell
codex --version
grok --version
```

Authenticate with each Provider's own supported login command before its real smoke test.

## Authorize projects

```powershell
dist\bin\agent-remote-host.exe project add C:\work\my-project --name "My project" --provider codex --provider grok
dist\bin\agent-remote-host.exe project list
dist\bin\agent-remote-host.exe project remove <project-id>
```

Project paths are canonicalized when added. If a directory is moved or removed, it is shown invalid and is not silently replaced.

## Direct localhost

```powershell
dist\bin\agent-remote-host.exe pair
dist\bin\agent-remote-host.exe serve --listen 127.0.0.1:7437 --web-root dist\web
```

Open the printed pairing URL in a browser. The URL fragment carries the one-use pair token, so it is not sent in the ordinary HTTP request target. The browser clears the fragment after pairing and stores the issued device credential for that origin.

## LAN

LAN listening is explicit:

```powershell
dist\bin\agent-remote-host.exe pair --base-url http://192.168.1.25:7437
dist\bin\agent-remote-host.exe serve --listen 0.0.0.0:7437 --dev-insecure --web-root dist\web
```

LAN clients still pair and authenticate. `--dev-insecure` makes the plaintext boundary visible. For an untrusted LAN, place the Host behind a trusted HTTPS reverse proxy and use HTTPS/WSS.

## Devices

```powershell
dist\bin\agent-remote-host.exe device list
dist\bin\agent-remote-host.exe device revoke <device-id>
```

Revocation applies to the next authentication/reconnection. Each browser has a separate random device token.

## Development

```powershell
cargo xtask dev-host
cargo xtask dev-relay -- --listen 127.0.0.1:8443
```

Provider state shown by the UI distinguishes not installed, not authenticated, starting, ready, crashed, protocol incompatible, and offline. A missing capability disables the corresponding control instead of queuing or faking it.
