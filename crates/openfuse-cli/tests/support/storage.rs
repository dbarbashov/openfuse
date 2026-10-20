use super::*;

#[test]
fn mounted_storage_cli_inspects_and_previews_without_creating_files() {
    let scratch = Scratch::new();
    let root = scratch.0.join("card");
    let output_dir = scratch.0.join("downloads");
    for directory in [root.join("Audio"), root.join("Presets"), output_dir.clone()] {
        fs::create_dir_all(directory).unwrap();
    }
    fs::write(root.join("Audio/existing.wav"), b"stored audio fixture").unwrap();
    let source = scratch.write("fresh.wav", b"local audio fixture");
    let inspect = successful_json(
        command()
            .args(["storage-inspect", "--root"])
            .arg(&root)
            .args(["--kind", "sd"])
            .output()
            .unwrap(),
    );
    assert_eq!(inspect["files_written"], false);
    assert_eq!(inspect["usb_access"], false);
    assert_eq!(inspect["data"]["entries"].as_array().unwrap().len(), 1);
    let plan = successful_json(
        command()
            .args(["storage-upload-plan", "--root"])
            .arg(&root)
            .args(["--kind", "sd", "--file"])
            .arg(&source)
            .output()
            .unwrap(),
    );
    assert_eq!(plan["files_written"], false);
    assert_eq!(plan["transmitted"], false);
    assert_eq!(plan["data"]["items"].as_array().unwrap().len(), 1);
    assert!(!root.join("Audio/fresh.wav").exists());
    assert_eq!(fs::read(&source).unwrap(), b"local audio fixture");
    let download = successful_json(
        command()
            .args(["storage-download-plan", "--root"])
            .arg(&root)
            .args(["--kind", "sd", "--entry", "Audio/existing.wav", "--out"])
            .arg(&output_dir)
            .output()
            .unwrap(),
    );
    assert_eq!(download["files_written"], false);
    assert_eq!(download["data"]["items"].as_array().unwrap().len(), 1);
    assert_eq!(fs::read_dir(&output_dir).unwrap().count(), 0);
}

#[test]
fn mounted_storage_cli_refuses_collisions_and_escape_and_can_plan_keep_both() {
    let scratch = Scratch::new();
    let root = scratch.0.join("card");
    fs::create_dir_all(root.join("Audio")).unwrap();
    let source = scratch.write("same.wav", b"new bytes");
    fs::write(root.join("Audio/same.wav"), b"original bytes").unwrap();
    let failed = command()
        .args(["storage-upload-plan", "--root"])
        .arg(&root)
        .args(["--kind", "sd", "--file"])
        .arg(&source)
        .output()
        .unwrap();
    assert!(!failed.status.success());
    assert!(failed.stdout.is_empty());
    let plan = successful_json(
        command()
            .args(["storage-upload-plan", "--root"])
            .arg(&root)
            .args(["--kind", "sd", "--file"])
            .arg(&source)
            .arg("--keep-both")
            .output()
            .unwrap(),
    );
    assert_eq!(plan["files_written"], false);
    assert_eq!(fs::read_dir(root.join("Audio")).unwrap().count(), 1);
    assert_eq!(
        fs::read(root.join("Audio/same.wav")).unwrap(),
        b"original bytes"
    );
    for entry in ["../same.wav", "/same.wav", "Audio/missing.wav"] {
        let result = command()
            .args(["storage-download-plan", "--root"])
            .arg(&root)
            .args(["--kind", "sd", "--entry", entry, "--out"])
            .arg(&scratch.0)
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert!(result.stdout.is_empty());
    }
    for extra in [vec!["--execute"], vec!["--kind", "sd"], vec!["--file"]] {
        let result = command()
            .args(["storage-inspect", "--root"])
            .arg(&root)
            .args(["--kind", "sd"])
            .args(extra)
            .output()
            .unwrap();
        assert!(!result.status.success());
    }
}
