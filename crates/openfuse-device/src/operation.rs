//! One bounded working-preset transaction, followed by an explicit readback.
//! No persistence commands, retry, rollback or automatic replay.
use openfuse_core::{audition::Plan, protocol::Report64};
use serde::Serialize;
use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationPhase {
    #[default]
    Idle,
    Sending,
    Reading,
    Verified,
    Failed,
    Cancelled,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct OperationStatus {
    pub phase: OperationPhase,
    pub kind: String,
    pub sent: usize,
    pub total: usize,
    pub detail: String,
    pub readback_generation: Option<u64>,
}
impl OperationStatus {
    pub fn busy(&self) -> bool {
        matches!(
            self.phase,
            OperationPhase::Sending | OperationPhase::Reading
        )
    }
}
#[derive(Default)]
pub(crate) struct OperationQueue {
    status: OperationStatus,
    plan: Option<Plan>,
    remaining: VecDeque<Report64>,
    pending: Option<(u8, u64)>,
    last_sent: Option<Report64>,
    deadline: Option<u64>,
    reset_transport: bool,
}
impl OperationQueue {
    pub(crate) fn status(&self) -> &OperationStatus {
        &self.status
    }
    pub(crate) fn begin(&mut self, plan: Plan, now: u64) -> Result<(), String> {
        if self.status.busy() {
            return Err("A working-preset operation is already active".into());
        }
        if plan.steps().is_empty() || plan.steps().len() > 32 {
            return Err("Invalid working-preset transaction size".into());
        }
        self.status = OperationStatus {
            phase: OperationPhase::Sending,
            kind: plan.label().into(),
            sent: 0,
            total: plan.steps().len(),
            detail: "Sending temporary working-preset commands".into(),
            readback_generation: None,
        };
        self.remaining = plan.steps().iter().copied().collect();
        self.plan = Some(plan);
        self.pending = None;
        self.last_sent = None;
        self.deadline = Some(now.saturating_add(60_000));
        self.reset_transport = false;
        Ok(())
    }
    fn clear(&mut self) {
        self.remaining.clear();
        self.pending = None;
        self.plan = None;
        self.last_sent = None;
        self.deadline = None;
    }
    pub(crate) fn fail(&mut self, detail: impl Into<String>) {
        self.reset_transport |=
            self.status.phase == OperationPhase::Sending && self.status.sent > 0;
        self.clear();
        self.status.phase = OperationPhase::Failed;
        self.status.detail = detail.into();
    }
    pub(crate) fn reject(&mut self, kind: &str, detail: impl Into<String>) {
        self.clear();
        self.status = OperationStatus {
            phase: OperationPhase::Failed,
            kind: kind.into(),
            detail: detail.into(),
            ..Default::default()
        };
    }
    pub(crate) fn cancel(&mut self) {
        if self.status.busy() {
            self.reset_transport |=
                self.status.phase == OperationPhase::Sending && self.status.sent > 0;
            self.clear();
            self.status.phase = OperationPhase::Cancelled;
            self.status.detail="Cancelled unsent commands; delivered changes may remain. Read current before continuing".into();
        }
    }
    pub(crate) fn cancelled_before_start(&mut self) {
        if !self.status.busy() {
            self.clear();
            self.status = OperationStatus {
                phase: OperationPhase::Cancelled,
                detail: "Cancelled before transmission".into(),
                ..Default::default()
            };
        }
    }
    pub(crate) fn tick(&mut self, now: u64) {
        if self.pending.is_some_and(|(_, deadline)| now >= deadline) {
            self.fail("Working-preset ACK timed out; delivery is uncertain. Read current before continuing");
        } else if self.deadline.is_some_and(|deadline| now >= deadline) {
            self.fail("Working-preset operation timed out; read current before continuing");
        }
    }
    pub(crate) fn outgoing(&mut self, now: u64) -> Option<Report64> {
        self.tick(now);
        if self.status.phase != OperationPhase::Sending || self.pending.is_some() {
            return None;
        }
        let report = self.remaining.pop_front()?;
        if report.as_bytes()[1] & 2 != 0 {
            self.pending = Some((report.message_id(), now.saturating_add(5_000)));
        }
        self.last_sent = Some(report);
        self.status.sent += 1;
        Some(report)
    }
    pub(crate) fn on_report(&mut self, report: Report64, now: u64) {
        self.tick(now);
        if self.status.phase != OperationPhase::Sending {
            return;
        }
        let b = report.as_bytes();
        if b[0] == 0 {
            if let Some((command, _)) = self.pending
                && b[2] == command
            {
                if b[3] == 0 {
                    self.pending = None;
                } else {
                    self.fail(format!("Working-preset command {command:02x} rejected (ACK status {}); read current before continuing",b[3]));
                }
            }
        } else if Some(report) != self.last_sent
            && matches!(
                b[0],
                0x05 | 0x06 | 0x08 | 0x13 | 0x19 | 0x1b | 0x1c | 0x1d | 0x20 | 0xff
            )
        {
            self.fail("Amplifier changed during the working-preset transaction; read current before continuing");
        }
    }
    pub(crate) fn take_transport_reset(&mut self) -> bool {
        std::mem::take(&mut self.reset_transport)
    }
    pub(crate) fn needs_read(&self) -> bool {
        self.status.phase == OperationPhase::Sending
            && self.remaining.is_empty()
            && self.pending.is_none()
    }
    pub(crate) fn reading(&mut self) {
        self.status.phase = OperationPhase::Reading;
        self.status.detail = "Commands acknowledged; verifying a fresh current-preset read".into();
    }
    pub(crate) fn readback(&mut self, reports: &[Report64], generation: u64) {
        if self.status.phase != OperationPhase::Reading {
            return;
        }
        let result = self
            .plan
            .as_ref()
            .expect("reading has a plan")
            .matches_readback(reports);
        self.status.readback_generation = Some(generation);
        match result {
            Ok(()) => {
                self.clear();
                self.status.phase = OperationPhase::Verified;
                self.status.detail = "Fresh readback matches the requested working state".into();
            }
            Err(error) => self.fail(format!(
                "Readback differs: {error}. Local document remains unchanged"
            )),
        }
    }
}
