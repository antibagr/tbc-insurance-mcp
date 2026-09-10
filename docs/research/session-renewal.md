# Session lifecycle and credential isolation

## Session import

`prepare_tbc_session_import` creates a temporary loopback listener with a
random 256-bit ticket secret. It binds only to `127.0.0.1`, accepts bounded
newline-delimited JSON, expires after 120 seconds, and permits at most four
attempts.

A local browser-control process observes a bearer credential from an
authenticated request to the allowlisted TBC API host and sends it directly to
the loopback channel. The credential never enters an MCP argument or result.

## Storage

On macOS, a small separately installed and signed credential service owns the
Keychain item. The MCP and service verify exact code-signing identifiers and the
same dedicated local certificate. Routine MCP updates preserve the service
binary so its Keychain identity remains stable.

The saved record is limited to the bearer credential, expiry metadata,
credential generation, and renewal state. Profile, policy, claim, appointment,
message, and document data are excluded.

Non-macOS development builds keep the session in process memory for the current
run.

## Renewal

The MCP checks session expiry while it is running and renews inside a bounded
window. A successful response contributes only the replacement bearer and
expiry metadata.

Each attempt records a renewal marker before network activity. Cancellation,
transport ambiguity, or process termination cannot silently convert the same
marker into another automatic attempt after restart. A rejected or failed
renewal stops automatic renewal for that credential. A still-valid credential
can remain usable until expiry.

A credential generation changes whenever the saved bearer rotates. Running
connections adopt newer generations, and outstanding action reviews become
invalid after a rotation.

## Cross-process coordination

A private per-user lock serializes renewal across MCP processes. The lock file
contains no credential data. Keychain compare-and-swap operations prevent a
stale process from overwriting or deleting a newer generation.

A TBC authentication rejection marks the matching generation expired. Deletion
uses the expected generation, so an old response cannot remove a newer session.

## Failure behavior

- `login_required` means the TBC session is absent, expired, or rejected.
- `local_session_unavailable` means the protected local record could not be
  accessed. The record is preserved for recovery.
- renewal failures use sanitized diagnostic codes and omit response bodies.
- computer sleep, process exit, or an outage across the expiry window can
  require another portal login.

## Verification

Synthetic tests exercise expiry boundaries, stale responses, concurrent
renewal, process cancellation, compare-and-swap behavior, malformed records,
prompt suppression, and credential-service peer authorization. Native signing
acceptance is also checked on macOS because unsigned unit tests cannot establish
a valid production peer.
