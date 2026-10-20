//! Offline message codecs. Static field layouts from DevComms 2.7.1, with
//! synthetic fixtures. No device I/O.
use crate::protocol::Report64;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ParameterChange {
    /// HID module type, not the FXDB category ID.
    pub module_type: u8,
    pub model_id: u16,
    pub parameter_index: u8,
    pub control_index: u8,
    pub parameter_type: u16,
    /// Wire value. G-DEC delay conversion must happen before constructing this.
    pub wire_value: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MessageFields {
    Handshake {
        command: u8,
        status: u8,
        major: u8,
        minor: u8,
    },
    Version {
        major: u8,
        minor: u8,
        build: u16,
        build_date: [u8; 16],
    },
    Parameter(ParameterChange),
    TunerControl {
        enabled: bool,
        snap_to_state: bool,
        muted: bool,
    },
    TunerInfo {
        tone: u8,
        offset: i8,
    },
    PresetLoad {
        slot: u8,
    },
    EffectState {
        module_type: u8,
        active: bool,
        position: u8,
    },
    EffectPositions {
        stomp: u8,
        modulation: u8,
        delay: u8,
        reverb: u8,
    },
    KnobPosition {
        position: u8,
        knob: u8,
    },
    BypassMode {
        module_type: u8,
        active: bool,
        mode: u8,
    },
    Sync,
    Unknown {
        id: u8,
    },
}

/// Interpret a report in the device-to-host direction. ID 0 is a handshake
/// inbound but device detection outbound. The original report is never mutated;
/// callers retain it for flags, reserved bytes and unknown enum values.
pub fn decode_inbound(report: &Report64) -> MessageFields {
    let b = report.as_bytes();
    match b[0] {
        0 => MessageFields::Handshake {
            command: b[2],
            status: b[3],
            major: b[4],
            minor: b[5],
        },
        1 => MessageFields::Version {
            major: b[2],
            minor: b[3],
            build: word(b, 20),
            build_date: b[4..20].try_into().expect("fixed report"),
        },
        5 => MessageFields::Parameter(ParameterChange {
            module_type: b[2],
            model_id: word(b, 3),
            parameter_index: b[5],
            control_index: b[6],
            parameter_type: word(b, 7),
            wire_value: word(b, 9),
        }),
        10 => MessageFields::TunerControl {
            enabled: b[2] != 0,
            snap_to_state: b[3] != 0,
            muted: b[4] != 0,
        },
        11 => MessageFields::TunerInfo {
            tone: b[2],
            offset: b[3] as i8,
        },
        19 => MessageFields::PresetLoad { slot: b[2] },
        25 => MessageFields::EffectState {
            module_type: b[2],
            active: b[3] == 0,
            position: b[4],
        },
        27 => MessageFields::EffectPositions {
            stomp: b[2],
            modulation: b[3],
            delay: b[4],
            reverb: b[5],
        },
        29 => MessageFields::KnobPosition {
            position: b[2],
            knob: b[3],
        },
        32 => MessageFields::BypassMode {
            module_type: b[2],
            active: b[3] == 0,
            mode: b[4],
        },
        255 => MessageFields::Sync,
        id => MessageFields::Unknown { id },
    }
}

fn word(b: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([b[offset], b[offset + 1]])
}
fn command(id: u8, flags: u8, payload: &[u8]) -> Report64 {
    let mut bytes = [0; 64];
    bytes[0] = id;
    bytes[1] = flags;
    bytes[2..2 + payload.len()].copy_from_slice(payload);
    Report64::from_bytes(bytes)
}

impl ParameterChange {
    pub fn encode(self) -> Report64 {
        let m = self.model_id.to_le_bytes();
        let t = self.parameter_type.to_le_bytes();
        let v = self.wire_value.to_le_bytes();
        command(
            5,
            0xc3,
            &[
                self.module_type,
                m[0],
                m[1],
                self.parameter_index,
                self.control_index,
                t[0],
                t[1],
                v[0],
                v[1],
            ],
        )
    }
}

pub fn effect_positions(stomp: u8, modulation: u8, delay: u8, reverb: u8) -> Report64 {
    command(27, 0xc3, &[stomp, modulation, delay, reverb])
}
pub fn effect_state(module_type: u8, active: bool, position: u8) -> Report64 {
    command(25, 0xc3, &[module_type, u8::from(!active), position])
}
pub fn bypass_mode(module_type: u8, active: bool, mode: u8) -> Report64 {
    command(32, 0xc3, &[module_type, u8::from(!active), mode])
}
pub fn tuner_control(enabled: bool, snap_to_state: bool, muted: bool) -> Report64 {
    command(10, 1, &[enabled.into(), snap_to_state.into(), muted.into()])
}
pub fn preset_load(slot: u8) -> Report64 {
    command(19, 0xc3, &[slot])
}
pub fn knob_position(position: u8, knob: u8) -> Report64 {
    command(29, 1, &[position, knob])
}
pub fn sync_request(current_preset_only: bool) -> Report64 {
    command(255, if current_preset_only { 0xc5 } else { 0xc1 }, &[])
}

/// Legacy two-message module upload. This is distinct from Mustang 0x1c
/// preset blocks and intentionally has no transport/session implementation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleLoad {
    pub module_type: u8,
    pub model_id: u16,
    pub position: u8,
    /// Parameter index / already converted wire value pairs. Maximum 19.
    pub parameters: Vec<(u8, u16)>,
}
impl ModuleLoad {
    pub fn encode(&self) -> Result<[Report64; 2], &'static str> {
        if self.parameters.len() > 19 {
            return Err("module report allows at most 19 parameters");
        }
        if self.model_id == 0 && !self.parameters.is_empty() {
            return Err("None model must have no parameters");
        }
        let mut seen = [false; 256];
        for &(index, _) in &self.parameters {
            if seen[index as usize] {
                return Err("duplicate parameter index");
            }
            seen[index as usize] = true;
        }
        let mut b = [0; 64];
        b[0] = 6;
        b[1] = 0x43;
        b[2] = self.module_type;
        b[3..5].copy_from_slice(&self.model_id.to_le_bytes());
        b[5] = self.parameters.len() as u8;
        b[6] = self.position;
        for (n, &(index, value)) in self.parameters.iter().enumerate() {
            b[7 + n] = index;
            b[26 + 2 * n..28 + 2 * n].copy_from_slice(&value.to_le_bytes());
        }
        Ok([
            Report64::from_bytes(b),
            command(7, 0x83, &[self.module_type]),
        ])
    }
}
