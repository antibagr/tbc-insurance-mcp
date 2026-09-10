//! Synthetic appointment contracts; never recorded member data.

use super::*;
use crate::api::HttpMethod;

fn query() -> SlotQuery {
    SlotQuery {
        service_id: 21,
        location_id: 22,
        clinician_id: 23,
        from_date: "2028-02-28".to_owned(),
        to_date: "2028-03-01".to_owned(),
    }
}

fn slot() -> Value {
    json!({"id": null, "doctorId": 23, "startDate": "2028-02-29T11:00:00+04:00",
        "endDate": "2028-02-29T11:20:00+04:00"})
}

#[test]
fn appointment_routes_and_numeric_payloads_match_the_live_contract() {
    let services = services_request();
    assert_eq!(services.method(), HttpMethod::Get);
    assert_eq!(
        services.path_and_query(),
        "/api/DoctorBooking/GetHealthcareServices"
    );
    assert_eq!(services.operation(), "list_bookable_services");
    assert_eq!(services.body(), None);
    let cases = [
        (
            locations_request(&ServiceQuery { service_id: 21 }).unwrap(),
            "/api/DoctorBooking/GetClinicBranches",
            "list_provider_locations",
            json!({"HealthcareServiceId": 21}),
        ),
        (
            clinicians_request(&ClinicianQuery {
                service_id: 21,
                location_id: 22,
                date: "2028-02-29".to_owned(),
            })
            .unwrap(),
            "/api/DoctorBooking/GetHealthcareServiceDoctors",
            "list_available_clinicians",
            json!({"HealthcareServiceId": 21, "ClinicBranchId": 22, "exactDate": "2028-02-29"}),
        ),
        (
            slots_request(&query()).unwrap(),
            "/api/DoctorBooking/GetDoctorSlots",
            "list_appointment_slots",
            json!({"HealthcareServiceId": 21, "ClinicBranchId": 22, "DoctorId": 23,
                "FromDate": "2028-02-28", "ToDate": "2028-03-01"}),
        ),
    ];
    for (request, path, operation, body) in cases {
        assert_eq!(request.method(), HttpMethod::Post);
        assert_eq!(request.path_and_query(), path);
        assert_eq!(request.operation(), operation);
        assert_eq!(request.body(), Some(&body));
    }
}

#[test]
fn invalid_identifiers_dates_and_unbounded_windows_never_build_requests() {
    assert!(locations_request(&ServiceQuery { service_id: 0 }).is_err());
    for position in 0..3 {
        let mut input = query();
        match position {
            0 => input.service_id = 0,
            1 => input.location_id = 0,
            _ => input.clinician_id = 0,
        }
        assert!(slots_request(&input).is_err());
    }
    for date in [
        "",
        "0000-01-01",
        "2027-02-29",
        "2100-02-29",
        "2028-04-31",
        "2028-00-01",
        "2028-13-01",
        "2028-01-00",
        "2028-1-01",
        "２０２８-01-01",
        "2028-01-01\n",
        "2028-01-01T00:00:00",
        "2028-01-+1",
    ] {
        assert!(calendar_day(date).is_none(), "accepted {date:?}");
        assert!(
            clinicians_request(&ClinicianQuery {
                service_id: 21,
                location_id: 22,
                date: date.to_owned()
            })
            .is_err()
        );
        let mut input = query();
        input.from_date = date.to_owned();
        assert!(slots_request(&input).is_err());
        input = query();
        input.to_date = date.to_owned();
        assert!(slots_request(&input).is_err());
    }
    for (from, to, valid) in [
        ("2028-02-01", "2028-03-02", true),
        ("2028-02-01", "2028-03-03", false),
        ("2028-03-01", "2028-03-01", true),
        ("2028-03-02", "2028-03-01", false),
        ("2027-12-20", "2028-01-19", true),
    ] {
        let mut input = query();
        input.from_date = from.to_owned();
        input.to_date = to.to_owned();
        assert_eq!(slots_request(&input).is_ok(), valid, "{from} {to}");
    }
    assert_eq!(
        calendar_day("2000-03-01").unwrap() - calendar_day("2000-02-28").unwrap(),
        2
    );
    assert_eq!(
        calendar_day("2100-03-01").unwrap() - calendar_day("2100-02-28").unwrap(),
        1
    );
}

