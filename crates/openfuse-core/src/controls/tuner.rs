//! Original TunerInfoMessage enum/layout, not a frequency or cents estimator.
use crate::protocol::Report64;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum TunerNote {
    A,
    ASharp,
    B,
    C,
    CSharp,
    D,
    DSharp,
    E,
    F,
    FSharp,
    G,
    GSharp,
    Invalid,
    Unknown(u8),
}
impl TunerNote {
    pub fn from_raw(value: u8) -> Self {
        match value {
            0 => Self::A,
            1 => Self::ASharp,
            2 => Self::B,
            3 => Self::C,
            4 => Self::CSharp,
            5 => Self::D,
            6 => Self::DSharp,
            7 => Self::E,
            8 => Self::F,
            9 => Self::FSharp,
            10 => Self::G,
            11 => Self::GSharp,
            12 => Self::Invalid,
            other => Self::Unknown(other),
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::ASharp => "A♯",
            Self::B => "B",
            Self::C => "C",
            Self::CSharp => "C♯",
            Self::D => "D",
            Self::DSharp => "D♯",
            Self::E => "E",
            Self::F => "F",
            Self::FSharp => "F♯",
            Self::G => "G",
            Self::GSharp => "G♯",
            Self::Invalid => "No note",
            Self::Unknown(_) => "Unknown note",
        }
    }
    pub fn is_pitch(self) -> bool {
        !matches!(self, Self::Invalid | Self::Unknown(_))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct TunerReading {
    pub tone: u8,
    /// Signed original display deviation; calibration in cents is not proven.
    pub offset: i8,
    pub note: TunerNote,
}
impl TunerReading {
    pub fn decode(report: &Report64) -> Option<Self> {
        let b = report.as_bytes();
        (b[0] == 11).then_some(Self {
            tone: b[2],
            offset: b[3] as i8,
            note: TunerNote::from_raw(b[2]),
        })
    }
}

/// Retain the raw bool bytes: an unfamiliar value is not rewritten on decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct TunerControlState {
    pub enabled_raw: u8,
    pub snap_to_state_raw: u8,
    pub muted_raw: u8,
}
impl TunerControlState {
    pub fn decode(report: &Report64) -> Option<Self> {
        let b = report.as_bytes();
        (b[0] == 10).then_some(Self {
            enabled_raw: b[2],
            snap_to_state_raw: b[3],
            muted_raw: b[4],
        })
    }
    pub fn enabled(self) -> bool {
        self.enabled_raw != 0
    }
    pub fn muted(self) -> bool {
        self.muted_raw != 0
    }
}
