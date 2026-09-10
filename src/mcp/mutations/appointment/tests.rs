use rmcp::model::CallToolResponse;

mod confirmation;

use super::*;
use crate::mcp::mutations::{
    MutationCoordinator,
    tests::{scripted_http_responses, scripted_json_responses},
};

fn input() -> BookingInput {
    BookingInput {
        policy_id: "42".to_owned(),
        slot: AppointmentSlot {
            slot_id: None,
            clinician_id: 3,
            service_id: 1,
            location_id: 2,
            starts_at: "2099-09-04T12:00:00+04:00".to_owned(),
            ends_at: "2099-09-04T12:20:00+04:00".to_owned(),
        },
    }
}

fn preparation() -> Vec<Value> {
    vec![
        json!([{"polId":42,"medCustomerId":77,"idn":"synthetic-private-id",
        "insObject":"synthetic-policy-1234","fullName":"Member Unit",
            "toFromDate":"2099-01-01T00:00:00","toToDate":"2099-12-31T00:00:00",
        "dateOfBirthd":"02.01.2000"}]),
        json!({"id":77,"firstName":"Unit","lastName":"Member",
            "birthDate":"2000-01-02T00:00:00","dateOfBirthd":"02.01.2000","mobile":"+995000000000"}),
        json!({"healthcareServices":[{"id":1,"name":"Example consultation"}]}),
        json!({"clinicBranches":[{"id":2,"name":"Example Clinic","address":"Example Street"}]}),
        json!({"doctors":[{"id":3,"clinicBranchId":2,"name":"Example Doctor"}]}),
        json!({"notifications":[],"notificationsHistory":[]}),
        json!({"slots":[{"id":null,"doctorId":3,"startDate":input().slot.starts_at,
            "endDate":input().slot.ends_at}]}),
    ]
}

fn readback() -> Value {
    json!({"notifications":[{"medicalBookingDetails":{"bookingId":90,
        "doctorName":"Example Doctor","clinicName":"Example Clinic","date":"2099-09-04T12:00:00"}}],
        "notificationsHistory":[]})
}

#[test]
fn input_rejects_invalid_or_past_intervals_and_accepts_null_slot() {
    let original = input();
    assert!(validate_slot(&original, 0).is_ok());
    for slot in [
        AppointmentSlot {
            clinician_id: 0,
            ..original.slot.clone()
        },
        AppointmentSlot {
            ends_at: original.slot.starts_at.clone(),
            ..original.slot.clone()
        },
        AppointmentSlot {
            starts_at: "1900-01-01T12:00:00+04:00".to_owned(),
            ..original.slot.clone()
        },
        AppointmentSlot {
            starts_at: "2099-09-04T08:00:00Z".to_owned(),
            ..original.slot.clone()
        },
        AppointmentSlot {
            slot_id: Some("../bad".to_owned()),
            ..original.slot.clone()
        },
    ] {
        assert!(
            validate_slot(
                &BookingInput {
                    slot,
                    ..original.clone()
                },
                0
            )
            .is_err()
        );
    }
    assert!(validate_slot(&original, i64::MAX).is_err());
    let mut json = serde_json::to_value(original).unwrap();
    json["slot"]["patient_personal_number"] = json!("never accept caller identity");
    assert!(serde_json::from_value::<BookingInput>(json).is_err());
}

#[test]
fn time_matching_requires_the_exact_interval_start() {
    let expected = &input().slot.starts_at;
    for time in ["2099-09-04T12:00:00", "2099-09-04T08:00:00Z", expected] {
        assert!(same_time(time, expected));
    }
    for time in [
        "2099-09-04",
        "2099-09-04T12:20:00",
        "2099-09-04T12:00:00Z",
        "bad",
    ] {
        assert!(!same_time(time, expected));
    }
}

