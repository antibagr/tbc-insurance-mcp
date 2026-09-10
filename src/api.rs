//! Closed TBC health-account endpoint catalog and wire normalization.

pub(crate) mod appointments;
pub(crate) mod bookings;
mod client;
mod mutation;
mod wire;

use std::fmt;

use serde_json::Value;

use crate::domain::{
    ClaimLine, CoverageBenefit, CoverageSummary, HealthPolicy, InboxMessage, InboxMessageSummary,
    MedicalClaimSummary, MedicalReferral, MemberRequestDetail, MemberRequestSummary,
    NetworkProvider, ReferralServiceOption, ServiceCity, UnreadMessageCount,
};

pub use appointments::{
    SlotQuery as AppointmentSlotQuery, normalize_clinicians as normalize_available_clinicians,
    normalize_locations as normalize_appointment_locations,
    normalize_services as normalize_bookable_services,
    normalize_slots as normalize_appointment_slots,
};
pub(crate) use client::{TbcClient, TbcMutationError, TbcReadError};
pub(crate) use mutation::{MutationEndpoint, UploadedDocument};
pub use wire::ResponseCompatibilityError;

/// HTTP verb used by a query-style TBC endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HttpMethod {
    /// HTTP GET.
    Get,
    /// HTTP POST used by a query-style endpoint with a JSON body.
    Post,
}

/// A fully selected request from the read-only endpoint allowlist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReadRequest {
    operation: &'static str,
    method: HttpMethod,
    path_and_query: String,
    body: Option<Value>,
}

/// A fully selected request from the reviewed mutation allowlist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MutationRequest {
    operation: &'static str,
    method: HttpMethod,
    path: String,
    body: Value,
}

impl MutationRequest {
    /// Stable business operation name used in sanitized diagnostics.
    #[must_use]
    pub(crate) const fn operation(&self) -> &'static str {
        self.operation
    }

    /// HTTP verb required by this mutation.
    #[must_use]
    pub(crate) const fn method(&self) -> HttpMethod {
        self.method
    }

    /// Fixed allowlisted mutation path without caller-controlled URL text.
    #[must_use]
    pub(crate) fn path(&self) -> &str {
        &self.path
    }

    /// Exact JSON body built by the private wire adapter.
    #[must_use]
    pub(crate) const fn body(&self) -> &Value {
        &self.body
    }
}

impl ReadRequest {
    /// Stable business operation name used in sanitized diagnostics.
    #[must_use]
    pub(crate) const fn operation(&self) -> &'static str {
        self.operation
    }

    /// HTTP verb required by this read operation.
    #[must_use]
    pub(crate) const fn method(&self) -> HttpMethod {
        self.method
    }

    /// Fixed allowlisted path with validated parameters and bounded query values.
    #[must_use]
    pub(crate) fn path_and_query(&self) -> &str {
        &self.path_and_query
    }

    /// Query-style JSON body required by a subset of POST reads.
    #[must_use]
    pub(crate) const fn body(&self) -> Option<&Value> {
        self.body.as_ref()
    }
}

/// One read-only operation supported by the current TBC contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReadEndpoint {
    /// List the member's health policies.
    ListPolicies,
    /// Read the member's current coverage summary.
    GetCoverageSummary,
    /// List covered benefits and usage for one policy.
    ListCoverageBenefits {
        /// Validated policy identifier.
        policy_id: EndpointId,
    },
    /// List claim summaries processed under one covered benefit.
    ListMedicalClaimsForBenefit {
        /// Validated policy identifier.
        policy_id: EndpointId,
        /// Validated covered-benefit identifier.
        benefit_id: EndpointId,
    },
    /// Read service lines for one medical claim.
    GetMedicalClaim {
        /// Validated claim identifier.
        claim_id: EndpointId,
    },
    /// List reimbursement and guarantee-of-payment requests.
    ListMemberRequests {
        /// One-based page number.
        page: u32,
        /// Number of records requested, from 1 through 100.
        page_size: u8,
    },
    /// Read one reimbursement or guarantee-of-payment request.
    GetMemberRequest {
        /// Validated request identifier.
        request_id: EndpointId,
    },
    /// List providers that can fulfill referral requests.
    ListReferralProviders,
    /// List referral services available from one provider.
    ListReferralServiceOptions {
        /// Validated provider identifier.
        provider_id: EndpointId,
    },
    /// List existing medical referrals.
    ListReferrals,
    /// List cities used in provider and appointment search.
    ListServiceCities,
    /// List insurer inbox messages.
    ListInboxMessages,
    /// Count unread insurer inbox messages.
    CountUnreadMessages,
    /// Read one insurer inbox message.
    GetInboxMessage {
        /// Validated message identifier.
        message_id: EndpointId,
    },
    /// List the replies in one insurer message thread.
    ListMessageThread {
        /// Validated parent-message identifier.
        parent_message_id: EndpointId,
    },
}

