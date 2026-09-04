# Architecture

## Authority and data flow

```text
phone / desktop browser
        Rust/Yew Web UI
               |
     versioned CBOR commands
        +------+------+
        |             |
 direct or LAN     public WSS
        |             |
        |       agent-remote-relay
        |        opaque forwarding
        +------+------+
               |
       agent-remote-host
       only state authority
          +----+------------------------+
          |                             |
 codex app-server          shared ACP v1 adapter
 JSONL stdio               profile-selected process
                           Grok / Claude Code bridge /
                           Gemini / Copilot / OpenCode /
                           Cursor / Cline / Goose / Junie
```

The Host owns Provider processes, authorized project paths, native-session mappings, conversation state, merged timeline items, approvals, device credentials, command idempotency, Provider sync cursors, and managed images. The Relay and clients are projections of Host state. A reconnect first receives a fast Snapshot from local SQLite/memory state, then Provider capabilities and project metadata refresh incrementally. Project sync imports conversation metadata only; opening a conversation loads stale remote history and older local items use a stable `(created_at_ms, item_id)` cursor. Android and Web each retain at most one unacknowledged send for replay after reconnect; this is not a general offline work queue.

## Crate responsibilities

- `agent-remote-protocol` contains wire-safe IDs, Provider capability summaries, conversations, timeline variants, attachment metadata, commands, server messages, and Relay frames. Application CBOR envelopes use protocol version `5`; the unchanged opaque Host↔Relay tunnel remains version `1`. Attachment and tunneled payload bytes use CBOR byte strings.
- `agent-remote-host` validates project boundaries, persists state with SQLite, runs Provider protocol adapters, imports remote session history, merges history and streaming deltas by stable item ID/revision, serves the web application, authenticates direct clients, and maintains the optional outbound Relay tunnel.
- `agent-remote-relay` maintains an in-memory `host_id -> Host connection` registry. Its per-Host and per-client channels are bounded. A slow client is closed and must reconnect for a Snapshot.
- `agent-remote-web` is a Yew CSR application. It sends and receives only binary CBOR WebSocket frames. Device credentials are origin-scoped browser `localStorage` records; browsers do not expose an OS credential vault to WASM.
- `agent-remote-web-entry` is the root package's minimal WASM launcher. Trunk 0.21.14 requires a root package when it reads Cargo metadata from a workspace, so this entry delegates immediately to `agent-remote-web`; UI and protocol logic remain in the Web crate.
- `agent-remote-testkit` provides deterministic stdio protocol peers rather than PTY/TUI output.
- `xtask` owns the pinned WebAssembly build and release layout.

## Provider boundary

Codex keeps its built-in app-server adapter because that protocol exposes Codex-native thread, turn, model, effort, steering, and approval semantics. Grok, Claude Code, Gemini CLI, GitHub Copilot CLI, OpenCode, Cursor Agent, Cline, Goose, and JetBrains Junie use one typed ACP v1 transport with profile-specific executable arguments. Grok's version-gated model and interjection extensions remain confined to its profile.

The shared ACP adapter treats the initialization response and session config options as authoritative. Session listing, load/resume, close, modes, model choices, effort choices, and permission options are only exposed when advertised. Model and effort config options commonly arrive with `session/new` or `session/load`, so an ACP profile can legitimately have no selectable model before a native session exists. The current ACP prompt path is text-only and does not expose attachments even when an Agent advertises richer prompt content. Gemini CLI currently omits `session/list`; the Host does not parse its terminal session picker or scan its private session files to manufacture that capability.

The Host owns each child process and passes only an authorized project cwd. Installation, updates, login, tokens, subscriptions, and model billing remain owned by the Provider CLI on the Host machine. Provider credentials never cross the application or Relay protocols.

The WASM build uses Cargo release mode with thin LTO. The `wasm-opt` stage bundled with Trunk 0.21.14 is disabled because Binaryen v123 rejects bulk-memory instructions emitted by the current Rust stable compiler.

## Direct and Relay equivalence

Direct `/ws` and logical Relay clients both enter the same `ApplicationSession`. The first application message must be `Pair` or `Authenticate`; subsequent messages are handled by the same Host service. Relay multiplexing adds only `OpenClient`, opaque `Payload`, and `Close` frames and does not reinterpret the application message. Slow synchronization work is scoped by project while mutations are ordered by conversation; `Send`, `Steer`, and `Interrupt` do not wait behind another project's refresh or history load.

## Project confinement

The Host CLI canonicalizes every added project. A conversation permanently binds `ProviderId + ProjectId + native_session_id`. Provider cwd, Codex thread classification, ACP filesystem calls, and ACP terminal cwd use that project root. Codex remains the sole owner of its native session store (including a configured `CODEX_HOME`): GeneralAgentRemote only uses `thread/list`, `thread/read`, `thread/resume`, and `thread/start`, and never scans or writes `.codex/sessions`. Other Provider session stores likewise remain Provider-owned and are accessed only through advertised protocol methods. A paginated Codex or ACP listing is filtered on the Host by exact authorized cwd; unmatched sessions are excluded. Existing paths are canonicalized before checking containment. New write paths reject parent traversal and validate their canonical parent. Only project-relative paths are placed in timeline messages.

## Persistence

SQLite tables cover:

- projects;
- conversations and merged timeline items;
- attachment metadata (not image bytes);
- paired devices and single-use pair tokens;
- used device command IDs;
- Provider session mappings, remote history watermarks, and stable Provider item IDs.

Rows left in `running` or `needs_approval` are changed to `interrupted` at Host restart. Conversations remain permanently scoped to their Host, Provider, and authorized project. The native Provider session is resumed only when its current protocol reports that capability.

## Attachments

Codex image paths, ACP output image blocks, and Codex-capability-approved client prompt images enter one attachment pipeline. Before copying, the Host enforces the applicable input limit and its managed-image limit, detects the real format, decodes it, accepts only PNG/JPEG/WebP/GIF, and records dimensions. A random attachment ID becomes the managed filename. Remote timeline data never contains the original absolute path or managed path. ACP client prompt attachments remain disabled in the current shared adapter.

## Trust boundary

Public browser and Host links use HTTPS/WSS. Device authorization is decided by the Host. The Relay access token authorizes a Host registration but is not a device credential. The Relay is a trusted transport endpoint and can observe forwarded payloads in memory; v0.1 does **not** claim end-to-end encryption. It does not persist messages, projects, credentials, or images.