#[tokio::test]
async fn review_reads_only_and_is_minimized() {
    let (url, captured, worker) = scripted_json_responses(preparation());
    let client = TbcClient::build(&url, "synthetic-token", 8192).unwrap();
    let prepared = prepare(&client, &input()).await.unwrap();
    let review = prepared.review_message();
    for expected in [
        "Example Doctor",
        "Example Clinic",
        "Example Street",
        "12:00:00+04:00",
        "12:20:00+04:00",
        "No medical documents",
        "copay",
    ] {
        assert!(review.contains(expected));
    }
    for private in [
        "synthetic-private-id",
        "synthetic-policy-1234",
        "+995000000000",
        "2000-01-02",
    ] {
        assert!(!review.contains(private));
        assert!(!format!("{:?}", prepared.payload).contains(private));
    }
    let body = MutationEndpoint::CreateHealthcareBooking(prepared.payload).request();
    assert_eq!(
        body.body(),
        &json!({"SlotId":null,"StartDate":input().slot.starts_at,
        "EndDate":input().slot.ends_at,"DoctorId":3,"HealthcareServiceId":1,
        "PatientPersonalNumber":"synthetic-private-id","PatientFirstName":"Unit",
        "PatientLastName":"Member","PatientPhoneNumber":"+995000000000",
        "PatientBirthDate":"2000-01-02","ClinicBranchId":2})
    );
    assert_eq!(
        body.path(),
        "/api/DoctorBooking/CreateHealthcareServiceBooking"
    );
    worker.join().unwrap();
    let requests = captured.into_iter().collect::<Vec<_>>();
    assert_eq!(requests.len(), 7);
    assert!(
        !requests
            .iter()
            .any(|request| request.contains("CreateHealthcareServiceBooking"))
    );
}

