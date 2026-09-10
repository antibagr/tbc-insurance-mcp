//! Private translations from TBC payload vocabulary into domain entities.

use std::fmt;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::domain::{
    BenefitLimit, ClaimLine, CoverageBenefit, CoveragePeriod, CoverageSummary, FamilyDoctorSummary,
    HealthPolicy, InboxMessage, InboxMessageSummary, MedicalClaimSummary, MedicalReferral,
    MemberRequestDetail, MemberRequestKind, MemberRequestStatus, MemberRequestSummary,
    MessageAuthor, Money, NetworkProvider, ReferralKind, ReferralServiceOption,
    ReimbursementAccountSummary, ServiceCity, UnreadMessageCount,
};

use super::{EndpointId, HttpMethod, ReadEndpoint, ReadRequest};

const HEALTH_CURRENCY: &str = "GEL";
const MAX_LABEL_BYTES: usize = 512;
const MAX_MEMBER_TEXT_BYTES: usize = 16 * 1024;
const MAX_PREVIEW_CHARACTERS: usize = 200;

pub(super) fn read_request(endpoint: &ReadEndpoint) -> ReadRequest {
    match endpoint {
        ReadEndpoint::ListPolicies => get("list_policies", "/api/Medical/GetPolicies"),
        ReadEndpoint::GetCoverageSummary => {
            get("get_coverage_summary", "/api/Medical/GetInsuredInfo")
        }
        ReadEndpoint::ListCoverageBenefits { policy_id } => get(
            "list_coverage_benefits",
            format!("/api/Medical/GetPolicyRisks/{policy_id}"),
        ),
        ReadEndpoint::ListMedicalClaimsForBenefit {
            policy_id,
            benefit_id,
        } => get(
            "list_medical_claims",
            format!("/api/Medical/GetClaimsByRisk/{policy_id}/{benefit_id}"),
        ),
        ReadEndpoint::GetMedicalClaim { claim_id } => get(
            "get_medical_claim",
            format!("/api/Medical/GetClaimDetails/{claim_id}"),
        ),
        ReadEndpoint::ListMemberRequests { page, page_size } => get(
            "list_member_requests",
            format!("/api/MedRequest/GetListByInsurerPN?page={page}&pageSize={page_size}"),
        ),
        ReadEndpoint::GetMemberRequest { request_id } => get(
            "get_member_request",
            format!("/api/MedRequest/GetMedRequestDetails/{request_id}"),
        ),
        ReadEndpoint::ListReferralProviders => post(
            "list_referral_providers",
            "/api/RequestAppeal/GetProviders",
            json!({}),
        ),
        ReadEndpoint::ListReferralServiceOptions { provider_id } => post(
            "list_referral_service_options",
            "/api/RequestAppeal/GetProviderAppeals",
            json!({ "clinicId": provider_id.as_str() }),
        ),
        ReadEndpoint::ListReferrals => get("list_referrals", "/api/RequestAppeal/GetAppeals"),
        ReadEndpoint::ListServiceCities => {
            get("list_service_cities", "/api/DoctorBooking/GetCities")
        }
        ReadEndpoint::ListInboxMessages => get(
            "list_inbox_messages",
            "/api/Notification/GetMessagesByClientId",
        ),
        ReadEndpoint::CountUnreadMessages => get(
            "count_unread_messages",
            "/api/Notification/GetUnreadMessagesCount",
        ),
        ReadEndpoint::GetInboxMessage { message_id } => get(
            "get_inbox_message",
            format!("/api/Notification/GetMessageDetails/{message_id}"),
        ),
        ReadEndpoint::ListMessageThread { parent_message_id } => get(
            "list_message_thread",
            format!("/api/Notification/GetMessagesByParentId/{parent_message_id}"),
        ),
    }
}

pub(super) fn get(operation: &'static str, path_and_query: impl Into<String>) -> ReadRequest {
    ReadRequest {
        operation,
        method: HttpMethod::Get,
        path_and_query: path_and_query.into(),
        body: None,
    }
}

