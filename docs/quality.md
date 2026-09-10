# Quality and release gates

The project treats credentials, health-account data, and state-changing
workflows as trust boundaries. Automated checks use synthetic fixtures and
canaries only.

## Fast lane

Pull requests and pushes run the `fast` and `session-vault` jobs in
`.github/workflows/quality.yml`. The main checks are:

```sh
cargo fmt --all -- --check
cargo fmt --manifest-path fuzz/Cargo.toml -- --check
cargo fmt --manifest-path crates/session-vault/Cargo.toml -- --check
cargo check --locked --all-targets --all-features
cargo clippy --locked --all-targets --all-features -- -D warnings
rust-analyzer diagnostics . --severity warning
RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps --all-features
cargo test --locked --doc --all-features
cargo nextest run --locked --all-features
cargo test --locked --manifest-path crates/session-vault/Cargo.toml --all-targets
cargo shear
typos
rumdl check .
tombi lint
tombi format --check
jq empty contracts/*.json
cargo deny --locked check
cargo deny --manifest-path fuzz/Cargo.toml --config fuzz/deny.toml --locked check
cargo audit
cargo audit --file fuzz/Cargo.lock
gitleaks git --redact --config .gitleaks.toml --no-banner
gitleaks dir . --redact --config .gitleaks.toml --no-banner
actionlint
zizmor .github/workflows/quality.yml
```

The macOS matrix exercises signed-vault-compatible code paths and the complete
MCP test suite. Linux exercises portability and keeps native Keychain behavior
behind platform boundaries.

## Scheduled deep lane

The weekly and manually dispatched jobs add:

- branch coverage thresholds of 80% functions, 85% lines, and 80% regions;
- feature-power-set checks;
- `cargo-careful` and focused Miri checks;
- ThreadSanitizer coverage for concurrent review consumption;
- a one-minute sanitizer-backed fuzz target;
- the normalization benchmark;
- an auditable release binary and compiled-binary advisory scan;
- a clean install smoke test;
- a CycloneDX SBOM with JSON validation;
- mutation testing with a separate time budget.

All GitHub Actions are pinned to immutable commit SHAs. Release tools and the
Rust toolchain are pinned to exact versions.

## Test strategy

Tests emphasize invariant violations and adversarial boundaries:

- arbitrary endpoint, redirect, request, response-size, and protocol-frame input;
- malformed or drifting upstream JSON;
- missing, expired, forged, reused, or cross-session action reviews;
- replacement arguments or document bytes after preparation;
- insurer text that resembles an agent instruction;
- cancellation at request and response boundaries;
- uncertain write outcomes and replay attempts;
- path traversal, symlinks, unsupported files, and oversized documents;
- credentials or private text reaching output and diagnostics;
- session renewal races, stale generations, and cleanup failures;
- unsigned or incorrectly identified credential-service peers.

Every behavioral fix should add the smallest synthetic regression that fails
without it.

## Upstream compatibility

The TBC profile API is undocumented. Contract files record minimized method,
path, field, and interpretation evidence. Raw authenticated responses, browser
archives, personal records, and portal screenshots remain outside the
repository.

An authorized maintainer may perform a private account smoke check before a
release. Public evidence is limited to the revision, tool name, sanitized
outcome, and pass/fail status. Account values, dates of care, appointments,
messages, and identifiers are excluded.

## Release checklist

1. Update `Cargo.toml`, lockfiles, README, and changelog.
2. Run the fast lane from a clean checkout.
3. Run the deep lane or document any unavailable platform-specific gate.
4. Inspect the tracked-file list and a `git archive` for unintended files.
5. Scan the working tree and complete candidate history with Gitleaks.
6. Search tracked text and author metadata for personal identifiers.
7. Review binary files, symbolic links, generated artifacts, and file modes.
8. Build and install from a clean clone of the source archive.
9. Create an annotated tag from the verified commit.
10. Publish the release notes and source-derived SBOM without account-specific
    evidence or locally built executables.

A green CI run establishes the checked revision and declared gates. It carries
no guarantee about insurer availability, account eligibility, or future
upstream compatibility.

Crates.io packaging remains disabled. The macOS credential service is an
internal path dependency and is distributed in the GitHub source archive.
Publishing it separately becomes relevant only when registry installation is a
supported release path.
