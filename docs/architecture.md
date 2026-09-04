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
          +----+----+
          |         |
 codex app-server  grok agent stdio
 JSONL stdio       ACP v1 JSON-RPC
```

The Host owns Provider processes, authorized project paths, native-session mappings, conversation state, merged timeline items, approvals, device credentials, command idempotency, Provider sync cursors, and managed images. The Relay and clients are projections of Host state. A reconnect first receives a fast Snapshot from local SQLite/memory state, then Provider capabilities and project metadata refresh incrementally. Project sync imports conversation metadata only; opening a conversation loads stale remote history and older local items use a stable `(created_at_ms, item_id)` cursor. Android and Web each retain at most one unacknowledged send for replay after reconnect; this is not a general offline work queue.

## Crate responsibilities

- `agent-remote-protocol` contains wire-safe IDs, Provider capability summaries, conversations, timeline variants, attachment metadata, commands, server messages, and Relay frames. Application CBOR envelopes use protocol version `2`; the unchanged opaque Host↔Relay tunnel remains version `1`. Attachment and tunneled payload bytes use CBOR byte strings.
- `agent-remote-host` validates project boundaries, persists state with SQLite, runs Provider protocol adapters, imports remote session history, merges history and streaming deltas by stable item ID/revision, serves the web application, authenticates direct clients, and maintains the optional outbound Relay tunnel.
- `agent-remote-relay` maintains an in-memory `host_id -> Host connection` registry. Its per-Host and per-client channels are bounded. A slow client is closed and must reconnect for a Snapshot.
- `agent-remote-web` is a Yew CSR application. It sends and receives only binary CBOR WebSocket frames. Device credentials are origin-scoped browser `localStorage` records; browsers do not expose an OS credential vault to WASM.
- `agent-remote-web-entry` is the root package's minimal WASM launcher. Trunk 0.21.14 requires a root package when it reads Cargo metadata from a workspace, so this entry delegates immediately to `agent-remote-web`; UI and protocol logic remain in the Web crate.
- `agent-remote-testkit` provides deterministic stdio protocol peers rather than PTY/TUI output.
- `xtask` owns the pinned WebAssembly build and release layout.

The WASM build uses Cargo release mode with thin LTO. The `wasm-opt` stage bundled with Trunk 0.21.14 is disabled because Binaryen v123 rejects bulk-memory instructions emitted by the current Rust stable compiler.

## Direct and Relay equivalence

Direct `/ws` and logical Relay clients both enter the same `ApplicationSession`. The first application message must be `Pair` or `Authenticate`; subsequent messages are handled by the same Host service. Relay multiplexing adds only `OpenClient`, opaque `Payload`, and `Close` frames and does not reinterpret the application message. Slow synchronization work is scoped by project while mutations are ordered by conversation; `Send`, `Steer`, and `Interrupt` do not wait behind another project's refresh or history load.

## Project confinement

The Host CLI canonicalizes every added project. A conversation permanently binds `ProviderId + ProjectId + native_session_id`. Provider cwd, Codex thread classification, ACP filesystem calls, and ACP terminal cwd use that project root. Codex remains the sole owner of its native session store (including a configured `CODEX_HOME`): GeneralAgentRemote only uses `thread/list`, `thread/read`, `thread/resume`, and `thread/start`, and never scans or writes `.codex/sessions`. A paginated Codex listing is classified on the Host by exact normalized cwd against authorized projects; unmatched threads are excluded and only their count may appear in diagnostics. Existing paths are canonicalized before checking containment. New write paths reject parent traversal and validate their canonical parent. Only project-relative paths are placed in timeline messages.

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

Codex paths, ACP base64 images, and capability-approved client prompt images enter one attachment pipeline. Before copying, the Host enforces the Provider-advertised input limit and its managed-image limit, detects the real format, decodes it, accepts only PNG/JPEG/WebP/GIF, and records dimensions. A random attachment ID becomes the managed filename. Remote timeline data never contains the original absolute path or managed path.

## Trust boundary

Public browser and Host links use HTTPS/WSS. Device authorization is decided by the Host. The Relay access token authorizes a Host registration but is not a device credential. The Relay is a trusted transport endpoint and can observe forwarded payloads in memory; v0.1 does **not** claim end-to-end encryption. It does not persist messages, projects, credentials, or images.
