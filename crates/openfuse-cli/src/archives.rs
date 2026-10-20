use openfuse_core::{
    archive::Archive,
    bank::{self, SlotMapping},
    protocol::Report64,
};
use openfuse_device::{MAX_CAPTURE_BYTES, parse_capture_json};
use std::{collections::BTreeMap, ffi::OsString, path::Path};

pub fn inspect(args: Vec<OsString>) -> Result<(), String> {
    if args.len() != 2 || args[0] != "--path" {
        return Err("archive-inspect requires --path DIRECTORY_OR_ARCHIVE".into());
    }
    let archive = Archive::read_path(Path::new(&args[1])).map_err(|e| e.to_string())?;
    let presets: Vec<_> = archive
        .presets()
        .iter()
        .map(|p| {
            serde_json::json!({
                "slot":p.slot,"name":p.document.metadata().name,"family":p.document.product_id()
            })
        })
        .collect();
    let inventory = archive.legacy().map(|legacy| legacy.entries().iter().map(|e| serde_json::json!({
        "path":e.path(),"bytes":e.bytes().len(),"sha256":e.sha256(),"kind":e.kind(),"metadata":e.metadata()
    })).collect::<Vec<_>>());
    print_json(
        serde_json::json!({"kind":"archive_inventory","transmitted":false,
        "pid":archive.native().map(|b|b.pid()).or_else(||archive.auxiliary().map(|s|s.pid())).or_else(||archive.control_preset().map(|d|d.pid())),
        "control_preset":archive.control_preset().map(|d|serde_json::json!({"pid":d.pid(),"control":d.control(),"name":d.name()})),
        "auxiliary":archive.auxiliary().map(|s| s.entries().iter().map(|e| serde_json::json!({"key":e.key(),"name":e.label(),"restore_error":e.restore_error()})).collect::<Vec<_>>()),"complete_bank":archive.native().map(|b|b.is_complete()),
        "legacy":archive.legacy().is_some(),"presets":presets,"inventory":inventory,"warnings":archive.warnings()}),
    )
}

pub fn restore_plan(args: Vec<OsString>) -> Result<(), String> {
    let mut options = BTreeMap::new();
    if !args.len().is_multiple_of(2) {
        return Err(usage());
    }
    for pair in args.as_chunks::<2>().0 {
        let key = pair[0].to_str().ok_or_else(usage)?;
        if !["--capture", "--archive", "--map", "--source-pid"].contains(&key)
            || options.insert(key, &pair[1]).is_some()
        {
            return Err(usage());
        }
    }
    let get = |key| options.get(key).copied().ok_or_else(usage);
    let capture = parse_capture_json(&super::wire_preset::bounded(
        Path::new(get("--capture")?),
        MAX_CAPTURE_BYTES,
    )?)?;
    let archive = Archive::read_path(Path::new(get("--archive")?)).map_err(|e| e.to_string())?;
    let pid = capture.device.product_id;
    if archive.legacy().is_some() {
        let declared = options.get("--source-pid").ok_or(
            "Historical folders do not establish an exact PID; choose --source-pid HEX explicitly",
        )?;
        let declared = declared.to_str().ok_or("Invalid source PID")?;
        let declared = u16::from_str_radix(declared.trim_start_matches("0x"), 16)
            .map_err(|_| "Source PID must be hexadecimal")?;
        if declared != pid {
            return Err("Explicit source PID must equal the captured target PID".into());
        }
    } else if options.contains_key("--source-pid") {
        return Err(
            "Native archives already establish their PID; --source-pid cannot override it".into(),
        );
    }
    let mappings = get("--map")?
        .to_str()
        .ok_or("Invalid mapping text")?
        .split(',')
        .map(|part| {
            let (source, target) = part
                .split_once(':')
                .ok_or("Mappings require zero-based SOURCE:TARGET pairs")?;
            Ok(SlotMapping {
                source: source.parse().map_err(|_| "Invalid source slot")?,
                target: target.parse().map_err(|_| "Invalid destination slot")?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let selection = archive
        .restore_selection(pid, &mappings)
        .map_err(|e| e.to_string())?;
    let reports = capture
        .reports
        .iter()
        .map(|r| Report64::try_from(r.as_slice()).map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    let plans = selection.plans(&reports).map_err(|e| e.to_string())?;
    let operations = selection.entries().iter().zip(&plans).map(|(entry,plan)|{
        let steps:Vec<_>=plan.steps().iter().map(|r|serde_json::json!({"report":r.as_bytes().to_vec(),"ack_required":r.as_bytes()[1]&2!=0})).collect();
        Ok(serde_json::json!({"source":entry.source(),"target":entry.target(),"name":entry.document().metadata().name,
            "steps":steps,"warnings":plan.warnings(),"verification_request":bank::stored_request(pid,entry.target()).map_err(|e|e.to_string())?.as_bytes().to_vec()}))
    }).collect::<Result<Vec<_>,String>>()?;
    print_json(
        serde_json::json!({"kind":"offline_selective_restore_plan","transmitted":false,"hardware_validated":false,
        "target_pid":pid,"legacy_source_pid_declared":archive.legacy().is_some(),"operations":operations,"warnings":archive.warnings()}),
    )
}
fn usage() -> String {
    "archive-restore-plan requires --capture CURRENT.json --archive ARCHIVE --map SOURCE:TARGET,... [--source-pid HEX]".into()
}
fn print_json(value: serde_json::Value) -> Result<(), String> {
    println!(
        "{}",
        serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?
    );
    Ok(())
}
