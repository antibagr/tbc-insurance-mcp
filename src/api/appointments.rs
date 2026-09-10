//! Verified appointment-search reads and context-bound response validation.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::domain::{AppointmentSlot, AvailableClinician, BookableService, ProviderLocation};

use super::{EndpointId, ReadRequest, ResponseCompatibilityError as Error, wire};

#[cfg(test)]
mod tests;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ServiceQuery {
    /// Positive booking-service ID returned by `list_bookable_services`.
    pub service_id: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClinicianQuery {
    /// Positive booking-service ID returned by `list_bookable_services`.
    pub service_id: u32,
    /// Positive branch ID returned by `list_provider_locations`.
    pub location_id: u32,
    /// Tbilisi calendar date in YYYY-MM-DD format; an empty result applies only to this date.
    pub date: String,
}

/// Selected clinician, branch, service, and inclusive Tbilisi date window.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SlotQuery {
    /// Positive booking-service ID returned by `list_bookable_services`.
    pub service_id: u32,
    /// Positive branch ID returned by `list_provider_locations`.
    pub location_id: u32,
    /// Positive clinician ID returned by `list_available_clinicians` for this branch and service.
    pub clinician_id: u32,
    /// First Tbilisi calendar date in YYYY-MM-DD format.
    pub from_date: String,
    /// Last Tbilisi calendar date in YYYY-MM-DD format; search at most 31 calendar dates.
    pub to_date: String,
}

pub fn services_request() -> ReadRequest {
    wire::get(
        "list_bookable_services",
        "/api/DoctorBooking/GetHealthcareServices",
    )
}

pub fn locations_request(query: &ServiceQuery) -> Result<ReadRequest, String> {
    validate_ids(&[query.service_id])?;
    Ok(wire::post(
        "list_provider_locations",
        "/api/DoctorBooking/GetClinicBranches",
        json!({"HealthcareServiceId": query.service_id}),
    ))
}

pub fn clinicians_request(query: &ClinicianQuery) -> Result<ReadRequest, String> {
    validate_ids(&[query.service_id, query.location_id])?;
    calendar_day(&query.date).ok_or("appointment date must be a valid YYYY-MM-DD date")?;
    Ok(wire::post(
        "list_available_clinicians",
        "/api/DoctorBooking/GetHealthcareServiceDoctors",
        json!({"HealthcareServiceId": query.service_id, "ClinicBranchId": query.location_id,
            "exactDate": query.date}),
    ))
}

pub fn slots_request(query: &SlotQuery) -> Result<ReadRequest, String> {
    validate_ids(&[query.service_id, query.location_id, query.clinician_id])?;
    let first =
        calendar_day(&query.from_date).ok_or("from_date must be a valid YYYY-MM-DD date")?;
    let last = calendar_day(&query.to_date).ok_or("to_date must be a valid YYYY-MM-DD date")?;
    if last.checked_sub(first).is_none_or(|days| days >= 31) {
        return Err(
            "appointment window must contain 1 through 31 ordered calendar dates".to_owned(),
        );
    }
    Ok(wire::post(
        "list_appointment_slots",
        "/api/DoctorBooking/GetDoctorSlots",
        json!({"HealthcareServiceId": query.service_id, "ClinicBranchId": query.location_id,
            "DoctorId": query.clinician_id, "FromDate": query.from_date, "ToDate": query.to_date}),
    ))
}

fn validate_ids(ids: &[u32]) -> Result<(), String> {
    if ids.contains(&0) {
        Err("appointment identifiers must be positive integers".to_owned())
    } else {
        Ok(())
    }
}

#[derive(Deserialize)]
struct ServicesWire {
    #[serde(rename = "healthcareServices")]
    services: Vec<NamedRecord>,
}

#[derive(Deserialize)]
struct LocationsWire {
    #[serde(rename = "clinicBranches")]
    locations: Vec<LocationWire>,
}

#[derive(Deserialize)]
struct CliniciansWire {
    doctors: Vec<ClinicianWire>,
}

#[derive(Deserialize)]
struct SlotsWire {
    slots: Vec<SlotWire>,
}

#[derive(Deserialize)]
struct NamedRecord {
    id: u32,
    name: String,
}

