# ADR 0001: Rust MCP stack

- Status: accepted
- Date: 2026-08-25
- Last amended: 2026-09-10
- Scope: local stdio MCP server for TBC Insurance health-account workflows

## Context

The server handles a browser-derived bearer credential and sensitive
health-account responses. Its protocol transport, upstream client, structured
output, diagnostics, and local persistence all cross trust boundaries.

The implementation needs a small auditable dependency surface, bounded I/O,
strict wire normalization, explicit protocol-version behavior, and macOS
Keychain support.

## Decision

### MCP SDK and protocol

Use the official `rmcp` 3.2.0 release with the smallest production feature set:
`macros` and `server`. Client and transport helpers are enabled only for
tests.

Advertise MCP revision `2026-07-28` explicitly. Support the compatibility
revision exercised by the stdio contract tests. Keep MCP messages on stdout and
all diagnostics on stderr.

### Transport

Use the SDK's JSON-RPC codec behind a project-owned newline-delimited stdio
transport. Reject inbound and outbound frames larger than 1 MiB. Treat an
oversized or malformed frame as a failed input channel and terminate cleanly.

### Runtime

Use Tokio's current-thread runtime for the server. Session import, renewal,
network reads, and MCP calls remain asynchronous. Tests can use the multi-thread
runtime where concurrency behavior is under examination.

### HTTP and wire boundary

Use Reqwest with Rustls. The client has a fixed API base, closed endpoint enums,
a 20-second timeout, disabled redirects, disabled automatic retries, and an
8 MiB streamed-response limit.

Deserialize upstream JSON through private wire types. Convert it into public
insurance-domain types with explicit bounds, null handling, money precision,
and unknown-enum states. Raw upstream JSON never crosses the MCP boundary.

### Credentials and local state

Import the browser bearer through a one-time loopback ticket. Mark authorization
headers sensitive and keep owned transient credential buffers under
`Zeroizing`.

On macOS, a separately signed local service owns the minimized Keychain record.
The MCP and service verify fixed code-signing identifiers and the same dedicated
certificate. Non-macOS development runs keep the session in process memory.

Pending action reviews remain in memory, expire after five minutes, bind to the
current credential generation, and are single use.

### Diagnostics

Use structured `tracing` events with stable operation names, local trace IDs,
sanitized outcome codes, and elapsed time. Exclude URLs, headers, request and
response bodies, credentials, personal identifiers, medical text, and document
contents.

## Consequences

- Dependencies and the Rust toolchain are pinned exactly.
- The custom transport and normalization layer remain security-critical code
  with dedicated adversarial tests.
- Persistent session support requires macOS signing and Keychain behavior.
- Upstream API drift produces explicit compatibility errors.
- A process restart discards pending actions and non-macOS sessions.
- New endpoints must enter a closed enum and a documented domain workflow.

## Evidence

- MCP specification: <https://modelcontextprotocol.io/specification/2026-07-28>
- Official Rust SDK: <https://github.com/modelcontextprotocol/rust-sdk>
- Reqwest: <https://docs.rs/reqwest/>
- Tokio: <https://docs.rs/tokio/>
- Apple Keychain Services:
  <https://developer.apple.com/documentation/security/keychain-services>
- Exact dependency versions and checksums: `Cargo.lock`
- Exact Rust channel: `rust-toolchain.toml`
- Source-selection process: `docs/research/mcp-reading-ledger.md`
