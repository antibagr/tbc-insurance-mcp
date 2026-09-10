# TBC health-insurance domain language

- Status: accepted baseline for the reviewed-workflow Rust server
- Contract basis: minimized public sources and authenticated observations
- Scope: health coverage, claims, member requests, referrals, provider search,
  appointment search and booking, and insurer messages

## Purpose

TBC's public web client contains implementation names translated directly from
Georgian. Those names describe controllers and historical UI code. The Rust
package uses health-insurance language that remains clear to an English-speaking
member and keeps the original names inside the private wire adapter.

The model follows current TBC screens and bundle behavior, TBC's published
coverage material, and established US health-insurance terminology. A mapping
stays provisional until a sanitized contract observation confirms its
field-level shape.

## Member journeys

| Situation | Domain workflow | Outcome |
| --- | --- | --- |
| The member already paid a medical bill | `ReimbursementClaim` | TBC reviews medical and financial documents and decides how much to repay. |
| A provider expects payment from TBC | `GuaranteeOfPaymentRequest` | TBC reviews the case and may guarantee direct payment to the provider; its UI calls the document a guarantee letter. |
| The member needs a specialist, diagnostic service, or medication | `MedicalReferral` | TBC issues or records a numbered referral with a status and validity period. |
| The member wants a visit or test time | `AppointmentSearch` | The member chooses a covered service, location, clinician, and available slot. |
| The member wants to reserve a selected visit | `AppointmentBooking` | After reviewing the exact visit and financial uncertainty, the member authorizes one booking attempt and checks the reservation reference. |
| The member wants to understand past care and payment | `MedicalClaim` | The claim shows the provider, covered benefit, billed amount, insurer-paid amount, and service lines. |
| The member disputes a denial or payment decision | `CoverageAppeal` | A genuine coverage appeal challenges an insurer decision. No verified read endpoint currently exposes this workflow. |
| The member reports poor service or conduct | `Complaint` | A complaint records dissatisfaction through TBC's complaint channels. |

TBC describes two payment paths in its published medical guidance. Network
care can be paid directly to the provider within the policy limit. Other care
can be reviewed for reimbursement after complete medical and financial
documents arrive.

## Canonical terms

