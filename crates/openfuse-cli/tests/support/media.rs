use super::*;
#[test]
fn media_cli_imports_lists_and_preserves_originals_offline() {
    let scratch = Scratch::new();
    let source = scratch.write("track.WAV", b"inventory bytes");
    let library = scratch.0.join("library");
    fs::create_dir(&library).unwrap();
    let result = successful_json(
        command()
            .args(["media-import", "--library"])
            .arg(&library)
            .arg("--path")
            .arg(&source)
            .output()
            .unwrap(),
    );
    assert_eq!(result["device_access"], false);
    assert_eq!(result["playback_started"], false);
    assert_eq!(result["data"]["imported"]["name"], "track.WAV");
    assert_eq!(fs::read(&source).unwrap(), b"inventory bytes");
    let list = successful_json(
        command()
            .args(["media-list", "--library"])
            .arg(&library)
            .output()
            .unwrap(),
    );
    assert_eq!(list["data"]["entries"].as_array().unwrap().len(), 1);
    assert_eq!(list["data"]["entries"][0]["format"], "Wav");
    let absent = scratch.0.join("absent");
    assert!(
        !command()
            .args(["media-list", "--library"])
            .arg(&absent)
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(!absent.exists());
    assert!(
        !command()
            .args(["media-list", "--library"])
            .arg(&library)
            .arg("--unknown")
            .output()
            .unwrap()
            .status
            .success()
    );
}
#[test]
fn band_inspection_reports_original_values_without_writing() {
    let scratch = Scratch::new();
    let capture = scratch.write("current.json", session().stdout);
    let value = successful_json(decode(&capture));
    let path = scratch.write("preset.fuse", value["xml"].as_str().unwrap());
    let before = fs::read(&path).unwrap();
    let value = successful_json(
        command()
            .args(["band-inspect", "--file"])
            .arg(&path)
            .output()
            .unwrap(),
    );
    assert_eq!(value["device_access"], false);
    assert_eq!(value["playback_started"], false);
    assert_eq!(value["data"]["product_id"], 13);
    assert_eq!(fs::read(path).unwrap(), before);
}
