# Appointment-booking safety design

## Scope

The public booking surface consists of read tools for services, locations,
clinicians, slots, and current reservations, plus the reviewed
`book_appointment` preparation tool. Appointment cancellation is outside the
current contract.

The upstream booking interface is undocumented. Its minimized method, path, and
field evidence is recorded in `contracts/tbc-read-api-v1.json` and
`contracts/tbc-write-api-v1.json`.

## Preparation

The caller selects:

- a current policy;
- a bookable healthcare service;
- a provider location;
- a clinician;
- an exact start and end interval;
- an optional slot ID.

The server resolves patient fields from the current policy-to-profile
relationship. Patient names and identifiers are never accepted as MCP
arguments. Preparation validates the selected catalog objects, confirms the
interval belongs to the selected clinician and location, and stores an
immutable action under a random review ID.

Slot IDs can be null. The exact interval, service, location, and clinician
remain mandatory. Timestamps preserve explicit offsets. Reservation reads can
also contain separate display-date and display-time strings whose timezone is
unspecified; the server retains both strings without inventing a timestamp.

## Execution

`execute_reviewed_action` accepts only the review ID. Immediately before the
write, the server:

1. consumes the single-use review;
2. verifies the review still belongs to the active session;
3. resolves the current policy-to-profile relationship again;
4. re-queries the exact appointment availability;
5. rejects changed patient, service, location, clinician, or interval data;
6. submits one request.

The HTTP client disables redirects and automatic retries.

## Readback

After the submission response, the server lists current reservations and looks
for a newly correlated active booking. The match uses stable appointment
details and handles an absent slot ID.

A recognized creation reference or a correlated new reservation can establish a
successful booking according to the implementation's response rules. Any
unrecognized or conflicting result remains uncertain and reports
`retry_safe: false`. Callers inspect current reservations or contact the
insurer before considering another submission.

Price, copay, policy coverage, and final eligibility remain unverified by the
booking workflow.

## Tests

Synthetic tests cover:

- null and present slot IDs;
- changed availability and patient relationships;
- cross-session, expired, forged, and reused reviews;
- duplicate or pre-existing reservations;
- missing, malformed, and conflicting creation responses;
- separate date and time fields;
- one-attempt behavior after cancellation or transport ambiguity.
