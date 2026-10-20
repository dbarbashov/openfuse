use super::{DeviceInfo, MAX_CAPTURE_BYTES, WireCapture, parse_capture_json};
use serde_json::{Value, json};
fn fixture() -> WireCapture {
    WireCapture {
        device: DeviceInfo {
            id: "exact-hid-path".into(),
            name: "Saved Mustang".into(),
            product_id: 0x14,
            serial: Some("SERIAL-1".into()),
            interface_number: 0,
            unavailable_reason: None,
        },
        generation: 42,
        version: Some("2.0.42".into()),
        reports: vec![(0..64).collect()],
        received: vec![vec![0xfe; 64]],
        dropped_received: 9,
    }
}
fn parse(value: &Value) -> Result<WireCapture, String> {
    parse_capture_json(&serde_json::to_string(value).unwrap())
}
#[test]
fn raw_and_wrapped_capture_json_keep_all_bytes_and_device_identity() {
    let capture = fixture();
    let raw = serde_json::to_value(&capture).unwrap();
    for value in [
        raw.clone(),
        json!({"kind":"saved_session","capture":raw,"hardware_validated":false}),
    ] {
        let read = parse(&value).unwrap();
        assert_eq!(read.device, capture.device);
        assert_eq!(read.generation, 42);
        assert_eq!(read.version, capture.version);
        assert_eq!(read.reports, capture.reports);
        assert_eq!(read.received, capture.received);
        assert_eq!(read.dropped_received, 9);
    }
}
#[test]
fn capture_length_accepts_exactly_64_bytes_for_data_and_diagnostics() {
    let base = serde_json::to_value(fixture()).unwrap();
    for field in ["reports", "received"] {
        for length in [0, 1, 63, 65, 128] {
            let mut value = base.clone();
            value[field][0] = json!(vec![0u8; length]);
            assert!(parse(&value).unwrap_err().contains("64 bytes"));
        }
    }
    let mut bad = base;
    bad["reports"][0][2] = json!(256);
    assert!(parse(&bad).is_err());
    bad["reports"][0][2] = json!(-1);
    assert!(parse(&bad).is_err());
}
#[test]
fn capture_report_limits_accept_boundaries_and_reject_overflow() {
    let mut capture = fixture();
    capture.reports = vec![vec![0; 64]; 512];
    capture.received = vec![vec![255; 64]; 256];
    assert!(parse(&serde_json::to_value(&capture).unwrap()).is_ok());
    capture.reports.push(vec![0; 64]);
    assert!(
        parse(&serde_json::to_value(&capture).unwrap())
            .unwrap_err()
            .contains("limits")
    );
    capture.reports.pop();
    capture.received.push(vec![0; 64]);
    assert!(
        parse(&serde_json::to_value(&capture).unwrap())
            .unwrap_err()
            .contains("limits")
    );
    capture.reports.clear();
    capture.received.clear();
    assert!(parse(&serde_json::to_value(&capture).unwrap()).is_err());
}
#[test]
fn capture_json_byte_budget_is_checked_before_deserialization() {
    let mut source = serde_json::to_string(&fixture()).unwrap();
    source.extend(std::iter::repeat_n(' ', MAX_CAPTURE_BYTES - source.len()));
    assert_eq!(source.len(), MAX_CAPTURE_BYTES);
    assert!(parse_capture_json(&source).is_ok());
    source.push(' ');
    assert!(parse_capture_json(&source).unwrap_err().contains("4 MiB"));
}
#[test]
fn capture_identity_bounds_and_malformed_envelopes_fail_explicitly() {
    let mut capture = fixture();
    capture.device.id = "x".repeat(4096);
    capture.device.name = "n".repeat(1024);
    capture.device.serial = Some("s".repeat(1024));
    assert!(parse(&serde_json::to_value(&capture).unwrap()).is_ok());
    for field in ["id", "name", "serial"] {
        let mut value = serde_json::to_value(&capture).unwrap();
        let text = value["device"][field].as_str().unwrap().to_owned() + "x";
        value["device"][field] = json!(text);
        assert!(parse(&value).unwrap_err().contains("identity"));
    }
    for input in [
        "",
        "[]",
        "{",
        "null",
        "{}",
        r#"{"capture":null}"#,
        r#"{"capture":{"reports":[]}}"#,
    ] {
        assert!(parse_capture_json(input).is_err());
    }
}