#[tokio::test]
async fn confirmed_review_rechecks_then_submits_exactly_once_and_verifies_reference() {
    let mut replies = preparation();
    replies.extend(preparation());
    replies.extend([json!({"bookingId":90}), readback()]);
    let (url, captured, worker) = scripted_json_responses(replies);
    let client = TbcClient::build(&url, "synthetic-token", 8192).unwrap();
    let coordinator = MutationCoordinator::default();
    let mut selected = input();
    let review = coordinator
        .book_appointment(&client, &selected)
        .await
        .unwrap();
    selected.slot.clinician_id += 1;
    let state = review.review_id;
    let response = coordinator.finish(&client, state.clone()).await.unwrap();
    let CallToolResponse::Complete(result) = response else {
        panic!("result required");
    };
    let result = result.structured_content.unwrap();
    assert_eq!(result["outcome"], "booked");
    assert_eq!(result["booking_id"], "90");
    assert_eq!(result["retry_safe"], false);
    assert!(coordinator.finish(&client, state).await.is_err());
    worker.join().unwrap();
    let requests = captured.into_iter().collect::<Vec<_>>();
    assert_eq!(requests.len(), 16);
    assert_eq!(
        requests
            .iter()
            .filter(|request| request
                .starts_with("POST /api/DoctorBooking/CreateHealthcareServiceBooking "))
            .count(),
        1
    );
    assert!(requests[13].starts_with("POST /api/DoctorBooking/GetDoctorSlots "));
    assert!(requests[14].contains(r#""DoctorId":3"#));
    assert!(requests[15].starts_with("GET /api/Mobile/GetNotifications "));
}

#[tokio::test]
async fn stale_patient_options_slots_and_existing_bookings_prevent_submission() {
    for (index, changed) in [
        (0, json!([])),
        (1, json!({"id":88})),
        (2, json!({"healthcareServices":[]})),
        (3, json!({"clinicBranches":[]})),
        (
            4,
            json!({"doctors":[{"id":3,"clinicBranchId":999,"name":"Example Doctor"}]}),
        ),
        (5, readback()),
        (6, json!({"slots":[]})),
    ] {
        let mut replies = preparation();
        replies[index] = changed;
        replies.truncate(index + 1);
        let (url, captured, worker) = scripted_json_responses(replies);
        let client = TbcClient::build(&url, "synthetic-token", 8192).unwrap();
        assert!(prepare(&client, &input()).await.is_err());
        worker.join().unwrap();
        assert_eq!(captured.into_iter().count(), index + 1);
    }
}

#[tokio::test]
async fn post_review_patient_change_and_stale_slot_never_write() {
    for index in [1, 6] {
        let mut replies = preparation();
        let mut next = preparation();
        if index == 1 {
            next[1]["mobile"] = json!("+995111111111");
        } else {
            next[6] = json!({"slots":[]});
        }
        replies.extend(next);
        let (url, captured, worker) = scripted_json_responses(replies);
        let client = TbcClient::build(&url, "synthetic-token", 8192).unwrap();
        let prepared = prepare(&client, &input()).await.unwrap();
        assert!(prepared.execute(&client).await.is_err());
        worker.join().unwrap();
        let requests = captured.into_iter().collect::<Vec<_>>();
        assert_eq!(requests.len(), 14);
        assert!(
            !requests
                .iter()
                .any(|request| request.contains("CreateHealthcareServiceBooking"))
        );
    }
}

#[tokio::test]
async fn ambiguous_response_is_not_retried_or_reported_as_confirmed() {
    let mut replies = preparation()
        .into_iter()
        .chain(preparation())
        .map(|body| ("200 OK", body))
        .collect::<Vec<_>>();
    replies.extend([
        ("500 Internal Server Error", json!({})),
        ("200 OK", readback()),
    ]);
    let (url, captured, worker) = scripted_http_responses(replies);
    let client = TbcClient::build(&url, "synthetic-token", 8192).unwrap();
    let prepared = prepare(&client, &input()).await.unwrap();
    let result = prepared
        .execute(&client)
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert_eq!(result["outcome"], "outcome_unknown");
    assert_eq!(result["retry_safe"], false);
    worker.join().unwrap();
    assert_eq!(
        captured
            .into_iter()
            .filter(|request| request.contains("CreateHealthcareServiceBooking"))
            .count(),
        1
    );
}

#[tokio::test]
async fn readback_requires_a_new_exact_reservation_and_correlated_reference() {
    let (url, _captured, worker) = scripted_json_responses(preparation());
    let client = TbcClient::build(&url, "synthetic-token", 8192).unwrap();
    let mut prepared = prepare(&client, &input()).await.unwrap();
    worker.join().unwrap();
    for response in [json!({}), json!(false), json!({"bookingId":91})] {
        let result = prepared.readback_result(&Ok(response), bookings::normalize(readback()));
        assert_eq!(
            result.structured_content.unwrap()["outcome"],
            "submitted_readback_unverified"
        );
    }
    let mut wrong_time = readback();
    wrong_time["notifications"][0]["medicalBookingDetails"]["date"] = json!("2099-09-04T15:00:00");
    let result = prepared.readback_result(
        &Ok(json!({"bookingId":90})),
        bookings::normalize(wrong_time),
    );
    assert_eq!(
        result.structured_content.unwrap()["outcome"],
        "submitted_readback_unverified"
    );
    prepared.before = bookings::normalize(readback()).unwrap();
    let result = prepared.readback_result(
        &Ok(json!({"bookingId":90})),
        bookings::normalize(readback()),
    );
    assert_eq!(
        result.structured_content.unwrap()["outcome"],
        "submitted_readback_unverified"
    );
    let result =
        prepared.readback_result(&Ok(json!({"bookingId":90})), Err("unavailable".to_owned()));
    assert_eq!(
        result.structured_content.unwrap()["readback"]["performed"],
        false
    );
}

#[test]
fn policy_identity_names_birth_dates_and_phone_are_checked_without_guessing() {
    let values = preparation();
    let policy = values[0][0].as_object().unwrap();
    let selected = selected_policy(policy, json!(42)).unwrap();
    for (key, value) in [
        ("id", json!(88)),
        ("firstName", json!("Other")),
        ("birthDate", json!("bad")),
        ("dateOfBirthd", json!("03.01.2000")),
        ("mobile", json!("invalid")),
        ("lastName", json!(null)),
    ] {
        let mut profile = values[1].clone();
        profile[key] = value;
        assert!(patient_payload(policy, &profile, &selected, &input().slot).is_err());
    }
}

#[tokio::test]
async fn expired_missing_and_cross_session_reviews_never_contact_the_write_endpoint() {
    for mode in ["expired", "session", "missing", "different_server"] {
        let (url, captured, worker) = scripted_json_responses(preparation());
        let client = TbcClient::build(&url, "synthetic-token", 8192).unwrap();
        let coordinator = MutationCoordinator::default();
        let review = coordinator
            .book_appointment(&client, &input())
            .await
            .unwrap();
        let mut state = review.review_id;
        let active_client = if mode == "session" {
            TbcClient::build(&url, "synthetic-new-token", 8192).unwrap()
        } else {
            client
        };
        if mode == "expired" {
            coordinator
                .reviews
                .lock()
                .await
                .get_mut(&state)
                .unwrap()
                .expires_at = tokio::time::Instant::now() - std::time::Duration::from_secs(1);
        }
        if mode == "missing" {
            state = "unknown-review".to_owned();
        }
        let coordinator = if mode == "different_server" {
            MutationCoordinator::default()
        } else {
            coordinator
        };
        let result = coordinator.finish(&active_client, state).await;
        assert!(result.is_err());
        worker.join().unwrap();
        assert_eq!(captured.into_iter().count(), 7);
    }
}

#[tokio::test]
async fn coverage_dates_are_inclusive_and_unsupported_periods_fail_before_profile_read() {
    for (start, end, valid) in [
        ("2099-09-04", "2099-09-04", true),
        ("2099-09-05", "2099-12-31", false),
        ("2099-01-01", "2099-09-03", false),
    ] {
        let mut replies = preparation();
        replies[0][0]["toFromDate"] = json!(start);
        replies[0][0]["toToDate"] = json!(end);
        replies.truncate(if valid { 2 } else { 1 });
        let (url, captured, worker) = scripted_json_responses(replies);
        let client = TbcClient::build(&url, "synthetic-token", 8192).unwrap();
        let result = resolve_member(&client, &input()).await;
        assert_eq!(result.is_ok(), valid);
        if !valid {
            assert_eq!(
                result.err().unwrap(),
                "The selected policy does not establish coverage dates for this appointment"
            );
        }
        worker.join().unwrap();
        assert_eq!(captured.into_iter().count(), if valid { 2 } else { 1 });
    }
}

#[test]
fn private_text_and_selection_boundaries_are_enforced() {
    for text in ["", " ", "a\nb", &"a".repeat(257)] {
        assert!(private_text(&json!(text)).is_err());
    }
    assert_eq!(private_text(&json!("a".repeat(256))).unwrap().len(), 256);
    assert_eq!(exactly_one([1].into_iter()).unwrap(), 1);
    assert!(exactly_one([1, 2].into_iter()).is_err());
    assert!(exactly_one(Vec::<u8>::new().into_iter()).is_err());
    assert!(unix_now().unwrap() > 1_700_000_000);
    let original = input();
    let starts_at =
        timestamp(&original.slot.starts_at).unwrap() - timestamp("1970-01-01T00:00:00Z").unwrap();
    assert!(validate_slot(&original, starts_at - 1).is_ok());
    assert!(validate_slot(&original, starts_at).is_err());
}

#[tokio::test]
async fn duplicate_guard_allows_other_visits_and_stops_incomplete_matching_ones() {
    for (field, value, allowed) in [
        ("doctorName", json!("Other Doctor"), true),
        ("clinicName", json!("Other Clinic"), true),
        ("date", json!("2099-09-04T13:00:00"), true),
        ("date", json!("2099-09-04"), false),
        ("date", json!(null), false),
    ] {
        let mut replies = preparation();
        let mut existing = readback();
        existing["notifications"][0]["medicalBookingDetails"][field] = value;
        replies[5] = existing;
        if !allowed {
            replies.truncate(6);
        }
        let (url, captured, worker) = scripted_json_responses(replies);
        let client = TbcClient::build(&url, "synthetic-token", 8192).unwrap();
        let result = prepare(&client, &input()).await;
        assert_eq!(result.is_ok(), allowed);
        if !allowed {
            assert!(
                result
                    .err()
                    .unwrap()
                    .starts_with("A matching or incomplete active booking already exists")
            );
        }
        worker.join().unwrap();
        assert_eq!(captured.into_iter().count(), if allowed { 7 } else { 6 });
    }
}

#[tokio::test]
async fn every_reviewed_label_and_policy_change_blocks_the_write() {
    for index in [0, 2, 3, 4] {
        let mut replies = preparation();
        let mut next = preparation();
        match index {
            0 => next[0][0]["insObject"] = json!("changed-policy-1234"),
            2 => next[2]["healthcareServices"][0]["name"] = json!("Changed service"),
            3 => next[3]["clinicBranches"][0]["address"] = json!("Changed address"),
            _ => next[4]["doctors"][0]["name"] = json!("Changed doctor"),
        }
        replies.extend(next);
        let (url, captured, worker) = scripted_json_responses(replies);
        let client = TbcClient::build(&url, "synthetic-token", 8192).unwrap();
        let prepared = prepare(&client, &input()).await.unwrap();
        assert!(prepared.execute(&client).await.is_err());
        worker.join().unwrap();
        assert_eq!(captured.into_iter().count(), 14);
    }
}

#[test]
fn patient_validation_checks_each_identity_component_independently() {
    let values = preparation();
    let mut policy = values[0][0].as_object().unwrap().clone();
    let selected = selected_policy(&policy, json!(42)).unwrap();
    let mut profile = values[1].clone();
    for phone in ["123", "995000000000x"] {
        profile["mobile"] = json!(phone);
        assert!(patient_payload(&policy, &profile, &selected, &input().slot).is_err());
    }
    profile = values[1].clone();
    profile["id"] = json!(0);
    policy.insert("medCustomerId".to_owned(), json!(0));
    assert!(patient_payload(&policy, &profile, &selected, &input().slot).is_err());
    policy = values[0][0].as_object().unwrap().clone();
    profile = values[1].clone();
    policy.insert("dateOfBirthd".to_owned(), json!("03.01.2000"));
    assert!(patient_payload(&policy, &profile, &selected, &input().slot).is_err());
    policy = values[0][0].as_object().unwrap().clone();
    let mut reversed = selected;
    reversed.insured_full_name = "Unit Member".to_owned();
    assert!(patient_payload(&policy, &profile, &reversed, &input().slot).is_ok());
}

#[tokio::test]
async fn readback_does_not_confuse_other_doctors_clinics_or_multiple_bookings() {
    let (url, _captured, worker) = scripted_json_responses(preparation());
    let client = TbcClient::build(&url, "synthetic-token", 8192).unwrap();
    let prepared = prepare(&client, &input()).await.unwrap();
    worker.join().unwrap();
    for field in ["doctorName", "clinicName"] {
        let mut after = readback();
        after["notifications"][0]["medicalBookingDetails"][field] = json!("Unrelated");
        let result =
            prepared.readback_result(&Ok(json!({"bookingId":90})), bookings::normalize(after));
        let result = result.structured_content.unwrap();
        assert_eq!(result["outcome"], "submitted_readback_unverified");
        assert_eq!(result["readback"]["matching_bookings"], json!([]));
    }
    let mut after = readback();
    let mut second = after["notifications"][0].clone();
    second["medicalBookingDetails"]["bookingId"] = json!(91);
    after["notifications"].as_array_mut().unwrap().push(second);
    let result = prepared.readback_result(&Ok(json!({"bookingId":90})), bookings::normalize(after));
    assert_eq!(
        result.structured_content.unwrap()["outcome"],
        "submitted_readback_unverified"
    );
}
