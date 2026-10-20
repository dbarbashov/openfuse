use openfuse_core::{
    bank::{self, Backup, MAX_BACKUP_BYTES},
    protocol::Report64,
};
use openfuse_device::{MAX_CAPTURE_BYTES, parse_capture_json};
use std::{ffi::OsString, path::Path};
pub fn inspect(args: Vec<OsString>) -> Result<(), String> {
    if args.len() != 2 || args[0] != "--file" {
        return Err("bank-inspect requires --file BANK.json".into());
    }
    let backup = Backup::parse(&super::wire_preset::bounded(
        Path::new(&args[1]),
        MAX_BACKUP_BYTES,
    )?)
    .map_err(|e| e.to_string())?;
    let slots:Vec<_>=backup.slots().iter().map(|s|serde_json::json!({"slot":s.slot(),"name":s.name(),"reports":s.raw_reports().len()})).collect();
    println!("{}",serde_json::to_string_pretty(&serde_json::json!({"kind":"validated_preset_bank","complete":backup.is_complete(),"product_id":backup.pid(),"family":backup.family(),"slots":slots,"transmitted":false,"scope":"preset sounds only; excludes system settings and knob banks"})).map_err(|e|e.to_string())?);
    Ok(())
}
pub fn save_plan(args: Vec<OsString>) -> Result<(), String> {
    if args.len() != 4 || args[0] != "--capture" || args[2] != "--slot" {
        return Err("save-plan requires --capture CURRENT.json --slot ZERO_BASED_INDEX".into());
    }
    let capture = parse_capture_json(&super::wire_preset::bounded(
        Path::new(&args[1]),
        MAX_CAPTURE_BYTES,
    )?)?;
    let slot = args[3]
        .to_str()
        .ok_or("Invalid slot")?
        .parse::<u16>()
        .map_err(|_| "Slot must be a zero-based integer")?;
    let reports = capture
        .reports
        .iter()
        .map(|r| Report64::try_from(r.as_slice()).map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    let plan =
        bank::save_current(capture.device.product_id, &reports, slot).map_err(|e| e.to_string())?;
    let steps:Vec<_>=plan.steps().iter().map(|r|serde_json::json!({"report":r.as_bytes().to_vec(),"ack_required":r.as_bytes()[1]&2!=0})).collect();
    println!("{}",serde_json::to_string_pretty(&serde_json::json!({"kind":"offline_persistent_save_plan","transmitted":false,"hardware_validated":false,"slot":slot,"steps":steps,"warnings":plan.warnings(),"verification_request":bank::stored_request(capture.device.product_id,slot).map_err(|e|e.to_string())?.as_bytes().to_vec()})).map_err(|e|e.to_string())?);
    Ok(())
}
