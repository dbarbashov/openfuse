//! Exclusive bank operations. No automatic retry or replay of persistent writes.
use crate::{DeviceInfo, OperationPhase};
mod auxiliary;
use openfuse_core::bank::Backup;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct BankStatus {
    pub phase: OperationPhase,
    pub kind: String,
    /// Successfully read/verified entries, never just acknowledged writes.
    pub completed: usize,
    pub total: usize,
    pub detail: String,
}
impl BankStatus {
    pub fn busy(&self) -> bool {
        matches!(
            self.phase,
            OperationPhase::Sending | OperationPhase::Reading
        )
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct BankNames {
    pub device: DeviceInfo,
    pub generation: u64,
    pub entries: BTreeMap<u16, String>,
}
#[derive(Debug, Clone, Serialize)]
pub struct BankBackup {
    pub device: DeviceInfo,
    pub generation: u64,
    pub backup: Backup,
}
pub(crate) enum Request {
    AuxiliaryRead,
    WholeRestore(Box<openfuse_core::auxiliary::WholeRestore>),
    AuxiliaryRestore(Box<openfuse_core::auxiliary::Selection>),
    Names,
    Backup,
    BackupSlots(Vec<u16>),
    Save(u16),
    Restore(Box<Backup>),
    RestoreSelection(Box<bank::RestoreSelection>),
}
impl Request {
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::WholeRestore(_) => "Restore whole device",
            Self::AuxiliaryRead => "Read system and control banks",
            Self::AuxiliaryRestore(_) => "Restore selected control banks",
            Self::Names => "Read bank names",
            Self::Backup => "Back up amplifier bank",
            Self::BackupSlots(_) => "Back up selected slots",
            Self::Save(_) => "Save current sound to slot",
            Self::Restore(_) => "Restore amplifier bank",
            Self::RestoreSelection(_) => "Restore selected slots",
        }
    }
}

use openfuse_core::{audition::Plan, bank, protocol::Report64};
use std::collections::VecDeque;

