use openfuse_core::{
    preset::{MAX_PRESET_BYTES, PresetDocument},
    protocol::Report64,
    wire_preset,
};
use openfuse_device::{DeviceInfo, MAX_CAPTURE_BYTES, WireCapture, parse_capture_json};
use std::{ffi::OsString, fs::File, io::Read, path::Path};
pub(crate) fn bounded(path: &Path, limit: usize) -> Result<String, String> {
    let mut input = String::new();
    File::open(path)
        .map_err(|e| e.to_string())?
        .take(limit as u64 + 1)
        .read_to_string(&mut input)
        .map_err(|e| e.to_string())?;
    if input.len() > limit {
        return Err(format!("Input exceeds {limit} bytes"));
    }
    Ok(input)
}
pub fn decode(mut args: Vec<OsString>) -> Result<(), String> {
    let json = super::take_flag(&mut args, "--json")?;
    if args.len() != 2 || args[0] != "--file" {
        return Err("capture-decode requires --file CAPTURE.json [--json]".into());
    }
    let capture = parse_capture_json(&bounded(Path::new(&args[1]), MAX_CAPTURE_BYTES)?)?;
    let reports = capture
        .reports
        .iter()
        .map(|r| Report64::try_from(r.as_slice()).map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    let decoded =
        wire_preset::decode(capture.device.product_id, &reports).map_err(|e| e.to_string())?;
    let xml = decoded.document.to_xml().map_err(|e| e.to_string())?;
    if json {
        println!("{}",serde_json::to_string_pretty(&serde_json::json!({"kind":"decoded_device_preset","product_id":decoded.document.product_id(),"warnings":decoded.warnings,"xml":xml,"device_access":false})).map_err(|e|e.to_string())?);
    } else {
        for warning in decoded.warnings {
            eprintln!("Capture: {warning}");
        }
        println!("{xml}");
    }
    Ok(())
}
pub fn encode(args: Vec<OsString>) -> Result<(), String> {
    if args.len() != 4 || args[0] != "--pid" || args[2] != "--file" {
        return Err("preset-encode requires --pid HEX --file PRESET.fuse".into());
    }
    let text = args[1].to_str().ok_or("PID must be hexadecimal text")?;
    let pid = u16::from_str_radix(text.trim_start_matches("0x"), 16)
        .map_err(|_| "PID must be hexadecimal, e.g. 0014")?;
    let doc = PresetDocument::parse(&bounded(Path::new(&args[3]), MAX_PRESET_BYTES)?)
        .map_err(|e| e.to_string())?;
    let encoded = wire_preset::encode(pid, &doc).map_err(|e| e.to_string())?;
    let capture = WireCapture {
        device: DeviceInfo {
            id: "offline-encoded".into(),
            name: "Offline encoded preset".into(),
            product_id: pid,
            serial: None,
            interface_number: 0,
            unavailable_reason: None,
        },
        generation: 0,
        version: None,
        reports: encoded
            .reports
            .iter()
            .map(|r| r.as_bytes().to_vec())
            .collect(),
        received: vec![],
        dropped_received: 0,
    };
    println!("{}",serde_json::to_string_pretty(&serde_json::json!({"kind":"offline_encoded_preset","transmitted":false,"hardware_validated":false,"capture":capture,"warnings":encoded.warnings})).map_err(|e|e.to_string())?);
    Ok(())
}
