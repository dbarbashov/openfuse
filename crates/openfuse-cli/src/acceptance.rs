use openfuse_device::{acceptance::Acceptance, trace};
use std::{ffi::OsString, path::Path};

pub fn run(command: &str, args: Vec<OsString>) -> Result<(), String> {
    let report = command == "acceptance-report";
    if args.len() != if report { 4 } else { 2 }
        || args[0] != "--file"
        || (report && args[2] != "--out")
    {
        return Err(
            "Use --file SESSION.json, plus --out NEW_REPORT.html for acceptance-report".into(),
        );
    }
    let bundle = Acceptance::parse(&trace::read_bounded(Path::new(&args[1]))?)?;
    if report {
        return trace::write_new(Path::new(&args[3]), bundle.html()?.as_bytes());
    }
    let replay = trace::replay_bootstrap(&bundle.trace);
    if command == "trace-replay" {
        println!(
            "{}",
            serde_json::to_string_pretty(&replay?).map_err(|e| e.to_string())?
        );
    } else {
        let result = serde_json::json!({
            "origin": bundle.trace.origin, "trace_sha256": bundle.trace_sha256,
            "events": bundle.trace.entries.len(), "dropped_events": bundle.trace.dropped_events,
            "os": bundle.trace.os, "architecture": bundle.trace.architecture,
            "bootstrap_replay": replay, "owner_observations": bundle.observations,
            "hardware_validated": false,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&result).map_err(|e| e.to_string())?
        );
    }
    Ok(())
}
