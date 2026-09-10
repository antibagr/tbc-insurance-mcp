//! Health-insurance entities returned by MCP tools.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// An exact decimal amount paired with an ISO 4217 currency code.
///
/// The API adapter validates `amount` as a non-negative decimal string. A
/// string preserves the insurer's monetary precision without floating-point
/// rounding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Money {
    /// Decimal amount such as `"125.50"`.
    pub amount: String,
    /// ISO 4217 currency code such as `"GEL"`.
    pub currency: String,
}

/// The period during which a health policy provides coverage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CoveragePeriod {
    /// Inclusive start date normalized to an ISO 8601 calendar date.
    pub starts_on: Option<String>,
    /// Inclusive end date normalized to an ISO 8601 calendar date.
    pub ends_on: Option<String>,
}

/// A member-visible health policy with direct identifiers minimized.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HealthPolicy {
    /// Exact opaque identifier used by later tools.
    pub policy_id: String,
    /// Last four visible characters of the policy number.
    pub policy_number_ending: String,
    /// Policy coverage period.
    pub coverage_period: CoveragePeriod,
}

/// Reimbursement payment destination with the account number minimized.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReimbursementAccountSummary {
    /// Last four visible characters of the bank-account number.
    pub account_ending: String,
}

/// Member-facing details for the assigned family doctor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FamilyDoctorSummary {
    /// Doctor's member-facing name.
    pub name: String,
    /// Medical specialty reported by TBC.
    pub specialty: Option<String>,
    /// Clinic where the doctor practices.
    pub clinic_name: Option<String>,
    /// Clinic address reported by TBC.
    pub clinic_address: Option<String>,
}

/// Minimized health-account information used for care and reimbursement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CoverageSummary {
    /// Available reimbursement accounts with only their endings exposed.
    pub reimbursement_accounts: Vec<ReimbursementAccountSummary>,
    /// Assigned family doctor when TBC supplies one.
    pub family_doctor: Option<FamilyDoctorSummary>,
}

/// A person entitled to benefits under a health policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CoveredMember {
    /// Exact opaque member identifier used by later tools.
    pub member_id: String,
    /// Exact opaque policy identifier.
    pub policy_id: String,
    /// A masked label suitable for choosing between covered family members.
    pub member_label: String,
}

/// The reported limit for one covered benefit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BenefitLimit {
    /// TBC reports a numeric limit without identifying its unit.
    UnspecifiedUnit {
        /// Exact non-negative decimal text; currency and service counts are unverified.
        value: String,
    },
    /// TBC explicitly reports the benefit as unlimited.
    Unlimited,
}

/// A health care item or service category covered by a policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CoverageBenefit {
    /// Exact opaque benefit identifier used by claim tools.
    pub benefit_id: String,
    /// Member-facing benefit name.
    pub name: String,
    /// Reported benefit limit, retaining uncertainty about its unit.
    pub limit: BenefitLimit,
    /// Percentage of eligible cost paid by TBC; undisplayed rates remain unspecified.
    pub insurer_coverage_percent: Option<u8>,
    /// Amount already consumed from the benefit limit.
    pub used_amount: Option<Money>,
}

/// Summary of a payment record for medical care already received.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MedicalClaimSummary {
    /// Exact opaque claim identifier used by the claim-detail tool.
    pub claim_id: String,
    /// Service date supplied by TBC.
    pub service_date: String,
    /// Clinic, pharmacy, or other provider that delivered care.
    pub provider_name: String,
    /// Covered benefit under which the claim was processed.
    pub benefit_name: String,
    /// Amount billed for the claim when TBC supplies it.
    pub billed_amount: Option<Money>,
    /// Amount paid by TBC when TBC supplies it.
    pub insurer_paid_amount: Option<Money>,
}

/// One medical service within a claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ClaimLine {
    /// Member-facing service name.
    pub service_name: String,
    /// Number of units recorded by TBC.
    pub quantity: String,
    /// Amount billed for this service line when available.
    pub billed_amount: Option<Money>,
    /// Amount paid by TBC for this service line when available.
    pub insurer_paid_amount: Option<Money>,
}

