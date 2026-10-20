use openfuse_core::{
    archive::Archive,
    auxiliary::{Key, Mapping, Selection, Snapshot, WholeRestore},
};
use std::{collections::BTreeMap, ffi::OsString, path::Path};

pub fn inspect(args: Vec<OsString>) -> Result<(), String> {
    let options = options(args, &["--path", "--source-pid"])?;
    let path = options
        .get("--path")
        .ok_or("auxiliary-inspect requires --path ARCHIVE [--source-pid HEX]")?;
    let archive = Archive::read_path(Path::new(path)).map_err(|e| e.to_string())?;
    let snapshot = source(&archive, options.get("--source-pid"))?;
    let entries:Vec<_>=snapshot.entries().iter().map(|e|serde_json::json!({"key":e.key(),"name":e.label(),"xml":e.xml(),"restore_error":e.restore_error(),"reports":e.raw_reports().len()})).collect();
    print(
        serde_json::json!({"kind":"auxiliary_inventory","transmitted":false,"hardware_validated":false,"pid":snapshot.pid(),"legacy":snapshot.is_legacy(),"entries":entries,"presets":snapshot.presets().len(),"fingerprint":snapshot.fingerprint(),"warnings":snapshot.warnings()}),
    )
}
pub fn restore_plan(args: Vec<OsString>) -> Result<(), String> {
    let options = options(
        args,
        &["--source", "--destination", "--map", "--source-pid"],
    )?;
    let get = |key| {
        options.get(key).ok_or("auxiliary-restore-plan requires --source ARCHIVE --destination SNAPSHOT --map CONTROL:SLOT=CONTROL:SLOT,... [--source-pid HEX]")
    };
    let src = Archive::read_path(Path::new(get("--source")?)).map_err(|e| e.to_string())?;
    let src = source(&src, options.get("--source-pid"))?;
    let dst = Archive::read_path(Path::new(get("--destination")?)).map_err(|e| e.to_string())?;
    let dst = dst
        .auxiliary()
        .ok_or("Destination must be a complete native auxiliary snapshot")?;
    let mappings = get("--map")?
        .to_str()
        .ok_or("Invalid mapping text")?
        .split(',')
        .map(|m| {
            let (source, target) = m
                .split_once('=')
                .ok_or("Expected CONTROL:SLOT=CONTROL:SLOT mapping")?;
            Ok(Mapping {
                source: key(source)?,
                target: key(target)?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let selection = Selection::new(&src, dst, &mappings).map_err(|e| e.to_string())?;
    let steps:Vec<_>=selection.steps().iter().map(|r|serde_json::json!({"report":r.as_bytes().to_vec(),"ack_required":r.as_bytes()[1]&2!=0})).collect();
    print(
        serde_json::json!({"kind":"offline_auxiliary_restore_plan","transmitted":false,"hardware_validated":false,"pid":selection.pid(),"entries":selection.len(),"mappings":mappings,"steps":steps,"destination_fingerprint":dst.fingerprint(),"verification":"Fresh complete backup before writes and after all selected entries; compare selected and unselected banks"}),
    )
}
pub fn whole_restore_plan(args: Vec<OsString>) -> Result<(), String> {
    let options = options(args, &["--source", "--destination", "--source-pid"])?;
    let get = |key| {
        options.get(key).ok_or(
            "whole-restore-plan requires --source ARCHIVE --destination SNAPSHOT [--source-pid HEX]",
        )
    };
    let source = Archive::read_path(Path::new(get("--source")?)).map_err(|e| e.to_string())?;
    let destination =
        Archive::read_path(Path::new(get("--destination")?)).map_err(|e| e.to_string())?;
    let destination = destination
        .auxiliary()
        .ok_or("Destination must be a complete native auxiliary snapshot")?;
    let explicit_pid = options
        .get("--source-pid")
        .map(|value| {
            let text = value.to_str().ok_or("Invalid hexadecimal PID")?;
            u16::from_str_radix(text.trim_start_matches("0x"), 16)
                .map_err(|_| "Invalid hexadecimal PID")
        })
        .transpose()?;
    if source.auxiliary().is_some() && explicit_pid.is_some() {
        return Err("Native snapshot PID cannot be overridden".into());
    }
    let pid = source
        .auxiliary()
        .map(Snapshot::pid)
        .or(explicit_pid)
        .ok_or("Historical folders require explicit --source-pid HEX")?;
    let plan = WholeRestore::from_archive(&source, pid, destination).map_err(|e| e.to_string())?;
    let steps: Vec<_> = plan
        .steps()
        .iter()
        .map(|step| {
            serde_json::json!({
                "label": step.label,
                "report": step.report.as_bytes().to_vec(),
                "ack_timeout_ms": step.ack_timeout_ms,
                "ack_required": step.report.as_bytes()[1] & 2 != 0
            })
        })
        .collect();
    print(serde_json::json!({
        "kind": "offline_whole_device_restore_plan",
        "transmitted": false,
        "hardware_validated": false,
        "pid": plan.pid(),
        "presets": plan.preset_count(),
        "bank_entries": plan.bank_count(),
        "system": plan.has_system(),
        "source_fingerprint": plan.source_fingerprint(),
        "destination_fingerprint": plan.destination_fingerprint(),
        "normalizations": plan.normalizations(),
        "warnings": plan.warnings(),
        "steps": steps,
        "verification": "Fresh complete destination before writes; full readback after the restore envelope. ACK alone is not verified completion.",
        "recovery": "Cancellation or failure can leave partial changes. No rollback or automatic replay."
    }))
}
fn key(text: &str) -> Result<Key, String> {
    if text == "system" {
        return Ok(Key::System);
    }
    let (control, slot) = text
        .split_once(':')
        .ok_or("Key must be CONTROL:SLOT or system")?;
    Ok(Key::Bank {
        control: control.parse().map_err(|_| "Invalid control number")?,
        slot: slot.parse().map_err(|_| "Invalid zero-based slot")?,
    })
}
fn source(archive: &Archive, pid: Option<&OsString>) -> Result<Snapshot, String> {
    if let Some(native) = archive.auxiliary() {
        if pid.is_some() {
            return Err("Native snapshot PID cannot be overridden".into());
        }
        return Ok(native.clone());
    }
    let legacy = archive
        .legacy()
        .ok_or("Archive does not contain auxiliary banks")?;
    let pid = pid
        .and_then(|p| p.to_str())
        .ok_or("Historical folders require explicit --source-pid HEX")?;
    let pid = u16::from_str_radix(pid.trim_start_matches("0x"), 16)
        .map_err(|_| "Invalid hexadecimal PID")?;
    Snapshot::from_legacy(pid, legacy).map_err(|e| e.to_string())
}
fn options(args: Vec<OsString>, allowed: &[&str]) -> Result<BTreeMap<String, OsString>, String> {
    if !args.len().is_multiple_of(2) {
        return Err("Options require a value".into());
    }
    let mut result = BTreeMap::new();
    for pair in args.as_chunks::<2>().0 {
        let key = pair[0].to_str().ok_or("Invalid option")?;
        if !allowed.contains(&key) || result.insert(key.into(), pair[1].clone()).is_some() {
            return Err(format!("Unknown or duplicate option: {key}"));
        }
    }
    Ok(result)
}
fn print(value: serde_json::Value) -> Result<(), String> {
    println!(
        "{}",
        serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?
    );
    Ok(())
}
