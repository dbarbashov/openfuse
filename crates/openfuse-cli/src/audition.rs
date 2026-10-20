use openfuse_core::{
    audition::{self, Plan},
    preset::{MAX_PRESET_BYTES, PresetDocument},
    protocol::Report64,
};
use openfuse_device::{MAX_CAPTURE_BYTES, parse_capture_json};
use std::{ffi::OsString, path::Path};
fn print(plan: Plan) -> Result<(), String> {
    let expected = plan
        .expected()
        .map(PresetDocument::to_xml)
        .transpose()
        .map_err(|e| e.to_string())?;
    let steps:Vec<_>=plan.steps().iter().enumerate().map(|(index,report)|serde_json::json!({"index":index,"ack_required":report.as_bytes()[1]&2!=0,"report":report.as_bytes().to_vec()})).collect();
    println!("{}",serde_json::to_string_pretty(&serde_json::json!({"kind":"offline_working_preset_plan","operation":plan.label(),"product_id":plan.pid(),"slot":plan.expected_slot(),"transmitted":false,"hardware_validated":false,"warnings":plan.warnings(),"steps":steps,"expected_readback_xml":expected})).map_err(|e|e.to_string())?);
    Ok(())
}
pub fn preset(args: Vec<OsString>) -> Result<(), String> {
    if args.len() != 4 || args[0] != "--capture" || args[2] != "--file" {
        return Err("audition-plan requires --capture CURRENT.json --file LOCAL.fuse".into());
    }
    let capture = parse_capture_json(&super::wire_preset::bounded(
        Path::new(&args[1]),
        MAX_CAPTURE_BYTES,
    )?)?;
    let current = capture
        .reports
        .iter()
        .map(|r| Report64::try_from(r.as_slice()).map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    let document = PresetDocument::parse(&super::wire_preset::bounded(
        Path::new(&args[3]),
        MAX_PRESET_BYTES,
    )?)
    .map_err(|e| e.to_string())?;
    print(
        audition::preset(capture.device.product_id, &document, &current)
            .map_err(|e| e.to_string())?,
    )
}
pub fn slot(args: Vec<OsString>) -> Result<(), String> {
    if args.len() != 4 || args[0] != "--pid" || args[2] != "--slot" {
        return Err("slot-plan requires --pid HEX --slot ZERO_BASED_INDEX".into());
    }
    let pid = u16::from_str_radix(
        args[1]
            .to_str()
            .ok_or("Invalid PID")?
            .trim_start_matches("0x"),
        16,
    )
    .map_err(|_| "PID must be hexadecimal")?;
    let slot = args[3]
        .to_str()
        .ok_or("Invalid slot")?
        .parse::<u16>()
        .map_err(|_| "Slot must be a zero-based integer")?;
    print(audition::slot_select(pid, slot).map_err(|e| e.to_string())?)
}
