//! Closed write contract for reviewed TBC health-account workflows.

use serde_json::{Value, json};

use super::{HttpMethod, MutationRequest};

/// One mutation supported by today's verified TBC health-account bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MutationEndpoint {
    /// Reserve one exact healthcare-service interval selected from live availability.
    CreateHealthcareBooking(super::bookings::HealthcareBooking),
    /// Upload one document for a medical request or referral.
    UploadMedicalDocument {
        file_name: String,
        content_base64: String,
    },
    /// Remove an uploaded medical document that no submitted request references.
    DeleteMedicalDocument { file_store_id: Value },
    /// Submit a reimbursement claim or guarantee-of-payment request.
    CreateMemberRequest {
        insured_personal_number: String,
        insured_full_name: String,
        bank_account_number: Option<String>,
        documents: Vec<UploadedDocument>,
        comment: String,
        request_type: u8,
    },
    /// Request a specialist consultation from a selected network provider.
    CreateSpecialistReferral {
        policy_id: Value,
        provider_id: Value,
        service_id: Value,
    },
    /// Request a diagnostic-service or medication referral by email workflow.
    SendReferralRequest {
        policy_number: String,
        file_store_ids: Vec<Value>,
        comment: String,
        provider_name: Option<String>,
        service_type: u8,
        insured_personal_number: String,
        insured_full_name: String,
    },
    /// Upload one attachment for an insurer-message reply.
    UploadMessageAttachment {
        file_name: String,
        content_base64: String,
    },
    /// Remove an uploaded message attachment that no sent reply references.
    DeleteMessageAttachment { file_store_id: Value },
    /// Reply to one existing insurer message thread.
    ReplyToMessage {
        files: Vec<UploadedDocument>,
        message_id: String,
        text: String,
        object_id: Value,
    },
}

impl MutationEndpoint {
    /// Materialize the selected mutation into its fixed HTTP contract.
    #[must_use]
    pub(crate) fn request(&self) -> MutationRequest {
        let (operation, path) = self.route();
        MutationRequest {
            operation,
            method: HttpMethod::Post,
            path: path.to_owned(),
            body: self.body(),
        }
    }

    const fn route(&self) -> (&'static str, &'static str) {
        match self {
            Self::CreateHealthcareBooking(_) => (
                "book_appointment",
                "/api/DoctorBooking/CreateHealthcareServiceBooking",
            ),
            Self::UploadMedicalDocument { .. } => {
                ("upload_medical_document", "/api/MedRequest/UploadFile")
            }
            Self::DeleteMedicalDocument { .. } => (
                "delete_unsubmitted_medical_document",
                "/api/MedRequest/DeleteFile",
            ),
            Self::CreateMemberRequest { .. } => (
                "submit_reimbursement_or_guarantee_request",
                "/api/MedRequest/CreateMedRequest",
            ),
            Self::CreateSpecialistReferral { .. } => (
                "request_specialist_referral",
                "/api/RequestAppeal/CreateAppeal",
            ),
            Self::SendReferralRequest { .. } => (
                "request_diagnostic_or_medication_referral",
                "/api/RequestAppeal/SendAppealsEmail",
            ),
            Self::UploadMessageAttachment { .. } => {
                ("upload_message_attachment", "/api/Notification/UploadFile")
            }
            Self::DeleteMessageAttachment { .. } => (
                "delete_unsubmitted_message_attachment",
                "/api/Notification/DeleteFile",
            ),
            Self::ReplyToMessage { .. } => (
                "reply_to_insurer_message",
                "/api/Notification/SendMessageWithFiles",
            ),
        }
    }

