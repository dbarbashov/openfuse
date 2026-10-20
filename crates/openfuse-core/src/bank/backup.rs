use super::*;
use crate::hex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const MAX_BACKUP_BYTES: usize = 4 * 1024 * 1024;
const MAX_SLOT_REPORTS: usize = 64;
const MAX_SLOT_XML_BYTES: usize = 256 * 1024;
const FORMAT: &str = "confuse-bank";
const VERSION: u32 = 1;
const SELECTED_VERSION: u32 = 2;

/// Exact stored-slot replies and their independent editable XML interpretation.
/// Fields are private so serialization cannot bypass shape/schema validation.
#[derive(Debug, Clone)]
pub struct SlotCapture {
    pid: u16,
    slot: u16,
    reports: Vec<Report64>,
    document: PresetDocument,
}
impl SlotCapture {
    pub fn new(pid: u16, slot: u16, reports: Vec<Report64>) -> Result<Self> {
        check_slot(pid, slot)?;
        if reports.is_empty() || reports.len() > MAX_SLOT_REPORTS {
            return Err(error("Stored preset must contain 1..64 reports"));
        }
        let names: Vec<_> = reports
            .iter()
            .filter(|r| {
                let b = r.as_bytes();
                b[0] == 0x1c && b[2] == 4 && b[3] == 0
            })
            .collect();
        if names.len() != 1 {
            return Err(error("Stored capture requires exactly one Name report"));
        }
        let name = names[0].as_bytes();
        if u16::from_le_bytes([name[4], name[5]]) != slot {
            return Err(error("Stored capture Name slot differs from its entry"));
        }
        // The common XML decoder accepts current working reports. Normalize
        // only the stored/current marker in a copy; preserve all input bytes.
        let normalized: Vec<_> = reports
            .iter()
            .map(|report| {
                let mut b = *report.as_bytes();
                if b[0] == 0x1c && b[2] == 4 && b[3] == 0 {
                    b[6] = 1;
                }
                Report64::from_bytes(b)
            })
            .collect();
        let document = wire_preset::decode(pid, &normalized)
            .map_err(text_error)?
            .document;
        Ok(Self {
            pid,
            slot,
            reports,
            document,
        })
    }
    pub fn pid(&self) -> u16 {
        self.pid
    }
    pub fn slot(&self) -> u16 {
        self.slot
    }
    pub fn name(&self) -> String {
        self.document.metadata().name
    }
    pub fn raw_reports(&self) -> &[Report64] {
        &self.reports
    }
    pub fn document(&self) -> &PresetDocument {
        &self.document
    }
}

