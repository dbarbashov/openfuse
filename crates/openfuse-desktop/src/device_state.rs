//! UI-owned device state. Device events have no reference to the editor document.
use openfuse_device::{
    DeviceController, DeviceEvent, DeviceInfo, DeviceStatus, LiveBinding, LiveStatus,
    OperationStatus, WireCapture,
};

pub struct DeviceState {
    pub acceptance: Option<openfuse_device::acceptance::Acceptance>,
    pub acceptance_message: Option<String>,
    pub acceptance_dirty: bool,
    controller: DeviceController,
    pub devices: Vec<DeviceInfo>,
    pub selected: Option<String>,
    pub status: DeviceStatus,
    pub error: Option<String>,
    pub simulated: bool,
    pub enumerated: bool,
    pub enumerating: bool,
    pub capture: Option<WireCapture>,
    pub live: LiveStatus,
    pub operation: OperationStatus,
    pub tuner: openfuse_device::TunerStatus,
    tuner_submitted: bool,
    pub selected_slot: u16,
    pub bank: openfuse_device::BankStatus,
    pub bank_names: Option<openfuse_device::BankNames>,
    pub auxiliary: Option<openfuse_device::AuxiliarySnapshot>,
    pub bank_backup: Option<openfuse_device::BankBackup>,
    operation_submitted: bool,
    connected: Option<String>,
    demo_connect_pending: bool,
    demo_read_pending: bool,
}

impl DeviceState {
    #[cfg(test)]
    pub fn new(simulated: bool) -> Self {
        Self::with_floor(simulated, false)
    }
    #[cfg(test)]
    pub fn with_floor(simulated: bool, floor: bool) -> Self {
        Self::with_model(simulated, floor, false)
    }
    pub fn with_model(simulated: bool, floor: bool, superchamp: bool) -> Self {
        let mut state = Self {
            acceptance: None,
            acceptance_dirty: false,
            acceptance_message: None,
            controller: if simulated && superchamp {
                DeviceController::simulated_superchamp()
            } else if simulated && floor {
                DeviceController::simulated_floor()
            } else if simulated {
                DeviceController::simulated()
            } else {
                DeviceController::new()
            },
            devices: Vec::new(),
            selected: None,
            status: DeviceStatus::default(),
            error: None,
            simulated,
            enumerated: false,
            enumerating: false,
            capture: None,
            live: LiveStatus::default(),
            operation: OperationStatus::default(),
            tuner: Default::default(),
            tuner_submitted: false,
            selected_slot: 0,
            bank: Default::default(),
            bank_names: None,
            bank_backup: None,
            auxiliary: None,
            operation_submitted: false,
            connected: None,
            demo_connect_pending: simulated,
            demo_read_pending: simulated,
        };
        if simulated {
            state.refresh();
        }
        state
    }

