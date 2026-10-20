use super::*;
use std::collections::{BTreeMap, BTreeSet};

/// Full-sync name list. Original FUSE emits a list item for every Name,
/// including the selected/current slot. If separate current and stored names
/// both arrive, prefer the stored one; physical duplicate patterns are unverified.
/// Complete zero-based coverage and sync-end are required.
#[derive(Debug)]
pub struct Names {
    pid: u16,
    entries: BTreeMap<u16, String>,
    seen: usize,
    identities: BTreeSet<(u16, bool)>,
    complete: bool,
}
impl Names {
    pub fn new(pid: u16) -> Result<Self> {
        count(pid)?;
        Ok(Self {
            pid,
            entries: BTreeMap::new(),
            seen: 0,
            identities: BTreeSet::new(),
            complete: false,
        })
    }
    pub fn entries(&self) -> &BTreeMap<u16, String> {
        &self.entries
    }
    pub fn is_complete(&self) -> bool {
        self.complete
    }
    pub fn observe(&mut self, report: Report64) -> Result<bool> {
        if self.complete {
            return Err(error("Name list already completed"));
        }
        self.seen += 1;
        if self.seen > 512 {
            return Err(error("Name list exceeded 512 reports"));
        }
        let b = report.as_bytes();
        if b[0] == 0xff {
            if self.entries.len() != count(self.pid)? as usize {
                return Err(error("Name sync ended with an incomplete stored bank"));
            }
            self.complete = true;
            return Ok(true);
        }
        if b[0] == 0x1c && b[2] == 4 && b[3] == 0 {
            let slot = u16::from_le_bytes([b[4], b[5]]);
            check_slot(self.pid, slot)?;
            let name: String = b[16..]
                .iter()
                .take_while(|&&v| v != 0)
                .map(|&v| char::from(v))
                .collect();
            let current = b[6] != 0;
            if !self.identities.insert((slot, current)) {
                return Err(error("Duplicate stored/current slot identity in name sync"));
            }
            if !current || !self.entries.contains_key(&slot) {
                self.entries.insert(slot, name.trim_end_matches(' ').into());
            }
        }
        Ok(false)
    }
}

#[derive(Debug)]
struct Stream {
    pid: u16,
    only: Option<u16>,
    pending: Option<(u16, Vec<Report64>)>,
    entries: BTreeMap<u16, SlotCapture>,
    done: bool,
    started: bool,
    seen: usize,
}
impl Stream {
    fn new(pid: u16, only: Option<u16>) -> Result<Self> {
        count(pid)?;
        if let Some(slot) = only {
            check_slot(pid, slot)?;
        }
        Ok(Self {
            pid,
            only,
            pending: None,
            entries: BTreeMap::new(),
            done: false,
            started: false,
            seen: 0,
        })
    }
    fn observe(&mut self, report: Report64) -> Result<bool> {
        if self.done {
            return Err(error("Stored read already completed"));
        }
        self.seen += 1;
        let expected = self.only.map_or(usize::from(count(self.pid)?), |_| 1);
        if self.seen > expected * 64 + 8 {
            return Err(error("Stored read exceeded its report limit"));
        }
        let b = report.as_bytes();
        if b[0] == 0x14 {
            if b[3] == 0 {
                // Receive policy: original handleBackup ignores non-terminal
                // notifications. Permit one correlated start, without assuming
                // real firmware emits it; never let it replace payload/terminal.
                if self.started
                    || self.pending.is_some()
                    || !self.entries.is_empty()
                    || u16::from(b[2]) != self.only.unwrap_or(0)
                    || b[4] != u8::from(self.only.is_none())
                {
                    return Err(error("Duplicate, late, or uncorrelated backup start"));
                }
                self.started = true;
                return Ok(false);
            }
            if b[3] != 2 {
                return Err(error("Unexpected backup operation in stored read"));
            }
            if self.pending.is_some() || self.entries.len() != expected {
                return Err(error(
                    "Backup ended with a missing or incomplete stored preset",
                ));
            }
            self.done = true;
            return Ok(true);
        }
        if b[0] == 0 {
            return Ok(false);
        } // transport ACKs are owned by worker
        if b[0] == 0x1c && b[2] == 4 && b[3] == 0 {
            if self.pending.is_some() {
                return Err(error("Stored Name arrived before previous Complete"));
            }
            let slot = u16::from_le_bytes([b[4], b[5]]);
            check_slot(self.pid, slot)?;
            if self.only.is_some_and(|wanted| wanted != slot) {
                return Err(error("Stored read returned the wrong slot"));
            }
            if self.entries.contains_key(&slot) {
                return Err(error("Duplicate stored slot in backup"));
            }
            self.pending = Some((slot, vec![report]));
            return Ok(false);
        }
        let (_, reports) = self
            .pending
            .as_mut()
            .ok_or_else(|| error("Stored payload arrived before Name"))?;
        if reports.len() >= 64 {
            return Err(error("Stored preset exceeded 64 reports"));
        }
        reports.push(report);
        if b[0] == 0x1c && b[2] == 0 && b[3] == 0 {
            let (slot, reports) = self.pending.take().expect("pending exists");
            let capture = SlotCapture::new(self.pid, slot, reports)?;
            self.entries.insert(slot, capture);
        }
        Ok(false)
    }
}

/// Collector for a complete, preset-only stream. This is not a parser for the
/// original whole-device backup: knob/system sections intentionally fail. The
/// device worker uses bounded `StoredRead` requests to build a preset archive.
#[derive(Debug)]
pub struct BackupCollector(Stream);
impl BackupCollector {
    pub fn new(pid: u16) -> Result<Self> {
        Ok(Self(Stream::new(pid, None)?))
    }
    pub fn completed_slots(&self) -> usize {
        self.0.entries.len()
    }
    pub fn observe(&mut self, report: Report64) -> Result<Option<Backup>> {
        if !self.0.observe(report)? {
            return Ok(None);
        }
        Ok(Some(Backup::new(
            self.0.pid,
            std::mem::take(&mut self.0.entries).into_values().collect(),
        )?))
    }
}
#[derive(Debug)]
pub struct StoredRead(Stream);
impl StoredRead {
    pub fn new(pid: u16, slot: u16) -> Result<Self> {
        Ok(Self(Stream::new(pid, Some(slot))?))
    }
    pub fn observe(&mut self, report: Report64) -> Result<Option<SlotCapture>> {
        if !self.0.observe(report)? {
            return Ok(None);
        }
        Ok(self.0.entries.pop_first().map(|(_, capture)| capture))
    }
}