impl ReadEndpoint {
    /// Materialize the selected operation into its closed HTTP contract.
    #[must_use]
    pub(crate) fn request(&self) -> ReadRequest {
        wire::read_request(self)
    }

    /// Create a bounded member-request listing operation.
    ///
    /// # Errors
    ///
    /// Returns [`ApiContractError::InvalidPage`] when `page` is zero and
    /// [`ApiContractError::InvalidPageSize`] when `page_size` is outside 1 through 100.
    pub(crate) fn list_member_requests(
        page: u32,
        page_size: u16,
    ) -> Result<Self, ApiContractError> {
        if page == 0 {
            return Err(ApiContractError::InvalidPage);
        }
        let page_size = u8::try_from(page_size)
            .ok()
            .filter(|size| (1..=100).contains(size))
            .ok_or(ApiContractError::InvalidPageSize)?;
        Ok(Self::ListMemberRequests { page, page_size })
    }
}

/// Validated TBC identifier safe to place in one URL path segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointId(String);

impl EndpointId {
    /// Validate an opaque identifier for use in exactly one URL path segment.
    ///
    /// # Errors
    ///
    /// Returns [`ApiContractError::InvalidIdentifier`] when the value is empty,
    /// longer than 128 bytes, or contains an unsafe path character.
    pub fn parse(value: impl Into<String>) -> Result<Self, ApiContractError> {
        let value = value.into();
        let valid = !value.is_empty()
            && value.len() <= 128
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
        if valid {
            Ok(Self(value))
        } else {
            Err(ApiContractError::InvalidIdentifier)
        }
    }

    /// Return the validated identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for EndpointId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Sanitized validation error for a read request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiContractError {
    /// An identifier was empty, oversized, or unsafe for a path segment.
    InvalidIdentifier,
    /// A page number was zero.
    InvalidPage,
    /// A page size fell outside the inclusive range 1 through 100.
    InvalidPageSize,
}

impl fmt::Display for ApiContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidIdentifier => "identifier has an invalid format",
            Self::InvalidPage => "page must be at least 1",
            Self::InvalidPageSize => "page size must be between 1 and 100",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for ApiContractError {}

/// Translate a current TBC policy-benefits response into coverage benefits.
///
/// # Errors
///
/// Returns [`ResponseCompatibilityError`] when the response shape or a field value
/// falls outside the verified read contract.
pub fn normalize_coverage_benefits(
    payload: Value,
) -> Result<Vec<CoverageBenefit>, ResponseCompatibilityError> {
    wire::normalize_coverage_benefits(payload)
}

/// Translate the current TBC policy list into minimized health policies.
///
/// # Errors
///
/// Returns [`ResponseCompatibilityError`] when the response shape or a field value
/// falls outside the verified read contract.
pub fn normalize_health_policies(
    payload: Value,
) -> Result<Vec<HealthPolicy>, ResponseCompatibilityError> {
    wire::normalize_health_policies(payload)
}

/// Translate current TBC claim summaries into medical-claim summaries.
///
/// # Errors
///
/// Returns [`ResponseCompatibilityError`] when the response shape or a field value
/// falls outside the verified read contract.
pub fn normalize_claim_summaries(
    payload: Value,
) -> Result<Vec<MedicalClaimSummary>, ResponseCompatibilityError> {
    wire::normalize_claim_summaries(payload)
}

/// Translate current TBC claim-detail rows into medical claim lines.
///
/// # Errors
///
/// Returns [`ResponseCompatibilityError`] when the response shape or a field value
/// falls outside the verified read contract.
pub fn normalize_claim_lines(payload: Value) -> Result<Vec<ClaimLine>, ResponseCompatibilityError> {
    wire::normalize_claim_lines(payload)
}

/// Translate the current TBC insured-info response into a minimized coverage summary.
///
/// # Errors
///
/// Returns [`ResponseCompatibilityError`] when the response shape or a field value
/// falls outside the verified read contract.
pub fn normalize_coverage_summary(
    payload: Value,
) -> Result<CoverageSummary, ResponseCompatibilityError> {
    wire::normalize_coverage_summary(payload)
}

/// Translate the current TBC member-request list into typed request summaries.
///
/// # Errors
///
/// Returns [`ResponseCompatibilityError`] when the response shape or a field value
/// falls outside the verified read contract.
pub fn normalize_member_requests(
    payload: Value,
) -> Result<Vec<MemberRequestSummary>, ResponseCompatibilityError> {
    wire::normalize_member_requests(payload)
}

/// Translate one current TBC member-request response into typed request details.
///
/// # Errors
///
/// Returns [`ResponseCompatibilityError`] when the response shape or a field value
/// falls outside the verified read contract.
pub fn normalize_member_request(
    payload: Value,
) -> Result<MemberRequestDetail, ResponseCompatibilityError> {
    wire::normalize_member_request(payload)
}

