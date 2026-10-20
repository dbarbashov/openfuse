use super::*;
use openfuse_core::{auxiliary::Snapshot, protocol::Report64};
pub(super) fn fixture() -> Snapshot {
    let value = successful_json(session());
    let current = openfuse_device::parse_capture_json(&value.to_string()).unwrap();
    let base: Vec<[u8; 64]> = current
        .reports
        .iter()
        .map(|r| r.as_slice().try_into().unwrap())
        .filter(|r: &[u8; 64]| r[0] == 0x1c)
        .collect();
    let mut reports = Vec::new();
    for slot in 0u16..24 {
        for b in &base {
            let mut b = *b;
            if b[2] == 4 {
                b[4..6].copy_from_slice(&slot.to_le_bytes());
                b[6] = 0;
            }
            reports.push(Report64::from_bytes(b));
        }
    }
    let mut system = [0; 64];
    system[0] = 0x1c;
    system[1] = 1;
    system[2] = 11;
    reports.push(Report64::from_bytes(system));
    for (control, kinds) in [(1, vec![7]), (2, vec![8, 9])] {
        for slot in 0u16..12 {
            let mut name = [0; 64];
            name[0] = 0x1c;
            name[1] = 1;
            name[2] = 4;
            name[3] = control;
            name[4..6].copy_from_slice(&slot.to_le_bytes());
            let label = format!("CLI bank {control} slot {slot}");
            name[16..16 + label.len()].copy_from_slice(label.as_bytes());
            reports.push(Report64::from_bytes(name));
            for kind in &kinds {
                let mut b = *base.iter().find(|b| b[2] == *kind).unwrap();
                b[1] = 1;
                b[3] = control;
                b[4..6].copy_from_slice(&slot.to_le_bytes());
                b[6] = 0;
                b[7] = 0;
                reports.push(Report64::from_bytes(b));
            }
            let mut complete = [0; 64];
            complete[0] = 0x1c;
            complete[1] = 1;
            complete[3] = control;
            reports.push(Report64::from_bytes(complete));
        }
    }
    let mut end = [0; 64];
    end[0] = 0x14;
    end[1] = 1;
    end[3] = 2;
    end[4] = 1;
    reports.push(Report64::from_bytes(end));
    Snapshot::from_reports(20, &reports).unwrap()
}
#[test]
fn auxiliary_cli_inspects_and_plans_without_transmission_or_pid_override() {
    let scratch = Scratch::new();
    let snapshot = fixture();
    let path = scratch.write("auxiliary.json", snapshot.to_json().unwrap());
    let inventory = successful_json(
        command()
            .args(["auxiliary-inspect", "--path"])
            .arg(&path)
            .output()
            .unwrap(),
    );
    assert_eq!(inventory["transmitted"], false);
    assert_eq!(inventory["entries"].as_array().unwrap().len(), 25);
    assert_eq!(inventory["presets"], 24);
    let plan = successful_json(
        command()
            .args(["auxiliary-restore-plan", "--source"])
            .arg(&path)
            .arg("--destination")
            .arg(&path)
            .args(["--map", "1:1=1:9,2:2=2:8"])
            .output()
            .unwrap(),
    );
    assert_eq!(plan["transmitted"], false);
    assert_eq!(plan["hardware_validated"], false);
    assert_eq!(plan["entries"], 2);
    assert!(
        plan["steps"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["report"][0] == 0x1c && s["report"][3] != 0)
    );
    for mapping in [
        "system=system",
        "1:1=2:2",
        "1:1=1:9,1:2=1:9",
        "1:99=1:1",
        "bad",
    ] {
        let output = command()
            .args(["auxiliary-restore-plan", "--source"])
            .arg(&path)
            .arg("--destination")
            .arg(&path)
            .args(["--map", mapping])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
    let output = command()
        .args(["auxiliary-inspect", "--path"])
        .arg(&path)
        .args(["--source-pid", "14"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("overridden"));
}
#[test]
fn auxiliary_cli_legacy_needs_declared_profile_and_retains_original_xml() {
    let scratch = Scratch::new();
    let snapshot = fixture();
    let dst = scratch.write("destination.json", snapshot.to_json().unwrap());
    let folder = scratch.0.join("old");
    fs::create_dir_all(folder.join("Effects/MOD")).unwrap();
    let source = snapshot
        .entries()
        .iter()
        .find(|e| {
            matches!(
                e.key(),
                openfuse_core::auxiliary::Key::Bank {
                    control: 1,
                    slot: 1
                }
            )
        })
        .unwrap();
    let xml = source.xml().unwrap();
    fs::write(folder.join("Effects/MOD/01_Source.fuse"), xml).unwrap();
    let failed = command()
        .args(["auxiliary-inspect", "--path"])
        .arg(&folder)
        .output()
        .unwrap();
    assert!(!failed.status.success());
    let inventory = successful_json(
        command()
            .args(["auxiliary-inspect", "--path"])
            .arg(&folder)
            .args(["--source-pid", "14"])
            .output()
            .unwrap(),
    );
    assert_eq!(inventory["legacy"], true);
    assert_eq!(inventory["entries"][0]["xml"], xml);
    let plan = successful_json(
        command()
            .args(["auxiliary-restore-plan", "--source"])
            .arg(&folder)
            .arg("--destination")
            .arg(&dst)
            .args(["--map", "1:1=1:9", "--source-pid", "14"])
            .output()
            .unwrap(),
    );
    assert_eq!(plan["entries"], 1);
    assert_eq!(plan["transmitted"], false);
}

#[test]
fn whole_restore_cli_requires_complete_source_and_never_transmits() {
    let scratch = Scratch::new();
    let captured = fixture();
    let expression = captured
        .reports()
        .iter()
        .find(|report| report.as_bytes()[2] == 10)
        .unwrap();
    let mut reports = captured.reports().to_vec();
    for report in &mut reports {
        if report.as_bytes()[2] == 11 {
            let mut bytes = *report.as_bytes();
            bytes[35] = expression.as_bytes()[17];
            bytes[36] = expression.as_bytes()[18];
            *report = Report64::from_bytes(bytes);
        }
    }
    let snapshot = Snapshot::from_reports(captured.pid(), &reports).unwrap();
    let path = scratch.write("full-device.json", snapshot.to_json().unwrap());
    let original = fs::read(&path).unwrap();
    let plan = successful_json(
        command()
            .args(["whole-restore-plan", "--source"])
            .arg(&path)
            .arg("--destination")
            .arg(&path)
            .output()
            .unwrap(),
    );
    assert_eq!(plan["transmitted"], false);
    assert_eq!(plan["hardware_validated"], false);
    assert_eq!(plan["presets"], 24);
    assert_eq!(plan["bank_entries"], 24);
    assert_eq!(plan["system"], true);
    assert!(!plan["normalizations"].as_array().unwrap().is_empty());
    assert!(!plan["warnings"].as_array().unwrap().is_empty());
    let steps = plan["steps"].as_array().unwrap();
    assert_eq!(steps[0]["report"][0], 0x14);
    assert_eq!(steps[0]["report"][3], 1);
    assert_eq!(steps.last().unwrap()["report"][3], 2);
    assert!(steps.iter().any(|step| step["report"][2] == 11));
    assert_eq!(fs::read(&path).unwrap(), original);

    for extra in [
        vec!["--source-pid", "14"],
        vec!["--map", "1:1=1:2"],
        vec!["--execute", "true"],
    ] {
        let output = command()
            .args(["whole-restore-plan", "--source"])
            .arg(&path)
            .arg("--destination")
            .arg(&path)
            .args(extra)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
    let folder = scratch.0.join("incomplete");
    fs::create_dir_all(folder.join("Effects/MOD")).unwrap();
    let entry = snapshot
        .entries()
        .iter()
        .find(|entry| {
            entry.key()
                == openfuse_core::auxiliary::Key::Bank {
                    control: 1,
                    slot: 0,
                }
        })
        .unwrap();
    fs::write(
        folder.join("Effects/MOD/00_Only.fuse"),
        entry.xml().unwrap(),
    )
    .unwrap();
    let output = command()
        .args(["whole-restore-plan", "--source"])
        .arg(&folder)
        .arg("--destination")
        .arg(&path)
        .args(["--source-pid", "14"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(fs::read(&path).unwrap(), original);
}
