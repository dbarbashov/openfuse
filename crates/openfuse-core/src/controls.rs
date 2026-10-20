//! Isolated device settings and tuner controls recovered from FUSE 2.7.1.
//! None of these capabilities imply hardware validation.
use crate::{audition, messages, preset::expression::PedalSlot, protocol::Report64, wire_preset};
use serde::Serialize;
use std::fmt;
mod tuner;
pub use tuner::{TunerControlState, TunerNote, TunerReading};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlsError(String);
impl fmt::Display for ControlsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for ControlsError {}
type Result<T> = std::result::Result<T, ControlsError>;
fn error(message: impl fmt::Display) -> ControlsError {
    ControlsError(message.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Capabilities {
    pub tuner: bool,
    pub usb_gain: bool,
    pub global_volume: bool,
    pub fx_loop: bool,
    pub first_pedal: bool,
    pub second_pedal: bool,
}
/// Physical device UI capabilities differ from optional expression XML blocks.
pub fn capabilities(pid: u16) -> Result<Capabilities> {
    wire_preset::product_for_pid(pid).ok_or_else(|| error("Unknown device controls profile"))?;
    let first_pedal = matches!(pid, 5 | 0x16 | 0x12);
    Ok(Capabilities {
        tuner: pid != 0xe,
        usb_gain: true,
        global_volume: first_pedal,
        fx_loop: pid == 0x12,
        first_pedal,
        second_pedal: pid == 0x12,
    })
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct VolumeEndpoints {
    pub heel: u16,
    pub toe: u16,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Settings {
    pub warnings: Vec<String>,
    pub usb_gain: Option<u8>,
    /// Historical serialized `FxLoopBypass` bit. Despite its name, true is
    /// displayed as active by original FUSE and sends FxKill.IsActive = true.
    pub fx_loop_bypassed: Option<bool>,
    pub global_volume: Option<VolumeEndpoints>,
    pub first_pedal_mode: Option<u8>,
    pub second_pedal_mode: Option<u8>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SettingsPatch {
    pub usb_gain: Option<u16>,
    /// Same historical bit as Settings::fx_loop_bypassed; true is UI-active.
    pub fx_loop_bypassed: Option<bool>,
    pub global_volume: Option<VolumeEndpoints>,
    /// Original PedalMode values: 0 = Capture, 1 = Live.
    pub first_pedal_mode: Option<u8>,
    pub second_pedal_mode: Option<u8>,
}
impl SettingsPatch {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}
fn auxiliary(reports: &[Report64], kind: u8, slot: u8) -> Option<&Report64> {
    reports.iter().find(|r| {
        let b = r.as_bytes();
        b[0] == 0x1c && b[2] == kind && b[3] == 0 && (kind != 10 || b[25] == slot)
    })
}
/// Missing reports stay absent, so a UI cannot mistake a default for device state.
/// Decode validates the complete capture and keeps unfamiliar enum bytes visible.
pub fn settings(pid: u16, reports: &[Report64]) -> Result<Settings> {
    let caps = capabilities(pid)?;
    wire_preset::decode(pid, reports).map_err(error)?;
    let first = auxiliary(reports, 10, 0).map(Report64::as_bytes);
    let second = auxiliary(reports, 10, 1).map(Report64::as_bytes);
    let endpoints: Vec<_> = reports
        .iter()
        .filter_map(|r| {
            let b = r.as_bytes();
            (b[0] == 0x1c && b[2] == 10 && b[3] == 0).then_some(VolumeEndpoints {
                heel: u16::from(b[17]) * 256,
                toe: u16::from(b[18]) * 256,
            })
        })
        .collect();
    let globals = endpoints.last().copied();
    let mut warnings = Vec::new();
    if caps.global_volume && endpoints.iter().any(|value| Some(*value) != globals) {
        warnings.push("Pedal blocks disagree about shared volume endpoints; showing the last received pair as original FUSE does. Applying endpoints explicitly updates both Floor pedal blocks".into());
    }
    Ok(Settings {
        warnings,
        usb_gain: auxiliary(reports, 13, 0).map(|r| r.as_bytes()[16]),
        fx_loop_bypassed: if caps.fx_loop {
            auxiliary(reports, 14, 0).map(|r| r.as_bytes()[16] != 0)
        } else {
            None
        },
        global_volume: if caps.global_volume { globals } else { None },
        first_pedal_mode: if caps.first_pedal {
            first.map(|b| b[24])
        } else {
            None
        },
        second_pedal_mode: if caps.second_pedal {
            second.map(|b| b[24])
        } else {
            None
        },
    })
}
fn frame(kind: u8, current: bool) -> [u8; 64] {
    let mut b = [0; 64];
    b[0] = 0x1c;
    b[1] = 3;
    b[2] = kind;
    if current {
        b[6] = 1;
        b[7] = 1;
    }
    b
}
fn replace(reports: &mut [Report64], kind: u8, slot: u8, offset: usize, value: u8) -> Result<()> {
    let target = reports
        .iter_mut()
        .find(|r| {
            let b = r.as_bytes();
            b[0] == 0x1c && b[2] == kind && b[3] == 0 && (kind != 10 || b[25] == slot)
        })
        .ok_or_else(|| error("Read the required device settings before changing them"))?;
    let mut b = *target.as_bytes();
    b[offset] = value;
    *target = Report64::from_bytes(b);
    Ok(())
}

fn expression_packet(
    document: &crate::preset::PresetDocument,
    slot: PedalSlot,
    captured: &[u8; 64],
) -> Result<Report64> {
    let report =
        audition::expression_packet(document, slot, (captured[17], captured[18])).map_err(error)?;
    let mut bytes = *report.as_bytes();
    if slot == PedalSlot::Second {
        // Original isolated ExpressionModule getter/setter copies these fields
        // for BOTH slots; second-pedal XML omits them. Reserved bytes stay zero.
        bytes[16] = captured[16];
        bytes[28] = captured[28];
    }
    Ok(Report64::from_bytes(bytes))
}

/// Immutable isolated patch. Scalars follow their original direct handlers;
/// expression payloads use the audited constructor and reject lossy unknown bytes.
/// Editing shared global endpoints explicitly sends each supported physical pedal.
pub fn settings_plan(
    pid: u16,
    current: &[Report64],
    patch: &SettingsPatch,
) -> Result<audition::Plan> {
    let caps = capabilities(pid)?;
    let state = settings(pid, current)?;
    if patch.is_empty() {
        return Err(error("No device settings were selected"));
    }
    if patch.fx_loop_bypassed.is_some() && !caps.fx_loop {
        return Err(error("FX loop is unavailable on this device"));
    }
    if patch.global_volume.is_some() && !caps.global_volume {
        return Err(error(
            "Global expression volume is unavailable on this device",
        ));
    }
    if patch.first_pedal_mode.is_some() && !caps.first_pedal {
        return Err(error("First physical expression pedal is unavailable"));
    }
    if patch.second_pedal_mode.is_some() && !caps.second_pedal {
        return Err(error("Second physical expression pedal is unavailable"));
    }
    if patch
        .first_pedal_mode
        .into_iter()
        .chain(patch.second_pedal_mode)
        .any(|mode| mode > 1)
    {
        return Err(error("Pedal mode must be Capture (0) or Live (1)"));
    }
    let mut expected = current.to_vec();
    let mut steps = Vec::new();
    let mut warnings = state.warnings.clone();
    if let Some(value) = patch.usb_gain {
        let gain =
            u8::try_from(value).map_err(|_| error("USB gain must be in 0..255 (0..100%)"))?;
        if state.usb_gain.is_none() {
            return Err(error("Read USB gain before changing it"));
        }
        let original = auxiliary(current, 13, 0).expect("checked gain").as_bytes();
        if original[17..].iter().any(|byte| *byte != 0) {
            return Err(error(
                "USB gain contains unrepresented bytes; preserving this state requires further protocol evidence",
            ));
        }
        let mut b = frame(13, true);
        b[16] = gain;
        steps.push(Report64::from_bytes(b));
        steps.push(Report64::from_bytes(frame(0, false)));
        replace(&mut expected, 13, 0, 16, gain)?;
    }
    if let Some(bypassed) = patch.fx_loop_bypassed {
        if state.fx_loop_bypassed.is_none() {
            return Err(error("Read FX loop state before changing it"));
        }
        // The original bypass handler passes BypassStatus as FxKill.IsActive.
        steps.push(messages::effect_state(15, bypassed, 0));
        replace(&mut expected, 14, 0, 16, u8::from(bypassed))?;
    }
    if let Some(endpoints) = patch.global_volume
        && (endpoints.heel & 255 != 0 || endpoints.toe & 255 != 0)
    {
        warnings.push("Global volume endpoints use the high 8 bits; readback is quantized to multiples of 256".into());
    }
    for (slot, mode, supported) in [
        (PedalSlot::First, patch.first_pedal_mode, caps.first_pedal),
        (
            PedalSlot::Second,
            patch.second_pedal_mode,
            caps.second_pedal,
        ),
    ] {
        if !supported || (mode.is_none() && patch.global_volume.is_none()) {
            continue;
        }
        let index = slot.index() as u8;
        let before = auxiliary(current, 10, index)
            .ok_or_else(|| error("Read expression settings before changing them"))?
            .as_bytes();
        let doc = wire_preset::decode(pid, current).map_err(error)?.document;
        let original = expression_packet(&doc, slot, before)?;
        if original.as_bytes()[16..] != before[16..] {
            return Err(error(
                "Expression packet contains unrepresented bytes; this edit would discard unknown state",
            ));
        }
        if let Some(mode) = mode {
            replace(&mut expected, 10, index, 24, mode)?;
        }
        if let Some(endpoints) = patch.global_volume {
            replace(&mut expected, 10, index, 17, (endpoints.heel >> 8) as u8)?;
            replace(&mut expected, 10, index, 18, (endpoints.toe >> 8) as u8)?;
        }
        let after = auxiliary(&expected, 10, index)
            .expect("retained expression")
            .as_bytes();
        let doc = wire_preset::decode(pid, &expected).map_err(error)?.document;
        steps.push(expression_packet(&doc, slot, after)?);
        steps.push(Report64::from_bytes(frame(0, false)));
    }
    if patch.global_volume.is_some() && caps.second_pedal {
        warnings.push("Shared volume endpoints are sent explicitly to both Floor pedal blocks, with each original pedal transaction completed".into());
    }
    audition::settings_plan(pid, current, steps, &expected, warnings).map_err(error)
}

/// Tuner has no delivery ACK. Callers must await device control-state feedback,
/// expire stale pitch data, and avoid reporting a write as a confirmed state.
pub fn tuner_control(pid: u16, enabled: bool, muted: bool) -> Result<Report64> {
    if !capabilities(pid)?.tuner {
        return Err(error("Tuner is unavailable on this device"));
    }
    Ok(messages::tuner_control(enabled, enabled, enabled && muted))
}
