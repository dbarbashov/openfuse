//! Bounded volatile parameter queue. The only emitted command is 0x05.
use crate::{LiveBinding, LiveStatus};
use openfuse_core::{
    messages::ParameterChange, preset::PresetDocument, protocol::Report64, wire_preset,
};
use std::collections::VecDeque;

const CAPACITY: usize = 64;
const ACK_TIMEOUT_MS: u64 = 5_000;
struct Pending {
    change: ParameterChange,
    deadline: u64,
}
#[derive(Default)]
pub(crate) struct LiveQueue {
    binding: Option<LiveBinding>,
    document: Option<PresetDocument>,
    pid: u16,
    queued: VecDeque<ParameterChange>,
    pending: Option<Pending>,
    last_acknowledged: Option<ParameterChange>,
    acknowledged: u64,
    error: Option<String>,
    requires_read: bool,
}
impl LiveQueue {
    pub(crate) fn status(&self) -> LiveStatus {
        LiveStatus {
            enabled: self.binding.is_some(),
            pending: self.queued.len() + usize::from(self.pending.is_some()),
            acknowledged: self.acknowledged,
            error: self.error.clone(),
        }
    }
    pub(crate) fn needs_refresh(&self) -> bool {
        self.requires_read
    }
    pub(crate) fn fresh_read(&mut self) {
        *self = Self::default();
    }
    pub(crate) fn disable(&mut self) {
        self.requires_read |= self.pending.is_some() || !self.queued.is_empty();
        self.binding = None;
        self.document = None;
        self.queued.clear();
        self.pending = None;
    }
    pub(crate) fn fail(&mut self, error: impl Into<String>) {
        self.disable();
        self.requires_read = true;
        self.error = Some(error.into());
    }
    pub(crate) fn enable(
        &mut self,
        binding: LiveBinding,
        pid: u16,
        document: PresetDocument,
    ) -> Result<(), String> {
        if self.requires_read {
            return Err("Read and import a fresh preset before enabling live edits again".into());
        }
        if self.binding.is_some() {
            return Err("Live editing is already enabled".into());
        }
        self.binding = Some(binding);
        self.pid = pid;
        self.document = Some(document);
        self.error = None;
        self.acknowledged = 0;
        self.last_acknowledged = None;
        Ok(())
    }
    pub(crate) fn enqueue(
        &mut self,
        binding: &LiveBinding,
        changes: Vec<ParameterChange>,
    ) -> Result<(), String> {
        if self.binding.as_ref() != Some(binding) {
            return Err("Live editing is disabled or belongs to an older capture".into());
        }
        if changes.is_empty() || changes.len() > CAPACITY {
            return Err("Live edit requires 1 to 64 parameters".into());
        }
        let document = self.document.as_ref().ok_or("No live preset")?;
        let mut next = self.queued.clone();
        for change in changes {
            let key = module_key(change.module_type).ok_or("Unknown live module type")?;
            let mut expected =
                wire_preset::parameter_change(self.pid, document, key, change.parameter_index)
                    .map_err(|e| e.to_string())?;
            expected.wire_value = change.wire_value;
            if expected != change {
                return Err(
                    "Live parameter does not match the imported device model/control/type".into(),
                );
            }
            // Coalesce only unsent values; an in-flight write must receive its own ACK.
            if let Some(slot) = next.iter_mut().find(|old| {
                old.module_type == change.module_type
                    && old.parameter_index == change.parameter_index
            }) {
                *slot = change;
            } else {
                next.push_back(change);
            }
        }
        if next.len() + usize::from(self.pending.is_some()) > CAPACITY {
            return Err("Live queue is full; wait for acknowledgements".into());
        }
        self.queued = next;
        Ok(())
    }
    pub(crate) fn outgoing(&mut self, now: u64) -> Option<Report64> {
        self.tick(now);
        if self.binding.is_none() || self.pending.is_some() {
            return None;
        }
        let change = self.queued.pop_front()?;
        // ACK confirms delivery; it does not update the frozen wire capture.
        self.requires_read = true;
        self.pending = Some(Pending {
            change,
            deadline: now.saturating_add(ACK_TIMEOUT_MS),
        });
        Some(change.encode())
    }
    pub(crate) fn tick(&mut self, now: u64) {
        if self.pending.as_ref().is_some_and(|p| now >= p.deadline) {
            self.fail("Live parameter ACK timed out; delivery is uncertain. Read current before continuing");
        }
    }
    pub(crate) fn on_report(&mut self, report: Report64, now: u64) {
        self.tick(now);
        let b = report.as_bytes();
        if self.binding.is_none() {
            if matches!(
                b[0],
                0x05 | 0x06 | 0x08 | 0x13 | 0x19 | 0x1b | 0x1c | 0x1d | 0x20 | 0xff
            ) {
                self.fail("Amplifier state changed since the captured preset; read current again");
            }
            return;
        }
        if b[0] == 0 && b[2] == 5 {
            if self.pending.is_none() {
                return;
            }
            if b[3] != 0 {
                self.fail(format!(
                    "Live parameter rejected (ACK status {}); read current before continuing",
                    b[3]
                ));
                return;
            }
            self.last_acknowledged = self.pending.take().map(|p| p.change);
            self.acknowledged += 1;
        } else if b[0] == 5 {
            if let openfuse_core::messages::MessageFields::Parameter(change) =
                openfuse_core::messages::decode_inbound(&report)
                && self.pending.as_ref().is_none_or(|p| p.change != change)
                && self.last_acknowledged != Some(change)
            {
                self.fail(
                    "Amplifier parameters changed outside this live edit; read current again",
                );
            }
        } else if matches!(
            b[0],
            0x06 | 0x08 | 0x13 | 0x19 | 0x1b | 0x1c | 0x1d | 0x20 | 0xff
        ) {
            self.fail("Amplifier state changed outside this live edit; read current again");
        }
    }
}
pub(crate) fn module_key(kind: u8) -> Option<&'static str> {
    match kind {
        2 => Some("Amplifier"),
        3 => Some("Stompbox"),
        4 => Some("Modulation"),
        5 => Some("Delay"),
        6 => Some("Reverb"),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
