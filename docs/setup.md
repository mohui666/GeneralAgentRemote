# Host setup

## Build prerequisites

Install Rust stable, then add the WebAssembly standard library:

```powershell
rustup target add wasm32-unknown-unknown --toolchain stable
cargo xtask build
```

The first Web build installs the pinned Trunk 0.21.14 binary under `target/xtask-tools`. No Node runtime is used. Release files are written to `dist/bin` and `dist/web`.

Providers are optional independently. Install the Provider CLI globally on the Host computer, make its executable available through `PATH`, and complete that Provider's own login flow before running a real smoke test. The Host discovers and starts these commands; it does not download, install, update, or authenticate a Provider.

Provider credentials remain in the Provider-owned store on the Host. GeneralAgentRemote does not copy them into SQLite or send them to the Relay, browser, or Android client. In particular, the Claude Code profile requires a globally installed `claude-agent-acp` bridge; the Host never substitutes a runtime `npx` download.

| Provider | Required `PATH` command | Protocol process started by Host |
|---|---|---|
| OpenAI Codex | `codex` | `codex app-server --stdio` |
| Grok Build | `grok` | `grok --no-auto-update agent stdio` |
| Claude Code | `claude-agent-acp` | `claude-agent-acp` |
| Gemini CLI | `gemini` | `gemini --acp` |
| GitHub Copilot CLI | `copilot` | `copilot --acp --stdio --no-auto-update` |
| OpenCode | `opencode` | `opencode acp` |
| Cursor Agent | `agent` | `agent acp` |
| Cline | `cline` | `cline --acp` |
| Goose | `goose` | `goose acp` |
| JetBrains Junie | `junie` | `junie --acp=true` |

`PATH` is the default. These optional variables select an already-installed executable when a Host service has a different `PATH` or an administrator needs an explicit location:

```powershell
$env:AGENT_REMOTE_GROK_BIN = "C:\path\to\grok.exe"
$env:AGENT_REMOTE_CLAUDE_CODE_ACP_BIN = "C:\path\to\claude-agent-acp.cmd"
$env:AGENT_REMOTE_GEMINI_BIN = "C:\path\to\gemini.cmd"
$env:AGENT_REMOTE_COPILOT_BIN = "C:\path\to\copilot.exe"
$env:AGENT_REMOTE_OPENCODE_BIN = "C:\path\to\opencode.exe"
$env:AGENT_REMOTE_CURSOR_BIN = "C:\path\to\agent.exe"
$env:AGENT_REMOTE_CLINE_BIN = "C:\path\to\cline.cmd"
$env:AGENT_REMOTE_GOOSE_BIN = "C:\path\to\goose.exe"
$env:AGENT_REMOTE_JUNIE_BIN = "C:\path\to\junie.cmd"
```

Run the Host and Provider in the same operating-system path domain. A Windows Provider launched by a WSL Host may reject `/mnt/...` project or temporary paths even when stdio itself works.

Install the Claude bridge globally with its published package before using that profile:

```powershell
npm install --global @agentclientprotocol/claude-agent-acp
```

Install the other CLIs from their official distributions. These checks confirm command discovery only; they do not prove authentication or a working model request:

```powershell
codex --version
grok --version
claude-agent-acp --version
gemini --version
copilot --version --no-auto-update
opencode --version
agent --version
cline --version
goose --version
junie --version
```

Authenticate with each Provider's own supported login command before its real smoke test.
For Cursor, run `agent login`; the Host then uses the advertised ACP `cursor_login` method during initialization.

## Authorize projects

```powershell
dist\bin\agent-remote-host.exe project add C:\work\my-project --name "My project"
dist\bin\agent-remote-host.exe project set-providers <project-id> --provider codex --provider opencode --provider cursor --provider cline
dist\bin\agent-remote-host.exe project list
dist\bin\agent-remote-host.exe project remove <project-id>
```

Project paths are canonicalized when added. If `project add` omits every `--provider` flag, all ten built-in profiles are enabled. `set-providers` replaces the enabled set for an existing project. If a directory is moved or removed, it is shown invalid and is not silently replaced.

The `--provider` values are `codex`, `grok`, `claude-code`, `gemini-cli`, `copilot-cli`, `opencode`, `cursor`, `cline`, `goose`, and `junie`. Their versioned wire/database values use underscores for `claude_code`, `gemini_cli`, `copilot_cli`, and `open_code`; the other names are unchanged.

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

ACP profiles negotiate session listing, loading/resuming, permission choices, and other controls with the running CLI. Standard model, effort, and mode config options become authoritative after `session/new` or `session/load` returns them; before then the UI must not invent a model list. Gemini CLI currently does not advertise ACP `session/list`, so GeneralAgentRemote cannot discover Gemini sessions created outside its own known mappings. The shared ACP adapter currently sends text prompts only; prompt attachments remain disabled even if an Agent advertises richer prompt content.

## Real Provider smoke tests

```powershell
cargo xtask provider-smoke
```

The command uses a temporary authorized directory and asks each available Provider for the exact marker `AGENT_REMOTE_SMOKE_OK` without commands or file changes. A result has one of three meanings:

- `PASS`: the installed and authenticated Provider completed a real model turn and returned the marker;
- `SKIP`: a prerequisite such as the executable, authentication, quota, balance, or payment is unavailable;
- `FAIL`: an installed/authenticated Provider reached a protocol or execution error, timed out, or returned the wrong result.

`cargo xtask test` validates deterministic mock protocol paths only. Record real Provider output separately after running the smoke command on the intended Host; the current dated evidence is maintained in [Provider compatibility](provider-compatibility.md#verification-status-and-real-smoke-policy).