/// Translate the current TBC referral-provider list into network providers.
///
/// # Errors
///
/// Returns [`ResponseCompatibilityError`] when the response shape or a field value
/// falls outside the verified read contract.
pub fn normalize_referral_providers(
    payload: Value,
) -> Result<Vec<NetworkProvider>, ResponseCompatibilityError> {
    wire::normalize_referral_providers(payload)
}

/// Translate current provider-specific referral services into selection options.
///
/// # Errors
///
/// Returns [`ResponseCompatibilityError`] when the response shape or a field value
/// falls outside the verified read contract.
pub fn normalize_referral_service_options(
    payload: Value,
) -> Result<Vec<ReferralServiceOption>, ResponseCompatibilityError> {
    wire::normalize_referral_service_options(payload)
}

/// Translate the current TBC referral history into medical referrals.
///
/// # Errors
///
/// Returns [`ResponseCompatibilityError`] when the response shape or a field value
/// falls outside the verified read contract.
pub fn normalize_medical_referrals(
    payload: Value,
) -> Result<Vec<MedicalReferral>, ResponseCompatibilityError> {
    wire::normalize_medical_referrals(payload)
}

/// Translate the current TBC health-service city list.
///
/// # Errors
///
/// Returns [`ResponseCompatibilityError`] when the response shape or a field value
/// falls outside the verified read contract.
pub fn normalize_service_cities(
    payload: Value,
) -> Result<Vec<ServiceCity>, ResponseCompatibilityError> {
    wire::normalize_service_cities(payload)
}

/// Translate the current TBC inbox listing into minimized message summaries.
///
/// # Errors
///
/// Returns [`ResponseCompatibilityError`] when the response shape or a field value
/// falls outside the verified read contract.
pub fn normalize_inbox_message_summaries(
    payload: Value,
) -> Result<Vec<InboxMessageSummary>, ResponseCompatibilityError> {
    wire::normalize_inbox_message_summaries(payload)
}

/// Translate TBC's current unread-message counter.
///
/// # Errors
///
/// Returns [`ResponseCompatibilityError`] when the response is not a non-negative integer.
pub fn normalize_unread_message_count(
    payload: Value,
) -> Result<UnreadMessageCount, ResponseCompatibilityError> {
    wire::normalize_unread_message_count(payload)
}

/// Translate one current TBC inbox-message detail response.
///
/// # Errors
///
/// Returns [`ResponseCompatibilityError`] when the response shape or a field value
/// falls outside the verified read contract.
pub fn normalize_inbox_message(payload: Value) -> Result<InboxMessage, ResponseCompatibilityError> {
    wire::normalize_inbox_message(payload)
}

/// Translate the current TBC message-thread response.
///
/// # Errors
///
/// Returns [`ResponseCompatibilityError`] when the response shape or a field value
/// falls outside the verified read contract.
pub fn normalize_message_thread(
    payload: Value,
) -> Result<Vec<InboxMessage>, ResponseCompatibilityError> {
    wire::normalize_message_thread(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_ids_reject_path_injection() {
        assert!(EndpointId::parse("policy-42").is_ok());
        assert!(EndpointId::parse("policy_42").is_ok());
        assert_eq!(
            EndpointId::parse("."),
            Err(ApiContractError::InvalidIdentifier)
        );
        assert_eq!(
            EndpointId::parse(".."),
            Err(ApiContractError::InvalidIdentifier)
        );
        assert_eq!(
            EndpointId::parse("../account/GetUserProfile"),
            Err(ApiContractError::InvalidIdentifier)
        );
        assert_eq!(
            EndpointId::parse("claim/42"),
            Err(ApiContractError::InvalidIdentifier)
        );
    }

    #[test]
    fn policy_benefits_use_business_identifier_name() {
        let endpoint = ReadEndpoint::ListCoverageBenefits {
            policy_id: EndpointId::parse("42").expect("valid policy ID"),
        };
        assert_eq!(endpoint.request().operation(), "list_coverage_benefits");
    }

    #[test]
    fn referral_service_options_use_domain_language() {
        let endpoint = ReadEndpoint::ListReferralServiceOptions {
            provider_id: EndpointId::parse("clinic_9").expect("valid provider ID"),
        };
        assert_eq!(
            endpoint.request().operation(),
            "list_referral_service_options"
        );
    }

    #[test]
    fn request_paging_is_bounded() {
        assert!(ReadEndpoint::list_member_requests(1, 100).is_ok());
        assert_eq!(
            ReadEndpoint::list_member_requests(0, 20),
            Err(ApiContractError::InvalidPage)
        );
        assert_eq!(
            ReadEndpoint::list_member_requests(1, 101),
            Err(ApiContractError::InvalidPageSize)
        );
    }
}