/// The business subtype of a member-initiated service request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MemberRequestKind {
    /// Repayment of eligible medical costs already paid by the member.
    ReimbursementClaim,
    /// Provider-facing guarantee that TBC will pay for approved eligible care.
    GuaranteeOfPayment,
    /// A new upstream request type awaiting a reviewed mapping.
    Unknown {
        /// Numeric TBC request-type code.
        code: i64,
    },
}

/// Normalized processing state of a member service request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MemberRequestStatus {
    /// TBC accepted a newly created request.
    New,
    /// TBC is reviewing the request.
    InReview,
    /// TBC has placed processing on hold.
    OnHold,
    /// TBC denied the request.
    Denied,
    /// Processing finished without a more specific mapped outcome.
    Completed,
    /// TBC approved or confirmed the request.
    Approved,
    /// A new upstream state awaiting a reviewed mapping.
    Unknown {
        /// Numeric TBC status code.
        code: i64,
    },
}

/// Summary of a reimbursement or guarantee-of-payment request tracked by TBC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MemberRequestSummary {
    /// Exact opaque request identifier used by the request-detail tool.
    pub request_id: String,
    /// Member-visible request number.
    pub request_number: String,
    /// Reimbursement-claim or guarantee-of-payment subtype.
    pub kind: MemberRequestKind,
    /// Normalized processing state.
    pub status: MemberRequestStatus,
    /// Creation date supplied by TBC.
    pub submitted_on: String,
    /// Number of attached documents, excluding their private filenames.
    pub attachment_count: Option<usize>,
}

/// Detailed state of one reimbursement or guarantee-of-payment request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MemberRequestDetail {
    /// Status, subtype, dates, and minimized attachment metadata.
    pub summary: MemberRequestSummary,
    /// Text submitted by the member when present.
    pub member_comment: Option<String>,
    /// Untrusted account text returned by TBC when present.
    pub insurer_comment: Option<String>,
    /// Reimbursement amount reported by TBC when present.
    pub reimbursement_amount: Option<Money>,
}

/// The kind of care covered by a medical referral.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReferralKind {
    /// Consultation with a medical specialist.
    SpecialistConsultation,
    /// Laboratory, imaging, or other diagnostic service.
    DiagnosticService,
    /// A request for a named medication to be prescribed.
    MedicationPrescription,
    /// A new upstream referral kind awaiting a reviewed mapping.
    Unknown {
        /// Numeric TBC referral-type code.
        code: i64,
    },
}

/// A numbered referral for a provider service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MedicalReferral {
    /// Member-visible referral number used for support conversations.
    pub referral_number: String,
    /// Specialist, diagnostic, medication, or an explicit unknown category.
    pub kind: ReferralKind,
    /// Covered service named on the referral.
    pub service_name: String,
    /// Clinic or other network provider named on the referral.
    pub provider_name: String,
    /// Last date on which TBC reports the referral as valid.
    pub valid_until: Option<String>,
    /// Member-facing status supplied by TBC.
    pub status: String,
}

/// A provider service that can be selected for a referral request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReferralServiceOption {
    /// Exact opaque service identifier used by later workflow steps.
    pub service_id: String,
    /// Member-facing service name.
    pub service_name: String,
}

/// A contracted health care organization in TBC's provider network.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NetworkProvider {
    /// Exact opaque provider identifier.
    pub provider_id: String,
    /// Member-facing provider name.
    pub name: String,
}

/// City available in TBC's health-service search.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ServiceCity {
    /// Exact opaque city identifier used by later search tools.
    pub city_id: String,
    /// Member-facing city name.
    pub name: String,
}

/// A service offered through TBC's online appointment search.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BookableService {
    /// Numeric booking-service ID; referral-service IDs use a separate catalog.
    pub service_id: u32,
    /// Service name reported by TBC.
    pub name: String,
}