pub(super) fn post(
    operation: &'static str,
    path_and_query: impl Into<String>,
    body: Value,
) -> ReadRequest {
    ReadRequest {
        operation,
        method: HttpMethod::Post,
        path_and_query: path_and_query.into(),
        body: Some(body),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HealthPolicyWire {
    pol_id: Value,
    ins_object: Value,
    #[serde(default)]
    to_from_date: Option<String>,
    #[serde(default)]
    to_to_date: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PolicyBenefitsWire {
    risks: Vec<CoverageBenefitWire>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CoverageBenefitWire {
    tagid: Value,
    name: String,
    amount: Value,
    norate: Value,
    used_limit: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MedicalClaimSummaryWire {
    rsid: Value,
    date: String,
    provider_name: String,
    risk_name: String,
    service_price_sum: Value,
    insurer_amount: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaimLineWire {
    name: String,
    count: Value,
    service_price: Value,
    insurer_amount: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CoverageSummaryWire {
    #[serde(default)]
    accounts_list: Vec<ReimbursementAccountWire>,
    #[serde(default)]
    family_doctor: Option<FamilyDoctorWire>,
}

#[derive(Debug, Deserialize)]
struct ReimbursementAccountWire {
    accunt: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FamilyDoctorWire {
    doctor_name: String,
    specialty: Option<String>,
    clinic_name: Option<String>,
    full_address: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MemberRequestListWire {
    med_requests: Vec<MemberRequestWire>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MemberRequestWire {
    id: Value,
    identifier_number: Value,
    current_status: i64,
    req_date: String,
    #[serde(rename = "type")]
    kind: i64,
    #[serde(default)]
    files: Option<Vec<Value>>,
    #[serde(default)]
    insured_comment: Option<String>,
    #[serde(default)]
    operator_comment: Option<String>,
    #[serde(default)]
    reimbursement_amount_str: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReferralProviderWire {
    clinic_id: Value,
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReferralServiceOptionWire {
    service_id: Value,
    service_name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MedicalReferralWire {
    appeal_number: Value,
    appeal_name: String,
    clinic_name: String,
    expiration_date: Option<String>,
    status_name: String,
    appeal_type: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ServiceCityWire {
    cities_id: Value,
    cities_name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InboxMessageSummaryWire {
    id: Value,
    date_created: String,
    is_read: bool,
    #[serde(default)]
    message_text: Option<String>,
    #[serde(default)]
    identifier_number: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InboxMessageWire {
    id: Value,
    date_created: String,
    is_read: bool,
    direction: i64,
    #[serde(default)]
    message_subject: Option<String>,
    #[serde(default)]
    message_text: Option<String>,
    #[serde(default)]
    sender_name: Option<String>,
    #[serde(default)]
    can_answer: Option<bool>,
    #[serde(default)]
    med_request_status: Option<i64>,
}

pub(super) fn normalize_health_policies(
    payload: Value,
) -> Result<Vec<HealthPolicy>, ResponseCompatibilityError> {
    let wire: Vec<HealthPolicyWire> = serde_json::from_value(payload)
        .map_err(|_| ResponseCompatibilityError::InvalidShape("health policies"))?;
    wire.into_iter().map(HealthPolicy::try_from).collect()
}

pub(super) fn normalize_coverage_benefits(
    payload: Value,
) -> Result<Vec<CoverageBenefit>, ResponseCompatibilityError> {
    let wire: PolicyBenefitsWire = serde_json::from_value(payload)
        .map_err(|_| ResponseCompatibilityError::InvalidShape("coverage benefits"))?;
    wire.risks
        .into_iter()
        .map(CoverageBenefit::try_from)
        .collect()
}

pub(super) fn normalize_claim_summaries(
    payload: Value,
) -> Result<Vec<MedicalClaimSummary>, ResponseCompatibilityError> {
    let wire: Vec<MedicalClaimSummaryWire> = serde_json::from_value(payload)
        .map_err(|_| ResponseCompatibilityError::InvalidShape("claim summaries"))?;
    wire.into_iter()
        .map(MedicalClaimSummary::try_from)
        .collect()
}

pub(super) fn normalize_claim_lines(
    payload: Value,
) -> Result<Vec<ClaimLine>, ResponseCompatibilityError> {
    let wire: Vec<ClaimLineWire> = serde_json::from_value(payload)
        .map_err(|_| ResponseCompatibilityError::InvalidShape("claim lines"))?;
    wire.into_iter().map(ClaimLine::try_from).collect()
}

pub(super) fn normalize_coverage_summary(
    payload: Value,
) -> Result<CoverageSummary, ResponseCompatibilityError> {
    let wire: CoverageSummaryWire = serde_json::from_value(payload)
        .map_err(|_| ResponseCompatibilityError::InvalidShape("coverage summary"))?;
    CoverageSummary::try_from(wire)
}

pub(super) fn normalize_member_requests(
    payload: Value,
) -> Result<Vec<MemberRequestSummary>, ResponseCompatibilityError> {
    let wire: MemberRequestListWire = serde_json::from_value(payload)
        .map_err(|_| ResponseCompatibilityError::InvalidShape("member requests"))?;
    wire.med_requests
        .iter()
        .map(MemberRequestSummary::try_from)
        .collect()
}

pub(super) fn normalize_member_request(
    payload: Value,
) -> Result<MemberRequestDetail, ResponseCompatibilityError> {
    let wire: MemberRequestWire = serde_json::from_value(payload)
        .map_err(|_| ResponseCompatibilityError::InvalidShape("member request"))?;
    MemberRequestDetail::try_from(wire)
}

pub(super) fn normalize_referral_providers(
    payload: Value,
) -> Result<Vec<NetworkProvider>, ResponseCompatibilityError> {
    let wire: Vec<ReferralProviderWire> = serde_json::from_value(payload)
        .map_err(|_| ResponseCompatibilityError::InvalidShape("referral providers"))?;
    wire.into_iter().map(NetworkProvider::try_from).collect()
}

pub(super) fn normalize_referral_service_options(
    payload: Value,
) -> Result<Vec<ReferralServiceOption>, ResponseCompatibilityError> {
    let wire: Vec<ReferralServiceOptionWire> = serde_json::from_value(payload)
        .map_err(|_| ResponseCompatibilityError::InvalidShape("referral services"))?;
    wire.into_iter()
        .map(ReferralServiceOption::try_from)
        .collect()
}

pub(super) fn normalize_medical_referrals(
    payload: Value,
) -> Result<Vec<MedicalReferral>, ResponseCompatibilityError> {
    let wire: Vec<MedicalReferralWire> = serde_json::from_value(payload)
        .map_err(|_| ResponseCompatibilityError::InvalidShape("medical referrals"))?;
    wire.into_iter().map(MedicalReferral::try_from).collect()
}

pub(super) fn normalize_service_cities(
    payload: Value,
) -> Result<Vec<ServiceCity>, ResponseCompatibilityError> {
    let wire: Vec<ServiceCityWire> = serde_json::from_value(payload)
        .map_err(|_| ResponseCompatibilityError::InvalidShape("service cities"))?;
    wire.into_iter().map(ServiceCity::try_from).collect()
}

pub(super) fn normalize_inbox_message_summaries(
    payload: Value,
) -> Result<Vec<InboxMessageSummary>, ResponseCompatibilityError> {
    let wire: Vec<InboxMessageSummaryWire> = serde_json::from_value(payload)
        .map_err(|_| ResponseCompatibilityError::InvalidShape("inbox messages"))?;
    wire.into_iter()
        .map(InboxMessageSummary::try_from)
        .collect()
}

pub(super) fn normalize_unread_message_count(
    payload: Value,
) -> Result<UnreadMessageCount, ResponseCompatibilityError> {
    let Value::Number(number) = payload else {
        return Err(ResponseCompatibilityError::InvalidField(
            "unread message count",
        ));
    };
    number
        .as_u64()
        .map(|count| UnreadMessageCount { count })
        .ok_or(ResponseCompatibilityError::InvalidField(
            "unread message count",
        ))
}

pub(super) fn normalize_inbox_message(
    payload: Value,
) -> Result<InboxMessage, ResponseCompatibilityError> {
    let wire: InboxMessageWire = serde_json::from_value(payload)
        .map_err(|_| ResponseCompatibilityError::InvalidShape("inbox message"))?;
    InboxMessage::try_from(wire)
}

pub(super) fn normalize_message_thread(
    payload: Value,
) -> Result<Vec<InboxMessage>, ResponseCompatibilityError> {
    let wire: Vec<InboxMessageWire> = serde_json::from_value(payload)
        .map_err(|_| ResponseCompatibilityError::InvalidShape("message thread"))?;
    wire.into_iter().map(InboxMessage::try_from).collect()
}

pub(super) const fn member_request_kind(code: i64) -> MemberRequestKind {
    match code {
        0 => MemberRequestKind::ReimbursementClaim,
        1 => MemberRequestKind::GuaranteeOfPayment,
        code => MemberRequestKind::Unknown { code },
    }
}

pub(super) const fn member_request_status(code: i64) -> MemberRequestStatus {
    match code {
        1 => MemberRequestStatus::New,
        2 => MemberRequestStatus::InReview,
        3 => MemberRequestStatus::OnHold,
        4 => MemberRequestStatus::Denied,
        5 => MemberRequestStatus::Completed,
        6 => MemberRequestStatus::Approved,
        code => MemberRequestStatus::Unknown { code },
    }
}

pub(super) const fn referral_kind(code: i64) -> ReferralKind {
    match code {
        0 => ReferralKind::SpecialistConsultation,
        1 => ReferralKind::DiagnosticService,
        2 => ReferralKind::MedicationPrescription,
        code => ReferralKind::Unknown { code },
    }
}

const fn message_author(code: i64) -> MessageAuthor {
    match code {
        1 => MessageAuthor::Insurer,
        2 => MessageAuthor::Member,
        code => MessageAuthor::Unknown { code },
    }
}

impl TryFrom<CoverageBenefitWire> for CoverageBenefit {
    type Error = ResponseCompatibilityError;

    fn try_from(wire: CoverageBenefitWire) -> Result<Self, Self::Error> {
        let limit = match scalar_text(&wire.amount, "benefit limit")?.as_str() {
            "-" => BenefitLimit::Unlimited,
            value if is_non_negative_decimal(value) => BenefitLimit::UnspecifiedUnit {
                value: value.to_owned(),
            },
            _ => return Err(ResponseCompatibilityError::InvalidField("benefit limit")),
        };
        Ok(Self {
            benefit_id: identifier(&wire.tagid, "benefit ID")?,
            name: label(&wire.name, "benefit name")?,
            limit,
            // The portal displays zero-rate category rows with an unspecified-rate dash.
            insurer_coverage_percent: optional_percent(&wire.norate, "insurer coverage")?
                .filter(|percent| *percent != 0),
            used_amount: optional_money(&wire.used_limit, "used benefit amount")?,
        })
    }
}

impl TryFrom<HealthPolicyWire> for HealthPolicy {
    type Error = ResponseCompatibilityError;

    fn try_from(wire: HealthPolicyWire) -> Result<Self, Self::Error> {
        Ok(Self {
            policy_id: identifier(&wire.pol_id, "policy ID")?,
            policy_number_ending: private_ending(&wire.ins_object, "policy number")?,
            coverage_period: CoveragePeriod {
                starts_on: policy_date(wire.to_from_date.as_deref(), "policy start date")?,
                ends_on: policy_date(wire.to_to_date.as_deref(), "policy end date")?,
            },
        })
    }
}

impl TryFrom<CoverageSummaryWire> for CoverageSummary {
    type Error = ResponseCompatibilityError;

    fn try_from(wire: CoverageSummaryWire) -> Result<Self, Self::Error> {
        let reimbursement_accounts = wire
            .accounts_list
            .into_iter()
            .map(|account| {
                private_ending(&account.accunt, "reimbursement account")
                    .map(|account_ending| ReimbursementAccountSummary { account_ending })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let family_doctor = wire
            .family_doctor
            .map(FamilyDoctorSummary::try_from)
            .transpose()?;
        Ok(Self {
            reimbursement_accounts,
            family_doctor,
        })
    }
}

impl TryFrom<FamilyDoctorWire> for FamilyDoctorSummary {
    type Error = ResponseCompatibilityError;

    fn try_from(wire: FamilyDoctorWire) -> Result<Self, Self::Error> {
        Ok(Self {
            name: label(&wire.doctor_name, "family doctor name")?,
            specialty: optional_label(wire.specialty.as_deref(), "family doctor specialty")?,
            clinic_name: optional_label(wire.clinic_name.as_deref(), "family doctor clinic")?,
            clinic_address: optional_label(
                wire.full_address.as_deref(),
                "family doctor clinic address",
            )?,
        })
    }
}

impl TryFrom<&MemberRequestWire> for MemberRequestSummary {
    type Error = ResponseCompatibilityError;

    fn try_from(wire: &MemberRequestWire) -> Result<Self, Self::Error> {
        Ok(Self {
            request_id: identifier(&wire.id, "request ID")?,
            request_number: scalar_label(&wire.identifier_number, "request number")?,
            kind: member_request_kind(wire.kind),
            status: member_request_status(wire.current_status),
            submitted_on: label(&wire.req_date, "request submission date")?,
            attachment_count: wire.files.as_ref().map(Vec::len),
        })
    }
}

impl TryFrom<MemberRequestWire> for MemberRequestDetail {
    type Error = ResponseCompatibilityError;

    fn try_from(wire: MemberRequestWire) -> Result<Self, Self::Error> {
        let summary = MemberRequestSummary::try_from(&wire)?;
        Ok(Self {
            summary,
            member_comment: optional_member_text(
                wire.insured_comment.as_deref(),
                "member request comment",
            )?,
            insurer_comment: optional_member_text(
                wire.operator_comment.as_deref(),
                "insurer request comment",
            )?,
            reimbursement_amount: optional_money_value(
                wire.reimbursement_amount_str.as_ref(),
                "reimbursement amount",
            )?,
        })
    }
}

impl TryFrom<ReferralProviderWire> for NetworkProvider {
    type Error = ResponseCompatibilityError;

    fn try_from(wire: ReferralProviderWire) -> Result<Self, Self::Error> {
        Ok(Self {
            provider_id: identifier(&wire.clinic_id, "provider ID")?,
            name: label(&wire.name, "provider name")?,
        })
    }
}

impl TryFrom<ReferralServiceOptionWire> for ReferralServiceOption {
    type Error = ResponseCompatibilityError;

    fn try_from(wire: ReferralServiceOptionWire) -> Result<Self, Self::Error> {
        Ok(Self {
            service_id: identifier(&wire.service_id, "referral service ID")?,
            service_name: label(&wire.service_name, "referral service name")?,
        })
    }
}

impl TryFrom<MedicalReferralWire> for MedicalReferral {
    type Error = ResponseCompatibilityError;

    fn try_from(wire: MedicalReferralWire) -> Result<Self, Self::Error> {
        Ok(Self {
            referral_number: scalar_label(&wire.appeal_number, "referral number")?,
            kind: referral_kind(wire.appeal_type),
            service_name: label(&wire.appeal_name, "referral service name")?,
            provider_name: label(&wire.clinic_name, "referral provider name")?,
            valid_until: policy_date(wire.expiration_date.as_deref(), "referral expiration date")?,
            status: label(&wire.status_name, "referral status")?,
        })
    }
}

impl TryFrom<ServiceCityWire> for ServiceCity {
    type Error = ResponseCompatibilityError;

    fn try_from(wire: ServiceCityWire) -> Result<Self, Self::Error> {
        Ok(Self {
            city_id: identifier(&wire.cities_id, "city ID")?,
            name: label(&wire.cities_name, "city name")?,
        })
    }
}

impl TryFrom<InboxMessageSummaryWire> for InboxMessageSummary {
    type Error = ResponseCompatibilityError;

    fn try_from(wire: InboxMessageSummaryWire) -> Result<Self, Self::Error> {
        Ok(Self {
            message_id: identifier(&wire.id, "message ID")?,
            linked_request_number: optional_scalar_label(
                wire.identifier_number.as_ref(),
                "linked request number",
            )?,
            preview: message_preview(wire.message_text.as_deref())?,
            sent_at: label(&wire.date_created, "message date")?,
            unread: !wire.is_read,
        })
    }
}

impl TryFrom<InboxMessageWire> for InboxMessage {
    type Error = ResponseCompatibilityError;

    fn try_from(wire: InboxMessageWire) -> Result<Self, Self::Error> {
        Ok(Self {
            message_id: identifier(&wire.id, "message ID")?,
            subject: optional_label(wire.message_subject.as_deref(), "message subject")?,
            body: optional_member_text(wire.message_text.as_deref(), "message body")?,
            sender_name: optional_label(wire.sender_name.as_deref(), "message sender")?,
            sent_at: label(&wire.date_created, "message date")?,
            unread: !wire.is_read,
            author: message_author(wire.direction),
            can_reply: wire.can_answer,
            linked_request_status: wire.med_request_status.map(member_request_status),
        })
    }
}

impl TryFrom<MedicalClaimSummaryWire> for MedicalClaimSummary {
    type Error = ResponseCompatibilityError;

    fn try_from(wire: MedicalClaimSummaryWire) -> Result<Self, Self::Error> {
        Ok(Self {
            claim_id: identifier(&wire.rsid, "claim ID")?,
            service_date: label(&wire.date, "service date")?,
            provider_name: label(&wire.provider_name, "provider name")?,
            benefit_name: label(&wire.risk_name, "benefit name")?,
            billed_amount: optional_money(&wire.service_price_sum, "billed amount")?,
            insurer_paid_amount: optional_money(&wire.insurer_amount, "insurer-paid amount")?,
        })
    }
}

impl TryFrom<ClaimLineWire> for ClaimLine {
    type Error = ResponseCompatibilityError;

    fn try_from(wire: ClaimLineWire) -> Result<Self, Self::Error> {
        Ok(Self {
            service_name: label(&wire.name, "service name")?,
            quantity: scalar_text(&wire.count, "service quantity")?,
            billed_amount: optional_money(&wire.service_price, "line billed amount")?,
            insurer_paid_amount: optional_money(&wire.insurer_amount, "line insurer-paid amount")?,
        })
    }
}

fn identifier(value: &Value, field: &'static str) -> Result<String, ResponseCompatibilityError> {
    let value = scalar_text(value, field)?;
    EndpointId::parse(value)
        .map(|identifier| identifier.as_str().to_owned())
        .map_err(|_| ResponseCompatibilityError::InvalidField(field))
}

pub(super) fn label(
    value: &str,
    field: &'static str,
) -> Result<String, ResponseCompatibilityError> {
    let value = value.trim();
    if value.is_empty() || value.len() > MAX_LABEL_BYTES || value.chars().any(char::is_control) {
        return Err(ResponseCompatibilityError::InvalidField(field));
    }
    Ok(value.to_owned())
}

fn scalar_text(value: &Value, field: &'static str) -> Result<String, ResponseCompatibilityError> {
    match value {
        Value::String(text) => Ok(text.trim().to_owned()),
        Value::Number(number) => Ok(number.to_string()),
        _ => Err(ResponseCompatibilityError::InvalidField(field)),
    }
}

fn optional_money(
    value: &Value,
    field: &'static str,
) -> Result<Option<Money>, ResponseCompatibilityError> {
    let amount = scalar_text(value, field)?;
    if amount == "-" || amount.is_empty() {
        Ok(None)
    } else {
        money(&amount, field).map(Some)
    }
}

fn optional_money_value(
    value: Option<&Value>,
    field: &'static str,
) -> Result<Option<Money>, ResponseCompatibilityError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(value) => optional_money(value, field),
    }
}

fn money(amount: &str, field: &'static str) -> Result<Money, ResponseCompatibilityError> {
    if !is_non_negative_decimal(amount) {
        return Err(ResponseCompatibilityError::InvalidField(field));
    }
    Ok(Money {
        amount: amount.to_owned(),
        currency: HEALTH_CURRENCY.to_owned(),
    })
}

fn optional_percent(
    value: &Value,
    field: &'static str,
) -> Result<Option<u8>, ResponseCompatibilityError> {
    let text = scalar_text(value, field)?;
    if text == "-" || text.is_empty() {
        return Ok(None);
    }
    let digits = text
        .strip_suffix('%')
        .ok_or(ResponseCompatibilityError::InvalidField(field))?;
    if !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ResponseCompatibilityError::InvalidField(field));
    }
    digits
        .parse::<u8>()
        .ok()
        .filter(|percent| *percent <= 100)
        .map(Some)
        .ok_or(ResponseCompatibilityError::InvalidField(field))
}

fn scalar_label(value: &Value, field: &'static str) -> Result<String, ResponseCompatibilityError> {
    label(&scalar_text(value, field)?, field)
}

fn optional_scalar_label(
    value: Option<&Value>,
    field: &'static str,
) -> Result<Option<String>, ResponseCompatibilityError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(value) => scalar_label(value, field).map(Some),
    }
}

fn optional_label(
    value: Option<&str>,
    field: &'static str,
) -> Result<Option<String>, ResponseCompatibilityError> {
    match value.map(str::trim) {
        None | Some("") => Ok(None),
        Some(value) => label(value, field).map(Some),
    }
}

fn optional_member_text(
    value: Option<&str>,
    field: &'static str,
) -> Result<Option<String>, ResponseCompatibilityError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.trim().is_empty() {
        return Ok(None);
    }
    let valid = value.len() <= MAX_MEMBER_TEXT_BYTES
        && !value
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'));
    valid
        .then(|| Some(value.to_owned()))
        .ok_or(ResponseCompatibilityError::InvalidField(field))
}

fn message_preview(value: Option<&str>) -> Result<Option<String>, ResponseCompatibilityError> {
    optional_member_text(value, "message preview").map(|text| {
        text.map(|text| {
            text.chars()
                .take(MAX_PREVIEW_CHARACTERS)
                .collect::<String>()
        })
    })
}

fn private_ending(
    value: &Value,
    field: &'static str,
) -> Result<String, ResponseCompatibilityError> {
    let private_value = scalar_text(value, field)?;
    let character_count = private_value.chars().count();
    if character_count < 4
        || private_value.len() > 128
        || private_value.chars().any(char::is_control)
    {
        return Err(ResponseCompatibilityError::InvalidField(field));
    }
    let mut ending = private_value.chars().rev().take(4).collect::<Vec<_>>();
    ending.reverse();
    Ok(ending.into_iter().collect())
}

fn policy_date(
    value: Option<&str>,
    field: &'static str,
) -> Result<Option<String>, ResponseCompatibilityError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    let date = value.strip_suffix("T00:00:00").unwrap_or(value);
    is_iso_calendar_date(date)
        .then(|| Some(date.to_owned()))
        .ok_or(ResponseCompatibilityError::InvalidField(field))
}

pub(super) fn is_iso_calendar_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit())
    {
        return false;
    }
    let Ok(year) = value[0..4].parse::<u16>() else {
        return false;
    };
    let Ok(month) = value[5..7].parse::<u8>() else {
        return false;
    };
    let Ok(day) = value[8..10].parse::<u8>() else {
        return false;
    };
    let maximum_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => {
            29
        }
        2 => 28,
        _ => return false,
    };
    (1..=maximum_day).contains(&day)
}

fn is_non_negative_decimal(value: &str) -> bool {
    let mut parts = value.split('.');
    let whole = parts.next().unwrap_or_default();
    let fraction = parts.next();
    !whole.is_empty()
        && whole.bytes().all(|byte| byte.is_ascii_digit())
        && fraction.is_none_or(|digits| {
            !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
        })
        && parts.next().is_none()
}

/// A sanitized error raised when TBC returns an incompatible read response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseCompatibilityError {
    /// The top-level payload did not match the current response contract.
    InvalidShape(&'static str),
    /// A response field failed its type, range, or content validation.
    InvalidField(&'static str),
}

impl fmt::Display for ResponseCompatibilityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidShape(name) => {
                write!(formatter, "TBC returned an incompatible {name} response")
            }
            Self::InvalidField(name) => write!(formatter, "TBC returned an invalid {name}"),
        }
    }
}

impl std::error::Error for ResponseCompatibilityError {}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn policy_benefits_map_to_the_current_wire_operation() {
        let endpoint = ReadEndpoint::ListCoverageBenefits {
            policy_id: super::super::EndpointId::parse("42").expect("valid policy ID"),
        };
        assert_eq!(
            endpoint.request(),
            ReadRequest {
                operation: "list_coverage_benefits",
                method: HttpMethod::Get,
                path_and_query: "/api/Medical/GetPolicyRisks/42".to_owned(),
                body: None,
            }
        );
    }

    #[test]
    fn referral_service_options_map_to_the_current_wire_operation() {
        let endpoint = ReadEndpoint::ListReferralServiceOptions {
            provider_id: super::super::EndpointId::parse("clinic_9").expect("valid provider ID"),
        };
        assert_eq!(
            endpoint.request().body(),
            Some(&json!({ "clinicId": "clinic_9" }))
        );
        assert_eq!(
            endpoint.request(),
            ReadRequest {
                operation: "list_referral_service_options",
                method: HttpMethod::Post,
                path_and_query: "/api/RequestAppeal/GetProviderAppeals".to_owned(),
                body: Some(json!({ "clinicId": "clinic_9" })),
            }
        );
    }

    #[test]
    fn referral_providers_use_the_live_query_method() {
        assert_eq!(
            ReadEndpoint::ListReferralProviders.request(),
            ReadRequest {
                operation: "list_referral_providers",
                method: HttpMethod::Post,
                path_and_query: "/api/RequestAppeal/GetProviders".to_owned(),
                body: Some(json!({})),
            }
        );
    }

    #[test]
    fn policy_risks_become_coverage_benefits() {
        let benefits = normalize_coverage_benefits(json!({
            "policy": {"polId": 42},
            "risks": [{
                "tagid": 9,
                "name": "Outpatient care",
                "amount": "1000.00",
                "norate": "80%",
                "usedLimit": "125.50",
                "riskChartColor": "purple"
            }]
        }))
        .expect("valid coverage response");

        assert_eq!(
            benefits,
            vec![CoverageBenefit {
                benefit_id: "9".to_owned(),
                name: "Outpatient care".to_owned(),
                limit: BenefitLimit::UnspecifiedUnit {
                    value: "1000.00".to_owned(),
                },
                insurer_coverage_percent: Some(80),
                used_amount: Some(Money {
                    amount: "125.50".to_owned(),
                    currency: "GEL".to_owned(),
                }),
            }]
        );
    }

    #[test]
    fn coverage_rates_name_the_insurer_share() {
        for (rate, expected) in [
            ("85%", json!(85)),
            ("90%", json!(90)),
            ("100%", json!(100)),
            ("0%", Value::Null),
            ("-", Value::Null),
            ("", Value::Null),
        ] {
            let benefits = normalize_coverage_benefits(json!({
                "risks": [{
                    "tagid": 9, "name": "Consultation", "amount": "1000",
                    "norate": rate, "usedLimit": "0"
                }]
            }))
            .expect("valid benefit rate");
            let output = serde_json::to_value(&benefits[0]).expect("serialized benefit");
            assert_eq!(output.get("insurer_coverage_percent"), Some(&expected));
            assert!(output.get("member_copay_percent").is_none());
        }
    }

    #[test]
    fn coverage_limit_units_remain_unspecified() {
        for (amount, expected) in [
            (json!(0), "0"),
            (json!(1), "1"),
            (json!("2"), "2"),
            (json!("10"), "10"),
            (json!("1000.00"), "1000.00"),
            (json!("12345678901234567890.12"), "12345678901234567890.12"),
        ] {
            let benefits = normalize_coverage_benefits(json!({
                "risks": [{
                    "tagid": 9, "name": "Consultation", "amount": amount,
                    "norate": "90%", "usedLimit": "125.50"
                }]
            }))
            .expect("valid unspecified-unit limit");
            let output = serde_json::to_value(&benefits[0]).expect("serialized benefit");
            assert_eq!(
                output["limit"],
                json!({"kind": "unspecified_unit", "value": expected})
            );
            assert_eq!(
                output["used_amount"],
                json!({"amount": "125.50", "currency": "GEL"})
            );
        }
    }

    #[test]
    fn policy_records_minimize_identity_and_normalize_current_dates() {
        let policies = normalize_health_policies(json!([{
            "polId": 42,
            "insObject": "HEALTH-001234",
            "toFromDate": "2026-03-15T00:00:00",
            "toToDate": "2027-02-28T00:00:00",
            "fullName": "Sensitive Member Name",
            "idn": "sensitive-national-identifier"
        }]))
        .expect("valid policy response");

        assert_eq!(
            policies,
            vec![crate::domain::HealthPolicy {
                policy_id: "42".to_owned(),
                policy_number_ending: "1234".to_owned(),
                coverage_period: crate::domain::CoveragePeriod {
                    starts_on: Some("2026-03-15".to_owned()),
                    ends_on: Some("2027-02-28".to_owned()),
                },
            }]
        );
        let serialized = serde_json::to_string(&policies).expect("serializable policies");
        assert!(!serialized.contains("Sensitive Member Name"));
        assert!(!serialized.contains("sensitive-national-identifier"));
        assert!(!serialized.contains("HEALTH-001234"));
    }

    #[test]
    fn invalid_policy_dates_fail_closed() {
        let result = normalize_health_policies(json!([{
            "polId": 42,
            "insObject": "HEALTH-001234",
            "toFromDate": "2026-02-30T00:00:00",
            "toToDate": "2027-02-28T00:00:00"
        }]));
        assert_eq!(
            result,
            Err(ResponseCompatibilityError::InvalidField(
                "policy start date"
            ))
        );
    }

    #[test]
    fn calendar_date_syntax_and_leap_year_boundaries_are_exact() {
        for valid in ["2024-02-29", "2000-02-29", "2026-08-29"] {
            assert!(is_iso_calendar_date(valid), "expected valid date: {valid}");
        }
        for invalid in [
            "2026-2-28",
            "2026/02-28",
            "2026-02/28",
            "202x-02-28",
            "2023-02-29",
            "1900-02-29",
        ] {
            assert!(
                !is_iso_calendar_date(invalid),
                "expected invalid date: {invalid}"
            );
        }
    }

    #[test]
    fn member_text_byte_limit_is_exact() {
        assert_eq!(MAX_MEMBER_TEXT_BYTES, 16_384);
        let maximum = "a".repeat(16_384);
        assert_eq!(
            optional_member_text(Some(&maximum), "message body"),
            Ok(Some(maximum))
        );
        let oversized = "a".repeat(16_385);
        assert_eq!(
            optional_member_text(Some(&oversized), "message body"),
            Err(ResponseCompatibilityError::InvalidField("message body"))
        );
        let multiline = "first line\nsecond line\r\n\tindented";
        assert_eq!(
            optional_member_text(Some(multiline), "message body"),
            Ok(Some(multiline.to_owned()))
        );
        assert_eq!(
            optional_member_text(Some("unsafe\u{0}text"), "message body"),
            Err(ResponseCompatibilityError::InvalidField("message body"))
        );
    }

    #[test]
    fn private_endings_enforce_exact_length_boundaries() {
        assert_eq!(
            private_ending(&json!("1234"), "account"),
            Ok("1234".to_owned())
        );
        assert_eq!(
            private_ending(&json!("123"), "account"),
            Err(ResponseCompatibilityError::InvalidField("account"))
        );

        let maximum = format!("{}1234", "a".repeat(124));
        assert_eq!(
            private_ending(&json!(maximum), "account"),
            Ok("1234".to_owned())
        );
        let oversized = format!("{}1234", "a".repeat(125));
        assert_eq!(
            private_ending(&json!(oversized), "account"),
            Err(ResponseCompatibilityError::InvalidField("account"))
        );
    }

    #[test]
    fn coverage_summary_exposes_only_account_endings_and_provider_details() {
        let summary = normalize_coverage_summary(json!({
            "firstName": "Sensitive Member",
            "accountsList": [{"accunt": "GE00TB0000000012345678"}],
            "familyDoctor": {
                "doctorName": "Doctor Example",
                "specialty": "Family medicine",
                "clinicName": "Example Clinic",
                "fullAddress": "Example address"
            }
        }))
        .expect("valid coverage summary");

        assert_eq!(summary.reimbursement_accounts[0].account_ending, "5678");
        assert_eq!(
            summary.family_doctor.as_ref().expect("family doctor").name,
            "Doctor Example"
        );
        let serialized = serde_json::to_string(&summary).expect("serializable summary");
        assert!(!serialized.contains("GE00TB0000000012345678"));
        assert!(!serialized.contains("Sensitive Member"));
    }

    #[test]
    fn member_request_list_and_detail_share_business_status_mapping() {
        let request = json!({
            "id": 7,
            "identifierNumber": "REQ-7",
            "currentStatus": 4,
            "reqDate": "2026-06-23T19:21:05.097315",
            "type": 0,
            "files": [{"fileName": "private.pdf"}],
            "insuredComment": "Please reimburse this visit.",
            "operatorComment": "The request was reviewed.",
            "reimbursementAmountStr": "25.50"
        });
        let list = normalize_member_requests(json!({"medRequests": [request]}))
            .expect("valid member-request list");
        let detail = normalize_member_request(request).expect("valid member-request detail");

        assert_eq!(list[0].kind, MemberRequestKind::ReimbursementClaim);
        assert_eq!(list[0].status, MemberRequestStatus::Denied);
        assert_eq!(list[0].attachment_count, Some(1));
        assert_eq!(detail.summary, list[0]);
        assert_eq!(
            detail.reimbursement_amount.as_ref().expect("amount").amount,
            "25.50"
        );
        let serialized = serde_json::to_string(&detail).expect("serializable request");
        assert!(!serialized.contains("private.pdf"));
    }

    #[test]
    fn referral_catalog_and_history_use_member_facing_entities() {
        let providers = normalize_referral_providers(json!([{
            "clinicId": 3,
            "name": "Example Clinic",
            "phone": "private"
        }]))
        .expect("valid provider list");
        let services = normalize_referral_service_options(json!([{
            "serviceId": 4,
            "serviceName": "Cardiology consultation"
        }]))
        .expect("valid service list");
        let referrals = normalize_medical_referrals(json!([{
            "appealNumber": "REF-5",
            "appealName": "Cardiology consultation",
            "clinicName": "Example Clinic",
            "expirationDate": "2026-06-28",
            "statusName": "Active",
            "appealType": 0,
            "ClietPersonalNumber": "private"
        }]))
        .expect("valid referral list");
        let cities = normalize_service_cities(json!([{
            "citiesId": "TB",
            "citiesName": "Tbilisi"
        }]))
        .expect("valid city list");

        assert_eq!(providers[0].provider_id, "3");
        assert_eq!(services[0].service_id, "4");
        assert_eq!(referrals[0].kind, ReferralKind::SpecialistConsultation);
        assert_eq!(referrals[0].valid_until.as_deref(), Some("2026-06-28"));
        assert_eq!(cities[0].name, "Tbilisi");
        assert!(
            !serde_json::to_string(&referrals)
                .expect("serializable referrals")
                .contains("private")
        );
    }

    #[test]
    fn inbox_summary_detail_thread_and_counter_share_one_message_model() {
        let summaries = normalize_inbox_message_summaries(json!([{
            "id": 9,
            "dateCreated": "2026-06-24T08:23:46.835362",
            "isRead": false,
            "messageText": "A short preview",
            "identifierNumber": "REQ-7",
            "objectId": "private"
        }]))
        .expect("valid inbox list");
        let detail = normalize_inbox_message(json!({
            "id": 9,
            "dateCreated": "2026-06-24T08:23:46.835362",
            "isRead": false,
            "direction": 1,
            "messageSubject": "Documents needed",
            "messageText": "Please attach the missing receipt.",
            "senderName": "TBC operator",
            "canAnswer": true,
            "medRequestStatus": 3
        }))
        .expect("valid inbox message");
        let thread = normalize_message_thread(json!([{
            "id": 10,
            "dateCreated": "2026-06-24T09:00:00",
            "isRead": true,
            "direction": 2,
            "messageText": "I will attach it.",
            "senderName": "Member"
        }]))
        .expect("valid message thread");
        let count = normalize_unread_message_count(json!(1)).expect("valid unread count");

        assert!(summaries[0].unread);
        assert!(detail.unread);
        assert_eq!(detail.author, MessageAuthor::Insurer);
        assert_eq!(
            detail.linked_request_status,
            Some(MemberRequestStatus::OnHold)
        );
        assert!(!thread[0].unread);
        assert_eq!(thread[0].author, MessageAuthor::Member);
        assert_eq!(count.count, 1);
        assert!(normalize_unread_message_count(json!(-1)).is_err());
    }

    #[test]
    fn unlimited_benefit_stays_distinct_from_missing_usage() {
        let benefits = normalize_coverage_benefits(json!({
            "risks": [{
                "tagid": "benefit_1",
                "name": "Emergency care",
                "amount": "-",
                "norate": "-",
                "usedLimit": "-"
            }]
        }))
        .expect("valid unlimited benefit");

        assert_eq!(benefits[0].limit, BenefitLimit::Unlimited);
        assert_eq!(benefits[0].insurer_coverage_percent, None);
        assert_eq!(benefits[0].used_amount, None);
    }

    #[test]
    fn claim_wire_amounts_preserve_decimal_precision() {
        let claims = normalize_claim_summaries(json!([{
            "rsid": 17,
            "date": "2026-08-20",
            "providerName": "Example Clinic",
            "riskName": "Specialist consultation",
            "servicePriceSum": "123.45",
            "insurerAmount": "98.76"
        }]))
        .expect("valid claim response");

        assert_eq!(
            claims[0].billed_amount.as_ref().expect("amount").amount,
            "123.45"
        );
        assert_eq!(
            claims[0]
                .insurer_paid_amount
                .as_ref()
                .expect("amount")
                .amount,
            "98.76"
        );
    }

    #[test]
    fn literal_appeal_and_research_codes_map_to_referral_language() {
        assert_eq!(referral_kind(0), ReferralKind::SpecialistConsultation);
        assert_eq!(referral_kind(1), ReferralKind::DiagnosticService);
        assert_eq!(referral_kind(2), ReferralKind::MedicationPrescription);
    }

    #[test]
    fn medical_request_codes_map_to_business_workflows() {
        assert_eq!(
            member_request_kind(0),
            MemberRequestKind::ReimbursementClaim
        );
        assert_eq!(
            member_request_kind(1),
            MemberRequestKind::GuaranteeOfPayment
        );
        assert_eq!(member_request_status(3), MemberRequestStatus::OnHold);
        assert_eq!(member_request_status(6), MemberRequestStatus::Approved);
    }

    #[test]
    fn invalid_benefit_limits_and_rates_fail_closed_independently() {
        for (amount, rate) in [
            (json!("NaN"), json!("90%")),
            (json!("-1"), json!("90%")),
            (json!("1e3"), json!("90%")),
            (json!(""), json!("90%")),
            (Value::Null, json!("90%")),
            (json!("1000"), json!("101%")),
            (json!("1000"), json!("-1%")),
            (json!("1000"), json!("+20%")),
            (json!("1000"), json!("20%%")),
            (json!("1000"), json!("20.5%")),
            (json!("1000"), json!(20)),
            (json!("1000"), Value::Null),
        ] {
            let result = normalize_coverage_benefits(json!({
                "risks": [{
                    "tagid": 9, "name": "Consultation", "amount": amount,
                    "norate": rate, "usedLimit": "0"
                }]
            }));
            assert!(result.is_err());
        }
    }

    #[test]
    fn current_coverage_rate_wire_format_requires_one_percent_suffix() {
        assert_eq!(
            optional_percent(&json!("20%"), "insurer coverage"),
            Ok(Some(20))
        );
        for invalid in [json!("20"), json!("20%%"), json!("101%"), json!("%")] {
            assert_eq!(
                optional_percent(&invalid, "insurer coverage"),
                Err(ResponseCompatibilityError::InvalidField("insurer coverage"))
            );
        }
    }

    #[test]
    fn wire_identifiers_reject_each_invalid_condition() {
        for invalid in [String::new(), "a".repeat(129), "unsafe/id".to_owned()] {
            assert_eq!(
                identifier(&Value::String(invalid), "test ID"),
                Err(ResponseCompatibilityError::InvalidField("test ID"))
            );
        }
        assert!(identifier(&Value::String("a".repeat(128)), "test ID").is_ok());
    }

    #[test]
    fn labels_reject_empty_oversized_and_control_text() {
        for invalid in [String::new(), "a".repeat(513), "line\nbreak".to_owned()] {
            assert_eq!(
                label(&invalid, "test label"),
                Err(ResponseCompatibilityError::InvalidField("test label"))
            );
        }
        assert!(label(&"a".repeat(512), "test label").is_ok());
    }

    #[test]
    fn decimal_validation_covers_each_grammar_rule() {
        for valid in ["0", "123", "0.00", "123.45"] {
            assert!(is_non_negative_decimal(valid), "valid decimal: {valid}");
        }
        for invalid in ["", "-1", "abc", ".1", "1.", "1.a", "1.2.3"] {
            assert!(
                !is_non_negative_decimal(invalid),
                "invalid decimal: {invalid}"
            );
        }
    }

    #[test]
    fn claim_lines_use_service_language() {
        let lines = normalize_claim_lines(json!([{
            "name": "Consultation",
            "count": 1,
            "servicePrice": "75.00",
            "insurerAmount": "60.00"
        }]))
        .expect("valid claim lines");
        assert_eq!(lines[0].service_name, "Consultation");
        assert_eq!(lines[0].quantity, "1");
    }
}
