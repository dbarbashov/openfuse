use std::{env, ffi::OsString, fs, process::ExitCode};

use openfuse_core::{catalog, hex, protocol, report::ByteInspection};

mod acceptance;
mod archives;
mod audition;
mod auxiliary;
mod bank;
mod control_presets;
mod conversion;
mod devices;
mod media;
mod storage;
mod usb_session;
mod wire_preset;

const HELP: &str = "openfuse — open amplifier preservation research tools

Usage:
  openfuse inspect [--json] HEX...
  openfuse inspect [--json] --file PATH
  openfuse inspect [--json] --hex-file PATH
  openfuse media-list --library DIR [--trash]
  openfuse media-import --library DIR --path FILE_OR_FOLDER
  openfuse band-inspect --file PRESET.fuse
  openfuse storage-inspect --root DIR --kind sd|internal
  openfuse storage-upload-plan --root DIR --kind sd|internal --file FILE [--file FILE...] [--keep-both]
  openfuse storage-download-plan --root DIR --kind sd|internal --entry RELATIVE [--entry RELATIVE...] --out DIR [--keep-both]
  openfuse catalog [--json]
  openfuse devices [--json]
  openfuse session (--simulate | --device ID) [--json] [--trace NEW_SESSION.json]
  openfuse trace-inspect --file SESSION.json
  openfuse trace-replay --file SESSION.json
  openfuse acceptance-report --file SESSION.json --out NEW_REPORT.html
  openfuse capture-decode --file CAPTURE.json [--json]
  openfuse preset-encode --pid HEX --file PRESET.fuse
  openfuse preset-convert-plan --file PRESET.fuse --target PRODUCT_ID [--repair-double-entities] [--normalize-legacy-delays] [--assume-missing-gdec]
  openfuse preset-convert --file PRESET.fuse --target PRODUCT_ID --out NEW_FILE --accept-changes [--repair-double-entities] [--normalize-legacy-delays] [--assume-missing-gdec]
  openfuse audition-plan --capture CURRENT.json --file LOCAL.fuse
  openfuse slot-plan --pid HEX --slot ZERO_BASED_INDEX
  openfuse save-plan --capture CURRENT.json --slot ZERO_BASED_INDEX
  openfuse bank-inspect --file BANK.json
  openfuse control-preset-inspect --path FILE [--pid HEX --control N]
  openfuse control-preset-plan --source FILE --destination BANKS.json --slot N [--pid HEX --control N]
  openfuse control-preset-extract --capture CURRENT.json --control N --out NEW_FILE
  openfuse auxiliary-inspect --path ARCHIVE [--source-pid HEX]
  openfuse auxiliary-restore-plan --source ARCHIVE --destination SNAPSHOT --map CONTROL:SLOT=CONTROL:SLOT,... [--source-pid HEX]
  openfuse whole-restore-plan --source ARCHIVE --destination SNAPSHOT [--source-pid HEX]
  openfuse archive-inspect --path ARCHIVE
  openfuse archive-restore-plan --capture CURRENT.json --archive ARCHIVE --map SOURCE:TARGET,... [--source-pid HEX]
  openfuse decode [--json] [--hid-output] HEX...
  openfuse encode [--json] [--hid-output] device-detect|version-request
  openfuse --version

inspect     Show bytes and offsets without assuming protocol semantics.
--file      Read a raw binary capture (not a hexadecimal text file).
--hex-file  Read UTF-8 hexadecimal text; accepts 0x prefixes, spaces, : and -.
catalog     List device IDs verified in the original software.
session     Explicit detect/version/current-preset read; --simulate needs no HID.
capture-decode  Offline capture to editable FUSE XML; warnings go to stderr.
preset-encode   Offline FUSE to packet JSON; no device is opened or written.
audition-plan   Preview a temporary upload with preserved current global settings.
bank-inspect    Validate a complete or selected preset bank archive offline.
archive-inspect Inspect a historical folder, native bank or catalog bundle offline.
archive-restore-plan Preview selected slot writes offline; legacy folders require a source PID choice.
whole-restore-plan Preview complete device restore, known normalizations and ACK timeouts offline.
save-plan       Preview a persistent slot save offline; nothing is transmitted.
slot-plan       Generate the Mustang-style slot selection packet offline.
devices     Enumerate Fender HID interfaces (requires the hid build feature).
decode      Decode an inbound 64-byte report, preserving raw bytes.
encode      Generate verified request bytes offline. Nothing is transmitted.
--hid-output  Use the 65-byte output format with its zero report ID prefix.

