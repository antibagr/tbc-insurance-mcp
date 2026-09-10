# TBC Insurance MCP

A local [Model Context Protocol](https://modelcontextprotocol.io/) server for
TBC Insurance health-account workflows.

> [!IMPORTANT]
> This is an independent community project with no affiliation, endorsement, or
> support relationship with TBC Insurance. TBC names and trademarks belong to
> their respective owners. Use the server only with an account you are
> authorized to access and review the insurer's current
> [application terms](https://tbcinsurance.ge/en/terms-and-conditions-of-the-application)
> and
> [privacy policy](https://tbcinsurance.ge/ge/insurance-operation-privacy-policy).

The server exposes typed insurance data and five reviewed actions: appointment
booking, reimbursement, guarantee of payment, medical referral, and insurer
message reply. It is designed for local stdio hosts and keeps account
credentials out of MCP arguments, tool results, logs, and repository files.

## Status

Version 0.5.0 is an experimental source release distributed through GitHub.
Crates.io publishing is disabled, and no prebuilt binaries are published. It
depends on undocumented
upstream web APIs that may change without notice. Persistent credential storage
and the signed credential service are supported on macOS. Other platforms can
build and test the server with an in-memory session for development.

The server can help inspect account data and prepare requests. Insurer responses
remain authoritative for coverage, eligibility, payment, booking, and claim
decisions. This project provides no medical, insurance, or legal advice.

## Highlights

- 20 typed read tools for policies, coverage, claims, requests, referrals,
  appointment availability and reservations, service cities, and inbox data
- five action-preparation tools plus a single-use execution tool
- exact, five-minute reviews bound to the current session and immutable inputs
- one submission attempt followed by an account readback
- redirect-free and retry-free writes
- bounded network, protocol, file, and diagnostic inputs
- macOS Keychain persistence through a separately signed local credential service
- sanitized structured diagnostics and optional private debug capture
- pinned Rust toolchain, dependencies, GitHub Actions, and release tooling

The domain glossary and upstream-name mapping are in
[docs/domain/health-insurance.md](docs/domain/health-insurance.md).

## Safety model

Every state-changing workflow has two MCP calls:

1. A preparation tool validates current account data, snapshots any document,
   and returns an exact review with a random ID.
2. `execute_reviewed_action` accepts that ID, rechecks the live preconditions,
   consumes the review, submits once, and reads the account state back.

Review IDs expire after five minutes, belong to one imported session, and are
single use. They act as local execution capabilities, so MCP hosts must keep them
private. Upstream labels, messages, comments, and statuses are treated as
untrusted data and cannot authorize an action.

An ambiguous response is reported as `outcome_unknown` with
`retry_safe: false`. Inspect the readback or contact the insurer before
considering another submission.

Document inputs must be explicit absolute paths to regular files. Paths are MCP
arguments, so use neutral file and directory names. The server rejects symlinks,
empty files, unsupported names, files larger than 4 MiB, and synced
ChatGPT-project source paths. Accepted bytes are snapshotted in memory and
zeroized when the review disappears.

See [SECURITY.md](SECURITY.md) and [PRIVACY.md](PRIVACY.md) before connecting a
real account.

## Requirements

For the supported macOS installation:

- Rust 1.98.0 through `rustup`
- Apple command-line developer tools, including `codesign`
- a macOS login Keychain
- an MCP host that can launch a local stdio server
- local browser automation capable of observing a request from the signed-in
  official TBC Insurance portal

## Install on macOS

Clone the repository and create the dedicated local signing identity once:

```sh
git clone https://github.com/antibagr/tbc-insurance-mcp.git
cd tbc-insurance-mcp
sh scripts/create-signing-identity.sh
sh scripts/install-macos.sh
```

The setup creates a self-signed code-signing identity in the login Keychain and
leaves system trust unchanged. The installer builds and signs the MCP server and
its credential service, installs the main binary at
`~/.cargo/bin/tbc-insurance-mcp`, and registers the per-user credential
service with `launchd`.

Ordinary updates preserve the credential-service binary byte for byte:

```sh
git pull --ff-only
sh scripts/install-macos.sh
```

Use `sh scripts/install-macos.sh --update-vault` only when release notes require
a credential-service update. A new credential-service identity can require one
intentional Keychain approval:

```sh
~/.cargo/bin/tbc-insurance-mcp --authorize-session
```

Development builds on other platforms can use:

```sh
cargo install --locked --path .
```

Those builds keep an imported session in process memory for the current run.

## Configure an MCP host

Launch the installed binary as a stdio MCP server. Use its absolute path if the
host does not inherit the Cargo binary directory:

```json
{
  "mcpServers": {
    "tbc-insurance": {
      "command": "tbc-insurance-mcp",
      "args": []
    }
  }
}
```

Restart the host connection after installing an update. Call
`get_integration_status` and verify the reported server version before using
account tools.

## Connect a TBC session

The server imports a bearer credential through a short-lived loopback channel:

1. Sign in through the official portal at `https://on.tbcinsurance.ge`.
2. Call `prepare_tbc_session_import`.
3. Use a local Playwright or CDP request observer to reload the signed-in page
   and capture the bearer value only from an authenticated request to the
   allowlisted TBC API host.
4. Send the ticket secret and bearer value directly to the returned
   `127.0.0.1` endpoint as the documented newline-terminated JSON payload.
5. Require the `{"ok":true}` acknowledgement, then call
   `get_integration_status`.

Keep the bearer value inside the local automation process. It must never enter
chat, MCP arguments, shell history, the clipboard, screenshots, ordinary files,
or Git.

The import listener lasts 120 seconds, accepts at most four bounded attempts,
and requires a 256-bit one-time secret. On macOS, restarts recover the minimized
session record through the signed credential service. Session renewal runs
inside the MCP process. Expired or rejected sessions require another portal
login.

## Tool groups

| Group | Representative tools |
| --- | --- |
| Setup | `get_integration_status`, `prepare_tbc_session_import` |
| Coverage | `list_health_policies`, `list_coverage_benefits`, `get_coverage_summary` |
| Claims and requests | `list_medical_claims`, `get_medical_claim`, `list_member_requests`, `get_member_request` |
| Referrals | `list_referral_providers`, `list_referral_service_options`, `list_medical_referrals` |
| Appointments | `list_bookable_services`, `list_provider_locations`, `list_available_clinicians`, `list_appointment_slots`, `list_appointment_bookings` |
| Inbox | `list_inbox_messages`, `count_unread_messages`, `get_inbox_message`, `list_message_thread` |
| Prepare actions | `book_appointment`, `submit_reimbursement_claim`, `request_guarantee_of_payment`, `request_medical_referral`, `reply_to_insurer_message` |
| Execute | `execute_reviewed_action` |

The machine-readable upstream operation boundaries are recorded in
[contracts/tbc-read-api-v1.json](contracts/tbc-read-api-v1.json) and
[contracts/tbc-write-api-v1.json](contracts/tbc-write-api-v1.json).

## Diagnostics

Protocol traffic uses stdout exclusively. Sanitized JSON diagnostics use stderr
and include a local trace number, stable operation name, outcome, and elapsed
time. Request bodies, response bodies, credentials, medical text, identifiers,
and upstream URLs are excluded.

Private debug capture is opt-in:

```text
TBC_INSURANCE_DEBUG=1
TBC_INSURANCE_DIAGNOSTIC_DIR=/absolute/private/directory
```

The directory must have a safe existing parent. The server creates the leaf
with mode 0700 and each log with mode 0600, caps files at 2 MiB, and disables
capture after a write failure or size limit. Debug logs remain local until the
user removes them and can still reveal timing and workflow metadata.

## Remove the local installation

Disconnect the MCP host first. The installed components are:

- `~/.cargo/bin/tbc-insurance-mcp`
- `~/Library/Application Support/tbc-insurance/tbc-session-vault`
- `~/Library/LaunchAgents/dev.antibagr.tbc-insurance-mcp.session-vault.plist`
- a login-Keychain item for service
  `dev.antibagr.tbc-insurance-mcp.session-vault` and account
  `tbc-api-session`
- the local signing identity `TBC Insurance MCP Local Signing`

Removal can erase the saved session and signing identity. Inspect each target
and make any desired backup before deleting it.

## Development

The repository pins Rust 1.98.0 and resolves dependencies from lockfiles.

```sh
cargo fmt --all -- --check
cargo check --locked --all-targets --all-features
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
cargo test --locked --manifest-path crates/session-vault/Cargo.toml --all-targets
RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps --all-features
```

The complete local and CI gates are documented in
[docs/quality.md](docs/quality.md). Contributions must use synthetic fixtures;
see [CONTRIBUTING.md](CONTRIBUTING.md).

## Community and policy

- [Contributing](CONTRIBUTING.md)
- [Security policy](SECURITY.md)
- [Privacy model](PRIVACY.md)
- [Support](SUPPORT.md)
- [Code of conduct](CODE_OF_CONDUCT.md)
- [Changelog](CHANGELOG.md)
- [Apache License 2.0](LICENSE)