/// Pure transaction coordinator. Transport writes happen only in Worker.
#[derive(Default)]
pub(crate) struct BankQueue {
    status: BankStatus,
    auxiliary: Option<auxiliary::AuxiliaryQueue>,
    stage: Option<Stage>,
    writer: crate::operation::OperationQueue,
    plans: VecDeque<Plan>,
    active_plan: Option<Plan>,
    outgoing: Option<Report64>,
    deadline: Option<u64>,
    reset_transport: bool,
    sent_any: bool,
    pid: u16,
    captured: Vec<bank::SlotCapture>,
    remaining_reads: VecDeque<u16>,
    pub(crate) names: Option<BTreeMap<u16, String>>,
    pub(crate) backup: Option<Backup>,
}
enum Stage {
    Names(bank::Names),
    Backup(bank::StoredRead),
    Sending,
    Verify(bank::StoredRead),
}
impl BankQueue {
    pub(crate) fn status(&self) -> &BankStatus {
        self.auxiliary.as_ref().map_or(&self.status, |a| &a.status)
    }
    pub(crate) fn reject(&mut self, kind: &str, error: impl Into<String>) {
        self.auxiliary = None;
        self.status = BankStatus {
            phase: OperationPhase::Failed,
            kind: kind.into(),
            detail: error.into(),
            ..Default::default()
        };
    }
    pub(crate) fn begin(
        &mut self,
        pid: u16,
        request: Request,
        current: &[Report64],
        now: u64,
    ) -> Result<(), String> {
        if self.status().busy() {
            return Err("Another bank operation is active".into());
        }
        self.auxiliary = None;
        self.names = None;
        self.backup = None;
        let request = match request {
            Request::WholeRestore(plan) => {
                self.auxiliary = Some(auxiliary::AuxiliaryQueue::whole(pid, *plan, now)?);
                return Ok(());
            }
            Request::AuxiliaryRead => {
                self.auxiliary = Some(auxiliary::AuxiliaryQueue::new(pid, None, now)?);
                return Ok(());
            }
            Request::AuxiliaryRestore(selection) => {
                self.auxiliary = Some(auxiliary::AuxiliaryQueue::new(pid, Some(*selection), now)?);
                return Ok(());
            }
            request => request,
        };
        let count = openfuse_core::audition::slot_count(pid).map_err(|e| e.to_string())?;
        if count == 0 {
            return Err("This device has no full-preset slot bank".into());
        }
        let kind = request.label().to_owned();
        let request = match request {
            Request::Backup => Request::BackupSlots((0..count).collect()),
            other => other,
        };
        let mut plans = VecDeque::new();
        let mut remaining_reads = VecDeque::new();
        let (stage, outgoing, total) = match request {
            Request::WholeRestore(_) | Request::AuxiliaryRead | Request::AuxiliaryRestore(_) => {
                unreachable!("handled above")
            }
            Request::Names => (
                Some(Stage::Names(
                    bank::Names::new(pid).map_err(|e| e.to_string())?,
                )),
                Some(bank::names_request(pid).map_err(|e| e.to_string())?),
                usize::from(count),
            ),
            Request::Backup => unreachable!("whole bank expanded above"),
            Request::BackupSlots(mut slots) => {
                slots.sort_unstable();
                if slots.is_empty()
                    || slots.len() > usize::from(count)
                    || slots.iter().any(|s| *s >= count)
                    || slots.windows(2).any(|pair| pair[0] == pair[1])
                {
                    return Err("Select distinct stored slots within this device's bank".into());
                }
                let total = slots.len();
                remaining_reads = slots.into();
                let slot = remaining_reads.pop_front().unwrap();
                (
                    Some(Stage::Backup(
                        bank::StoredRead::new(pid, slot).map_err(|e| e.to_string())?,
                    )),
                    Some(bank::stored_request(pid, slot).map_err(|e| e.to_string())?),
                    total,
                )
            }
            Request::Save(slot) => {
                plans.push_back(bank::save_current(pid, current, slot).map_err(|e| e.to_string())?);
                (Some(Stage::Sending), None, 1)
            }
            Request::Restore(backup) => {
                backup.validate_for_pid(pid).map_err(|e| e.to_string())?;
                if !backup.is_complete() {
                    return Err("Partial archives require an explicit restore selection".into());
                }
                // Validate every slot before dispatching even the first persistent write.
                for entry in backup.slots() {
                    plans.push_back(
                        bank::restore_slot(pid, entry.document(), current, entry.slot())
                            .map_err(|e| e.to_string())?,
                    );
                }
                (Some(Stage::Sending), None, plans.len())
            }
            Request::RestoreSelection(selection) => {
                selection.validate_for_pid(pid).map_err(|e| e.to_string())?;
                plans = selection.plans(current).map_err(|e| e.to_string())?.into();
                (Some(Stage::Sending), None, plans.len())
            }
        };
        self.status = BankStatus {
            phase: if plans.is_empty() {
                OperationPhase::Reading
            } else {
                OperationPhase::Sending
            },
            kind,
            completed: 0,
            total,
            detail: "Operation prepared; each stored slot is checked before continuing".into(),
        };
        self.stage = stage;
        self.outgoing = outgoing;
        self.plans = plans;
        self.pid = pid;
        self.writer = Default::default();
        self.active_plan = None;
        self.captured.clear();
        self.remaining_reads = remaining_reads;
        self.names = None;
        self.backup = None;
        self.reset_transport = false;
        self.sent_any = false;
        self.deadline = Some(now.saturating_add(120_000));
        if matches!(self.stage, Some(Stage::Sending)) {
            self.next_plan(now)?;
        }
        Ok(())
    }
    fn next_plan(&mut self, now: u64) -> Result<(), String> {
        if let Some(plan) = self.plans.pop_front() {
            self.writer = Default::default();
            self.writer.begin(plan.clone(), now)?;
            self.active_plan = Some(plan);
            self.stage = Some(Stage::Sending);
            self.status.phase = OperationPhase::Sending;
            self.status.detail = format!(
                "Writing slot {}; {} of {} verified",
                self.active_plan.as_ref().unwrap().expected_slot(),
                self.status.completed,
                self.status.total
            );
            self.deadline = Some(now.saturating_add(120_000));
        } else {
            self.complete("All requested slots were read back and matched; hardware durability remains unverified");
        }
        Ok(())
    }
    fn complete(&mut self, detail: &str) {
        self.status.phase = OperationPhase::Verified;
        self.status.completed = self.status.total;
        self.status.detail = detail.into();
        self.stage = None;
        self.deadline = None;
    }
    pub(crate) fn fail(&mut self, detail: impl Into<String>) {
        if let Some(a) = &mut self.auxiliary {
            a.fail(detail);
            return;
        }
        if !self.status.busy() {
            return;
        }
        self.reset_transport |= self.sent_any;
        self.writer.cancel();
        self.plans.clear();
        self.remaining_reads.clear();
        self.active_plan = None;
        self.stage = None;
        self.outgoing = None;
        self.deadline = None;
        self.status.phase = OperationPhase::Failed;
        self.status.detail = format!(
            "{}; {} of {} slots verified. Delivered writes are not rolled back or replayed",
            detail.into(),
            self.status.completed,
            self.status.total
        );
    }
    pub(crate) fn cancel(&mut self) {
        if let Some(a) = &mut self.auxiliary {
            a.cancel();
            return;
        }
        if self.status.busy() {
            self.fail("Cancelled unsent work");
            self.status.phase = OperationPhase::Cancelled;
        }
    }
    pub(crate) fn cancelled_before_start(&mut self) {
        if !self.status().busy() {
            self.auxiliary = None;
            self.status = BankStatus {
                phase: OperationPhase::Cancelled,
                detail: "Cancelled before transmission".into(),
                ..Default::default()
            };
        }
    }
    pub(crate) fn take_transport_reset(&mut self) -> bool {
        if let Some(a) = &mut self.auxiliary {
            return a.take_reset();
        }
        std::mem::take(&mut self.reset_transport)
    }
    pub(crate) fn tick(&mut self, now: u64) {
        if let Some(a) = &mut self.auxiliary {
            a.tick(now);
            return;
        }
        if self.deadline.is_some_and(|t| now >= t) {
            self.fail("Bank operation response timed out");
        }
        if matches!(self.stage, Some(Stage::Sending)) {
            self.writer.tick(now);
            if self.writer.status().phase == OperationPhase::Failed {
                self.fail(self.writer.status().detail.clone());
            }
        }
    }
    pub(crate) fn outgoing(&mut self, now: u64) -> Option<Report64> {
        if let Some(a) = &mut self.auxiliary {
            return a.outgoing(now);
        }
        self.tick(now);
        if !self.status.busy() {
            return None;
        }
        let report = if matches!(self.stage, Some(Stage::Sending)) {
            self.writer.outgoing(now)
        } else {
            self.outgoing.take()
        };
        if report.is_some() {
            self.sent_any = true;
        }
        report
    }
    pub(crate) fn take_auxiliary(&mut self) -> Option<openfuse_core::auxiliary::Snapshot> {
        self.auxiliary.as_mut().and_then(|a| a.snapshot.take())
    }
    pub(crate) fn clear_results(&mut self) {
        self.names = None;
        self.backup = None;
        if let Some(a) = &mut self.auxiliary {
            a.snapshot = None;
        }
    }
    pub(crate) fn on_report(&mut self, report: Report64, now: u64) {
        if let Some(a) = &mut self.auxiliary {
            a.on_report(report, now);
            return;
        }
        self.tick(now);
        if !self.status.busy() || self.outgoing.is_some() {
            return;
        }
        if let Err(error) = self.receive(report, now) {
            self.fail(error);
        }
    }
    fn receive(&mut self, report: Report64, now: u64) -> Result<(), String> {
        // A physical control movement invalidates a frozen write source or bank stream.
        if matches!(
            report.message_id(),
            5 | 6 | 8 | 0x13 | 0x19 | 0x1b | 0x1d | 0x20
        ) {
            return Err("Amplifier changed during the bank operation".into());
        }
        match self.stage.as_mut() {
            Some(Stage::Sending) => {
                self.writer.on_report(report, now);
                if self.writer.status().phase == OperationPhase::Failed {
                    return Err(self.writer.status().detail.clone());
                }
                if self.writer.needs_read() {
                    let slot = self
                        .active_plan
                        .as_ref()
                        .ok_or("Missing save expectation")?
                        .expected_slot();
                    self.outgoing = Some(
                        bank::backup_request(self.pid, Some(slot)).map_err(|e| e.to_string())?,
                    );
                    self.stage = Some(Stage::Verify(
                        bank::StoredRead::new(self.pid, slot).map_err(|e| e.to_string())?,
                    ));
                    self.status.phase = OperationPhase::Reading;
                    self.status.detail =
                        format!("Reading stored slot {slot} to verify persistent save");
                    self.deadline = Some(now.saturating_add(15_000));
                }
            }
            Some(Stage::Names(names)) => {
                names.observe(report).map_err(|e| e.to_string())?;
                if names.is_complete() {
                    self.names = Some(names.entries().clone());
                    self.complete("All stored slot names read");
                }
            }
            Some(Stage::Backup(collector)) => {
                if let Some(capture) = collector.observe(report).map_err(|e| e.to_string())? {
                    self.captured.push(capture);
                    self.status.completed = self.captured.len();
                    if self.status.completed == self.status.total {
                        self.backup = Some(
                            Backup::selected(self.pid, std::mem::take(&mut self.captured))
                                .map_err(|e| e.to_string())?,
                        );
                        self.complete("All requested presets captured; working sound was not selected or saved");
                    } else {
                        let slot = self
                            .remaining_reads
                            .pop_front()
                            .ok_or("Missing selected backup slot")?;
                        self.stage = Some(Stage::Backup(
                            bank::StoredRead::new(self.pid, slot).map_err(|e| e.to_string())?,
                        ));
                        self.outgoing = Some(
                            bank::backup_request(self.pid, Some(slot))
                                .map_err(|e| e.to_string())?,
                        );
                        self.deadline = Some(now.saturating_add(15_000));
                        self.status.detail = format!(
                            "Reading stored slot {slot}; {} of {} captured",
                            self.status.completed, self.status.total
                        );
                    }
                }
            }
            Some(Stage::Verify(collector)) => {
                if let Some(capture) = collector.observe(report).map_err(|e| e.to_string())? {
                    self.active_plan
                        .as_ref()
                        .ok_or("Missing save expectation")?
                        .matches_stored_readback(capture.raw_reports())
                        .map_err(|e| e.to_string())?;
                    self.status.completed += 1;
                    self.next_plan(now)?;
                }
            }
            None => {}
        }
        Ok(())
    }
}
