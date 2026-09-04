# Provider compatibility baseline

Evidence date: **2026-09-01**. The implementation follows the installed versions when they differ from examples in upstream prose.

## Local tools

| Provider | Installed CLI | Protocol entry point | Result of Gate 0 probe |
|---|---:|---|---|
| OpenAI Codex | `codex-cli 0.150.1` | `codex app-server --stdio` | Initialization, `account/read`, `model/list`, and cwd-filtered `thread/list` succeeded without creating a thread |
| Grok Build | `grok 1.0.13 (5e9a58528b76)` | `grok --no-auto-update agent stdio` | ACP v1 initialization succeeded; local model metadata reported Grok 4.6 and 4.5 |

`Trunk` and `wasm-pack` were not globally installed. The repository uses a pinned local Trunk tool through `xtask`.

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

## Grok Build 1.0.13 and ACP v1

The adapter follows the official [ACP v1 specification](https://agentclientprotocol.com/protocol/v1) and uses the official [Rust SDK](https://github.com/agentclientprotocol/rust-sdk) with stable ACP v1 types. It starts Grok in protocol mode, not headless/TUI output mode.

Negotiated local capabilities:

- session new, list, load/resume, close, prompt, and cancel;
- client filesystem and terminal calls needed for coding work;
- permission requests and streamed `session/update` items;
- output ContentBlock images;
- prompt images are reported unsupported and are never sent.

Current Grok-specific compatibility boundary:

- Grok 1.0.13 does not implement standard `session/set_config_option`; it reports model/effort under `_meta.modelState` and accepts the version-gated legacy `session/set_model` request. The adapter keeps this in `providers/grok.rs` and does not claim generic ACP support for it.
- ACP v1 has no standard steer method. Grok 1.0.13 implements the unadvertised `_x.ai/interject` extension. The UI only enables steering when this exact compatible Provider version is active.
- Authentication has no reliable ACP status call. A successful lightweight `grok models` probe indicates a logged-in local CLI; otherwise the Host reports the actual launch/auth error rather than treating executable presence as authentication.
- ACP `session/load` replays the whole changed session and does not expose a portable incremental cursor. Text chunks are coalesced by turn, history events cross a persistence barrier before the sync watermark advances, and stable synthetic item IDs make repeated full reads idempotent. A missing/invalid ACP update timestamp deliberately forces another full replay. Grok does not advertise conversation rename or prompt-image input, so those controls remain unavailable for this Provider.

Filesystem and terminal reverse requests are bound to the conversation project. Terminal output is capped and visibly marked when truncated; exit status is preserved. Permissions use the exact Provider-supplied options.

## Real smoke policy

Automated tests use the two deterministic stdio mocks. Real smoke tests run only in a temporary authorized directory. They report `SKIP` with the actual missing installation, authentication, quota, or payment prerequisite and never substitute a generic API or TUI parser. Protocol and implementation errors remain `FAIL`.
