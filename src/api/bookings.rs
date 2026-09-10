//! Native healthcare-booking wire contract and minimized account readback.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::domain::{AppointmentBooking, AppointmentBookings};

use super::{EndpointId, ReadRequest, wire};

/// Private native-app request. Member identity is resolved internally from the account.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct HealthcareBooking {
    pub slot_id: Option<String>,
    pub start_date: String,
    pub end_date: String,
    pub doctor_id: u32,
    pub healthcare_service_id: u32,
    pub patient_personal_number: String,
    pub patient_first_name: String,
    pub patient_last_name: String,
    pub patient_phone_number: String,
    pub patient_birth_date: String,
    pub clinic_branch_id: u32,
}

// Debug is intentionally structural: this payload contains patient identifiers.
impl std::fmt::Debug for HealthcareBooking {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HealthcareBooking { patient: [REDACTED], appointment: [REDACTED] }")
    }
}

pub fn list_request() -> ReadRequest {
    wire::get("list_appointment_bookings", "/api/Mobile/GetNotifications")
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Notifications {
    notifications: Vec<Notification>,
    notifications_history: Vec<Notification>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Notification {
    medical_booking_details: Option<BookingDetails>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BookingDetails {
    booking_id: Value,
    doctor_name: String,
    clinic_name: String,
    date: Option<String>,
    #[serde(rename = "startTimeFormated")]
    start_time_formatted: Option<String>,
}

pub fn normalize(payload: Value) -> Result<AppointmentBookings, String> {
    let notifications: Notifications = serde_json::from_value(payload)
        .map_err(|_| "TBC returned an incompatible appointment-booking response")?;
    Ok(AppointmentBookings {
        active: booking_records(notifications.notifications)?,
        history: booking_records(notifications.notifications_history)?,
    })
}

fn booking_records(records: Vec<Notification>) -> Result<Vec<AppointmentBooking>, String> {
    records
        .into_iter()
        .filter_map(|record| record.medical_booking_details)
        .map(|record| {
            Ok(AppointmentBooking {
                booking_id: booking_id(&record.booking_id)
                    .ok_or("TBC returned an invalid booking reference")?,
                clinician_name: label(record.doctor_name)?,
                location_name: label(record.clinic_name)?,
                scheduled_date: record.date.clone().map(label).transpose()?,
                scheduled_time: record
                    .start_time_formatted
                    .or(record.date)
                    .map(label)
                    .transpose()?,
            })
        })
        .collect()
}

pub fn booking_id(value: &Value) -> Option<String> {
    let text = match value {
        Value::String(value) => value.clone(),
        Value::Number(value) if value.as_u64().is_some_and(|id| id > 0) => value.to_string(),
        _ => return None,
    };
    EndpointId::parse(text)
        .ok()
        .map(|id| id.as_str().to_owned())
}

fn label(text: String) -> Result<String, String> {
    if text.trim().is_empty() || text.len() > 512 || text.chars().any(char::is_control) {
        return Err("TBC returned an invalid appointment label".to_owned());
    }
    Ok(text)
}

#[cfg(test)]
mod tests;