    pub fn recorder(&self) -> openfuse_device::trace::Recorder {
        self.controller.recorder()
    }
    pub fn refresh(&mut self) {
        match self.controller.refresh() {
            Ok(()) => {
                self.enumerating = true;
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }
    pub fn selected_device(&self) -> Option<&DeviceInfo> {
        self.devices
            .iter()
            .find(|device| Some(&device.id) == self.selected.as_ref())
    }
    pub fn can_connect(&self) -> bool {
        !self.has_connection()
            && self
                .selected_device()
                .is_some_and(|device| device.unavailable_reason.is_none())
    }
    pub fn has_connection(&self) -> bool {
        self.connected.is_some() || self.status.can_sync || self.status.reconnecting
    }
    pub fn connect(&mut self) {
        let Some(device) = self.selected_device().cloned() else {
            self.error = Some("Select an amplifier first".into());
            return;
        };
        if let Some(reason) = &device.unavailable_reason {
            self.error = Some(reason.clone());
            return;
        }
        if self.has_connection() {
            self.error =
                Some("Disconnect the current session before choosing another amplifier".into());
            return;
        }
        match self.controller.connect(&device) {
            Ok(()) => {
                self.connected = Some(device.id);
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }
    pub fn disconnect(&mut self) {
        // Explicit disconnect also cancels demo automation; a future poll cannot reconnect it.
        self.demo_connect_pending = false;
        self.demo_read_pending = false;
        match self.controller.disconnect() {
            Ok(()) => {
                self.connected = None;
                self.auxiliary = None;
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }
    pub fn read_current(&mut self) {
        if !self.status.can_sync {
            self.error =
                Some("Wait for the amplifier connection before reading its current preset".into());
            return;
        }
        match self.controller.read_current() {
            Ok(()) => self.error = None,
            Err(error) => self.error = Some(error),
        }
    }
    pub fn poll(&mut self) {
        for event in self.controller.poll() {
            self.apply_event(event);
        }
        if self.demo_connect_pending && self.enumerated && !self.enumerating {
            self.demo_connect_pending = false;
            if self.can_connect() {
                self.connect();
            } else {
                self.demo_read_pending = false;
            }
        }
        if self.demo_read_pending && self.status.can_sync {
            self.demo_read_pending = false;
            if self.capture.is_none() {
                self.read_current();
            }
        }
    }
    pub fn apply_event(&mut self, event: DeviceEvent) {
        match event {
            // Raw captures belong to the diagnostic transport surface; they do
            // not replace a local editor document or its undo history.
            DeviceEvent::Capture(capture) => {
                if self.auxiliary.as_ref().is_some_and(|snapshot| {
                    snapshot.device.id != capture.device.id
                        || snapshot.generation != capture.generation
                }) {
                    self.auxiliary = None;
                }
                if self
                    .bank_names
                    .as_ref()
                    .is_some_and(|bank| bank.device.id != capture.device.id)
                {
                    self.bank_names = None;
                }
                self.capture = Some(capture);
            }
            DeviceEvent::BankNames(bank) => self.bank_names = Some(bank),
            DeviceEvent::Auxiliary(snapshot) => self.auxiliary = Some(snapshot),
            DeviceEvent::Backup(backup) => self.bank_backup = Some(backup),
            DeviceEvent::BankStatus(status) => {
                self.bank = status;
                self.operation_submitted = false;
            }
            DeviceEvent::Live(status) => self.live = status,
            DeviceEvent::Tuner(status) => {
                self.tuner = status;
                self.tuner_submitted = false;
            }
            DeviceEvent::Operation(status) => {
                self.operation = status;
                self.operation_submitted = false;
            }
            DeviceEvent::Devices(devices) => {
                if !devices
                    .iter()
                    .any(|device| Some(&device.id) == self.selected.as_ref())
                {
                    self.selected = devices
                        .iter()
                        .find(|device| device.unavailable_reason.is_none())
                        .or(devices.first())
                        .map(|device| device.id.clone());
                }
                self.devices = devices;
                self.enumerating = false;
                self.enumerated = true;
            }
            DeviceEvent::Status(status) => {
                if status.reconnecting
                    || matches!(status.phase.as_str(), "Disconnected" | "Connection failed")
                {
                    self.auxiliary = None;
                }
                if !status.reconnecting
                    && matches!(status.phase.as_str(), "Disconnected" | "Connection failed")
                {
                    self.connected = None;
                }
                if status.can_sync {
                    self.error = None;
                }
                if let Some(snapshot) = &status.snapshot
                    && self.status.snapshot.as_ref().map(|old| old.generation)
                        != Some(snapshot.generation)
                    && let Some(slot) = snapshot.slot
                {
                    self.selected_slot = slot;
                }
                self.status = status;
            }
            DeviceEvent::Error(error) => {
                self.error = Some(error);
                self.tuner_submitted = false;
                self.operation_submitted = false;
                self.enumerating = false;
                self.demo_connect_pending = false;
                self.demo_read_pending = false;
            }
        }
    }
    pub fn set_live(&mut self, binding: &LiveBinding, enabled: bool) -> Result<(), String> {
        self.controller.set_live(binding, enabled)
    }
    pub fn send_parameters(
        &mut self,
        binding: &LiveBinding,
        changes: Vec<openfuse_core::messages::ParameterChange>,
    ) -> Result<(), String> {
        self.controller.send_parameters(binding, changes)
    }
    pub fn apply_preset(
        &mut self,
        binding: &LiveBinding,
        document: openfuse_core::preset::PresetDocument,
    ) -> Result<(), String> {
        self.controller.apply_preset(binding, document)?;
        self.operation_submitted = true;
        Ok(())
    }
    pub fn select_slot(&mut self, binding: &LiveBinding, slot: u16) -> Result<(), String> {
        self.controller.select_slot(binding, slot)?;
        self.operation_submitted = true;
        Ok(())
    }
    pub fn read_bank(&mut self, binding: &LiveBinding) -> Result<(), String> {
        self.controller.read_bank(binding)?;
        self.operation_submitted = true;
        Ok(())
    }
    pub fn backup_bank(&mut self, binding: &LiveBinding) -> Result<(), String> {
        self.controller.backup_bank(binding)?;
        self.operation_submitted = true;
        Ok(())
    }
    pub fn backup_slots(&mut self, binding: &LiveBinding, slots: Vec<u16>) -> Result<(), String> {
        self.controller.backup_slots(binding, slots)?;
        self.operation_submitted = true;
        Ok(())
    }
    pub fn restore_slots(
        &mut self,
        binding: &LiveBinding,
        selection: openfuse_core::bank::RestoreSelection,
    ) -> Result<(), String> {
        self.controller.restore_slots(binding, selection)?;
        self.operation_submitted = true;
        Ok(())
    }
    pub fn save_slot(&mut self, binding: &LiveBinding, slot: u16) -> Result<(), String> {
        self.controller.save_slot(binding, slot)?;
        self.operation_submitted = true;
        Ok(())
    }
    pub fn restore_bank(
        &mut self,
        binding: &LiveBinding,
        backup: openfuse_core::bank::Backup,
    ) -> Result<(), String> {
        self.controller.restore_bank(binding, backup)?;
        self.operation_submitted = true;
        Ok(())
    }
    pub fn apply_settings(
        &mut self,
        binding: &LiveBinding,
        patch: openfuse_core::controls::SettingsPatch,
    ) -> Result<(), String> {
        self.controller.apply_settings(binding, patch)?;
        self.operation_submitted = true;
        Ok(())
    }
    pub fn set_tuner(
        &mut self,
        binding: &LiveBinding,
        enabled: bool,
        muted: bool,
    ) -> Result<(), String> {
        self.controller.set_tuner(binding, enabled, muted)?;
        self.tuner_submitted = true;
        Ok(())
    }
    pub fn read_auxiliary(&mut self, binding: &LiveBinding) -> Result<(), String> {
        self.controller.read_auxiliary(binding)?;
        self.operation_submitted = true;
        Ok(())
    }
    pub fn restore_auxiliary(
        &mut self,
        binding: &LiveBinding,
        selection: openfuse_core::auxiliary::Selection,
    ) -> Result<(), String> {
        self.controller.restore_auxiliary(binding, selection)?;
        self.operation_submitted = true;
        Ok(())
    }
    pub fn current_auxiliary(&self) -> Option<&openfuse_device::AuxiliarySnapshot> {
        let snapshot = self.auxiliary.as_ref()?;
        self.matches_capture(&LiveBinding {
            device_id: snapshot.device.id.clone(),
            generation: snapshot.generation,
        })
        .then_some(snapshot)
    }
    pub fn restore_whole(
        &mut self,
        binding: &LiveBinding,
        plan: openfuse_core::auxiliary::WholeRestore,
    ) -> Result<(), String> {
        self.controller.restore_whole(binding, plan)?;
        self.operation_submitted = true;
        Ok(())
    }
    pub fn current_bank_names(&self) -> Option<&openfuse_device::BankNames> {
        let names = self.bank_names.as_ref()?;
        self.matches_capture(&LiveBinding {
            device_id: names.device.id.clone(),
            generation: names.generation,
        })
        .then_some(names)
    }
    pub fn operation_busy(&self) -> bool {
        self.operation_submitted
            || self.operation.busy()
            || self.bank.busy()
            || self.tuner_submitted
            || self.tuner.enabled
            || self.tuner.pending
            || self.tuner.error.is_some()
    }
    pub fn cancel_operation(&mut self) -> Result<(), String> {
        self.controller.cancel_operation()?;
        self.operation_submitted = false;
        Ok(())
    }
    pub fn current_binding(&self) -> Option<LiveBinding> {
        let capture = self.capture.as_ref()?;
        let binding = LiveBinding {
            device_id: capture.device.id.clone(),
            generation: capture.generation,
        };
        self.matches_capture(&binding).then_some(binding)
    }
    pub fn matches_capture(&self, binding: &LiveBinding) -> bool {
        self.status.can_sync
            && self.connected.as_ref() == Some(&binding.device_id)
            && self
                .status
                .snapshot
                .as_ref()
                .is_some_and(|s| s.generation == binding.generation)
            && self.capture.as_ref().is_some_and(|c| {
                c.device.id == binding.device_id && c.generation == binding.generation
            })
    }
    pub fn needs_polling(&self) -> bool {
        self.enumerating
            || self.has_connection()
            || self.demo_connect_pending
            || self.demo_read_pending
    }
    pub fn capture_ready(&self) -> bool {
        !self.simulated
            || self.error.is_some()
            || (!self.demo_connect_pending
                && !self.demo_read_pending
                && self.status.can_sync
                && self.status.snapshot.is_some())
    }
    pub fn phase(&self) -> &str {
        if self.status.phase.is_empty() {
            "Offline"
        } else {
            &self.status.phase
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failure_releases_attempted_connection_and_success_clears_recovered_error() {
        let mut state = DeviceState::new(false);
        state.devices.push(DeviceInfo {
            id: "synthetic-path".into(),
            name: "Synthetic device".into(),
            product_id: 20,
            serial: None,
            interface_number: 0,
            unavailable_reason: None,
        });
        state.selected = Some("synthetic-path".into());
        state.connected = state.selected.clone();
        assert!(!state.can_connect());
        state.apply_event(DeviceEvent::Error("Opening failed".into()));
        state.apply_event(DeviceEvent::Status(DeviceStatus {
            phase: "Connection failed".into(),
            ..Default::default()
        }));
        assert!(state.can_connect());
        assert!(!state.has_connection());
        state.apply_event(DeviceEvent::Status(DeviceStatus {
            phase: "Connected".into(),
            can_sync: true,
            ..Default::default()
        }));
        assert!(state.error.is_none());
        assert!(!state.can_connect());
    }
}
