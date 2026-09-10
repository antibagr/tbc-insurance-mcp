//! Exact-slot booking through the shared single-use review boundary.

use std::time::{SystemTime, UNIX_EPOCH};

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    api::{
        EndpointId, MutationEndpoint, ReadEndpoint, TbcClient, TbcMutationError,
        appointments::{self, ClinicianQuery, ServiceQuery, SlotQuery, timestamp},
        bookings::{self, HealthcareBooking},
        normalize_health_policies,
    },
    domain::{
        AppointmentBooking, AppointmentBookings, AppointmentSlot, AvailableClinician,
        BookableService, ProviderLocation,
    },
};

use super::{
    SelectedPolicy, append_confirmation_warning, ending, mask_name, records, selected_policy,
};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BookingInput {
    /// Policy of the covered member, returned by the policy list.
    pub policy_id: String,
    /// Exact, unmodified slot returned by `list_appointment_slots`; `slot_id` may be null.
    pub slot: AppointmentSlot,
}

pub(super) struct PreparedBooking {
    input: BookingInput,
    policy: SelectedPolicy,
    service: BookableService,
    location: ProviderLocation,
    clinician: AvailableClinician,
    payload: HealthcareBooking,
    before: AppointmentBookings,
}

pub(super) async fn prepare(
    client: &TbcClient,
    input: &BookingInput,
) -> Result<PreparedBooking, String> {
    let query = validate_slot(input, unix_now()?)?;
    let (policy, payload) = resolve_member(client, input).await?;
    let (service, location, clinician) = resolve_selection(client, input, &query.from_date).await?;
    let before = read_bookings(client).await?;
    // An existing reservation with incomplete time data requires inspection before any write.
    if before.active.iter().any(|booking| {
        booking.clinician_name == clinician.name
            && booking.location_name == location.name
            && booking.scheduled_time.as_deref().is_none_or(|time| {
                let parsed = timestamp(time).or_else(|| timestamp(&format!("{time}+04:00")));
                parsed.is_none() || same_time(time, &input.slot.starts_at)
            })
    }) {
        return Err("A matching or incomplete active booking already exists; inspect current bookings before creating another".to_owned());
    }
    let payload_slots = client
        .execute(&appointments::slots_request(&query)?)
        .await
        .map_err(|error| error.to_string())?;
    let slots =
        appointments::normalize_slots(payload_slots, &query).map_err(|error| error.to_string())?;
    if slots.iter().filter(|slot| **slot == input.slot).count() != 1 {
        return Err(
            "That exact appointment is no longer uniquely available; choose a fresh slot"
                .to_owned(),
        );
    }
    Ok(PreparedBooking {
        input: input.clone(),
        policy,
        service,
        location,
        clinician,
        payload,
        before,
    })
}

fn unix_now() -> Result<i64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|value| i64::try_from(value.as_secs()).ok())
        .ok_or_else(|| "The local clock is unavailable".to_owned())
}

fn validate_slot(input: &BookingInput, now: i64) -> Result<SlotQuery, String> {
    EndpointId::parse(input.policy_id.clone()).map_err(|error| error.to_string())?;
    let slot = &input.slot;
    let start = timestamp(&slot.starts_at).ok_or("Invalid appointment start")?;
    let end = timestamp(&slot.ends_at).ok_or("Invalid appointment end")?;
    let epoch = timestamp("1970-01-01T00:00:00Z").expect("fixed epoch");
    if start - epoch <= now
        || end <= start
        || !slot.starts_at.ends_with("+04:00")
        || !slot.ends_at.ends_with("+04:00")
    {
        return Err(
            "Select a future, ordered appointment interval in Tbilisi time (+04:00)".to_owned(),
        );
    }
    if let Some(id) = &slot.slot_id {
        EndpointId::parse(id.clone()).map_err(|error| error.to_string())?;
    }
    let date = slot.starts_at[..10].to_owned();
    let query = SlotQuery {
        service_id: slot.service_id,
        location_id: slot.location_id,
        clinician_id: slot.clinician_id,
        from_date: date.clone(),
        to_date: date,
    };
    appointments::slots_request(&query)?;
    Ok(query)
}

