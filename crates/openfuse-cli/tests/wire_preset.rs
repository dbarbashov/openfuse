use openfuse_core::preset::PresetDocument;
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "openfuse-wire-cli-{}-{nonce}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn write(&self, name: &str, data: impl AsRef<[u8]>) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, data).unwrap();
        path
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_openfuse"))
}
fn decode(path: &Path) -> Output {
    command()
        .args(["capture-decode", "--file"])
        .arg(path)
        .arg("--json")
        .output()
        .unwrap()
}
fn successful_json(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
fn session() -> Output {
    command()
        .args(["session", "--simulate", "--json"])
        .output()
        .unwrap()
}
fn parameter(doc: &PresetDocument, key: &str, index: u8) -> u16 {
    doc.modules()
        .iter()
        .find(|m| m.key == key)
        .unwrap()
        .parameters
        .iter()
        .find(|p| p.index == index)
        .unwrap()
        .value
}

#[test]
fn simulated_capture_becomes_editable_xml_and_offline_wire_roundtrip_exposes_precision() {
    let scratch = Scratch::new();
    let first = session();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let session_json: Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(session_json["simulated"], true);
    assert_eq!(session_json["hardware_validated"], false);
    let capture = scratch.write("session.json", first.stdout);
    let decoded = successful_json(decode(&capture));
    assert_eq!(decoded["device_access"], false);
    assert_eq!(decoded["product_id"], 13);
    let mut doc = PresetDocument::parse(decoded["xml"].as_str().unwrap()).unwrap();
    assert_eq!(doc.metadata().name, "Simulated clean preset");
    let before = doc.modules();
    for (key, id, active, position) in [
        ("Amplifier", 103, true, 0),
        ("Stompbox", 60, false, 0),
        ("Modulation", 18, false, 1),
        ("Delay", 43, true, 4),
        ("Reverb", 33, true, 5),
    ] {
        let model = before.iter().find(|m| m.key == key).unwrap();
        assert_eq!(model.model_id, Some(id));
        assert_eq!(model.active, Some(active));
        assert_eq!(model.position, Some(position));
    }
    doc.set_parameter("Amplifier", 1, 0xabcd).unwrap();
    let xml_path = scratch.write("edited.fuse", doc.to_xml().unwrap());
    let encoded = successful_json(
        command()
            .args(["preset-encode", "--pid", "0014", "--file"])
            .arg(&xml_path)
            .output()
            .unwrap(),
    );
    assert_eq!(encoded["transmitted"], false);
    assert_eq!(encoded["hardware_validated"], false);
    assert!(
        encoded["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s.as_str().unwrap().contains("low 8 bits"))
    );
    assert_eq!(encoded["capture"]["reports"][1][33], 0xab);
    let packed_path = scratch.write("rehearsal.json", serde_json::to_vec(&encoded).unwrap());
    let after = successful_json(decode(&packed_path));
    let after = PresetDocument::parse(after["xml"].as_str().unwrap()).unwrap();
    assert_eq!(parameter(&after, "Amplifier", 1), 0xab00);
    assert_eq!(after.metadata(), doc.metadata());
    for model in after.modules() {
        let original = before.iter().find(|m| m.key == model.key).unwrap();
        assert_eq!(model.model_id, original.model_id);
        assert_eq!(model.position, original.position);
        assert_eq!(model.active, original.active);
    }
    // Default XML output and JSON output are the same reconstructed document.
    let xml_output = command()
        .args(["capture-decode", "--file"])
        .arg(&packed_path)
        .output()
        .unwrap();
    assert!(xml_output.status.success());
    assert!(!xml_output.stderr.is_empty());
    let xml_doc =
        PresetDocument::parse(std::str::from_utf8(&xml_output.stdout).unwrap().trim()).unwrap();
    assert_eq!(xml_doc.modules(), after.modules());
}

#[test]
fn cli_rejects_malformed_packets_unknown_models_and_unsupported_pid_without_output() {
    let scratch = Scratch::new();
    let session = successful_json(session());
    let mut malformed = session.clone();
    malformed["capture"]["reports"][0]
        .as_array_mut()
        .unwrap()
        .pop();
    let malformed_path = scratch.write("short.json", serde_json::to_vec(&malformed).unwrap());
    let output = decode(&malformed_path);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("64 bytes"));
    let mut unknown = session.clone();
    unknown["capture"]["reports"][1][16] = 255.into();
    unknown["capture"]["reports"][1][17] = 255.into();
    let unknown_path = scratch.write("unknown.json", serde_json::to_vec(&unknown).unwrap());
    let output = decode(&unknown_path);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Unknown model"));
    let capture_path = scratch.write("valid.json", serde_json::to_vec(&session).unwrap());
    let decoded = successful_json(decode(&capture_path));
    let xml_path = scratch.write("valid.fuse", decoded["xml"].as_str().unwrap());
    for pid in ["0002", "000c", "ffff"] {
        let output = command()
            .args(["preset-encode", "--pid", pid, "--file"])
            .arg(&xml_path)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("no evidenced"));
    }
}

