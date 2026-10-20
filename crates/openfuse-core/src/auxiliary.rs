//! System and physical-knob bank preservation.
//! Recognition, source-code execution and simulated transport are not hardware validation.
use crate::{bank, protocol::Report64, wire_preset};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt};
mod codec;
mod collect;
mod encode;
mod selection;
mod storage;
mod whole;
pub use codec::decode_system_xml;
pub use collect::{Collector, read_request};
pub use encode::plan_xml;
pub use selection::Selection;
pub use whole::{Normalization, RestoreStep, WholeRestore};
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuxiliaryError(String);
impl fmt::Display for AuxiliaryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for AuxiliaryError {}
pub type Result<T> = std::result::Result<T, AuxiliaryError>;
fn error(value: impl fmt::Display) -> AuxiliaryError {
    AuxiliaryError(value.to_string())
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Key {
    System,
    Bank { control: u8, slot: u16 },
}
impl Key {
    pub fn same_kind(self, other: Self) -> bool {
        match (self, other) {
            (Self::System, Self::System) => true,
            (Self::Bank { control: a, .. }, Self::Bank { control: b, .. }) => a == b,
            _ => false,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mapping {
    pub source: Key,
    pub target: Key,
}
#[derive(Debug, Clone)]
pub struct Entry {
    key: Key,
    label: String,
    xml: Option<String>,
    raw: Vec<Report64>,
    restore_error: Option<String>,
}
impl Entry {
    pub fn key(&self) -> Key {
        self.key
    }
    pub fn label(&self) -> String {
        self.label.clone()
    }
    pub fn xml(&self) -> Option<&str> {
        self.xml.as_deref()
    }
    pub fn raw_reports(&self) -> &[Report64] {
        &self.raw
    }
    pub fn restore_error(&self) -> Option<&str> {
        self.restore_error.as_deref()
    }
}
#[derive(Debug, Clone)]
pub struct Snapshot {
    pid: u16,
    entries: Vec<Entry>,
    presets: Vec<bank::SlotCapture>,
    raw: Vec<Report64>,
    warnings: Vec<String>,
    legacy: bool,
}
impl Snapshot {
    pub fn from_reports(pid: u16, reports: &[Report64]) -> Result<Self> {
        let mut collector = Collector::new(pid)?;
        let mut result = None;
        for report in reports {
            if result.is_some() {
                return Err(error("Reports follow the full backup terminal"));
            }
            result = collector.observe(*report)?;
        }
        result.ok_or_else(|| error("Incomplete full backup"))
    }
    pub fn pid(&self) -> u16 {
        self.pid
    }
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }
    pub fn reports(&self) -> &[Report64] {
        &self.raw
    }
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }
    pub fn is_legacy(&self) -> bool {
        self.legacy
    }
    pub fn presets(&self) -> &[bank::SlotCapture] {
        &self.presets
    }
    pub fn fingerprint(&self) -> String {
        storage::fingerprint(self)
    }
    pub fn to_json(&self) -> Result<String> {
        storage::serialize(self)
    }
    pub fn parse(source: &str) -> Result<Self> {
        storage::parse(source)
    }
    pub fn from_legacy(pid: u16, legacy: &crate::legacy_backup::LegacyBackup) -> Result<Self> {
        codec::legacy(pid, legacy)
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capabilities {
    pub system: bool,
    pub banks: BTreeMap<u8, u16>,
}
pub fn capabilities(pid: u16) -> Result<Capabilities> {
    wire_preset::product_for_pid(pid).ok_or_else(|| error("Unknown auxiliary device profile"))?;
    let banks = match pid {
        4 | 0x14 => [(1, 12), (2, 12)].into(),
        0x10 | 0xa | 0x26 => [(2, 12)].into(),
        0xe => [(1, 17), (2, 15)].into(),
        _ => BTreeMap::new(),
    };
    Ok(Capabilities {
        system: pid != 0xe,
        banks,
    })
}
fn kinds(pid: u16, control: u8) -> Option<&'static [u8]> {
    match (pid, control) {
        (4 | 0x14, 1) => Some(&[7]),
        (4 | 0x14, 2) => Some(&[8, 9]),
        (0x10 | 0xa | 0x26, 2) => Some(&[7, 8, 9]),
        (0xe, 1) => Some(&[5, 6, 13]),
        (0xe, 2) => Some(&[7, 8, 9, 10]),
        _ => None,
    }
}
fn bank_label(pid: u16, control: u8) -> &'static str {
    match (pid, control) {
        (4 | 0x14, 1) => "MOD",
        (4 | 0x14, 2) => "DLY/RVB",
        (0xe, 1) => "VOICE",
        (_, 2) => "FX",
        _ => "Unknown bank",
    }
}
