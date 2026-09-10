# Pull request

## Summary

Describe the user-visible change and the smallest reason it is needed.

## Safety boundary

Describe any impact on credentials, personal or medical data, upstream requests,
review capabilities, write replay, file access, or diagnostics.

## Verification

List the focused and full checks you ran.

## Checklist

- [ ] Tests use only synthetic identities, payloads, documents, and credentials.
- [ ] Commits and attachments contain no real account data, secrets, or private
  logs.
- [ ] New behavior has a regression test.
- [ ] Tool or contract changes include documentation updates.
- [ ] State-changing behavior preserves review, single-use, one-attempt, and
  readback safeguards.
- [ ] Formatting, Clippy, and relevant tests pass.
