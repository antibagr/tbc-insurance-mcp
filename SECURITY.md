# Security Policy

## Supported versions

Security fixes are applied to the current `main` branch and the newest tagged
release. Older releases should be upgraded before a report is evaluated.

## Report a vulnerability privately

Use a
[GitHub private vulnerability report](https://github.com/antibagr/tbc-insurance-mcp/security/advisories/new).

Include the affected revision, impact, a minimal synthetic reproduction, and
any suggested mitigation. Remove all real account data, credentials, medical
information, and insurer correspondence. If private reporting is unavailable,
open a short public issue requesting a private contact channel and include no
vulnerability details.

Good-faith reports will receive a best-effort acknowledgement and status
updates. Publication timing will be coordinated around a fix when practical.

## High-priority reports

Examples include:

- credential exposure, persistence outside the protected store, or leakage to
  MCP output, stdout, diagnostics, process arguments, or files;
- authentication or code-signing bypasses;
- action-review forgery, reuse, cross-session execution, or argument replacement;
- automatic replay after an ambiguous write;
- arbitrary upstream URLs, redirects, unbounded responses, or request injection;
- path traversal, symlink acceptance, or unintended document disclosure;
- personal or medical data in fixtures, documentation, releases, or Git history;
- compromised release or GitHub Actions dependencies.

Insurer availability, coverage decisions, rejected claims, appointment supply,
and changes to the upstream portal can still be important bugs. Report those
through the normal issue tracker after removing all account-specific data.

Suspected vulnerabilities in TBC Insurance systems belong under the insurer's
[responsible disclosure policy](https://tbcinsurance.ge/static/file/202412185307-responsible-disclosure-policy.pdf).
Do not test with accounts or data you are not authorized to access, and do not
publish upstream vulnerability details in this repository.

## Credential incident

If a real credential is exposed, end the affected portal session through
official TBC Insurance channels and contact the insurer when appropriate.
Assume any copied credential is compromised. Do not paste it into an issue,
chat, email, or reproduction.

## Design assumptions

The server runs locally under the user's operating-system account. The MCP host
and local browser-automation process are trusted to handle execution
capabilities and the one-time import ticket. Insurer-supplied content is
untrusted. The upstream APIs are undocumented and outside this project's
control.
