# Provider compatibility baseline

Evidence date: **2026-09-04**. GeneralAgentRemote has one built-in Codex app-server adapter and a shared ACP v1 adapter with explicit profiles. The implementation follows the installed Provider's negotiated schema and capabilities when they differ from upstream prose.

## Supported Provider processes

| Provider | Wire/database ID | Protocol process | Existing-session discovery |
|---|---|---|---|
| OpenAI Codex | `codex` | `codex app-server --stdio` | paginated `thread/list` |
| Grok Build | `grok` | `grok --no-auto-update agent stdio` | when ACP `session/list` is advertised |
| Claude Code | `claude_code` | globally installed `claude-agent-acp` bridge | when ACP `session/list` is advertised |
| Gemini CLI | `gemini_cli` | `gemini --acp` | unavailable in the current ACP capability set |
| GitHub Copilot CLI | `copilot_cli` | `copilot --acp --stdio --no-auto-update` | when ACP `session/list` is advertised |
| OpenCode | `open_code` | `opencode acp` | when ACP `session/list` is advertised |
| Cursor Agent | `cursor` | `agent acp` | when ACP `session/list` is advertised |
| Cline | `cline` | `cline --acp` | when ACP `session/list` is advertised |
| Goose | `goose` | `goose acp` | when ACP `session/list` is advertised |
| JetBrains Junie | `junie` | `junie --acp=true` | when ACP `session/list` is advertised |
| Qwen Code | `qwen_code` | `qwen --acp` | when ACP `session/list` is advertised |
| Kimi CLI | `kimi_cli` | `kimi acp` | when ACP `session/list` is advertised |
| Kiro CLI | `kiro_cli` | `kiro-cli acp` | when ACP `session/list` is advertised |
| Mistral Vibe | `mistral_vibe` | `vibe-acp` | when ACP `session/list` is advertised |
| Qoder CLI | `qoder_cli` | `qoder --acp` | when ACP `session/list` is advertised |

The Host resolves each executable from `PATH` by default. An administrator may point a profile at an already-installed executable with the profile-specific `AGENT_REMOTE_*_BIN` variables documented in [Host setup](setup.md). It does not install, download, auto-update, or authenticate them. The user or Host administrator owns installation, Provider login, subscriptions, API keys, quotas, and billing. Credentials remain in each Provider's own Host-side store and never enter GeneralAgentRemote persistence or cross the Relay.

The Claude profile specifically requires `claude-agent-acp` to be installed globally, for example from `@agentclientprotocol/claude-agent-acp`. A runtime `npx` download is not a supported fallback.

## Codex 0.150.1

