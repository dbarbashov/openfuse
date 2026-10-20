use super::*;
fn source(scratch: &Scratch) -> PathBuf {
    let capture = scratch.write("current.json", session().stdout);
    let value = successful_json(decode(&capture));
    scratch.write("original.fuse", value["xml"].as_str().unwrap())
}
#[test]
fn conversion_cli_previews_and_exports_a_new_copy_without_changing_source() {
    let scratch = Scratch::new();
    let path = source(&scratch);
    let original = fs::read(&path).unwrap();
    let plan = successful_json(
        command()
            .args(["preset-convert-plan", "--file"])
            .arg(&path)
            .args(["--target", "1"])
            .output()
            .unwrap(),
    );
    assert_eq!(plan["transmitted"], false);
    assert_eq!(plan["written"], false);
    assert_eq!(plan["source_product"], 13);
    assert_eq!(plan["target_product"], 1);
    let converted = PresetDocument::parse(plan["xml"].as_str().unwrap()).unwrap();
    assert_eq!(converted.product_id(), Some(1));
    assert!(!plan["changes"].as_array().unwrap().is_empty());
    assert_eq!(fs::read(&path).unwrap(), original);
    let out = scratch.0.join("converted.fuse");
    assert!(
        !command()
            .args(["preset-convert", "--file"])
            .arg(&path)
            .args(["--target", "1", "--out"])
            .arg(&out)
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(!out.exists());
    let result = successful_json(
        command()
            .args(["preset-convert", "--file"])
            .arg(&path)
            .args(["--target", "1", "--accept-changes", "--out"])
            .arg(&out)
            .output()
            .unwrap(),
    );
    assert_eq!(result["written"], true);
    assert_eq!(result["transmitted"], false);
    assert_eq!(
        fs::read_to_string(&out).unwrap(),
        plan["xml"].as_str().unwrap()
    );
    assert_eq!(fs::read(&path).unwrap(), original);
    for target in [&path, &out] {
        let failed = command()
            .args(["preset-convert", "--file"])
            .arg(&path)
            .args(["--target", "1", "--accept-changes", "--out"])
            .arg(target)
            .output()
            .unwrap();
        assert!(!failed.status.success());
        assert!(failed.stdout.is_empty());
    }
    assert_eq!(fs::read(&path).unwrap(), original);
}
#[test]
fn legacy_entity_repair_is_explicit_and_invalid_options_never_write() {
    let scratch = Scratch::new();
    let path = source(&scratch);
    let legacy = fs::read_to_string(&path)
        .unwrap()
        .replace("Simulated clean preset", "A &amp;lt; B");
    fs::write(&path, &legacy).unwrap();
    let plain = successful_json(
        command()
            .args(["preset-convert-plan", "--file"])
            .arg(&path)
            .args(["--target", "13"])
            .output()
            .unwrap(),
    );
    let repaired = successful_json(
        command()
            .args(["preset-convert-plan", "--file"])
            .arg(&path)
            .args(["--target", "13", "--repair-double-entities"])
            .output()
            .unwrap(),
    );
    assert_eq!(
        PresetDocument::parse(plain["xml"].as_str().unwrap())
            .unwrap()
            .metadata()
            .name,
        "A &lt; B"
    );
    assert_eq!(
        PresetDocument::parse(repaired["xml"].as_str().unwrap())
            .unwrap()
            .metadata()
            .name,
        "A < B"
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), legacy);
    for options in [
        vec!["--target", "999999"],
        vec!["--target", "13", "--target", "1"],
        vec!["--target", "13", "--bogus"],
        vec!["--target", "not-a-number"],
    ] {
        let failed = command()
            .args(["preset-convert-plan", "--file"])
            .arg(&path)
            .args(options)
            .output()
            .unwrap();
        assert!(!failed.status.success());
        assert!(failed.stdout.is_empty());
    }
    assert_eq!(fs::read_to_string(&path).unwrap(), legacy);
}

#[test]
fn missing_family_repair_requires_explicit_gdec_choice_and_does_not_relabel_unknown_ids() {
    use openfuse_core::model_catalog::{self, ModuleCategory};
    let scratch = Scratch::new();
    let model = model_catalog::model(10, ModuleCategory::Amplifier, 3).unwrap();
    let params = model
        .parameters
        .iter()
        .map(|p| {
            format!(
                "<Param ControlIndex=\"{}\">{}</Param>",
                p.index, p.default_raw
            )
        })
        .collect::<String>();
    let mut xml =
        format!("<Preset><Amplifier><Module ID=\"3\" POS=\"0\">{params}</Module></Amplifier><FX>");
    for (i, key) in ["Stompbox", "Modulation", "Delay", "Reverb"]
        .iter()
        .enumerate()
    {
        xml.push_str(&format!(
            "<{key} ID=\"{}\"><Module ID=\"0\" POS=\"0\"/></{key}>",
            i + 1
        ));
    }
    xml.push_str("</FX><FUSE><Name>Legacy GDEC</Name></FUSE></Preset>");
    let path = scratch.write("legacy.fuse", &xml);
    assert!(
        !command()
            .args(["preset-convert-plan", "--file"])
            .arg(&path)
            .args(["--target", "10"])
            .output()
            .unwrap()
            .status
            .success()
    );
    let repaired = successful_json(
        command()
            .args(["preset-convert-plan", "--file"])
            .arg(&path)
            .args(["--target", "10", "--assume-missing-gdec"])
            .output()
            .unwrap(),
    );
    assert_eq!(repaired["target_product"], 10);
    assert!(
        repaired["changes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["kind"] == "repair")
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), xml);
    for pid in ["2", "9999"] {
        fs::write(
            &path,
            xml.replace("<Preset>", &format!("<Preset ProductId=\"{pid}\">")),
        )
        .unwrap();
        let failed = command()
            .args(["preset-convert-plan", "--file"])
            .arg(&path)
            .args(["--target", "10", "--assume-missing-gdec"])
            .output()
            .unwrap();
        assert!(!failed.status.success());
        assert!(failed.stdout.is_empty());
    }
}
