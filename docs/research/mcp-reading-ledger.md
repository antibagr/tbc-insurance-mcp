# MCP and Rust source ledger

## Purpose

This ledger records the public sources that define the server's protocol and
runtime choices. Exact dependency versions and registry checksums live in
`Cargo.lock`; the Rust channel lives in `rust-toolchain.toml`. Local paths,
host details, authenticated account observations, and user activity are
excluded.

## Selected stack

| Area | Selection | Reason |
| --- | --- | --- |
| MCP SDK | `rmcp` 3.2.0 | Typed Rust server, tool macros, stdio transport, and supported protocol negotiation |
| Async runtime | Tokio | Runtime used by the selected SDK and bounded local/network I/O |
| HTTP | Reqwest with Rustls | HTTPS client with redirects disabled and bounded response streaming |
| Serialization | Serde and serde_json | Strict typed normalization at the upstream boundary |
| Schemas | Schemars | MCP input schemas derived from public Rust inputs |
| Secret lifetime | Zeroize | Explicit clearing for owned transient credential and document buffers |
| macOS credential store | security-framework plus the local signed service | Native per-user Keychain storage and code-signing checks |

The server advertises MCP revision `2026-07-28` and supports negotiation with
the compatibility revision covered by its tests. Protocol messages use stdout;
diagnostics use stderr.

## Evidence classes

- `read`: official documentation, specification text, repository source, or
  release metadata was inspected.
- `semantic-source-read`: implementation source was inspected where a public
  API page could not establish behavior.
- `verified-locally`: a synthetic build, test, or protocol probe confirmed the
  selected behavior.

Package popularity, search rank, or generated documentation alone does not
establish suitability for this security boundary.

## Updating a source

1. Retrieve the official source through HTTPS.
2. Record the exact dependency version in `Cargo.toml` and let Cargo record
   the registry checksum in `Cargo.lock`.
3. Inspect release notes and the source paths that govern behavior relied on by
   this project.
4. Run formatting, checks, tests, Rustdoc, dependency policy, advisory scans,
   and protocol probes.
5. Update the ADR when the architectural tradeoff changes.

Downloaded research corpora remain in the ignored `.research/` directory and
are excluded from packages, commits, and secret-scan claims.
