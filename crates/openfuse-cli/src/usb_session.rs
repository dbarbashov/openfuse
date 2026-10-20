use openfuse_device::{DeviceController, DeviceEvent};
use std::{
    ffi::OsString,
    time::{Duration, Instant},
};

pub fn run(mut args: Vec<OsString>) -> Result<(), String> {
    let trace_path = if let Some(index) = args.iter().position(|a| a == "--trace") {
        if index + 1 >= args.len() {
            return Err("--trace requires a new JSON path".into());
        }
        args.remove(index);
        Some(std::path::PathBuf::from(args.remove(index)))
    } else {
        None
    };
    if trace_path.as_ref().is_some_and(|p| p.exists()) {
        return Err("Trace export requires a new file".into());
    }
    let json = super::take_flag(&mut args, "--json")?;
    let simulate = super::take_flag(&mut args, "--simulate")?;
    let selected = if simulate && args.is_empty() {
        None
    } else if !simulate && args.len() == 2 && args[0] == "--device" {
        Some(
            args[1]
                .to_str()
                .ok_or("device ID must be UTF-8")?
                .to_owned(),
        )
    } else {
        return Err(
            "session requires exactly --simulate or --device ID; get IDs with devices --json"
                .into(),
        );
    };
    let controller = if simulate {
        DeviceController::simulated()
    } else {
        DeviceController::new()
    };
    let recorder = controller.recorder();
    if trace_path.is_some() {
        recorder.start()?;
    }
    let result = session(&controller, simulate, selected, json);
    if let Some(path) = trace_path {
        let export = recorder
            .stop()
            .ok_or_else(|| "Trace recorder was not active".to_owned())
            .and_then(openfuse_device::acceptance::Acceptance::new)
            .and_then(|bundle| bundle.json())
            .and_then(|bytes| openfuse_device::trace::write_new(&path, &bytes));
        if let Err(error) = export {
            return Err(format!(
                "Session result: {result:?}; trace export failed: {error}"
            ));
        }
    }
    result
}
fn session(
    controller: &DeviceController,
    simulate: bool,
    selected: Option<String>,
    json: bool,
) -> Result<(), String> {
    controller.refresh()?;
    let start = Instant::now();
    let mut connected = false;
    let mut captured = None;
    while start.elapsed() < Duration::from_secs(35) {
        for event in controller.poll() {
            match event {
                DeviceEvent::Devices(devices) if !connected => {
                    let device = devices
                        .iter()
                        .find(|d| selected.as_ref().is_none_or(|id| id == &d.id))
                        .ok_or("Selected device not found; refresh devices --json")?;
                    if let Some(reason) = &device.unavailable_reason {
                        return Err(reason.clone());
                    }
                    controller.connect(device)?;
                    connected = true;
                }
                DeviceEvent::Capture(capture) => captured = Some(capture),
                DeviceEvent::Status(status) => {
                    if !json {
                        eprintln!("{}: {}", status.phase, status.detail);
                    }
                    if status.phase == "Connection failed" {
                        return Err(status.detail);
                    }
                    if status.can_sync {
                        let capture = captured
                            .take()
                            .ok_or("Session completed without a capture")?;
                        if json {
                            let output = serde_json::json!({"simulated":simulate,"hardware_validated":false,"kind":"current_preset_wire_snapshot","status":status,"capture":capture});
                            println!(
                                "{}",
                                serde_json::to_string_pretty(&output).map_err(|e| e.to_string())?
                            );
                        } else if let Some(snapshot) = status.snapshot {
                            println!(
                                "{} — slot {}, {} modules, {} raw reports{}",
                                snapshot.name.unwrap_or_default(),
                                snapshot.slot.unwrap_or_default(),
                                snapshot.modules,
                                snapshot.reports,
                                if simulate {
                                    " (synthetic simulator)"
                                } else {
                                    " (unverified hardware read)"
                                }
                            );
                        }
                        controller.disconnect()?;
                        return Ok(());
                    }
                }
                DeviceEvent::Error(error) => {
                    eprintln!("USB: {error}");
                    if !connected {
                        return Err(error);
                    }
                }
                DeviceEvent::Devices(_)
                | DeviceEvent::Live(_)
                | DeviceEvent::Operation(_)
                | DeviceEvent::Tuner(_)
                | DeviceEvent::BankStatus(_)
                | DeviceEvent::BankNames(_)
                | DeviceEvent::Backup(_)
                | DeviceEvent::Auxiliary(_) => {}
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Err("USB session exceeded the 35-second CLI deadline".into())
}
