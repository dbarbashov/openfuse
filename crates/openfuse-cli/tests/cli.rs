use std::{fs, process::Command};

fn command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_openfuse"))
}

#[test]
fn inspects_hex_without_interpreting_protocol() {
    let output = command()
        .args(["inspect", "--json", "0x00:41", "ff"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["bytes"], serde_json::json!([0, 65, 255]));
    assert_eq!(value["byte_count"], 3);
    assert_eq!(value["hex"], "00 41 ff");
}

#[test]
fn malformed_input_fails_visibly() {
    for args in [
        vec!["inspect", "0x1"],
        vec!["inspect"],
        vec!["catalog", "--write"],
        vec!["connect"],
    ] {
        let output = command().args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(!output.stderr.is_empty());
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn distinguishes_binary_and_hex_text_files() {
    let directory = std::env::temp_dir().join(format!("openfuse-cli-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("capture.txt");
    fs::write(&path, "41 42").unwrap();
    for (option, expected) in [("--file", 5), ("--hex-file", 2)] {
        let output = command()
            .args(["inspect", "--json", option])
            .arg(&path)
            .output()
            .unwrap();
        assert!(output.status.success());
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["byte_count"], expected);
    }
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn help_is_available_offline() {
    let output = command().arg("--help").output().unwrap();
    assert!(output.status.success());
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("Enumeration never opens devices")
    );
}

#[test]
fn encoded_request_decodes_with_explicit_hid_framing() {
    let encoded = command()
        .args(["encode", "--hid-output", "version-request"])
        .output()
        .unwrap();
    assert!(encoded.status.success());
    let hex = String::from_utf8(encoded.stdout).unwrap();
    assert_eq!(hex.split_whitespace().count(), 65);
    assert!(hex.starts_with("00 1a c1 "));
    let decoded = command()
        .args(["decode", "--json", "--hid-output", &hex])
        .output()
        .unwrap();
    assert!(decoded.status.success());
    let value: serde_json::Value = serde_json::from_slice(&decoded.stdout).unwrap();
    assert_eq!(value["message_id"], 0x1a);
    assert_eq!(value["flags"], 0xc1);
    assert_eq!(value["unknown_flags"], 1);
    assert_eq!(value["start"], true);
    assert_eq!(value["end"], true);
    assert_eq!(value["ack"], false);

    let wrong_framing = command().args(["decode", &hex]).output().unwrap();
    assert_eq!(wrong_framing.status.code(), Some(2));
}

#[test]
fn catalog_serializes_product_ids_and_evidence() {
    let output = command().args(["catalog", "--json"]).output().unwrap();
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let devices = value.as_array().unwrap();
    let mustang = devices
        .iter()
        .find(|device| device["product_id"] == 4)
        .unwrap();
    assert_eq!(mustang["vendor_id"], 0x1ed8);
    assert_eq!(mustang["bootloader"], false);
    assert_eq!(mustang["evidence"], "Not hardware-validated");
    assert!(
        devices
            .iter()
            .any(|device| device["product_id"] == 39 && device["bootloader"] == true)
    );
}

#[cfg(not(feature = "hid"))]
#[test]
fn disabled_hid_support_is_not_reported_as_no_devices() {
    let output = command().args(["devices", "--json"]).output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("--features hid")
    );
}

#[test]
fn inbound_decode_retains_unknown_status_and_reserved_bytes() {
    let mut report = [0xaa; 64];
    report[..6].copy_from_slice(&[0, 0xc3, 0xfe, 0xee, 2, 7]);
    let input = report
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ");
    let output = command()
        .args(["decode", "--json", &input])
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["direction"], "device_to_host");
    assert_eq!(value["fields"]["kind"], "handshake");
    assert_eq!(value["fields"]["status"], 238);
    assert_eq!(value["fields"]["command"], 254);
    assert_eq!(
        value["body_hex"]
            .as_str()
            .unwrap()
            .split_whitespace()
            .count(),
        62
    );
    assert!(value["body_hex"].as_str().unwrap().ends_with("aa aa"));
}

#[test]
fn simulated_session_returns_complete_raw_capture_without_hid() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_openfuse"))
        .args(["session", "--simulate", "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["simulated"], true);
    assert_eq!(result["hardware_validated"], false);
    assert_eq!(result["status"]["snapshot"]["modules"], 5);
    let packets = result["capture"]["reports"].as_array().unwrap();
    assert_eq!(packets.len(), 9);
    assert!(packets.iter().all(|r| r.as_array().unwrap().len() == 64));
    assert_eq!(packets[0][2], 4);
    assert_eq!(packets[8][2], 0);
}

#[test]
fn session_requires_an_explicit_single_target() {
    for args in [
        vec!["session"],
        vec!["session", "--simulate", "--device", "anything"],
        vec!["session", "--device"],
    ] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_openfuse"))
            .args(args)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("session requires exactly"));
    }
}
