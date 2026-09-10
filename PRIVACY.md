# Privacy

TBC Insurance MCP processes sensitive health-account data locally. This
document describes the project's implemented data flow. The insurer's own
processing is governed by its
[privacy policy](https://tbcinsurance.ge/ge/insurance-operation-privacy-policy).

## Data handled

Depending on the selected tool, the server can receive policy, benefit, claim,
request, referral, provider, appointment, and inbox data from the user's
account. A reviewed action can also handle user-selected text and medical or
financial documents.

Public tool results omit bearer credentials, national identifiers, full bank
accounts, phone numbers, email addresses, raw member and policyholder names,
attachment bytes, and upstream attachment URLs. Action reviews can include a
masked member label so the user can distinguish covered people. Explicit detail
tools can return member-facing claim, request, or message text because that
content is needed for the requested account workflow.

Document paths are MCP arguments and can reveal local directory or account
names to the MCP host and model provider. Use a private directory with neutral
file and folder names, then review the host's retention settings.

## Storage and retention

| Data | Location | Retention |
| --- | --- | --- |
| Bearer credential and renewal metadata | macOS login Keychain through the signed credential service | Until expiry, rejection, replacement, or user removal |
| Non-macOS development credential | MCP process memory | Until process exit |
| Normalized account responses | MCP process memory and the requesting host | For the active call and according to the host's own retention |
| Pending action and selected document bytes | MCP process memory | Up to five minutes, execution, eviction, or process exit |
| Sanitized diagnostics | stderr and the MCP host | According to the host's configuration |
| Opt-in debug diagnostics | user-selected private directory | Until the user removes the files |

Pending documents and transient authorization buffers are zeroized when their
owners are dropped. Operating systems and allocators can still create copies
outside the program's direct control.

## Network destinations

Normal account traffic uses the fixed TBC profile API base recorded in the
contracts. Session import uses a temporary `127.0.0.1` listener and never
binds to an external interface. The credential service performs local Keychain
operations and no network requests.

The project contains no analytics, advertising, crash-reporting service, or
project-operated backend.

## Diagnostics

Default diagnostics contain stable operation names, local trace IDs, sanitized
outcomes, and elapsed time. They exclude request and response bodies, bearer
values, medical text, personal identifiers, and upstream URLs.

Opt-in debug files can reveal timing, tool names, HTTP status classes, and local
workflow activity. Store them in a private directory, review them before
sharing, and remove them when they are no longer needed.

## User responsibilities

- Connect only an account you are authorized to access.
- Review your MCP host's storage, model-provider, telemetry, and conversation
  retention settings.
- Keep review IDs and session-import tickets private.
- Never attach real account data or credentials to a public issue.
- Use official insurer channels for privacy requests, corrections, account
  access, emergencies, and authoritative coverage decisions.
