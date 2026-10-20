//! One frozen auxiliary selection: re-read destination, write, then re-read all banks.
use super::BankStatus;
use crate::OperationPhase;
use openfuse_core::{
    auxiliary::{self, Collector, RestoreStep, Selection, Snapshot, WholeRestore},
    protocol::Report64,
};
use std::collections::VecDeque;

pub(super) struct AuxiliaryQueue {
    pub status: BankStatus,
    pub snapshot: Option<Snapshot>,
    stage: Stage,
    selection: Option<Restore>,
    remaining: VecDeque<RestoreStep>,
    request: Option<Report64>,
    pending: Option<(u8, u64)>,
    last_sent: Option<Report64>,
    deadline: u64,
    sent_any: bool,
    reset: bool,
    pid: u16,
}
enum Stage {
    Read(Collector),
    Preflight(Collector),
    Sending,
    Verify(Collector),
    Done,
}
enum Restore {
    Selected(Selection),
    Whole(WholeRestore),
}
impl Restore {
    fn pid(&self) -> u16 {
        match self {
            Self::Selected(p) => p.pid(),
            Self::Whole(p) => p.pid(),
        }
    }
    fn len(&self) -> usize {
        match self {
            Self::Selected(p) => p.len(),
            Self::Whole(p) => p.preset_count() + p.bank_count() + usize::from(p.has_system()),
        }
    }
    fn steps(&self) -> Vec<RestoreStep> {
        match self {
            Self::Selected(p) => p
                .steps()
                .into_iter()
                .map(|report| RestoreStep {
                    ack_timeout_ms: if report.as_bytes()[1] & 2 != 0 {
                        5000
                    } else {
                        0
                    },
                    report,
                    label: String::new(),
                })
                .collect(),
            Self::Whole(p) => p.steps().to_vec(),
        }
    }
    fn validate_destination(&self, s: &Snapshot) -> Result<(), String> {
        match self {
            Self::Selected(p) => p.validate_destination(s),
            Self::Whole(p) => p.validate_destination(s),
        }
        .map_err(|e| e.to_string())
    }
    fn matches_readback(&self, s: &Snapshot) -> Result<(), String> {
        match self {
            Self::Selected(p) => p.matches_readback(s),
            Self::Whole(p) => p.matches_readback(s),
        }
        .map_err(|e| e.to_string())
    }
}
impl AuxiliaryQueue {
    pub fn new(pid: u16, selection: Option<Selection>, now: u64) -> Result<Self, String> {
        Self::with_restore(pid, selection.map(Restore::Selected), now)
    }
    pub fn whole(pid: u16, plan: WholeRestore, now: u64) -> Result<Self, String> {
        Self::with_restore(pid, Some(Restore::Whole(plan)), now)
    }
    fn with_restore(pid: u16, selection: Option<Restore>, now: u64) -> Result<Self, String> {
        if selection.as_ref().is_some_and(|s| s.pid() != pid) {
            return Err("Auxiliary selection PID does not match the connected amplifier".into());
        }
        let collector = Collector::new(pid).map_err(|e| e.to_string())?;
        let request = auxiliary::read_request(pid).map_err(|e| e.to_string())?;
        let total = selection.as_ref().map_or(0, Restore::len);
        let restoring = selection.is_some();
        Ok(Self {
            status: BankStatus {
                phase: OperationPhase::Reading,
                kind: if matches!(selection, Some(Restore::Whole(_))) {
                    "Restore whole device"
                } else if restoring {
                    "Restore selected control banks"
                } else {
                    "Read system and control banks"
                }
                .into(),
                completed: 0,
                total,
                detail: if restoring {
                    "Re-reading destination before any write"
                } else {
                    "Reading complete device backup stream"
                }
                .into(),
            },
            snapshot: None,
            stage: if restoring {
                Stage::Preflight(collector)
            } else {
                Stage::Read(collector)
            },
            selection,
            remaining: VecDeque::new(),
            request: Some(request),
            pending: None,
            last_sent: None,
            deadline: now.saturating_add(120_000),
            sent_any: false,
            reset: false,
            pid,
        })
    }
    pub fn fail(&mut self, error: impl Into<String>) {
        if !self.status.busy() {
            return;
        }
        self.reset |= self.sent_any;
        self.stage = Stage::Done;
        self.remaining.clear();
        self.request = None;
        self.pending = None;
        self.selection = None;
        self.status.phase = OperationPhase::Failed;
        self.status.detail = format!(
            "{}; delivered writes are not rolled back or replayed",
            error.into()
        );
    }
    pub fn cancel(&mut self) {
        if self.status.busy() {
            self.fail("Cancelled unsent auxiliary work");
            self.status.phase = OperationPhase::Cancelled;
        }
    }
    pub fn take_reset(&mut self) -> bool {
        std::mem::take(&mut self.reset)
    }
    pub fn tick(&mut self, now: u64) {
        if self.status.busy()
            && (now >= self.deadline || self.pending.is_some_and(|(_, t)| now >= t))
        {
            self.fail("Auxiliary response timed out; delivery is uncertain");
        }
    }
    pub fn outgoing(&mut self, now: u64) -> Option<Report64> {
        self.tick(now);
        if !self.status.busy() || self.pending.is_some() {
            return None;
        }
        let (report, timeout) = if matches!(self.stage, Stage::Sending) {
            let next = self.remaining.pop_front()?;
            if !next.label.is_empty() {
                self.status.detail = format!("Sending {}", next.label);
            }
            (next.report, next.ack_timeout_ms)
        } else {
            (self.request.take()?, 0)
        };
        self.sent_any = true;
        self.last_sent = Some(report);
        if matches!(self.stage, Stage::Sending) && report.as_bytes()[1] & 2 != 0 {
            self.pending = Some((report.message_id(), now.saturating_add(timeout)));
        }
        Some(report)
    }
    pub fn on_report(&mut self, report: Report64, now: u64) {
        self.tick(now);
        if !self.status.busy() || self.request.is_some() {
            return;
        }
        if let Err(e) = self.receive(report, now) {
            self.fail(e);
        }
    }
    fn receive(&mut self, report: Report64, now: u64) -> Result<(), String> {
        let b = report.as_bytes();
        if matches!(b[0], 5 | 6 | 8 | 0x13 | 0x19 | 0x1b | 0x1d | 0x20) {
            return Err("Amplifier changed during auxiliary operation".into());
        }
        if matches!(self.stage, Stage::Sending) {
            if b[0] == 0 {
                if let Some((id, _)) = self.pending
                    && b[2] == id
                {
                    if b[3] != 0 {
                        return Err(format!("Auxiliary command {id:02x} rejected: {}", b[3]));
                    }
                    self.pending = None;
                }
            } else if Some(report) != self.last_sent && matches!(b[0], 0x1c | 0x14 | 0xff) {
                return Err("Unexpected amplifier response during auxiliary write".into());
            }
            if self.remaining.is_empty() && self.pending.is_none() {
                self.begin_verification(now)?;
            }
            return Ok(());
        }
        let collector = match &mut self.stage {
            Stage::Read(c) | Stage::Preflight(c) | Stage::Verify(c) => c,
            _ => return Ok(()),
        };
        let Some(snapshot) = collector.observe(report).map_err(|e| e.to_string())? else {
            return Ok(());
        };
        match self.stage {
            Stage::Preflight(_) => {
                let selection = self
                    .selection
                    .as_ref()
                    .ok_or("Missing frozen auxiliary selection")?;
                selection
                    .validate_destination(&snapshot)
                    .map_err(|e| e.to_string())?;
                let steps = selection.steps();
                if steps.is_empty() || steps.len() > 4096 {
                    return Err("Invalid auxiliary transaction size".into());
                }
                self.remaining = steps.into();
                self.stage = Stage::Sending;
                self.status.phase = OperationPhase::Sending;
                self.status.detail =
                    "Destination matches preview; sending the complete reviewed queue".into();
                self.deadline = now.saturating_add(120_000);
            }
            Stage::Verify(_) => {
                self.selection
                    .as_ref()
                    .ok_or("Missing verification expectation")?
                    .matches_readback(&snapshot)
                    .map_err(|e| e.to_string())?;
                self.status.completed = self.status.total;
                self.finish(snapshot,"Every preset, system and physical-bank field matches full readback; hardware durability remains unverified");
            }
            Stage::Read(_) => {
                self.status.total = snapshot.entries().len();
                self.status.completed = self.status.total;
                self.finish(
                    snapshot,
                    "Complete system and control-bank snapshot captured",
                );
            }
            _ => {}
        }
        Ok(())
    }
    fn begin_verification(&mut self, now: u64) -> Result<(), String> {
        self.stage = Stage::Verify(Collector::new(self.pid).map_err(|e| e.to_string())?);
        self.request = Some(auxiliary::read_request(self.pid).map_err(|e| e.to_string())?);
        self.status.phase = OperationPhase::Reading;
        self.status.detail = "Reading the complete device to verify all sections".into();
        self.deadline = now.saturating_add(120_000);
        Ok(())
    }
    fn finish(&mut self, snapshot: Snapshot, detail: &str) {
        self.snapshot = Some(snapshot);
        self.status.phase = OperationPhase::Verified;
        self.status.detail = detail.into();
        self.stage = Stage::Done;
        self.selection = None;
    }
}