#[test]
fn named_catalogs_preserve_verified_fields_and_drop_unrelated_data() {
    assert_eq!(
        normalize_services(json!({"healthcareServices":[{"id":21,"name":"Example consultation"}]}))
            .unwrap(),
        [BookableService {
            service_id: 21,
            name: "Example consultation".to_owned()
        }]
    );
    assert_eq!(
        normalize_locations(json!({"clinicBranches":[{"id":22,"name":"Test clinic",
        "city":"Tbilisi","address":"Test address","email":"private@example.invalid"}]}))
        .unwrap(),
        [ProviderLocation {
            location_id: 22,
            name: "Test clinic".to_owned(),
            city: Some("Tbilisi".to_owned()),
            address: Some("Test address".to_owned())
        }]
    );
    assert_eq!(
        normalize_clinicians(
            json!({"doctors":[{"id":23,"clinicBranchId":22,"name":"Test clinician"}]}),
            22
        )
        .unwrap(),
        [AvailableClinician {
            clinician_id: 23,
            location_id: 22,
            name: "Test clinician".to_owned()
        }]
    );
    for payload in [
        json!({}),
        json!([]),
        json!({"healthcareServices":null}),
        json!({"healthcareServices":[{"id":0,"name":"Test"}]}),
        json!({"healthcareServices":[{"id":21,"name":"\n"}]}),
        json!({"healthcareServices":[{"id":21,"name":"a".repeat(513)}]}),
        json!({"healthcareServices":[{"healthcareServiceId":21,"name":"Old schema"}]}),
    ] {
        assert!(normalize_services(payload).is_err());
    }
    assert!(
        normalize_clinicians(
            json!({"doctors":[{"id":23,"clinicBranchId":99,"name":"Other branch"}]}),
            22
        )
        .is_err()
    );
    assert!(
        normalize_clinicians(
            json!({"doctors":[{"id":0,"clinicBranchId":22,"name":"Test"}]}),
            22
        )
        .is_err()
    );
    assert!(normalize_locations(json!({"clinicBranches":[{"id":0,"name":"Test"}]})).is_err());
}

#[test]
fn empty_results_are_distinct_from_missing_or_incompatible_arrays() {
    assert!(
        normalize_services(json!({"healthcareServices":[]}))
            .unwrap()
            .is_empty()
    );
    assert!(
        normalize_locations(json!({"clinicBranches":[]}))
            .unwrap()
            .is_empty()
    );
    assert!(
        normalize_clinicians(json!({"doctors":[]}), 22)
            .unwrap()
            .is_empty()
    );
    assert!(
        normalize_slots(json!({"slots":[]}), &query())
            .unwrap()
            .is_empty()
    );
    for payload in [json!({}), json!([]), json!({"slots":null})] {
        assert!(normalize_slots(payload, &query()).is_err());
    }
    assert!(normalize_locations(json!({})).is_err());
    assert!(normalize_clinicians(json!({}), 22).is_err());
}

#[test]
fn nullable_slot_ids_preserve_the_complete_interval_and_query_context() {
    let result = normalize_slots(json!({"slots":[slot()]}), &query()).unwrap();
    assert_eq!(
        result,
        [AppointmentSlot {
            slot_id: None,
            clinician_id: 23,
            service_id: 21,
            location_id: 22,
            starts_at: "2028-02-29T11:00:00+04:00".to_owned(),
            ends_at: "2028-02-29T11:20:00+04:00".to_owned()
        }]
    );
    let mut record = slot();
    record["id"] = json!("test_202802291100");
    assert_eq!(
        normalize_slots(json!({"slots":[record]}), &query()).unwrap()[0]
            .slot_id
            .as_deref(),
        Some("test_202802291100")
    );
}

