use super::*;
use std::collections::BTreeSet;
pub const MAX_REPORTS: usize = 4096;
pub fn read_request(pid: u16) -> Result<Report64> {
    capabilities(pid)?;
    let mut b = [0; 64];
    b[0] = 0x14;
    b[1] = 1;
    b[4] = 1;
    Ok(Report64::from_bytes(b))
}
#[derive(Debug)]
pub struct Collector {
    pid: u16,
    raw: Vec<Report64>,
    entries: BTreeMap<Key, Entry>,
    presets: BTreeMap<u16, bank::SlotCapture>,
    pending: Option<(Key, Vec<Report64>)>,
    done: bool,
    started: bool,
    warnings: Vec<String>,
}
impl Collector {
    pub fn new(pid: u16) -> Result<Self> {
        capabilities(pid)?;
        Ok(Self {
            pid,
            raw: vec![],
            entries: BTreeMap::new(),
            presets: BTreeMap::new(),
            pending: None,
            done: false,
            started: false,
            warnings: vec![],
        })
    }
    pub fn observe(&mut self, report: Report64) -> Result<Option<Snapshot>> {
        if self.done {
            return Err(error("Full backup already ended"));
        }
        if self.raw.len() >= MAX_REPORTS {
            return Err(error("Full backup exceeds 4096 reports"));
        }
        self.raw.push(report);
        let b = report.as_bytes();
        if b[0] == 0 {
            return Ok(None);
        }
        if b[0] == 0x14 {
            match b[3] {
                0 if !self.started
                    && self.pending.is_none()
                    && self.entries.is_empty()
                    && self.presets.is_empty() =>
                {
                    self.started = true;
                    return Ok(None);
                }
                2 => {
                    if self.pending.is_some() {
                        return Err(error("Backup ended inside a bank record"));
                    }
                    let caps = capabilities(self.pid)?;
                    let count = crate::audition::slot_count(self.pid).map_err(error)?;
                    if self.presets.len() != usize::from(count) {
                        return Err(error("Full backup is missing stored presets"));
                    }
                    if caps.system && !self.entries.contains_key(&Key::System) {
                        return Err(error("Full backup is missing system settings"));
                    }
                    for (control, count) in caps.banks {
                        for slot in 0..count {
                            if !self.entries.contains_key(&Key::Bank { control, slot }) {
                                return Err(error(format!(
                                    "Full backup is missing bank {control} slot {slot}"
                                )));
                            }
                        }
                    }
                    self.done = true;
                    return Ok(Some(Snapshot {
                        pid: self.pid,
                        entries: std::mem::take(&mut self.entries).into_values().collect(),
                        presets: std::mem::take(&mut self.presets).into_values().collect(),
                        raw: std::mem::take(&mut self.raw),
                        warnings: std::mem::take(&mut self.warnings),
                        legacy: false,
                    }));
                }
                _ => return Err(error("Unexpected, repeated or late backup boundary")),
            }
        }
        if b[0] != 0x1c {
            return Err(error("Unexpected message inside full backup"));
        }
        if b[2] == 11 && b[3] == 0 {
            if self.pending.is_some() || self.entries.contains_key(&Key::System) {
                return Err(error("Interleaved or duplicate system settings"));
            }
            let entry = codec::entry(self.pid, Key::System, vec![report]);
            self.entries.insert(Key::System, entry);
            return Ok(None);
        }
        if b[2] == 4 {
            if self.pending.is_some() {
                return Err(error("Bank Name arrived before previous Complete"));
            }
            let slot = u16::from_le_bytes([b[4], b[5]]);
            let key = Key::Bank {
                control: b[3],
                slot,
            };
            if b[3] == 0 {
                if slot >= crate::audition::slot_count(self.pid).map_err(error)?
                    || self.presets.contains_key(&slot)
                {
                    return Err(error("Invalid or duplicate stored preset slot"));
                }
            } else if self.entries.contains_key(&key) {
                return Err(error("Duplicate auxiliary slot"));
            }
            self.pending = Some((key, vec![report]));
            return Ok(None);
        }
        let (key, raw) = self
            .pending
            .as_mut()
            .ok_or_else(|| error("Bank payload arrived before Name"))?;
        let Key::Bank { control, slot } = *key else {
            unreachable!()
        };
        if b[3] != control {
            return Err(error("Interleaved bank control identities"));
        }
        if raw.len() >= 64 {
            return Err(error("Bank record exceeds 64 reports"));
        }
        if control != 0 && b[2] != 0 && u16::from_le_bytes([b[4], b[5]]) != slot {
            return Err(error("Bank payload slot differs from Name"));
        }
        raw.push(report);
        if b[2] != 0 {
            return Ok(None);
        }
        let (key, raw) = self.pending.take().unwrap();
        if control == 0 {
            self.presets.insert(
                slot,
                bank::SlotCapture::new(self.pid, slot, raw).map_err(error)?,
            );
        } else {
            if let Some(required) = kinds(self.pid, control) {
                let mut seen = BTreeSet::new();
                for r in &raw[1..raw.len() - 1] {
                    if !seen.insert(r.as_bytes()[2]) {
                        return Err(error("Duplicate module in auxiliary bank record"));
                    }
                }
                for kind in required {
                    if self.pid == 0xe && control == 1 && slot == 16 && *kind == 6 {
                        continue;
                    }
                    if !seen.contains(kind) {
                        return Err(error("Auxiliary bank record is missing required module"));
                    }
                }
            }
            let entry = codec::entry(self.pid, key, raw);
            if entry.restore_error.is_some() {
                self.warnings.push(format!(
                    "{}: {}",
                    entry.label,
                    entry.restore_error.as_deref().unwrap()
                ));
            }
            self.entries.insert(key, entry);
        }
        Ok(None)
    }
}