async fn resolve_selection(
    client: &TbcClient,
    input: &BookingInput,
    date: &str,
) -> Result<(BookableService, ProviderLocation, AvailableClinician), String> {
    let services = client
        .execute(&appointments::services_request())
        .await
        .map_err(|e| e.to_string())?;
    let service = exactly_one(
        appointments::normalize_services(services)
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|s| s.service_id == input.slot.service_id),
    )?;
    let locations = client
        .execute(&appointments::locations_request(&ServiceQuery {
            service_id: input.slot.service_id,
        })?)
        .await
        .map_err(|e| e.to_string())?;
    let location = exactly_one(
        appointments::normalize_locations(locations)
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|l| l.location_id == input.slot.location_id),
    )?;
    let clinicians = client
        .execute(&appointments::clinicians_request(&ClinicianQuery {
            service_id: input.slot.service_id,
            location_id: input.slot.location_id,
            date: date.to_owned(),
        })?)
        .await
        .map_err(|e| e.to_string())?;
    let clinician = exactly_one(
        appointments::normalize_clinicians(clinicians, input.slot.location_id)
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|c| c.clinician_id == input.slot.clinician_id),
    )?;
    Ok((service, location, clinician))
}

fn exactly_one<T>(mut values: impl Iterator<Item = T>) -> Result<T, String> {
    let first = values
        .next()
        .ok_or("The selected appointment option is unavailable")?;
    if values.next().is_some() {
        return Err("TBC returned ambiguous appointment options".to_owned());
    }
    Ok(first)
}

async fn resolve_member(
    client: &TbcClient,
    input: &BookingInput,
) -> Result<(SelectedPolicy, HealthcareBooking), String> {
    let policies = client
        .execute(&ReadEndpoint::ListPolicies.request())
        .await
        .map_err(|e| e.to_string())?;
    let normalized = normalize_health_policies(policies.clone()).map_err(|e| e.to_string())?;
    let policy = exactly_one(normalized.iter().filter(|p| p.policy_id == input.policy_id))?;
    let date = &input.slot.starts_at[..10];
    if policy
        .coverage_period
        .starts_on
        .as_deref()
        .is_none_or(|start| start > date)
        || policy
            .coverage_period
            .ends_on
            .as_deref()
            .is_none_or(|end| end < date)
    {
        return Err(
            "The selected policy does not establish coverage dates for this appointment".to_owned(),
        );
    }
    let record = exactly_one(records(&policies).into_iter().filter(|p| {
        bookings::booking_id(&p["polId"]).as_deref() == Some(input.policy_id.as_str())
    }))?;
    let selected = selected_policy(record, record["polId"].clone())?;
    let profile = client
        .execute(&ReadEndpoint::GetCoverageSummary.request())
        .await
        .map_err(|e| e.to_string())?;
    let payload = patient_payload(record, &profile, &selected, &input.slot)?;
    Ok((selected, payload))
}

fn patient_payload(
    policy: &serde_json::Map<String, Value>,
    profile: &Value,
    selected: &SelectedPolicy,
    slot: &AppointmentSlot,
) -> Result<HealthcareBooking, String> {
    if policy
        .get("medCustomerId")
        .and_then(Value::as_u64)
        .is_none_or(|id| id == 0)
        || policy["medCustomerId"] != profile["id"]
    {
        return Err(
            "The selected policy cannot be securely linked to this patient profile".to_owned(),
        );
    }
    let first = private_text(&profile["firstName"])?;
    let last = private_text(&profile["lastName"])?;
    if selected.insured_full_name != format!("{last} {first}")
        && selected.insured_full_name != format!("{first} {last}")
    {
        return Err("The policy and patient profile names disagree".to_owned());
    }
    let birth = private_text(&profile["birthDate"])?;
    let date = birth
        .get(..10)
        .ok_or("TBC returned an invalid patient birth date")?;
    timestamp(&format!("{date}T00:00:00Z")).ok_or("TBC returned an invalid patient birth date")?;
    let day_first = format!("{}.{}.{}", &date[8..10], &date[5..7], &date[..4]);
    if policy.get("dateOfBirthd").and_then(Value::as_str) != Some(day_first.as_str())
        || profile["dateOfBirthd"].as_str() != Some(day_first.as_str())
    {
        return Err("The policy and patient profile birth dates disagree".to_owned());
    }
    let phone = private_text(&profile["mobile"])?;
    if !(8..=15).contains(&phone.bytes().filter(u8::is_ascii_digit).count())
        || !phone
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '+' | ' ' | '-' | '(' | ')'))
    {
        return Err("The patient profile has no usable booking phone number".to_owned());
    }
    Ok(HealthcareBooking {
        slot_id: slot.slot_id.clone(),
        start_date: slot.starts_at.clone(),
        end_date: slot.ends_at.clone(),
        doctor_id: slot.clinician_id,
        healthcare_service_id: slot.service_id,
        clinic_branch_id: slot.location_id,
        patient_personal_number: selected.insured_personal_number.clone(),
        patient_first_name: first,
        patient_last_name: last,
        patient_phone_number: phone,
        patient_birth_date: date.to_owned(),
    })
}

