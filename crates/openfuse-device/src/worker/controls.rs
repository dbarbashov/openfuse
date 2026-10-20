use super::*;
use crate::LiveBinding;
use openfuse_core::controls::{self, SettingsPatch};
impl Worker {
    pub(super) fn publish_tuner(&mut self) {
        let status = self.tuner.status().clone();
        if self.last_tuner.as_ref() != Some(&status) {
            emit(&self.events, DeviceEvent::Tuner(status.clone()));
            self.last_tuner = Some(status);
        }
    }
    pub(super) fn stop_requested_tuner(&mut self) {
        let binding = self
            .tuner_stop
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
        if let Some(binding) = binding {
            self.start_tuner(binding, false, false);
        }
    }
    pub(super) fn start_tuner(&mut self, binding: LiveBinding, enabled: bool, muted: bool) {
        let result: Result<(), String> = (|| {
            let target = self.target.as_ref().ok_or("No amplifier selected")?;
            if self.connection.is_none()
                || binding.device_id != target.id
                || !matches!(self.engine.status(), SessionStatus::Ready)
            {
                return Err("Tuner request does not belong to a connected ready device".into());
            }
            if self.operation.status().busy() || self.bank.status().busy() {
                return Err("Finish the active amplifier operation before using the tuner".into());
            }
            if enabled
                && (binding.generation != self.generation
                    || !self.snapshot_current
                    || self.live.status().pending != 0
                    || self.live.needs_refresh())
            {
                return Err("Read current before starting the tuner".into());
            }
            if enabled && self.tuner.status().error.is_some() {
                return Err("Stop the tuner to confirm a known state before restarting".into());
            }
            self.tuner.request(target.product_id, enabled, muted)?;
            self.tuner_device = Some(target.clone());
            self.live.disable();
            self.publish_live();
            Ok(())
        })();
        if let Err(error) = result {
            self.error(error);
        }
        self.publish_tuner();
        self.publish(None);
    }
    /// Best-effort deactivation on explicit disconnect or application shutdown.
    /// No response is fabricated; transport close cannot prove mute was released.
    pub(super) fn shutdown_tuner(&mut self) {
        if self.tuner.status().exclusive()
            && let (Some(connection), Some(target)) = (&mut self.connection, &self.target)
            && let Ok(report) = controls::tuner_control(target.product_id, false, false)
        {
            let _ = connection.write(report);
        }
    }
    pub(super) fn start_settings(&mut self, binding: LiveBinding, patch: SettingsPatch, now: u64) {
        if self.operation.status().busy()
            || self.bank.status().busy()
            || self.tuner.status().exclusive()
        {
            self.error(
                "Finish the active amplifier operation or stop the tuner before applying settings",
            );
            return;
        }
        let result: Result<(), String> = (|| {
            let target = self.target.as_ref().ok_or("No amplifier selected")?;
            if self.connection.is_none()
                || !matches!(self.engine.status(), SessionStatus::Ready)
                || !self.snapshot_current
                || binding.device_id != target.id
                || binding.generation != self.generation
            {
                return Err("Settings preview is stale; read current again".into());
            }
            if self.live.status().pending != 0 || self.live.needs_refresh() {
                return Err("Read current after live edits before changing settings".into());
            }
            let snapshot = self.engine.snapshot().ok_or("No current preset")?;
            let plan = controls::settings_plan(target.product_id, &snapshot.reports, &patch)
                .map_err(|e| e.to_string())?;
            self.live.disable();
            self.publish_live();
            self.operation.begin(plan, now)?;
            self.snapshot_current = false;
            Ok(())
        })();
        if let Err(error) = result {
            self.operation.reject("Amplifier settings", error);
        }
        self.publish_operation();
        self.publish(None);
    }
}