| Domain term | TBC UI or wire term | Meaning and rule | Confidence |
| --- | --- | --- | --- |
| `HealthPolicy` | policy, `polId`, `policyNumber` | The health-insurance contract and its coverage period. The policyholder and covered member can be different people. | verified |
| `CoverageSummary` | insured info, `GetInsuredInfo` | Minimized health-account details: reimbursement-account endings and the assigned family doctor. Member identity and contact fields stay private. | contract-observed |
| `FamilyDoctorSummary` | family doctor | The assigned doctor's name, specialty, clinic, and clinic address. | contract-observed |
| `CoveredMember` | insured person, `InsuredPerson`, `insuredPN` | A person entitled to use benefits under a health policy. Personal-number fields stay inside the wire adapter. | domain concept; the former selection endpoint is retired |
| `CoverageBenefit` | `Risk`, policy risk | A covered health care item or service category with a reported limit, insurer payment percentage, and used amount. | contract-observed |
| `BenefitLimit` | limit, sublimit, `amount` | A numeric limit retains its exact decimal text with an unspecified unit. The current response and policy screen do not identify whether it is money or a service count. An explicit unlimited benefit remains distinct. | contract-observed; units unverified |
| `InsurerCoveragePercent` | co-payment heading, `norate` | TBC's share of eligible cost according to the portal label. The wire adapter validates one `%` suffix and the 0–100 range; zero, dash, and empty rates remain unspecified because the portal shows a dash for zero-rate category rows. This percentage alone cannot establish a final patient bill. | contract-observed |
| `BenefitUsage` | used limit, `usedLimit` | The amount already consumed from a benefit limit. | verified |
| `MedicalClaim` | claim | A payment record for care received. A summary contains the date, provider, benefit, billed amount, and insurer-paid amount. | verified |
| `ClaimLine` | received service | One service within a medical claim, including quantity, billed amount, and insurer-paid amount. | verified |
| `MemberRequestSummary` | `MedRequest`, request | A member-initiated workflow tracked by request number, kind, status, and creation date. It is the shared summary for reimbursement and guarantee-of-payment requests. | verified |
| `MemberRequestDetail` | medical request details | One request summary plus bounded member and insurer comments, reimbursement amount, and attachment count. Filenames and pre-signed URLs stay private. | contract-observed |
| `ReimbursementClaim` | reimbursement request, type `0` | A request to repay eligible medical costs already paid by the member. A payout account belongs only to this subtype. | verified |
| `GuaranteeOfPaymentRequest` | guarantee letter request, type `1` | A request for TBC to guarantee direct payment to a provider for approved eligible care. “Guarantee letter” remains a user-facing TBC search term. | verified category; exact TBC obligations require a current fixture |
| `DirectBilling` | insurer pays provider | TBC pays the provider for eligible care, subject to policy limits and the member's cost share. A guarantee of payment supports this workflow. | verified category |
| `PriorAuthorization` | pre-approval | An insurer's approval before care is delivered. This stays separate because current TBC evidence does not establish that every guarantee-of-payment request is prior authorization. | reserved distinction |
| `MemberRequestStatus` | status IDs `1` through `6` | Normalized states are `new`, `in_review`, `on_hold`, `denied`, `completed`, and `approved`. Unknown codes remain explicit. | verified codes; normalized wording provisional |
| `MedicalReferral` | `Appeal`, `RequestAppeal`, მიმართვა | A numbered referral for a provider service, with status and validity period. The word `Appeal` is reserved for disputes over insurer decisions. | verified |
| `ReferralKind::SpecialistConsultation` | `Specialist`, მიმართვა სპეციალისტთან | Referral for a specialist consultation. | verified |
| `ReferralKind::DiagnosticService` | `Research`, მიმართვა კვლევაზე | Referral for a diagnostic test, imaging study, laboratory test, or other diagnostic service. | verified |
| `ReferralKind::MedicationPrescription` | `Medication`, მიმართვა მედიკამენტზე | A request for a named medication to be prescribed. The current flow asks the member which medication they want prescribed. | verified category; precise fulfillment semantics provisional |
| `ReferralServiceOption` | provider appeal | A service that a selected provider can fulfill for the selected referral kind. | verified from current screen flow |
| `NetworkProvider` | provider, partner clinic | A contracted health care organization in TBC's network. | verified |
| `ProviderLocation` | clinic, clinic branch | A physical care location belonging to a network provider. | verified |
| `ServiceCity` | `citiesId`, `citiesName` | A city available in TBC's health-service search. | contract-observed |
| `CoveredService` | health care service | A service that can be searched or booked under a policy. | verified |
| `Clinician` | doctor | A health professional available for a covered service. | verified |
| `AppointmentSlot` | schedule, doctor slot | One bookable time interval for a clinician, service, and provider location. | verified |
| `AppointmentBooking` | booking, mobile notification | An existing appointment record. | provisional response mapping |
| `InboxMessageSummary` | notification list item | A minimized insurer-inbox item with its linked request number, preview, date, and unread state. | contract-observed |
| `InboxMessage` | message details, thread item | Bounded member-facing subject, body, sender, direction, date, reply availability, and linked request status. Direct client and object identifiers stay private. | contract-observed |
| `CoverageAppeal` | pretension or complaint in some TBC copy | A challenge to a denied or underpaid coverage decision. It never maps from `RequestAppeal`. | domain reservation |

## Wire-to-domain boundary

Literal names exist only in the private `api::wire` module. Public Rust types,
tool names, parameters, errors, and structured results use the canonical terms.