Offline commands do not require hardware. Enumeration never opens devices.
Only session --device sends read requests; no preset uploads or persistent writes.
";

fn main() -> ExitCode {
    match run(env::args_os().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("openfuse: {error}");
            ExitCode::from(2)
        }
    }
}

fn run(mut args: Vec<OsString>) -> Result<(), String> {
    if args.is_empty() || matches!(args[0].to_str(), Some("--help" | "-h" | "help")) {
        print!("{HELP}");
        return Ok(());
    }
    let command = args.remove(0);
    match command.to_str() {
        Some("--version" | "-V") if args.is_empty() => {
            println!("openfuse {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("media-list" | "media-import" | "band-inspect") => {
            media::run(command.to_str().unwrap(), args)
        }
        Some("inspect") => inspect(args),
        Some(command @ ("trace-inspect" | "trace-replay" | "acceptance-report")) => {
            acceptance::run(command, args)
        }
        Some("session") => usb_session::run(args),
        Some("capture-decode") => wire_preset::decode(args),
        Some("preset-encode") => wire_preset::encode(args),
        Some("preset-convert-plan") => conversion::run(args, false),
        Some("preset-convert") => conversion::run(args, true),
        Some("audition-plan") => audition::preset(args),
        Some("slot-plan") => audition::slot(args),
        Some("save-plan") => bank::save_plan(args),
        Some("bank-inspect") => bank::inspect(args),
        Some("control-preset-inspect") => control_presets::inspect(args),
        Some("control-preset-plan") => control_presets::plan(args),
        Some("control-preset-extract") => control_presets::extract(args),
        Some(command @ ("storage-inspect" | "storage-upload-plan" | "storage-download-plan")) => {
            storage::run(command, args)
        }
        Some("auxiliary-inspect") => auxiliary::inspect(args),
        Some("auxiliary-restore-plan") => auxiliary::restore_plan(args),
        Some("whole-restore-plan") => auxiliary::whole_restore_plan(args),
        Some("archive-inspect") => archives::inspect(args),
        Some("archive-restore-plan") => archives::restore_plan(args),
        Some("catalog") => print_catalog(args),
        Some("devices") => {
            let json = take_json_flag(&mut args)?;
            if !args.is_empty() {
                return Err("devices accepts only --json".into());
            }
            devices::enumerate(json)
        }
        Some("encode") => encode(args),
        Some("decode") => decode(args),
        _ => Err(format!("unknown command {:?}; use --help", command)),
    }
}

fn encode(mut args: Vec<OsString>) -> Result<(), String> {
    let json = take_json_flag(&mut args)?;
    let hid_output = take_flag(&mut args, "--hid-output")?;
    if args.len() != 1 {
        return Err("encode requires device-detect or version-request".into());
    }
    let report = match args[0].to_str() {
        Some("device-detect") => protocol::Report64::device_detect(),
        Some("version-request") => protocol::Report64::version_request(),
        _ => return Err("unknown request; use device-detect or version-request".into()),
    };
    let bytes = if hid_output {
        report.to_hid_output().to_vec()
    } else {
        report.as_bytes().to_vec()
    };
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&ByteInspection::new(bytes))
                .map_err(|error| error.to_string())?
        );
    } else {
        println!("{}", hex::encode(&bytes));
    }
    Ok(())
}