    fn body(&self) -> Value {
        match self {
            Self::CreateHealthcareBooking(booking) => serde_json::to_value(booking)
                .expect("the fixed booking payload contains only JSON scalars"),
            Self::UploadMedicalDocument {
                file_name,
                content_base64,
            }
            | Self::UploadMessageAttachment {
                file_name,
                content_base64,
            } => json!({ "fileName": file_name, "content": content_base64 }),
            Self::DeleteMedicalDocument { file_store_id }
            | Self::DeleteMessageAttachment { file_store_id } => {
                json!({ "fileStoreId": file_store_id })
            }
            Self::CreateMemberRequest {
                insured_personal_number,
                insured_full_name,
                bank_account_number,
                documents,
                comment,
                request_type,
            } => {
                let mut body = json!({
                    "insuredPN": insured_personal_number,
                    "insuredFullName": insured_full_name,
                    "documents": documents.iter().map(uploaded_document_json).collect::<Vec<_>>(),
                    "comment": comment,
                    "type": request_type,
                });
                if let Some(account) = bank_account_number {
                    body["accountNumber"] = Value::String(account.clone());
                }
                body
            }
            Self::CreateSpecialistReferral {
                policy_id,
                provider_id,
                service_id,
            } => json!({
                "policyId": policy_id,
                "clinicId": provider_id,
                "serviceId": service_id,
                "servicetype": 0,
            }),
            Self::SendReferralRequest {
                policy_number,
                file_store_ids,
                comment,
                provider_name,
                service_type,
                insured_personal_number,
                insured_full_name,
            } => json!({
                "policyNumber": policy_number,
                "fileStoreIds": file_store_ids,
                "comment": comment,
                "provider": provider_name,
                "servicetype": service_type,
                "clientPn": insured_personal_number,
                "clientFullName": insured_full_name,
            }),
            Self::ReplyToMessage {
                files,
                message_id,
                text,
                object_id,
            } => json!({
                "files": files.iter().map(uploaded_document_json).collect::<Vec<_>>(),
                "messageId": message_id,
                "isRead": true,
                "text": text,
                "objectId": object_id,
            }),
        }
    }
}

/// One uploaded file reference accepted by a final TBC mutation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadedDocument {
    pub file_store_id: Value,
    pub file_name: String,
}

