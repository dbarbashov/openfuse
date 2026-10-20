use super::*;
use openfuse_core::control_preset::ControlPreset;

#[test]
fn individual_bank_cli_extracts_current_and_plans_without_transmission() {
    let scratch = Scratch::new();
    let capture = scratch.write("current.json", session().stdout);
    let source = scratch.0.join("mod.json");
    let result = successful_json(
        command()
            .args(["control-preset-extract", "--capture"])
            .arg(&capture)
            .args(["--control", "1", "--out"])
            .arg(&source)
            .output()
            .unwrap(),
    );
    assert_eq!(result["transmitted"], false);
    let inspected = successful_json(
        command()
            .args(["control-preset-inspect", "--path"])
            .arg(&source)
            .output()
            .unwrap(),
    );
    assert_eq!(inspected["control"], 1);
    assert!(inspected["xml"].as_str().unwrap().contains("Modulation"));
    let dst = scratch.write("banks.json", auxiliary::fixture().to_json().unwrap());
    let plan = successful_json(
        command()
            .args(["control-preset-plan", "--source"])
            .arg(&source)
            .arg("--destination")
            .arg(&dst)
            .args(["--slot", "9"])
            .output()
            .unwrap(),
    );
    assert_eq!(plan["transmitted"], false);
    assert_eq!(plan["hardware_validated"], false);
    assert_eq!(plan["steps"].as_array().unwrap().len(), 3);
    assert_eq!(plan["steps"][0]["ack_required"], false);
    assert_eq!(plan["steps"][1]["report"][2], 7);
    assert_eq!(plan["steps"][1]["report"][4], 9);
    let original = fs::read(&source).unwrap();
    assert!(
        !command()
            .args(["control-preset-extract", "--capture"])
            .arg(&capture)
            .args(["--control", "1", "--out"])
            .arg(&source)
            .output()
            .unwrap()
            .status
            .success()
    );
    assert_eq!(fs::read(&source).unwrap(), original);
}

#[test]
fn individual_bank_cli_requires_explicit_xml_profile_and_rejects_invalid_destination() {
    let scratch = Scratch::new();
    let doc = ControlPreset::new(20, 1, "Offline chorus").unwrap();
    let path = scratch.write("mod.fuse", doc.to_xml().unwrap());
    let output = command()
        .args(["control-preset-inspect", "--path"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let output = successful_json(
        command()
            .args(["control-preset-inspect", "--path"])
            .arg(&path)
            .args(["--pid", "0014", "--control", "1"])
            .output()
            .unwrap(),
    );
    assert_eq!(output["name"], "Offline chorus");
    let native = scratch.write("mod.json", doc.to_json().unwrap());
    let dst = scratch.write("banks.json", auxiliary::fixture().to_json().unwrap());
    for slot in ["12", "-1", "65536"] {
        let output = command()
            .args(["control-preset-plan", "--source"])
            .arg(&native)
            .arg("--destination")
            .arg(&dst)
            .args(["--slot", slot])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
    assert!(
        !command()
            .args(["control-preset-inspect", "--path"])
            .arg(&native)
            .args(["--pid", "0014", "--control", "1"])
            .output()
            .unwrap()
            .status
            .success()
    );
}