fn private_text(value: &Value) -> Result<String, String> {
    value
        .as_str()
        .filter(|s| !s.trim().is_empty() && s.len() <= 256 && !s.chars().any(char::is_control))
        .map(ToOwned::to_owned)
        .ok_or_else(|| "A required patient profile field is missing or invalid".to_owned())
}

pub async fn read_bookings(client: &TbcClient) -> Result<AppointmentBookings, String> {
    bookings::normalize(
        client
            .execute(&bookings::list_request())
            .await
            .map_err(|e| e.to_string())?,
    )
}

impl PreparedBooking {
    pub(super) fn review_message(&self) -> String {
        let mut review = format!(
            "Review this TBC Insurance appointment booking.\n\nCovered member: {}\nPolicy: ending {}\nService: {}\nDoctor: {}\nClinic: {}\nAddress: {}\nStarts: {}\nEnds: {}\nTime zone: Asia/Tbilisi (UTC+04:00)\n\nCoverage, consultation price, and copay have not been verified. Booking can create a financial obligation.\nTBC and the clinic receive the patient's registered name, personal number, birth date, phone (ending {}), and selected appointment. No medical documents or medical notes will be sent.\n",
            mask_name(&self.policy.insured_full_name),
            ending(&self.policy.policy_number),
            self.service.name,
            self.clinician.name,
            self.location.name,
            self.location
                .address
                .as_deref()
                .unwrap_or("Not supplied by TBC"),
            self.input.slot.starts_at,
            self.input.slot.ends_at,
            ending(&self.payload.patient_phone_number)
        );
        append_confirmation_warning(&mut review);
        review
    }

    pub(super) async fn execute(self, client: &TbcClient) -> Result<CallToolResult, String> {
        let current = prepare(client, &self.input).await?;
        if self.payload != current.payload
            || self.service != current.service
            || self.location != current.location
            || self.clinician != current.clinician
            || self.policy.policy_number != current.policy.policy_number
        {
            return Err(
                "The patient or appointment details changed after review; create a fresh review"
                    .to_owned(),
            );
        }
        let response = client
            .execute_mutation(
                &MutationEndpoint::CreateHealthcareBooking(current.payload.clone()).request(),
            )
            .await;
        if let Err(TbcMutationError::Rejected(error)) = response {
            return Ok(CallToolResult::structured_error(
                json!({"operation":"book_appointment",
                "outcome":"rejected", "message":error.to_string(), "retry_safe":false}),
            ));
        }
        let after = read_bookings(client).await;
        Ok(current.readback_result(&response, after))
    }

    fn readback_result(
        &self,
        response: &Result<Value, TbcMutationError>,
        after: Result<AppointmentBookings, String>,
    ) -> CallToolResult {
        let ambiguous = response.is_err();
        let reference = response
            .as_ref()
            .ok()
            .and_then(|value| bookings::booking_id(&value["bookingId"]));
        let matching = after
            .as_ref()
            .map(|bookings| {
                bookings
                    .active
                    .iter()
                    .filter(|booking| {
                        !self
                            .before
                            .active
                            .iter()
                            .any(|before| before.booking_id == booking.booking_id)
                            && self.matches(booking)
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let confirmed = !ambiguous
            && matching.len() == 1
            && reference
                .as_ref()
                .is_some_and(|reference| matching[0].booking_id == *reference);
        let outcome = if ambiguous {
            "outcome_unknown"
        } else if confirmed {
            "booked"
        } else {
            "submitted_readback_unverified"
        };
        let result = json!({"operation":"book_appointment", "outcome":outcome,
            "booking_id": reference, "readback":{"performed":after.is_ok(),
                "matching_bookings":matching, "error":after.err()}, "retry_safe":false,
            "message": if confirmed { "The returned booking reference was verified in active bookings" }
                else { "Do not submit again. Inspect current bookings before taking further action" }});
        if confirmed {
            CallToolResult::structured(result)
        } else {
            CallToolResult::structured_error(result)
        }
    }

    fn matches(&self, booking: &AppointmentBooking) -> bool {
        booking.clinician_name == self.clinician.name
            && booking.location_name == self.location.name
            && booking
                .scheduled_time
                .as_deref()
                .is_some_and(|time| same_time(time, &self.input.slot.starts_at))
    }
}

fn same_time(observed: &str, expected: &str) -> bool {
    // The native feed also supplies local wall time without an offset. Compare the
    // complete wall time only against a reviewed, explicit Tbilisi timestamp.
    if observed.len() == 19 && expected.ends_with("+04:00") {
        return observed == &expected[..19];
    }
    timestamp(observed)
        .zip(timestamp(expected))
        .is_some_and(|(a, b)| a == b)
}

#[cfg(test)]
mod tests;