/// Complete v1 banks and explicitly selected v2 preset subsets. Invalid or duplicate slots and cross-device restores fail
/// before a worker can start a write. SHA-256 detects corruption, not authenticity.
#[derive(Debug, Clone)]
pub struct Backup {
    pid: u16,
    family: u32,
    slots: Vec<SlotCapture>,
}
impl Backup {
    pub fn new(pid: u16, slots: Vec<SlotCapture>) -> Result<Self> {
        if slots.len() != usize::from(count(pid)?) {
            return Err(error(format!(
                "Complete bank requires {} slots",
                count(pid)?
            )));
        }
        Self::selected(pid, slots)
    }
    /// A nonempty set of stored presets; slot numbers keep their source meaning.
    pub fn selected(pid: u16, mut slots: Vec<SlotCapture>) -> Result<Self> {
        let capacity = count(pid)?;
        let family =
            wire_preset::product_for_pid(pid).ok_or_else(|| error("Unknown device family"))?;
        if slots.is_empty() || slots.len() > usize::from(capacity) {
            return Err(error(
                "Selected archive must contain 1..bank-capacity slots",
            ));
        }
        slots.sort_by_key(SlotCapture::slot);
        let mut previous = None;
        for slot in &slots {
            if slot.pid != pid
                || slot.slot >= capacity
                || previous == Some(slot.slot)
                || slot.document.product_id() != Some(family)
            {
                return Err(error(
                    "Bank contains a duplicate, out-of-range, or wrong-device slot",
                ));
            }
            previous = Some(slot.slot);
        }
        Ok(Self { pid, family, slots })
    }
    pub fn is_complete(&self) -> bool {
        self.slots.len() == usize::from(count(self.pid).expect("validated PID"))
    }
    pub fn slot(&self, slot: u16) -> Option<&SlotCapture> {
        self.slots.iter().find(|entry| entry.slot == slot)
    }
    pub fn selection(&self, slots: &[u16]) -> Result<Self> {
        Self::selected(
            self.pid,
            slots
                .iter()
                .map(|slot| {
                    self.slot(*slot)
                        .cloned()
                        .ok_or_else(|| error("Requested source slot is absent from this archive"))
                })
                .collect::<Result<_>>()?,
        )
    }
    pub fn pid(&self) -> u16 {
        self.pid
    }
    pub fn family(&self) -> u32 {
        self.family
    }
    pub fn slots(&self) -> &[SlotCapture] {
        &self.slots
    }
    pub fn validate_for_pid(&self, pid: u16) -> Result<()> {
        if self.pid != pid {
            return Err(error("Backup PID must exactly match the selected device"));
        }
        Ok(())
    }
    pub fn to_json(&self) -> Result<String> {
        let slots = self
            .slots
            .iter()
            .map(Entry::from_capture)
            .collect::<Result<Vec<_>>>()?;
        let mut file = File {
            format: FORMAT.into(),
            version: if self.is_complete() {
                VERSION
            } else {
                SELECTED_VERSION
            },
            pid: self.pid,
            family: self.family,
            slot_count: count(self.pid)?,
            slots,
            sha256: String::new(),
        };
        file.sha256 = file.checksum()?;
        let json = serde_json::to_string_pretty(&file).map_err(text_error)?;
        if json.len() > MAX_BACKUP_BYTES {
            return Err(error("Bank backup exceeds the 4 MiB limit"));
        }
        Ok(json)
    }
    pub fn parse(json: &str) -> Result<Self> {
        if json.len() > MAX_BACKUP_BYTES {
            return Err(error("Bank backup exceeds the 4 MiB limit"));
        }
        let file: File = serde_json::from_str(json).map_err(text_error)?;
        if file.format != FORMAT || !matches!(file.version, VERSION | SELECTED_VERSION) {
            return Err(error("Unsupported bank backup format/version"));
        }
        if file.slot_count != count(file.pid)?
            || Some(file.family) != wire_preset::product_for_pid(file.pid)
        {
            return Err(error(
                "Bank backup family or slot count disagrees with its PID",
            ));
        }
        if file.version == VERSION && file.slots.len() != usize::from(file.slot_count) {
            return Err(error("Incomplete bank backup"));
        }
        if file.sha256 != file.checksum()? {
            return Err(error("Bank manifest checksum mismatch"));
        }
        let pid = file.pid;
        Self::selected(
            pid,
            file.slots
                .into_iter()
                .map(|slot| slot.into_capture(pid))
                .collect::<Result<_>>()?,
        )
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    format: String,
    version: u32,
    pid: u16,
    family: u32,
    slot_count: u16,
    slots: Vec<Entry>,
    sha256: String,
}
impl File {
    fn checksum(&self) -> Result<String> {
        let encoded = serde_json::to_vec(&(
            FORMAT,
            self.version,
            self.pid,
            self.family,
            self.slot_count,
            &self.slots,
        ))
        .map_err(text_error)?;
        Ok(format!("{:x}", Sha256::digest(encoded)))
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    slot: u16,
    raw_reports: Vec<String>,
    xml: String,
    sha256: String,
}
impl Entry {
    fn from_capture(capture: &SlotCapture) -> Result<Self> {
        let mut result = Self {
            slot: capture.slot,
            raw_reports: capture
                .reports
                .iter()
                .map(|r| hex::encode(r.as_bytes()))
                .collect(),
            xml: capture.document.to_xml().map_err(text_error)?,
            sha256: String::new(),
        };
        result.sha256 = result.checksum()?;
        Ok(result)
    }
    fn checksum(&self) -> Result<String> {
        let encoded =
            serde_json::to_vec(&(self.slot, &self.raw_reports, &self.xml)).map_err(text_error)?;
        Ok(format!("{:x}", Sha256::digest(encoded)))
    }
    fn into_capture(self, pid: u16) -> Result<SlotCapture> {
        if self.raw_reports.is_empty()
            || self.raw_reports.len() > MAX_SLOT_REPORTS
            || self.xml.len() > MAX_SLOT_XML_BYTES
        {
            return Err(error("Bank slot exceeds report/XML size limits"));
        }
        if self.sha256 != self.checksum()? {
            return Err(error("Bank slot checksum mismatch"));
        }
        let reports = self
            .raw_reports
            .iter()
            .map(|raw| {
                if raw.len() > 191 {
                    return Err(error("Bank report exceeds canonical 64-byte hex length"));
                }
                let bytes = hex::decode(raw).map_err(text_error)?;
                Report64::try_from(bytes.as_slice()).map_err(text_error)
            })
            .collect::<Result<Vec<_>>>()?;
        let capture = SlotCapture::new(pid, self.slot, reports)?;
        if capture.document.to_xml().map_err(text_error)? != self.xml {
            return Err(error("Bank XML disagrees with its raw reports"));
        }
        Ok(capture)
    }
}

impl Serialize for Backup {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        let text = self.to_json().map_err(serde::ser::Error::custom)?;
        let value: serde_json::Value =
            serde_json::from_str(&text).map_err(serde::ser::Error::custom)?;
        value.serialize(serializer)
    }
}
