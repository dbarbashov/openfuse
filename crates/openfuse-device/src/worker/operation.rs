use super::*;
use openfuse_core::{audition, preset::PresetDocument};
impl Worker {
    pub(super) fn publish_operation(&mut self) {
        let status = self.operation.status().clone();
        if self.last_operation.as_ref() != Some(&status) {
            emit(&self.events, DeviceEvent::Operation(status.clone()));
            self.last_operation = Some(status);
        }
    }
    pub(super) fn cancel_stale_operation(&mut self) {
        let epoch = self.operation_epoch.load(Ordering::Acquire);
        if epoch != self.observed_operation_epoch {
            self.observed_operation_epoch = epoch;
            if self.operation.status().busy() {
                self.snapshot_current = false;
            }
            self.operation.cancel();
            self.bank.cancel();
            self.publish_bank();
            self.publish_operation();
        }
    }
    pub(super) fn recover_uncertain_operation(&mut self, now: u64) {
        if self.operation.take_transport_reset() && self.connection.is_some() {
            self.lost("Interrupted working-preset transfer: reopening for a read-only handshake; no upload replay or rollback".into(),now);
        }
    }
    pub(super) fn fail_readback(&mut self) {
        self.snapshot_current = false;
        if self.operation.status().busy() {
            self.operation
                .fail("Fresh readback failed; working-preset delivery remains uncertain");
        }
        self.publish_operation();
    }
    pub(super) fn start_operation(
        &mut self,
        binding: crate::LiveBinding,
        document: Option<PresetDocument>,
        slot: Option<u16>,
        now: u64,
    ) {
        if self.operation.status().busy()
            || self.bank.status().busy()
            || self.tuner.status().exclusive()
        {
            self.error("A working-preset operation is already active");
            return;
        }
        let kind = if document.is_some() {
            "Audition preset"
        } else {
            "Select stored slot"
        };
        let result = (|| {
            let target = self.target.as_ref().ok_or("No amplifier selected")?;
            if self.connection.is_none()
                || !matches!(self.engine.status(), SessionStatus::Ready)
                || !self.snapshot_current
                || binding.device_id != target.id
                || binding.generation != self.generation
            {
                return Err(
                    "Device snapshot is stale; read current and prepare the operation again"
                        .to_string(),
                );
            }
            if self.live.status().pending != 0 || self.live.needs_refresh() {
                return Err(
                    "Read current after unfinished or uncertain live edits before auditioning"
                        .into(),
                );
            }
            let snapshot = self.engine.snapshot().ok_or("No complete current preset")?;
            let plan = if let Some(document) = document {
                audition::preset(target.product_id, &document, &snapshot.reports)
            } else {
                audition::slot_select(target.product_id, slot.ok_or("No target slot")?)
            }
            .map_err(|e| e.to_string())?;
            self.live.disable();
            self.publish_live();
            self.operation.begin(plan, now)?;
            self.snapshot_current = false;
            Ok(())
        })();
        if let Err(error) = result {
            self.operation.reject(kind, error);
        }
        self.publish_operation();
        self.publish(None);
    }
}
