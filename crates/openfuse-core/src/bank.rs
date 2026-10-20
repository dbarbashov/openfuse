//! Bounded, portable stored-bank captures and evidence-backed bank operations.
//! Raw reports are retained as evidence, never replayed as host commands.
use crate::{audition, preset::PresetDocument, protocol::Report64, wire_preset};
use std::fmt;
mod backup;
pub use backup::{Backup, MAX_BACKUP_BYTES, SlotCapture};
mod selection;
pub use selection::{RestoreEntry, RestoreSelection, SlotMapping};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BankError(String);
impl fmt::Display for BankError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for BankError {}
pub type Result<T> = std::result::Result<T, BankError>;
fn error(message: impl Into<String>) -> BankError {
    BankError(message.into())
}
fn text_error(message: impl fmt::Display) -> BankError {
    error(message.to_string())
}
fn count(pid: u16) -> Result<u16> {
    let count = audition::slot_count(pid).map_err(text_error)?;
    if count == 0 {
        return Err(error("This device has no evidenced stored preset bank"));
    }
    Ok(count)
}
fn check_slot(pid: u16, slot: u16) -> Result<()> {
    if slot >= count(pid)? {
        return Err(error("Stored slot is outside this device's bank"));
    }
    Ok(())
}

mod collect;
pub use collect::{BackupCollector, Names, StoredRead};

/// Full sync requests current state and the stored-name list; no device writes.
pub fn names_request(pid: u16) -> Result<Report64> {
    count(pid)?;
    Ok(crate::messages::sync_request(false))
}
/// Original BackupMessage: operation 0, no requested ACK. None requests the
/// original whole-device dump, which may include knob banks and system settings.
/// The preset-only archive worker must request individual slots sequentially;
/// `BackupCollector` deliberately rejects unsupported sections instead of
/// claiming a complete device backup after silently dropping them.
pub fn backup_request(pid: u16, slot: Option<u16>) -> Result<Report64> {
    if let Some(slot) = slot {
        return stored_request(pid, slot);
    }
    count(pid)?;
    let mut bytes = [0; 64];
    bytes[0] = 0x14;
    bytes[1] = 1;
    bytes[4] = 1;
    Ok(Report64::from_bytes(bytes))
}
/// Read one stored preset without selecting it or replacing working edits.
pub fn stored_request(pid: u16, slot: u16) -> Result<Report64> {
    check_slot(pid, slot)?;
    let mut bytes = [0; 64];
    bytes[0] = 0x14;
    bytes[1] = 1;
    bytes[2] = slot as u8;
    Ok(Report64::from_bytes(bytes))
}

/// Save only a verified working capture, using the original current-save path.
pub fn save_current(pid: u16, current: &[Report64], target: u16) -> Result<audition::Plan> {
    audition::save_current(pid, current, target).map_err(text_error)
}
/// Preset-only restore: audited temporary upload then SaveName/Complete.
/// Global device settings and unrepresented raw fields are not replayed.
pub fn restore_slot(
    pid: u16,
    desired: &PresetDocument,
    current: &[Report64],
    target: u16,
) -> Result<audition::Plan> {
    audition::restore_slot(pid, desired, current, target).map_err(text_error)
}
