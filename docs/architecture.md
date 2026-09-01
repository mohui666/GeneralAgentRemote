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

The Host owns Provider processes, authorized project paths, native-session mappings, conversation state, merged timeline items, approvals, device credentials, command idempotency, and managed images. The Relay and browser are projections of Host state. A reconnect requests a complete Snapshot; v0.1 has no offline command queue or event-sourcing layer.

## Crate responsibilities

- `agent-remote-protocol` contains wire-safe IDs, Provider capability summaries, conversations, timeline variants, attachment metadata, commands, server messages, and Relay frames. Every encoded CBOR envelope carries protocol version `1`. Attachment and tunneled payload bytes use CBOR byte strings.
- `agent-remote-host` validates project boundaries, persists minimal state with SQLite, runs Provider protocol adapters, merges streaming deltas by item ID/revision, serves the web application, authenticates direct clients, and maintains the optional outbound Relay tunnel.
- `agent-remote-relay` maintains an in-memory `host_id -> Host connection` registry. Its per-Host and per-client channels are bounded. A slow client is closed and must reconnect for a Snapshot.
- `agent-remote-web` is a Yew CSR application. It sends and receives only binary CBOR WebSocket frames. Device credentials are origin-scoped browser `localStorage` records; browsers do not expose an OS credential vault to WASM.
- `agent-remote-web-entry` is the root package's minimal WASM launcher. Trunk 0.21.14 requires a root package when it reads Cargo metadata from a workspace, so this entry delegates immediately to `agent-remote-web`; UI and protocol logic remain in the Web crate.
- `agent-remote-testkit` provides deterministic stdio protocol peers rather than PTY/TUI output.
- `xtask` owns the pinned WebAssembly build and release layout.

The WASM build uses Cargo release mode with thin LTO. The `wasm-opt` stage bundled with Trunk 0.21.14 is disabled because Binaryen v123 rejects bulk-memory instructions emitted by the current Rust stable compiler.

## Direct and Relay equivalence

Direct `/ws` and logical Relay clients both enter the same `ApplicationSession`. The first application message must be `Pair` or `Authenticate`; subsequent messages are handled by the same Host service. Relay multiplexing adds only `OpenClient`, opaque `Payload`, and `Close` frames and does not reinterpret the application message.

## Project confinement

The Host CLI canonicalizes every added project. A conversation permanently binds `ProviderId + ProjectId + native_session_id`. Provider cwd, Codex thread filtering, ACP filesystem calls, and ACP terminal cwd use that project root. Existing paths are canonicalized before checking containment. New write paths reject parent traversal and validate their canonical parent. Only project-relative paths are placed in timeline messages.

## Persistence

SQLite tables cover:

- projects;
- conversations and merged timeline items;
- attachment metadata (not image bytes);
- paired devices and single-use pair tokens;
- used device command IDs;
- Provider session mappings.

Rows left in `running` or `needs_approval` are changed to `interrupted` at Host restart. The native Provider session remains resumable only when its current protocol reports that capability.

## Attachments

Codex paths and ACP base64 images enter one attachment pipeline. Before copying, the Host enforces the configured byte limit (10 MiB by default), detects the real format, decodes it, accepts only PNG/JPEG/WebP/GIF, and records dimensions. A random attachment ID becomes the managed filename. Remote timeline data never contains the original absolute path or managed path.

## Trust boundary

Public browser and Host links use HTTPS/WSS. Device authorization is decided by the Host. The Relay access token authorizes a Host registration but is not a device credential. The Relay is a trusted transport endpoint and can observe forwarded payloads in memory; v0.1 does **not** claim end-to-end encryption. It does not persist messages, projects, credentials, or images.
