#![no_main]

use libfuzzer_sys::fuzz_target;
use serde_json::Value;
use tbc_insurance_mcp::api::{
    AppointmentSlotQuery, EndpointId, normalize_appointment_locations, normalize_appointment_slots,
    normalize_available_clinicians, normalize_bookable_services, normalize_claim_lines,
    normalize_claim_summaries, normalize_coverage_benefits, normalize_coverage_summary,
    normalize_health_policies, normalize_inbox_message, normalize_inbox_message_summaries,
    normalize_medical_referrals, normalize_member_request, normalize_member_requests,
    normalize_message_thread, normalize_referral_providers, normalize_referral_service_options,
    normalize_service_cities, normalize_unread_message_count,
};

fuzz_target!(|input: &[u8]| {
    if let Ok(identifier) = std::str::from_utf8(input) {
        let _ = EndpointId::parse(identifier);
    }
    if let Ok(payload) = serde_json::from_slice::<Value>(input) {
        let query = AppointmentSlotQuery {
            service_id: 21,
            location_id: 22,
            clinician_id: 23,
            from_date: "2028-02-28".to_owned(),
            to_date: "2028-03-01".to_owned(),
        };
        let _ = normalize_bookable_services(payload.clone());
        let _ = normalize_appointment_locations(payload.clone());
        let _ = normalize_available_clinicians(payload.clone(), query.location_id);
        let _ = normalize_appointment_slots(payload.clone(), &query);
        if let Ok(query) = serde_json::from_value::<AppointmentSlotQuery>(payload.clone()) {
            let _ = normalize_appointment_slots(serde_json::json!({"slots": []}), &query);
        }
        let _ = normalize_health_policies(payload.clone());
        let _ = normalize_coverage_benefits(payload.clone());
        let _ = normalize_claim_summaries(payload.clone());
        let _ = normalize_claim_lines(payload.clone());
        let _ = normalize_coverage_summary(payload.clone());
        let _ = normalize_member_requests(payload.clone());
        let _ = normalize_member_request(payload.clone());
        let _ = normalize_referral_providers(payload.clone());
        let _ = normalize_referral_service_options(payload.clone());
        let _ = normalize_medical_referrals(payload.clone());
        let _ = normalize_service_cities(payload.clone());
        let _ = normalize_inbox_message_summaries(payload.clone());
        let _ = normalize_unread_message_count(payload.clone());
        let _ = normalize_inbox_message(payload.clone());
        let _ = normalize_message_thread(payload);
    }
});
