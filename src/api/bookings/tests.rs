use serde_json::json;

use super::*;

#[test]
fn native_notifications_are_filtered_and_history_remains_separate() {
    let booking = json!({"medicalBookingDetails":{"bookingId":42,
        "doctorName":"Example Doctor", "clinicName":"Example Clinic", "date":"2099-09-04T12:00:00"},
        "policyNumber":"private-policy", "description":"private-medical-note"});
    let result = normalize(json!({"notifications":[booking],"notificationsHistory":[
        {"medicalBookingDetails":null,"medicalReimbursementDetails":{"insuredFullName":"private-name"}}
    ]})).unwrap();
    assert_eq!(result.active.len(), 1);
    assert_eq!(result.active[0].booking_id, "42");
    assert!(result.history.is_empty());
    assert!(!serde_json::to_string(&result).unwrap().contains("private-"));
    assert_eq!(
        list_request().path_and_query(),
        "/api/Mobile/GetNotifications"
    );
    assert_eq!(list_request().method(), super::super::HttpMethod::Get);
}

#[test]
fn a_separate_date_never_discards_the_booking_time() {
    let result = normalize(json!({"notifications":[{"medicalBookingDetails":{
        "bookingId":42,"doctorName":"Example Doctor","clinicName":"Example Clinic",
        "date":"04.09.2099","startTimeFormated":"12:00"
    }}],"notificationsHistory":[]}))
    .unwrap();
    assert_eq!(result.active[0].scheduled_time.as_deref(), Some("12:00"));
    assert_eq!(
        result.active[0].scheduled_date.as_deref(),
        Some("04.09.2099")
    );
}

#[test]
fn missing_groups_and_invalid_reservations_fail_closed() {
    for value in [
        json!({}),
        json!({"notifications":[],"notificationsHistory":null}),
        json!({"notifications":[{"medicalBookingDetails":{}}],"notificationsHistory":[]}),
        json!({"notifications":[{"medicalBookingDetails":{"bookingId":false,
            "doctorName":"A", "clinicName":"B"}}],"notificationsHistory":[]}),
    ] {
        assert!(normalize(value).is_err());
    }
    for value in [
        json!(null),
        json!(0),
        json!(-1),
        json!(1.5),
        json!("../id"),
        json!(""),
    ] {
        assert!(booking_id(&value).is_none());
    }
    for label in ["", "\n", &"a".repeat(513)] {
        assert!(super::label(label.to_owned()).is_err());
    }
    assert_eq!(super::label("a".repeat(512)).unwrap().len(), 512);
    assert_eq!(
        booking_id(&json!("booking-42")).as_deref(),
        Some("booking-42")
    );
}