fn decode(mut args: Vec<OsString>) -> Result<(), String> {
    let json = take_json_flag(&mut args)?;
    let hid_output = take_flag(&mut args, "--hid-output")?;
    let text = args
        .iter()
        .map(|arg| arg.to_str().ok_or("hex input must be UTF-8"))
        .collect::<Result<Vec<_>, _>>()?
        .join(" ");
    let bytes = hex::decode(&text).map_err(|error| error.to_string())?;
    let report = if hid_output {
        protocol::Report64::from_hid_output(&bytes)
    } else {
        protocol::Report64::try_from(bytes.as_slice())
    }
    .map_err(|error| error.to_string())?;
    let value = serde_json::json!({
        "message_id": report.message_id(),
        "flags": report.flags(),
        "start": report.has_flag(protocol::FLAG_START),
        "end": report.has_flag(protocol::FLAG_END),
        "ack": report.has_flag(protocol::FLAG_ACK),
        "secondary": report.has_flag(protocol::FLAG_SECONDARY),
        "unknown_flags": report.unknown_flags(),
        "body_hex": hex::encode(report.body()),
        "direction": "device_to_host",
        "fields": openfuse_core::messages::decode_inbound(&report),
    });
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?
        );
    } else {
        println!(
            "Message ID: 0x{:02x}; flags: 0x{:02x}; unknown flag bits: 0x{:02x}",
            report.message_id(),
            report.flags(),
            report.unknown_flags()
        );
        println!(
            "start={} end={} ack={} secondary={}",
            value["start"], value["end"], value["ack"], value["secondary"]
        );
        println!("Raw body: {}", hex::encode(report.body()));
        println!("Inbound fields: {}", value["fields"]);
    }
    Ok(())
}

fn inspect(mut args: Vec<OsString>) -> Result<(), String> {
    let json = take_json_flag(&mut args)?;
    let bytes = match args.first().and_then(|arg| arg.to_str()) {
        Some("--file" | "--hex-file") => {
            if args.len() != 2 {
                return Err("a file option requires exactly one path".into());
            }
            let path = &args[1];
            if args[0] == "--file" {
                fs::read(path).map_err(|error| format!("cannot read {:?}: {error}", path))?
            } else {
                let text = fs::read_to_string(path)
                    .map_err(|error| format!("cannot read UTF-8 text {:?}: {error}", path))?;
                hex::decode(&text).map_err(|error| error.to_string())?
            }
        }
        _ => {
            let text = args
                .iter()
                .map(|arg| arg.to_str().ok_or("hex input must be UTF-8"))
                .collect::<Result<Vec<_>, _>>()?
                .join(" ");
            hex::decode(&text).map_err(|error| error.to_string())?
        }
    };
    let report = ByteInspection::new(bytes);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?
        );
    } else {
        println!(
            "{} bytes — raw inspection; protocol framing not inferred",
            report.byte_count
        );
        print!("{}", report.hexdump());
    }
    Ok(())
}

fn print_catalog(mut args: Vec<OsString>) -> Result<(), String> {
    let json = take_json_flag(&mut args)?;
    if !args.is_empty() {
        return Err("catalog accepts only --json".into());
    }
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(catalog::DEVICES).map_err(|error| error.to_string())?
        );
    } else {
        println!("Fender vendor ID: {:04x}", catalog::FENDER_VENDOR_ID);
        println!("Verified software identities; hardware compatibility is not yet tested.");
        for device in catalog::DEVICES {
            println!(
                "{:04x}:{:04x}  {}",
                device.vendor_id, device.product_id, device.name
            );
            println!("           evidence: {}", device.evidence);
        }
        if catalog::DEVICES.is_empty() {
            println!("No product mappings have been entered yet.");
        }
    }
    Ok(())
}

fn take_json_flag(args: &mut Vec<OsString>) -> Result<bool, String> {
    take_flag(args, "--json")
}

fn take_flag(args: &mut Vec<OsString>, flag: &str) -> Result<bool, String> {
    let count = args.iter().filter(|arg| *arg == flag).count();
    if count > 1 {
        return Err(format!("{flag} was supplied more than once"));
    }
    args.retain(|arg| arg != flag);
    Ok(count == 1)
}