/// A physical clinic or branch in the online appointment catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProviderLocation {
    /// Numeric booking-branch ID; referral-provider IDs use a separate catalog.
    pub location_id: u32,
    /// Member-facing location name.
    pub name: String,
    /// City reported by TBC.
    pub city: Option<String>,
    /// Clinic address reported by TBC.
    pub address: Option<String>,
}

/// A clinician returned for a service, branch, and requested calendar date.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AvailableClinician {
    /// Numeric clinician identifier from the appointment catalog.
    pub clinician_id: u32,
    /// Numeric branch identifier checked against the requested branch.
    pub location_id: u32,
    /// Clinician name reported by TBC.
    pub name: String,
}

/// A currently available interval; availability alone does not establish coverage or reservation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AppointmentSlot {
    /// Exact slot identifier when supplied; some clinics return null.
    pub slot_id: Option<String>,
    /// Numeric clinician identifier verified against the requested clinician.
    pub clinician_id: u32,
    /// Selected service from the query that produced this interval.
    pub service_id: u32,
    /// Selected branch from the query that produced this interval.
    pub location_id: u32,
    /// Start date and time, preserving TBC's explicit UTC offset.
    pub starts_at: String,
    /// End date and time, preserving TBC's explicit UTC offset.
    pub ends_at: String,
}

/// Current reservations and historical appointment records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AppointmentBookings {
    /// Current bookings reported by TBC's native notification feed.
    pub active: Vec<AppointmentBooking>,
    /// Historical bookings, kept separate from active reservations.
    pub history: Vec<AppointmentBooking>,
}

/// A minimized reservation record; upstream text is untrusted account data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AppointmentBooking {
    /// Exact opaque reservation reference returned by TBC.
    pub booking_id: String,
    /// Clinician name supplied by TBC.
    pub clinician_name: String,
    /// Clinic name supplied by TBC.
    pub location_name: String,
    /// TBC's original date text, kept separately when the time is another field.
    pub scheduled_date: Option<String>,
    /// TBC's original start-time text, falling back to its date when absent.
    /// No timezone or precision is inferred from these display fields.
    pub scheduled_time: Option<String>,
}

/// Summary of a message in the member's TBC Insurance inbox.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InboxMessageSummary {
    /// Exact opaque message identifier.
    pub message_id: String,
    /// Related member-request number when TBC supplies one.
    pub linked_request_number: Option<String>,
    /// Untrusted account-text preview when the list response contains text.
    pub preview: Option<String>,
    /// Message date and time supplied by TBC.
    pub sent_at: String,
    /// Whether TBC marks the message as unread.
    pub unread: bool,
}

/// The party that authored an insurer-thread message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MessageAuthor {
    /// TBC Insurance or its operator.
    Insurer,
    /// The connected member.
    Member,
    /// A new upstream direction code awaiting a reviewed mapping.
    Unknown {
        /// Numeric TBC direction code.
        code: i64,
    },
}

/// One full message from the member's TBC Insurance inbox or thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InboxMessage {
    /// Exact opaque message identifier.
    pub message_id: String,
    /// Untrusted account-text subject supplied on a message-detail response.
    pub subject: Option<String>,
    /// Exact untrusted account text when present.
    pub body: Option<String>,
    /// Sender label supplied by TBC when present.
    pub sender_name: Option<String>,
    /// Message date and time supplied by TBC.
    pub sent_at: String,
    /// Whether TBC marks the message as unread.
    pub unread: bool,
    /// Insurer, member, or an explicit unknown upstream direction.
    pub author: MessageAuthor,
    /// Whether the detail response currently permits a reply.
    pub can_reply: Option<bool>,
    /// Related request status when the detail response supplies one.
    pub linked_request_status: Option<MemberRequestStatus>,
}

/// Current number of unread insurer inbox messages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UnreadMessageCount {
    /// Number reported by TBC.
    pub count: u64,
}