fn uploaded_document_json(document: &UploadedDocument) -> Value {
    json!({
        "fileStoreId": document.file_store_id,
        "fileName": document.file_name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "one declarative test covers the complete eight-route mutation allowlist"
    )]
    fn every_mutation_has_one_fixed_operation_name_and_path() {
        let contract: Value =
            serde_json::from_str(include_str!("../../contracts/tbc-write-api-v1.json"))
                .expect("valid write contract");
        let cases = [
            (
                MutationEndpoint::UploadMedicalDocument {
                    file_name: "a.pdf".to_owned(),
                    content_base64: "YQ==".to_owned(),
                },
                "upload_medical_document",
                "/api/MedRequest/UploadFile",
            ),
            (
                MutationEndpoint::DeleteMedicalDocument {
                    file_store_id: json!(1),
                },
                "delete_unsubmitted_medical_document",
                "/api/MedRequest/DeleteFile",
            ),
            (
                MutationEndpoint::CreateMemberRequest {
                    insured_personal_number: "test-id".to_owned(),
                    insured_full_name: "Test Member".to_owned(),
                    bank_account_number: None,
                    documents: Vec::new(),
                    comment: "test".to_owned(),
                    request_type: 1,
                },
                "submit_reimbursement_or_guarantee_request",
                "/api/MedRequest/CreateMedRequest",
            ),
            (
                MutationEndpoint::CreateSpecialistReferral {
                    policy_id: json!(1),
                    provider_id: json!(2),
                    service_id: json!(3),
                },
                "request_specialist_referral",
                "/api/RequestAppeal/CreateAppeal",
            ),
            (
                MutationEndpoint::SendReferralRequest {
                    policy_number: "policy".to_owned(),
                    file_store_ids: Vec::new(),
                    comment: String::new(),
                    provider_name: None,
                    service_type: 2,
                    insured_personal_number: "test-id".to_owned(),
                    insured_full_name: "Test Member".to_owned(),
                },
                "request_diagnostic_or_medication_referral",
                "/api/RequestAppeal/SendAppealsEmail",
            ),
            (
                MutationEndpoint::UploadMessageAttachment {
                    file_name: "a.pdf".to_owned(),
                    content_base64: "YQ==".to_owned(),
                },
                "upload_message_attachment",
                "/api/Notification/UploadFile",
            ),
            (
                MutationEndpoint::DeleteMessageAttachment {
                    file_store_id: json!(1),
                },
                "delete_unsubmitted_message_attachment",
                "/api/Notification/DeleteFile",
            ),
            (
                MutationEndpoint::ReplyToMessage {
                    files: Vec::new(),
                    message_id: "1".to_owned(),
                    text: "test".to_owned(),
                    object_id: json!(2),
                },
                "reply_to_insurer_message",
                "/api/Notification/SendMessageWithFiles",
            ),
        ];

        for (endpoint, operation, path) in cases {
            let request = endpoint.request();
            assert_eq!(request.operation(), operation);
            assert_eq!(request.path(), path);
            assert!(
                contract["operations"]
                    .as_array()
                    .expect("operation list")
                    .iter()
                    .any(|entry| {
                        entry["enabled"] == true
                            && entry["domainOperation"] == operation
                            && entry["method"] == "POST"
                            && entry["path"] == path
                    })
            );
        }
    }

    #[test]
    fn reimbursement_and_guarantee_share_the_verified_member_request_contract() {
        let document = UploadedDocument {
            file_store_id: json!(41),
            file_name: "receipt.pdf".to_owned(),
        };
        let reimbursement = MutationEndpoint::CreateMemberRequest {
            insured_personal_number: "personal-number".to_owned(),
            insured_full_name: "Covered Member".to_owned(),
            bank_account_number: Some("GE00TB0000".to_owned()),
            documents: vec![document.clone()],
            comment: "Please reimburse this visit.".to_owned(),
            request_type: 0,
        }
        .request();
        assert_eq!(reimbursement.path(), "/api/MedRequest/CreateMedRequest");
        assert_eq!(
            reimbursement.body(),
            &json!({
                "insuredPN": "personal-number",
                "insuredFullName": "Covered Member",
                "accountNumber": "GE00TB0000",
                "documents": [{"fileStoreId": 41, "fileName": "receipt.pdf"}],
                "comment": "Please reimburse this visit.",
                "type": 0,
            })
        );

        let guarantee = MutationEndpoint::CreateMemberRequest {
            insured_personal_number: "personal-number".to_owned(),
            insured_full_name: "Covered Member".to_owned(),
            bank_account_number: None,
            documents: vec![document],
            comment: "Please guarantee payment for planned care.".to_owned(),
            request_type: 1,
        }
        .request();
        assert!(guarantee.body().get("accountNumber").is_none());
        assert_eq!(guarantee.body()["type"], 1);
    }

    #[test]
    fn referral_payloads_preserve_the_current_upstream_field_spelling() {
        let specialist = MutationEndpoint::CreateSpecialistReferral {
            policy_id: json!(1),
            provider_id: json!(2),
            service_id: json!(3),
        }
        .request();
        assert_eq!(specialist.body()["servicetype"], 0);

        let diagnostic = MutationEndpoint::SendReferralRequest {
            policy_number: "POLICY-1".to_owned(),
            file_store_ids: vec![json!(9)],
            comment: "MRI referral".to_owned(),
            provider_name: Some("Example Clinic".to_owned()),
            service_type: 1,
            insured_personal_number: "personal-number".to_owned(),
            insured_full_name: "Covered Member".to_owned(),
        }
        .request();
        assert_eq!(diagnostic.path(), "/api/RequestAppeal/SendAppealsEmail");
        assert_eq!(diagnostic.body()["servicetype"], 1);
        assert_eq!(diagnostic.body()["clientPn"], "personal-number");
    }

    #[test]
    fn insurer_reply_uses_message_file_names_and_marks_the_message_read() {
        let request = MutationEndpoint::ReplyToMessage {
            files: vec![UploadedDocument {
                file_store_id: json!("stored-file"),
                file_name: "form-100.pdf".to_owned(),
            }],
            message_id: "7".to_owned(),
            text: "Attached is the requested document.".to_owned(),
            object_id: json!(12),
        }
        .request();
        assert_eq!(request.path(), "/api/Notification/SendMessageWithFiles");
        assert_eq!(request.body()["messageId"], "7");
        assert_eq!(request.body()["isRead"], true);
        assert_eq!(request.body()["files"][0]["fileName"], "form-100.pdf");
    }
}
