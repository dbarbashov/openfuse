use std::{fs, path::PathBuf, process::Command};
fn command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_openfuse"))
}
struct Folder(PathBuf);
impl Folder {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("openfuse-acceptance-{name}-{}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Folder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn simulator_record_replay_report_and_no_overwrite() {
    let folder = Folder::new("roundtrip");
    let path = folder.0.join("session.json");
    let output = command()
        .args(["session", "--simulate", "--json", "--trace"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let original = fs::read(&path).unwrap();
    for verb in ["trace-inspect", "trace-replay"] {
        let output = command()
            .args([verb, "--file"])
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["hardware_validated"], false);
    }
    let html = folder.0.join("report.html");
    let output = command()
        .args(["acceptance-report", "--file"])
        .arg(&path)
        .arg("--out")
        .arg(&html)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(fs::read_to_string(&html).unwrap().contains("SIMULATOR"));
    let output = command()
        .args(["session", "--simulate", "--trace"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(fs::read(&path).unwrap(), original);
    let output = command()
        .args(["acceptance-report", "--file"])
        .arg(&path)
        .arg("--out")
        .arg(&html)
        .output()
        .unwrap();
    assert!(!output.status.success());
}
#[test]
fn malformed_cli_never_reaches_a_session() {
    for args in [
        vec!["session", "--trace"],
        vec!["trace-replay"],
        vec!["trace-inspect", "--device", "fake"],
        vec!["acceptance-report", "--file", "missing"],
    ] {
        assert_eq!(
            command().args(args).output().unwrap().status.code(),
            Some(2)
        );
    }
}
#[test]
fn modified_bundle_fails_offline_replay() {
    let folder = Folder::new("corrupt");
    let path = folder.0.join("session.json");
    assert!(
        command()
            .args(["session", "--simulate", "--trace"])
            .arg(&path)
            .output()
            .unwrap()
            .status
            .success()
    );
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["trace"]["entries"][1]["event"]["bytes"][0] = 222.into();
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    let output = command()
        .args(["trace-replay", "--file"])
        .arg(&path)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("digest mismatch"));
}