#[test]
fn audition_plan_has_host_flags_and_preserves_captured_global_settings() {
    let scratch = Scratch::new();
    let capture = successful_json(session());
    let raw = scratch.write("current.json", serde_json::to_vec(&capture).unwrap());
    let decoded = successful_json(decode(&raw));
    let xml = scratch.write("local.fuse", decoded["xml"].as_str().unwrap());
    let plan = successful_json(
        command()
            .arg("audition-plan")
            .arg("--capture")
            .arg(&raw)
            .arg("--file")
            .arg(&xml)
            .output()
            .unwrap(),
    );
    assert_eq!(plan["transmitted"], false);
    assert_eq!(plan["hardware_validated"], false);
    assert!(
        plan["expected_readback_xml"]
            .as_str()
            .unwrap()
            .contains("Simulated")
    );
    let steps = plan["steps"].as_array().unwrap();
    assert_eq!(steps[0]["report"][0], 0x1c);
    assert_eq!(steps[0]["report"][1], 1);
    assert_eq!(steps[0]["ack_required"], false);
    for step in &steps[1..] {
        assert_eq!(step["report"][1], 3);
        assert_eq!(step["ack_required"], true);
    }
    let source = capture["capture"]["reports"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r[2] == 10)
        .unwrap();
    let expression = steps.iter().find(|r| r["report"][2] == 10).unwrap();
    assert_eq!(expression["report"][17], source[17]);
    assert_eq!(expression["report"][18], source[18]);
    assert!(
        steps
            .iter()
            .all(|s| s["report"][0] == 0x1c && s["report"][2] != 3)
    );
}
#[test]
fn slot_plan_is_not_legacy_load_and_rejects_out_of_range_or_slotless_profiles() {
    let plan = successful_json(
        command()
            .args(["slot-plan", "--pid", "0014", "--slot", "23"])
            .output()
            .unwrap(),
    );
    assert_eq!(plan["steps"][0]["report"][0], 0x1c);
    assert_eq!(plan["steps"][0]["report"][1], 0xc3);
    assert_eq!(plan["steps"][0]["report"][2], 1);
    assert_eq!(plan["steps"][0]["report"][4], 23);
    for (pid, slot) in [("0014", "24"), ("000e", "0"), ("0001", "0"), ("0014", "-1")] {
        let output = command()
            .args(["slot-plan", "--pid", pid, "--slot", slot])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
    }
}
#[test]
fn persistent_save_plan_is_offline_and_reads_stored_slot_after_ack() {
    let scratch = Scratch::new();
    let capture = scratch.write("capture.json", session().stdout);
    let plan = successful_json(
        command()
            .args(["save-plan", "--capture"])
            .arg(capture)
            .args(["--slot", "23"])
            .output()
            .unwrap(),
    );
    assert_eq!(plan["transmitted"], false);
    assert_eq!(plan["steps"].as_array().unwrap().len(), 2);
    assert_eq!(plan["steps"][0]["report"][2], 3);
    assert_eq!(plan["steps"][0]["ack_required"], false);
    assert_eq!(plan["steps"][1]["ack_required"], true);
    assert_eq!(plan["verification_request"][0], 0x14);
    assert_eq!(plan["verification_request"][2], 23);
}
#[test]
fn bank_inspect_validates_complete_archive_and_rejects_corruption() {
    use openfuse_core::{
        bank::{Backup, SlotCapture},
        protocol::Report64,
    };
    let scratch = Scratch::new();
    let session = successful_json(session());
    let capture = openfuse_device::parse_capture_json(&session.to_string()).unwrap();
    let slots = (0..24)
        .map(|slot| {
            let reports = capture
                .reports
                .iter()
                .map(|r| {
                    let mut b: [u8; 64] = r.as_slice().try_into().unwrap();
                    if b[0] == 0x1c && b[2] == 4 {
                        b[4] = slot;
                        b[5] = 0;
                        b[6] = 0;
                    }
                    Report64::from_bytes(b)
                })
                .collect();
            SlotCapture::new(20, u16::from(slot), reports).unwrap()
        })
        .collect();
    let bank = Backup::new(20, slots).unwrap();
    let path = scratch.write("bank.json", bank.to_json().unwrap());
    let result = successful_json(
        command()
            .args(["bank-inspect", "--file"])
            .arg(&path)
            .output()
            .unwrap(),
    );
    assert_eq!(result["slots"].as_array().unwrap().len(), 24);
    assert_eq!(result["transmitted"], false);
    fs::write(
        &path,
        bank.to_json()
            .unwrap()
            .replace("Simulated clean preset", "Corrupt"),
    )
    .unwrap();
    assert!(
        !command()
            .args(["bank-inspect", "--file"])
            .arg(&path)
            .output()
            .unwrap()
            .status
            .success()
    );
}

#[path = "support/archives.rs"]
mod archives;

#[path = "support/auxiliary.rs"]
mod auxiliary;

#[path = "support/control_presets.rs"]
mod control_presets;

#[path = "support/conversion.rs"]
mod conversion;

#[path = "support/media.rs"]
mod media;

#[path = "support/storage.rs"]
mod storage;
