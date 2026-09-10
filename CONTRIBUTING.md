# Contributing

Thank you for helping improve TBC Insurance MCP. Security and data minimization
take priority because this project handles health-account information and
state-changing insurance workflows.

By participating, you agree to follow the
[Code of Conduct](CODE_OF_CONDUCT.md). Security vulnerabilities belong in the
private process described in [SECURITY.md](SECURITY.md).

## Sensitive-data rule

Use synthetic fixtures and fictional identities in every issue, test, commit,
screenshot, log, and pull request.

Never submit:

- bearer credentials, cookies, one-time codes, passwords, or Keychain records;
- names, personal numbers, policy numbers, claim or request references;
- phone numbers, email addresses, bank accounts, medical documents, diagnoses,
  message text, or appointment details from a real account;
- raw authenticated responses, browser archives, debug logs, or screenshots of
  the portal.

If sensitive material enters Git, stop sharing the branch and report it through
the private security channel. Deleting the visible line does not remove it from
Git history.

## Development setup

Install Rust 1.98.0 with `rustup`. The fast local checks are:

```sh
cargo fmt --all -- --check
cargo fmt --manifest-path fuzz/Cargo.toml -- --check
cargo fmt --manifest-path crates/session-vault/Cargo.toml -- --check
cargo check --locked --all-targets --all-features
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
cargo test --locked --manifest-path crates/session-vault/Cargo.toml --all-targets
RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps --all-features
```

CI also checks spelling, Markdown, TOML, dependency policy, advisories, workflow
security, and secret scanning. See [docs/quality.md](docs/quality.md) for the
complete command list and slower scheduled gates.

## Change process

1. Open an issue for a new upstream endpoint or state-changing workflow.
2. Describe the insurance-domain behavior and safety invariants.
3. Keep upstream names in the wire adapter and use clear insurance terminology
   in public MCP types.
4. Add a synthetic regression test that fails without the change.
5. Update the machine-readable contract and user documentation when the tool
   surface or data interpretation changes.
6. Run the focused tests, then the full fast checks.

New write operations need an exact review, short expiry, session binding,
single-use execution, live precondition rechecks, one submission attempt, and a
readback strategy for ambiguous responses.

## Contract evidence

Upstream web APIs are undocumented and can drift. Commit only the smallest
sanitized facts needed to explain a contract: method, fixed path, field names,
types, nullability, and interpretation. Replace all account values with obvious
synthetic canaries. Raw portal bundles and authenticated payloads stay outside
the repository.

## Pull requests

Keep each pull request focused. Include:

- the user-visible behavior and threat boundary;
- tests run and their results;
- documentation or contract changes;
- confirmation that all fixtures and prose are synthetic;
- known limitations that remain relevant.

Maintainers may request a smaller change when a standard-library or existing
project mechanism covers the same need.

## License

Contributions intentionally submitted for inclusion are accepted under the
[Apache License 2.0](LICENSE), as described by section 5 of that license.