| Wire operation or field | Domain operation or field |
| --- | --- |
| `GetPolicies` | `list_health_policies` |
| `polId`, `insObject`, `toFromDate`, `toToDate` | `HealthPolicy` with an opaque ID, policy-number ending, and ISO coverage period |
| `GetPolicyRisks` | `list_coverage_benefits` |
| `risk`, `tagid` | `CoverageBenefit`, `benefit_id` |
| `amount` | `limit`: exact numeric value with an unspecified unit, or explicitly unlimited |
| `norate` | `insurer_coverage_percent`; undisplayed rates remain `null` |
| `usedLimit` | `used_amount` |
| `GetClaimsByRisk` | `list_medical_claims` |
| `servicePriceSum` | `billed_amount` |
| `insurerAmount` | `insurer_paid_amount` |
| `MedRequest` | `MemberRequestSummary` |
| `GetInsuredInfo` | `get_coverage_summary` |
| `GetListByInsurerPN`, `GetMedRequestDetails` | `list_member_requests`, `get_member_request` |
| `RequestAppeal`, `Appeal` | `MedicalReferral` |
| `ProviderAppeal` | `ReferralServiceOption` |
| `GetProviders`, `GetProviderAppeals`, `GetAppeals` | referral providers, service options, and referral history |
| `Research` | `DiagnosticService` |
| `GetCities` | `list_service_cities` |
| `DoctorBooking` | appointment search and booking |
| `CreateHealthcareServiceBooking` | `book_appointment`; reviewed reservation with internally verified patient data |
| `Mobile/GetNotifications`, `medicalBookingDetails` | `list_appointment_bookings`; current reservations separated from history |
| `Schedule` | `AppointmentSlot` |
| `GetMessagesByClientId`, `GetUnreadMessagesCount`, `GetMessageDetails`, `GetMessagesByParentId` | inbox summaries, unread count, message details, and thread entries |
| `Mobile` notification endpoint | existing appointment records |

No generic HTTP path reaches the client. Every request is selected from the
closed read or mutation endpoint enum. Public mutation tools compose those
endpoints into a complete member workflow; upload and cleanup primitives stay
internal.

## Mutation workflow rules

- `submit_reimbursement_claim` requires a covered policy, a verified payout
  account, a short explanation, and at least one supporting document.
- `request_guarantee_of_payment` requires a covered policy, a short explanation,
  and at least one supporting document for planned care.
- `request_medical_referral` uses a provider and service for specialist care.
  Diagnostic-service requests use a provider and supporting document.
  Medication requests skip provider selection. Both documented request types
  accept an optional explanation.
- `reply_to_insurer_message` binds the reply to a current message and its
  server-provided conversation object.
- Every workflow snapshots file bytes, presents an exact review, consumes the
  review once through `execute_reviewed_action`, sends one final write, and
  performs one immediate readback.
- A transport failure or server error on a write has an unknown outcome. The
  workflow stops after readback and never retries that write automatically.

## Output and privacy invariants

- Operational policy, benefit, claim, request, referral, provider, clinician,
  and slot IDs remain exact so later tools can reference them.
- National identifiers, bearer values, bank-account numbers, phone numbers,
  email addresses, and member or policyholder names stay out of structured tool
  results.
- Medical service names, coverage limits, insurer payment percentages, billed amounts,
  insurer-paid amounts, statuses, and dates remain available when relevant to
  the requested insurance task.
- Attachment contents and upstream file URLs remain unavailable. Attachment
  count can be exposed; filenames require a task-specific privacy review.
- Upstream JSON never crosses the MCP boundary directly.
- Unknown enum codes and fields produce explicit, sanitized compatibility
  states. They never silently acquire a business meaning.
- Current policy, claim, request, referral, appointment, and message data uses
  live reads. The first release has no persistent response cache.

## Evidence

- Current portal: <https://on.tbcinsurance.ge/main>
- Current web bundle: <https://on.tbcinsurance.ge/main.5aa120a338d6a850.js>
- Current medical module: <https://on.tbcinsurance.ge/282.2695de098823d0ac.js>
- TBC health product page:
  <https://tbcinsurance.ge/en/business/business-health-insurance/health-insurance>
- TBC direct-payment and reimbursement guidance:
  <https://on.tbcinsurance.ge/blog/entry/4>
- HealthCare.gov claim definition:
  <https://www.healthcare.gov/glossary/claim/>
- HealthCare.gov benefits definition:
  <https://www.healthcare.gov/glossary/benefits/>
- HealthCare.gov network definition:
  <https://www.healthcare.gov/glossary/network/>
- HealthCare.gov referral definition:
  <https://www.healthcare.gov/glossary/referral/>
- HealthCare.gov prior-authorization definition:
  <https://www.healthcare.gov/glossary/prior-authorization/>
- Cigna provider guarantee-of-payment request:
  <https://www.cignaglobal.com/health-care-providers/request-gop>
- Allianz direct-billing and guarantee-of-payment workflow:
  <https://www.allianzcare.com/en/support/insurance-partner-resources/agent360-learning-center/individuals.html>
