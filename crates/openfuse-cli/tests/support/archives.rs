use super::*;
use openfuse_core::{
    bank::{Backup, SlotCapture},
    protocol::Report64,
};
fn source(scratch: &Scratch) -> (PathBuf, PathBuf, String) {
    let raw = successful_json(session());
    let capture = openfuse_device::parse_capture_json(&raw.to_string()).unwrap();
    let slots = [2u16, 9]
        .into_iter()
        .map(|slot| {
            let reports = capture
                .reports
                .iter()
                .map(|r| {
                    let mut b: [u8; 64] = r.as_slice().try_into().unwrap();
                    if b[0] == 0x1c && b[2] == 4 {
                        b[4..6].copy_from_slice(&slot.to_le_bytes());
                        b[6] = 0;
                    }
                    Report64::from_bytes(b)
                })
                .collect();
            SlotCapture::new(20, slot, reports).unwrap()
        })
        .collect();
    let backup = Backup::selected(20, slots).unwrap();
    let xml = backup.slots()[0].document().to_xml().unwrap();
    (
        scratch.write("capture.json", raw.to_string()),
        scratch.write("selected.json", backup.to_json().unwrap()),
        xml,
    )
}
#[test]
fn selected_json_is_inspectable_and_remapped_offline_without_override() {
    let scratch = Scratch::new();
    let (capture, archive, _) = source(&scratch);
    let inspected = successful_json(
        command()
            .args(["archive-inspect", "--path"])
            .arg(&archive)
            .output()
            .unwrap(),
    );
    assert_eq!(inspected["complete_bank"], false);
    assert_eq!(inspected["presets"].as_array().unwrap().len(), 2);
    let plan = successful_json(
        command()
            .args(["archive-restore-plan", "--capture"])
            .arg(&capture)
            .arg("--archive")
            .arg(&archive)
            .args(["--map", "9:23,2:1"])
            .output()
            .unwrap(),
    );
    assert_eq!(plan["transmitted"], false);
    assert_eq!(plan["operations"][0]["source"], 9);
    assert_eq!(plan["operations"][0]["target"], 23);
    assert_eq!(plan["operations"][1]["verification_request"][2], 1);
    let failed = command()
        .args(["archive-restore-plan", "--capture"])
        .arg(&capture)
        .arg("--archive")
        .arg(&archive)
        .args(["--map", "9:1,2:1"])
        .output()
        .unwrap();
    assert!(!failed.status.success());
    assert!(failed.stdout.is_empty());
    let failed = command()
        .args(["archive-restore-plan", "--capture"])
        .arg(&capture)
        .arg("--archive")
        .arg(&archive)
        .args(["--map", "9:1", "--source-pid", "14"])
        .output()
        .unwrap();
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("cannot override"));
}
#[test]
fn legacy_inventory_retains_extra_files_and_restore_requires_explicit_pid() {
    let scratch = Scratch::new();
    let (capture, _, xml) = source(&scratch);
    let folder = scratch.0.join("2026_10_13_12_00_00");
    fs::create_dir_all(folder.join("Presets")).unwrap();
    fs::write(folder.join("MU_BackupName.fuse"), "Legacy test").unwrap();
    fs::write(folder.join("Presets/02_My sound.fuse"), xml).unwrap();
    fs::write(folder.join("opaque.bin"), [0, 128, 255]).unwrap();
    let inventory = successful_json(
        command()
            .args(["archive-inspect", "--path"])
            .arg(&folder)
            .output()
            .unwrap(),
    );
    assert_eq!(inventory["legacy"], true);
    assert_eq!(inventory["pid"], Value::Null);
    assert!(
        inventory["inventory"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["path"] == "opaque.bin" && v["bytes"] == 3)
    );
    for pid in [None, Some("05"), Some("14")] {
        let mut cmd = command();
        cmd.args(["archive-restore-plan", "--capture"])
            .arg(&capture)
            .arg("--archive")
            .arg(&folder)
            .args(["--map", "2:8"]);
        if let Some(pid) = pid {
            cmd.args(["--source-pid", pid]);
        }
        let output = cmd.output().unwrap();
        if pid == Some("14") {
            let plan = successful_json(output);
            assert_eq!(plan["legacy_source_pid_declared"], true);
            assert_eq!(plan["operations"][0]["target"], 8);
            assert_eq!(plan["transmitted"], false);
        } else {
            assert!(!output.status.success());
            assert!(output.stdout.is_empty());
        }
    }
}
