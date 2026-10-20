use super::*;
use crate::{BankBackup, BankNames, LiveBinding, bank::Request};
impl Worker {
    pub(super) fn publish_bank(&mut self) {
        let status = self.bank.status().clone();
        if self.last_bank.as_ref() != Some(&status) {
            emit(&self.events, DeviceEvent::BankStatus(status.clone()));
            self.last_bank = Some(status);
        }
    }
    pub(super) fn start_bank(&mut self, binding: LiveBinding, request: Request, now: u64) {
        if self.bank.status().busy()
            || self.operation.status().busy()
            || self.tuner.status().exclusive()
        {
            self.error("Wait for the active amplifier operation");
            return;
        }
        let label = request.label();
        let result: Result<(), String> = (|| {
            let target = self.target.as_ref().ok_or("No amplifier selected")?;
            if self.connection.is_none()
                || !matches!(self.engine.status(), SessionStatus::Ready)
                || !self.snapshot_current
                || binding.device_id != target.id
                || binding.generation != self.generation
            {
                return Err(
                    "Read current and prepare a new bank preview; the binding is stale".into(),
                );
            }
            if self.live.status().pending != 0 || self.live.needs_refresh() {
                return Err(
                    "Read current after unfinished live edits before bank operations".into(),
                );
            }
            let snapshot = self.engine.snapshot().ok_or("Missing current capture")?;
            self.bank
                .begin(target.product_id, request, &snapshot.reports, now)?;
            self.live.disable();
            self.publish_live();
            self.snapshot_current = false;
            self.bank_refresh_pending = true;
            Ok(())
        })();
        if let Err(error) = result {
            self.bank.reject(label, error);
        }
        self.publish_bank();
        self.publish(None);
    }
    pub(super) fn recover_uncertain_bank(&mut self, now: u64) {
        if self.bank.take_transport_reset() && self.connection.is_some() {
            self.lost(
                "Interrupted bank operation: reopening read-only, without replay or rollback"
                    .into(),
                now,
            );
        }
    }
    pub(super) fn finish_bank(&mut self, now: u64) {
        if self.bank.status().busy() {
            return;
        }
        if let Some(device) = self.target.clone() {
            if self.snapshot_current
                && let Some(snapshot) = self.bank.take_auxiliary()
            {
                emit(
                    &self.events,
                    DeviceEvent::Auxiliary(crate::AuxiliarySnapshot {
                        device: device.clone(),
                        generation: self.generation,
                        snapshot,
                    }),
                );
            }
            // A complete archive is valuable even if the follow-up current read fails.
            // Its original device/generation is provenance, never a live edit binding.
            if let Some(backup) = self.bank.backup.take() {
                emit(
                    &self.events,
                    DeviceEvent::Backup(BankBackup {
                        device: device.clone(),
                        generation: self.generation,
                        backup,
                    }),
                );
            }
            if self.snapshot_current
                && let Some(entries) = self.bank.names.take()
            {
                emit(
                    &self.events,
                    DeviceEvent::BankNames(BankNames {
                        device,
                        generation: self.generation,
                        entries,
                    }),
                );
            }
        }
        if self.bank_refresh_pending && self.bank.status().phase == crate::OperationPhase::Verified
        {
            self.bank_refresh_pending = false;
            // Bank reads never become editor captures. Obtain a distinct current snapshot.
            if let Err(error) = self.engine.request_sync(now) {
                self.error(error.to_string());
            }
        }
        self.publish_bank();
    }
}
