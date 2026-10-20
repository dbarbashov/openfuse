use openfuse_core::{
    archive::Archive,
    auxiliary::Selection,
    control_library::{export_document, read_document},
    control_preset::ControlPreset,
    protocol::Report64,
};
use std::{collections::BTreeMap, ffi::OsString, path::Path};
type Options = BTreeMap<String, OsString>;
fn options(args: Vec<OsString>, allowed: &[&str]) -> Result<Options, String> {
    if !args.len().is_multiple_of(2) {
        return Err("Each option requires a value".into());
    }
    let mut result = Options::new();
    for pair in args.as_chunks::<2>().0 {
        let key = pair[0].to_str().ok_or("Invalid option")?;
        if !allowed.contains(&key) || result.insert(key.into(), pair[1].clone()).is_some() {
            return Err(format!("Unknown or duplicate option {key}"));
        }
    }
    Ok(result)
}
fn get<'a>(options: &'a Options, key: &str) -> Result<&'a OsString, String> {
    options.get(key).ok_or_else(|| format!("Missing {key}"))
}
fn number(options: &Options, key: &str) -> Result<u16, String> {
    get(options, key)?
        .to_str()
        .ok_or("Invalid number")?
        .parse()
        .map_err(|_| format!("Invalid {key}"))
}
fn profile(options: &Options) -> Result<Option<(u16, u8)>, String> {
    match (options.get("--pid"), options.get("--control")) {
        (None, None) => Ok(None),
        (Some(pid), Some(control)) => {
            let pid = u16::from_str_radix(
                pid.to_str().ok_or("Invalid PID")?.trim_start_matches("0x"),
                16,
            )
            .map_err(|_| "PID must be hexadecimal")?;
            let control = control
                .to_str()
                .ok_or("Invalid bank")?
                .parse()
                .map_err(|_| "Invalid bank")?;
            Ok(Some((pid, control)))
        }
        _ => Err("XML imports require both --pid HEX and --control N".into()),
    }
}
pub fn inspect(args: Vec<OsString>) -> Result<(), String> {
    let options = options(args, &["--path", "--pid", "--control"])?;
    let document = read_document(Path::new(get(&options, "--path")?), profile(&options)?)
        .map_err(|e| e.to_string())?;
    print(
        serde_json::json!({"kind":"individual_control_preset","transmitted":false,"pid":document.pid(),"control":document.control(),"name":document.name(),"xml":document.to_xml().map_err(|e|e.to_string())?}),
    )
}
pub fn plan(args: Vec<OsString>) -> Result<(), String> {
    let options = options(
        args,
        &["--source", "--destination", "--slot", "--pid", "--control"],
    )?;
    let document = read_document(Path::new(get(&options, "--source")?), profile(&options)?)
        .map_err(|e| e.to_string())?;
    let destination = Archive::read_path(Path::new(get(&options, "--destination")?))
        .map_err(|e| e.to_string())?;
    let destination = destination
        .auxiliary()
        .ok_or("Destination must be a complete native bank snapshot")?;
    let slot = number(&options, "--slot")?;
    let selection =
        Selection::from_control_preset(&document, destination, slot).map_err(|e| e.to_string())?;
    let steps:Vec<_>=selection.steps().iter().map(|r|serde_json::json!({"report":r.as_bytes().to_vec(),"ack_required":r.as_bytes()[1]&2!=0})).collect();
    print(
        serde_json::json!({"kind":"offline_individual_preset_save_plan","transmitted":false,"hardware_validated":false,"pid":document.pid(),"control":document.control(),"slot":slot,"name":document.name(),"steps":steps,"destination_fingerprint":destination.fingerprint()}),
    )
}
pub fn extract(args: Vec<OsString>) -> Result<(), String> {
    let options = options(args, &["--capture", "--control", "--out"])?;
    let capture = openfuse_device::parse_capture_json(&super::wire_preset::bounded(
        Path::new(get(&options, "--capture")?),
        openfuse_device::MAX_CAPTURE_BYTES,
    )?)?;
    let reports = capture
        .reports
        .iter()
        .map(|r| Report64::try_from(r.as_slice()).map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    let control =
        u8::try_from(number(&options, "--control")?).map_err(|_| "Control exceeds byte range")?;
    let document = ControlPreset::from_reports(capture.device.product_id, control, &reports)
        .map_err(|e| e.to_string())?;
    let out = Path::new(get(&options, "--out")?);
    export_document(&document, out).map_err(|e| e.to_string())?;
    print(
        serde_json::json!({"kind":"offline_control_preset_extraction","transmitted":false,"pid":document.pid(),"control":document.control(),"name":document.name(),"output":out}),
    )
}
fn print(value: serde_json::Value) -> Result<(), String> {
    println!(
        "{}",
        serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?
    );
    Ok(())
}