The adapter uses the official [Codex app-server protocol](https://learn.chatgpt.com/docs/app-server) over newline-delimited JSON on stdio. It never uses the TUI or app-server's experimental WebSocket transport.

Stable methods used:

```text
initialize / initialized
account/read
model/list
thread/list
thread/read
thread/start
thread/resume
turn/start
turn/steer
turn/interrupt
```

- Picker entries come from paginated `model/list`. The wire `model` ID, advertised `supportedReasoningEfforts`, `defaultReasoningEffort`, and `hidden` flag are authoritative.
- `thread/list` requests `cli`, `vscode`, and `appServer` sources once with pagination, then the Host classifies each result by exact normalized cwd against authorized projects. Unmatched threads are excluded; absolute unmatched paths are never sent to clients.
- Project sync imports lightweight conversation metadata only. When a conversation is opened and its history is stale, the Host uses `thread/read`; the installed schema exposes no incremental history cursor, so Codex currently returns an idempotent full-thread read whose stable native item IDs are merged locally.
- Conversation rename uses `thread/name/set`. A locally generated first-message title remains until Codex reports a non-empty Provider title; a user title is never overwritten by later sync.
- Prompt input accepts only the adapter-advertised PNG/JPEG/WebP/GIF set (up to four images, 10 MiB each and 10 MiB total). Permission choices are adapter capability data rather than client constants because app-server does not enumerate them.
- `turn/steer` requires the active `expectedTurnId`; the control is unavailable when no current turn exists.
- Command, file, and permission approvals are server-initiated requests. The adapter retains their JSON-RPC request ID and answers that ID.
- Only commentary/final Agent messages and user-visible reasoning summaries enter the timeline. Raw reasoning deltas/content are ignored.
- `imageView` is a thread item containing a local path. `imageGeneration` contains base64 and may contain `savedPath`; both enter the managed attachment pipeline.
- Unknown fields and notification methods are tolerated and kept out of user-visible state.

The exact local schema was generated with `codex app-server generate-json-schema`; it takes precedence over older README examples. Experimental thread item paging, process APIs, and remote-control APIs are not used.

Codex itself remains the only authority for physical session files and resolves its normal store from its own configuration or `CODEX_HOME`. GeneralAgentRemote passes the authorized project cwd to start/resume/turn calls for logical ownership and execution, but it does not create a per-project session store or scan, parse, copy, move, or rewrite `.codex/sessions`.

## Shared ACP v1 adapter

The ACP profiles follow the official [ACP v1 specification](https://agentclientprotocol.com/protocol/v1) through the official Rust SDK. They start the Provider's protocol process directly and never parse a TUI, terminal transcript, or screenshot.

The common typed path covers:

- `initialize`, profile-required Provider authentication, `session/new`, `session/prompt`, and `session/cancel`;
- optional `session/list`, `session/load`, `session/resume`, and `session/close` only when advertised;
- streamed agent/user message chunks, plans, tool calls, command activity, file changes, images, completion, interruption, and errors from `session/update`;
- exact Provider-supplied `session/request_permission` option IDs and responses;
- output content blocks and capability-approved reverse filesystem/terminal requests;
- standard session config options returned by `session/new`, `session/load`, `session/resume`, or later `config_option_update` events.

Standard config options are the model, effort/thought-level, and mode source of truth. They may not exist until a native session has been created or loaded. The Host does not hard-code a model catalog to fill that gap, and the UI updates when the Provider supplies the options. Changes use `session/set_config_option`; obsolete or Provider-private model calls stay in version-gated compatibility code.

The current shared ACP prompt path sends text only. Prompt attachments stay disabled even when an Agent advertises image, audio, or embedded-context input. Output images remain supported through managed attachment handling.

ACP session history is a controlled full-replay fallback because the supported agents do not expose a portable incremental history cursor. Text is coalesced into stable turn items, repeated Provider item IDs are deduplicated, and a persistence barrier completes before the history watermark advances. Reverse file and terminal requests are confined to the conversation's authorized project. Terminal output is capped and marked when truncated, and exit status is preserved.

### Profile boundaries

- **Grok Build:** Grok 1.0.13 reports model/effort through `_meta.modelState` and uses its legacy `session/set_model`. Its version-gated `_x.ai/interject` extension enables steering. Other ACP profiles do not inherit either extension. Grok prompt images and rename remain unavailable.
- **Claude Code:** the Host launches the globally installed `claude-agent-acp` bridge. Session catalog, load/resume, modes, models, effort, and permission choices remain driven by the bridge's handshake and session config options. GeneralAgentRemote does not read Claude's private session files.
- **Gemini CLI:** the current `gemini --acp` handshake supports new/load, prompt/cancel, streamed updates, and permissions, but does not advertise `session/list`. The Host can continue a Gemini session already mapped in GeneralAgentRemote, but cannot discover unrelated Gemini CLI sessions. It does not parse `gemini --list-sessions` output to manufacture support.
- **GitHub Copilot CLI:** the profile uses ACP stdio with CLI auto-update disabled. Session list/load/close are exposed only when negotiated. Provider authentication remains owned by Copilot CLI. Reasoning settings that a Copilot version fixes at ACP-server startup are not presented as per-conversation controls.
- **OpenCode:** the profile uses `opencode acp`, not OpenCode's broader HTTP file APIs. Its advertised session list/load/resume, models, variants/effort, modes, prompts, and permissions flow through the same typed ACP path.
- **Cursor Agent:** the profile launches the official `agent acp` process and sends `authenticate(cursor_login)` only when that exact method is advertised. Standard sessions, streaming, modes, and permissions use ACP. Cursor's private `cursor/*` controls are not presented in this baseline; blocking question and plan requests receive an explicit cancelled response so a turn cannot hang waiting for an unsupported UI.
- **Cline:** the profile launches `cline --acp`. Provider/model/thinking and auto-approval options remain driven by the CLI's ACP session options and permission requests.
- **Goose:** the profile launches `goose acp`. Model/provider selection remains in Goose configuration unless the running version advertises a standard session option.
- **JetBrains Junie:** the profile launches `junie --acp=true`; its session and approval capabilities are exposed only when negotiated.
- **Qwen Code:** the profile launches the stable `qwen --acp` entry point. Sessions, modes, model choices, approvals, and filesystem or terminal calls are exposed only through negotiated ACP capabilities.
- **Kimi CLI:** the profile launches the multi-session `kimi acp` entry point and reuses Kimi's Host-side login. Session list/load and model/thinking options are consumed only when the running CLI advertises them.
- **Kiro CLI:** the profile launches `kiro-cli acp`. Standard sessions, streamed updates, permissions, and config options flow through the common adapter; `_kiro.dev/*` extensions are ignored.
- **Mistral Vibe:** the profile launches the dedicated `vibe-acp` executable. GeneralAgentRemote does not start or parse Vibe's interactive terminal UI.
- **Qoder CLI:** the profile launches `qoder --acp` and reuses Qoder's Host-side login or `QODER_PERSONAL_ACCESS_TOKEN`. Permission modes and other controls remain capability-driven.

ACP has no portable rename or mid-turn steer operation in the implemented baseline. Those controls remain disabled unless an exact profile advertises and implements a compatible operation.

## Deliberately unsupported agents

Aider is not supported. Its official CLI offers one-shot messages, terminal text streaming, history files, and broad automatic confirmation, but no machine-readable stream of messages, tool activity, stable event IDs, and approval requests comparable to Codex app-server or ACP. Integrating it would require scraping terminal output or pretending that `--yes-always` is an interactive approval protocol, both of which violate this product's Provider boundary. Roo Code is currently an editor extension rather than a supported standalone ACP process in this integration.

## Verification status and real smoke policy

Automated tests use deterministic stdio peers to validate the built-in Codex adapter, the shared ACP transport, capability differences, session replay, config-option mapping, permission responses, cancellation, and project confinement. These tests do not prove that a vendor account can complete a model turn.

`cargo xtask provider-smoke` runs each discovered Provider in a temporary authorized directory and requests the exact response `AGENT_REMOTE_SMOKE_OK` without commands or file changes:

- `PASS` means the installed and authenticated Provider completed that real turn and returned the marker;
- `SKIP` reports the actual missing executable, authentication, quota, balance, or payment prerequisite and is not a pass;
- `FAIL` means a smoke-testable Provider hit a protocol/execution error, timed out, or returned the wrong result.

Real-turn evidence collected on 2026-09-04:

- **Codex 0.150.1: PASS.** The Host adapter received the exact marker from the installed WSL CLI.
- **OpenCode 1.18.25: PASS.** The same Host ACP adapter received the exact marker from the installed Windows CLI in a temporary authorized directory, using `AGENT_REMOTE_OPENCODE_BIN` to select that executable.
- **Claude Code, Gemini CLI, GitHub Copilot CLI, Cursor Agent, Cline, Goose, JetBrains Junie, Qwen Code, Kimi CLI, Kiro CLI, Mistral Vibe, and Qoder CLI: SKIP.** Their protocol executables were not installed in the tested WSL or Windows environments.
- **Grok 1.0.13 cross-OS probe: FAIL.** Launching the Windows Grok executable from a WSL Host rejected the WSL temporary path as non-absolute. This result does not establish failure on a matching-platform Windows Host or with Grok installed inside WSL.

These real Provider results are separate from automated tests and physical-device evidence; another API, a fake model, or TUI parsing is never substituted for a skipped Provider.