#[test]
fn date_windows_use_tbilisi_calendar_dates_when_offsets_differ() {
    let mut input = query();
    input.from_date = "2028-02-29".to_owned();
    input.to_date = "2028-02-29".to_owned();
    let mut record = slot();
    record["startDate"] = json!("2028-02-28T21:00:00Z");
    record["endDate"] = json!("2028-02-28T21:20:00Z");
    assert!(normalize_slots(json!({"slots":[record.clone()]}), &input).is_ok());
    record["startDate"] = json!("2028-02-29T21:00:00Z");
    record["endDate"] = json!("2028-02-29T21:20:00Z");
    assert!(normalize_slots(json!({"slots":[record]}), &input).is_err());
}

#[test]
fn calendar_days_and_timestamp_units_match_independent_reference_values() {
    // Proleptic Gregorian ordinals, also used by Python's datetime.date.toordinal.
    for (date, ordinal) in [
        ("0001-01-01", 1),
        ("0001-12-31", 365),
        ("0004-03-01", 1156),
        ("0100-03-01", 36219),
        ("0400-03-01", 145_792),
        ("1970-01-01", 719_163),
        ("2000-01-01", 730_120),
        ("2028-02-29", 740_406),
        ("9999-12-31", 3_652_059),
    ] {
        assert_eq!(calendar_day(date), Some(ordinal), "{date}");
    }
    let midnight = 740_406 * 86400_i64;
    for (date_time, seconds) in [
        ("2028-02-29T00:00:00Z", 0),
        ("2028-02-29T01:02:03Z", 3723),
        ("2028-02-29T23:59:59Z", 86399),
        ("2028-02-29T01:02:03+04:30", -12477),
        ("2028-02-29T01:02:03-03:45", 17223),
    ] {
        assert_eq!(
            timestamp(date_time),
            Some(midnight + seconds),
            "{date_time}"
        );
    }
}

#[test]
fn malformed_cross_clinician_and_out_of_window_slots_fail_closed() {
    for (field, value) in [
        ("doctorId", json!(99)),
        ("id", json!("../unsafe")),
        ("id", json!(42)),
        ("startDate", json!("2028-02-27T11:00:00+04:00")),
        ("startDate", json!("2028-03-02T11:00:00+04:00")),
        ("endDate", json!("2028-02-29T11:00:00+04:00")),
        ("endDate", json!("2028-02-29T10:59:59+04:00")),
        ("endDate", json!("2028-02-29T11:20:00+05:00")),
        ("endDate", json!(null)),
    ] {
        let mut record = slot();
        record[field] = value;
        assert!(
            normalize_slots(json!({"slots":[record]}), &query()).is_err(),
            "{field}"
        );
    }
    for invalid in [
        "",
        "2028-02-29",
        "2028-02-29T11:00:00",
        "2028-02-29T24:00:00+04:00",
        "2028-02-29T11:60:00+04:00",
        "2028-02-29T11:00:60+04:00",
        "2028-02-29X11:00:00+04:00",
        "2028-02-29T11:00:00+24:00",
        "2028-02-29T11:00:00+04:60",
        "2028-02-29T11:00:00?04:00",
        "2028-02-29T11:00:00+0400",
        "2028-02-29T11:00:00z",
        "2028-02-29T+1:00:00+04:00",
        "2028-02-29T11:00:00+04x00",
        "2028-02-29T11x00:00+04:00",
        "2028-02-29T11:00x00+04:00",
    ] {
        assert!(timestamp(invalid).is_none(), "accepted {invalid}");
    }
    assert_eq!(
        timestamp("2028-02-29T11:00:00+04:00"),
        timestamp("2028-02-29T07:00:00Z")
    );
    assert_eq!(
        timestamp("2028-02-29T11:00:00-04:00"),
        timestamp("2028-02-29T15:00:00Z")
    );
}
