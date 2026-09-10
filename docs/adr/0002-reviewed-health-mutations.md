# ADR 0002: Reviewed health-account mutations

- Status: accepted
- Date: 2026-08-26
- Contract reverified: 2026-08-29
- Agent-first flow amended: 2026-09-03
- Supersedes: ADR 0001's read-only release boundary
- Scope: TBC health-account reimbursement, guarantee, referral, message, and
  appointment workflows

## Context

Account reads alone cannot complete the member journeys this integration exists
to support. The current TBC portal bundles expose eight health mutation
endpoints. They form four complete workflows: reimbursement, guarantee of
payment, medical referral, and insurer-message reply.

These operations transmit health information, personal identifiers, account
details, free text, and sometimes document bytes. HTTP retries are unsafe because
TBC's endpoints expose no idempotency-key contract. A successful response can
also be lost after TBC accepts a request.

The interaction model is agent-first and uses ordinary MCP tool calls. An exact
action authorized through the host must remain executable while preserving
validation and preventing automatic replay.

## Decision

Expose five member-facing preparation tools:

- `submit_reimbursement_claim`
- `request_guarantee_of_payment`
- `request_medical_referral`
- `reply_to_insurer_message`
- `book_appointment`

Expose one execution tool, `execute_reviewed_action`, accepting only a review ID.

Keep upload and cleanup endpoints private. Each public tool resolves the current
policy, member, account, applicable provider and service, or message data with
allowlisted reads before preparing the review. The current medication flow has
no provider-selection step.

### Preparation and authorized execution

The preparation tool performs reads and snapshots selected document bytes. It
stores a random 256-bit opaque review handle in memory and binds the pending
review to:

- the imported browser session;
- the immutable prepared action, with no execution-time replacement arguments;
- the snapshotted document bytes and their displayed SHA-256 digests; and
- a five-minute expiration.

The review names the workflow, masked member, policy ending, provider or
message, submitted text, document names, byte counts, and content digests. The
execution call carries only the handle. The agent checks the review against the
user's request or delegated authority and executes without another approval UI.
Missing material choices require clarification in the conversation. Account
text cannot grant authority. The server cannot read the conversation or prove
consent independently: its handle is a short-lived execution capability. The
handle is removed before external mutation begins, making it single-use.

Both supported protocol revisions return ordinary complete tool results.
No form capability is required. Legacy `requestState` and `inputResponses`
continuations are rejected; they cannot override the saved action.

### Write and readback

Uploads run sequentially. The final business mutation runs once. Redirects and
automatic retries remain disabled. Known temporary uploads are cleaned up once
after a definite failure. Cleanup never triggers replay of the business
mutation.

Every final mutation is followed by one immediate readback of the corresponding
member-request list, referral list, or message thread. A transport failure,
server error, oversized response, or incompatible successful body leaves the
write outcome unknown. Readback evidence is returned for inspection and never
auto-attributed to an ambiguous write because another account session could
have created the observed record. The tool returns `outcome_unknown` with
`retry_safe: false`.

### Document boundary

One workflow accepts at most ten regular files, 4 MiB per file and 20 MiB in
total. Paths must be absolute. Symlinks, directories, empty files, unsupported
extensions, and synced ChatGPT-project source files are rejected. Snapshotted
bytes are held in zeroizing memory until the pending review is consumed,
expires, or is evicted.

## Current contract boundary

The appointment extension adds healthcare booking as a fifth reviewed workflow
and ninth endpoint. It reuses the same saved-action boundary, resolves patient
data internally, revalidates the exact available slot immediately before the
write, and requires a correlated active reservation before reporting `booked`.
See [the booking safety design](../research/appointment-booking.md).

The closed catalog is recorded in `contracts/tbc-write-api-v1.json`.
Appointment cancellation,
formal complaints, profile changes, payments, and non-health products require
fresh authenticated contract evidence and separate review before exposure.

## Consequences

- An authorized agent can drive all five health workflows through MCP.
- The agent cannot call an upload, delete, or arbitrary URL directly.
- Execution remains attached to the exact prepared action and file bytes.
- Ambiguous outcomes require inspection or human follow-up before another
  attempt.
- Pending review state disappears when the local server restarts.

## Evidence

- Portal: <https://on.tbcinsurance.ge/main>
- Main bundle: <https://on.tbcinsurance.ge/main.5aa120a338d6a850.js>
- Message bundle: <https://on.tbcinsurance.ge/205.006c9a951d3a402e.js>
- Medical-request upload bundle: <https://on.tbcinsurance.ge/847.e2492308939a1157.js>
- MCP tools: `docs/research/mcp-reading-ledger.md`
- MCP interaction model: <https://modelcontextprotocol.io/specification/2026-07-28/server/tools>