#[derive(Deserialize)]
struct LocationWire {
    id: u32,
    name: String,
    city: Option<String>,
    address: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClinicianWire {
    id: u32,
    clinic_branch_id: u32,
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SlotWire {
    id: Option<String>,
    doctor_id: u32,
    start_date: String,
    end_date: String,
}

/// Translate the current TBC appointment-service catalog.
///
/// # Errors
///
/// Returns a compatibility error when an envelope, identifier, or name is invalid.
pub fn normalize_services(payload: Value) -> Result<Vec<BookableService>, Error> {
    let wire: ServicesWire =
        serde_json::from_value(payload).map_err(|_| Error::InvalidShape("bookable services"))?;
    wire.services
        .into_iter()
        .map(|record| {
            Ok(BookableService {
                service_id: positive_id(record.id)?,
                name: wire::label(&record.name, "booking-service name")?,
            })
        })
        .collect()
}

/// Translate the current appointment branch catalog into provider locations.
///
/// # Errors
///
/// Returns a compatibility error when a required field or supplied label is invalid.
pub fn normalize_locations(payload: Value) -> Result<Vec<ProviderLocation>, Error> {
    let wire: LocationsWire = serde_json::from_value(payload)
        .map_err(|_| Error::InvalidShape("appointment locations"))?;
    wire.locations
        .into_iter()
        .map(|record| {
            Ok(ProviderLocation {
                location_id: positive_id(record.id)?,
                name: wire::label(&record.name, "appointment-location name")?,
                city: record
                    .city
                    .map(|city| wire::label(&city, "appointment city"))
                    .transpose()?,
                address: record
                    .address
                    .map(|address| wire::label(&address, "appointment address"))
                    .transpose()?,
            })
        })
        .collect()
}

/// Translate date-specific clinicians, checking their selected branch.
///
/// # Errors
///
/// Returns a compatibility error for an invalid field or mismatched branch.
pub fn normalize_clinicians(
    payload: Value,
    location_id: u32,
) -> Result<Vec<AvailableClinician>, Error> {
    let wire: CliniciansWire =
        serde_json::from_value(payload).map_err(|_| Error::InvalidShape("available clinicians"))?;
    wire.doctors
        .into_iter()
        .map(|record| {
            if record.clinic_branch_id != positive_id(location_id)? {
                return Err(Error::InvalidField("clinician branch"));
            }
            Ok(AvailableClinician {
                clinician_id: positive_id(record.id)?,
                location_id,
                name: wire::label(&record.name, "clinician name")?,
            })
        })
        .collect()
}

/// Translate current availability into complete intervals within a selected date window.
///
/// # Errors
///
/// Returns a compatibility error for invalid input, malformed intervals, or a
/// clinician or date outside the selected query. A missing slot identifier is valid.
pub fn normalize_slots(payload: Value, query: &SlotQuery) -> Result<Vec<AppointmentSlot>, Error> {
    slots_request(query).map_err(|_| Error::InvalidField("appointment query"))?;
    let first = calendar_day(&query.from_date).ok_or(Error::InvalidField("appointment query"))?;
    let last = calendar_day(&query.to_date).ok_or(Error::InvalidField("appointment query"))?;
    let wire: SlotsWire =
        serde_json::from_value(payload).map_err(|_| Error::InvalidShape("appointment slots"))?;
    wire.slots
        .into_iter()
        .map(|record| {
            let start = timestamp(&record.start_date).ok_or(Error::InvalidField("slot start"))?;
            let end = timestamp(&record.end_date).ok_or(Error::InvalidField("slot end"))?;
            if record.doctor_id != query.clinician_id {
                return Err(Error::InvalidField("slot clinician"));
            }
            // TBC's date filters use Georgia's UTC+04:00 calendar day.
            let local_day = (start + 4 * 3600).div_euclid(86400);
            if end <= start || !(i64::from(first)..=i64::from(last)).contains(&local_day) {
                return Err(Error::InvalidField("slot selection or interval"));
            }
            let slot_id = record
                .id
                .map(EndpointId::parse)
                .transpose()
                .map_err(|_| Error::InvalidField("slot identifier"))?
                .map(|id| id.as_str().to_owned());
            Ok(AppointmentSlot {
                slot_id,
                clinician_id: record.doctor_id,
                service_id: query.service_id,
                location_id: query.location_id,
                starts_at: record.start_date,
                ends_at: record.end_date,
            })
        })
        .collect()
}

const fn positive_id(id: u32) -> Result<u32, Error> {
    if id == 0 {
        Err(Error::InvalidField("appointment identifier"))
    } else {
        Ok(id)
    }
}

fn calendar_day(value: &str) -> Option<u32> {
    if !wire::is_iso_calendar_date(value) {
        return None;
    }
    let year = value[..4].parse::<u32>().ok()?;
    let prior = year.checked_sub(1)?;
    let month = value[5..7].parse::<usize>().ok()?;
    let day = value[8..].parse::<u32>().ok()?;
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let months = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    Some(
        prior * 365 + prior / 4 - prior / 100
            + prior / 400
            + months[..month - 1].iter().sum::<u32>()
            + day,
    )
}

pub fn timestamp(value: &str) -> Option<i64> {
    let bytes = value.as_bytes();
    if !value.is_ascii()
        || !matches!(bytes.len(), 20 | 25)
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return None;
    }
    let day = calendar_day(&value[..10])?;
    let hours = digits(&value[11..13], 23)?;
    let minutes = digits(&value[14..16], 59)?;
    let seconds = digits(&value[17..19], 59)?;
    let offset = if bytes.len() == 20 {
        if bytes[19] != b'Z' {
            return None;
        }
        0
    } else {
        if !matches!(bytes[19], b'+' | b'-') || bytes[22] != b':' {
            return None;
        }
        let offset = digits(&value[20..22], 23)? * 3600 + digits(&value[23..25], 59)? * 60;
        if bytes[19] == b'-' { -offset } else { offset }
    };
    Some(i64::from(day) * 86400 + hours * 3600 + minutes * 60 + seconds - offset)
}

fn digits(value: &str, maximum: i64) -> Option<i64> {
    if !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse::<i64>().ok().filter(|value| *value <= maximum)
}
