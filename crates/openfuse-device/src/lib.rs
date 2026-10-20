//! Background sessions. Creating a controller does not enumerate or open USB.
//! Explicit connect reads detect/version/current-sync; live parameter writes require
//! a separate generation-bound opt-in. Temporary preset audition and slot selection
//! verify readback. Explicit bank save/restore compares stored-slot readback;
//! firmware and factory reset are not exposed.
pub mod acceptance;
mod backend;
mod bank;
pub mod trace;
pub use bank::{BankBackup, BankNames, BankStatus};
mod live;
mod tuner;
pub use tuner::TunerStatus;
mod operation;
pub use operation::{OperationPhase, OperationStatus};
mod simulator;
mod worker;

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfo {
    /// Opaque exact HID path identity, not a list index or a product ID.
    pub id: String,
    pub name: String,
    pub product_id: u16,
    pub serial: Option<String>,
    pub interface_number: i32,
    pub unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SnapshotSummary {
    pub generation: u64,
    pub reports: usize,
    pub name: Option<String>,
    pub slot: Option<u16>,
    pub modules: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DeviceStatus {
    pub phase: String,
    pub detail: String,
    pub version: Option<String>,
    pub snapshot: Option<SnapshotSummary>,
    pub can_sync: bool,
    pub reconnecting: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireCapture {
    pub device: DeviceInfo,
    pub generation: u64,
    pub version: Option<String>,
    /// Exact Name-to-Complete device reports, not a reconstructed FUSE file.
    pub reports: Vec<Vec<u8>>,
    /// Bounded diagnostic input since the preceding exported capture.
    pub received: Vec<Vec<u8>>,
    pub dropped_received: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LiveBinding {
    pub device_id: String,
    pub generation: u64,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct LiveStatus {
    pub enabled: bool,
    pub pending: usize,
    pub acknowledged: u64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuxiliarySnapshot {
    pub device: DeviceInfo,
    pub generation: u64,
    pub snapshot: openfuse_core::auxiliary::Snapshot,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", content = "data", rename_all = "snake_case")]
pub enum DeviceEvent {
    Devices(Vec<DeviceInfo>),
    Status(DeviceStatus),
    Error(String),
    Capture(WireCapture),
    Live(LiveStatus),
    Operation(OperationStatus),
    BankStatus(BankStatus),
    Tuner(TunerStatus),
    BankNames(BankNames),
    Backup(BankBackup),
    Auxiliary(AuxiliarySnapshot),
}

enum Command {
    Settings(u64, LiveBinding, openfuse_core::controls::SettingsPatch),
    Tuner(u64, LiveBinding, bool),
    Bank(u64, LiveBinding, bank::Request),
    Refresh,
    Connect(DeviceInfo),
    Disconnect,
    ReadCurrent,
    SetLive(u64, LiveBinding, bool),
    ApplyPreset(u64, LiveBinding, Box<openfuse_core::preset::PresetDocument>),
    SelectSlot(u64, LiveBinding, u16),
    Parameters(
        u64,
        LiveBinding,
        Vec<openfuse_core::messages::ParameterChange>,
    ),
}
type Mailbox = Arc<Mutex<VecDeque<DeviceEvent>>>;

/// Bounded command queue and coalesced event mailbox keep a paused GUI bounded.
/// Drop cancels the worker without joining a potentially stalled OS HID call.
pub struct DeviceController {
    recorder: trace::Recorder,
    commands: mpsc::SyncSender<Command>,
    events: Mailbox,
    stop: Arc<AtomicBool>,
    live_epoch: Arc<AtomicU64>,
    operation_epoch: Arc<AtomicU64>,
    tuner_epoch: Arc<AtomicU64>,
    tuner_stop: Arc<Mutex<Option<LiveBinding>>>,
}
impl Default for DeviceController {
    fn default() -> Self {
        Self::new()
    }
}
impl DeviceController {
    pub fn new() -> Self {
        Self::spawn_recorded(
            Box::new(backend::SystemBackend),
            trace::Origin::HardwareTransport,
        )
    }
    pub fn simulated() -> Self {
        Self::spawn(Box::new(simulator::Simulator::default()))
    }
    fn spawn(backend: Box<dyn backend::Backend>) -> Self {
        Self::spawn_recorded(backend, trace::Origin::Simulator)
    }
    pub fn recorder(&self) -> trace::Recorder {
        self.recorder.clone()
    }
    fn spawn_recorded(backend: Box<dyn backend::Backend>, origin: trace::Origin) -> Self {
        let recorder = trace::Recorder::new(origin);
        let backend = Box::new(trace::RecordingBackend {
            inner: backend,
            recorder: recorder.clone(),
        });
        let (tx, rx) = mpsc::sync_channel(16);
        let events = Arc::new(Mutex::new(VecDeque::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let worker_events = events.clone();
        let worker_stop = stop.clone();
        let live_epoch = Arc::new(AtomicU64::new(0));
        let worker_epoch = live_epoch.clone();
        let operation_epoch = Arc::new(AtomicU64::new(0));
        let worker_operation_epoch = operation_epoch.clone();
        let tuner_epoch = Arc::new(AtomicU64::new(0));
        let tuner_stop = Arc::new(Mutex::new(None));
        let worker_tuner_epoch = tuner_epoch.clone();
        let worker_tuner_stop = tuner_stop.clone();
        let result = std::thread::Builder::new()
            .name("openfuse-usb".into())
            .spawn(move || {
                worker::run(
                    backend,
                    rx,
                    worker_events,
                    worker_stop,
                    worker_epoch,
                    worker_operation_epoch,
                    (worker_tuner_epoch, worker_tuner_stop),
                )
            });
        if let Err(error) = result {
            emit(
                &events,
                DeviceEvent::Error(format!("Cannot start USB worker: {error}")),
            );
        }
        Self {
            recorder,
            commands: tx,
            events,
            stop,
            live_epoch,
            operation_epoch,
            tuner_epoch,
            tuner_stop,
        }
    }
    fn send(&self, command: Command) -> Result<(), String> {
        self.commands
            .try_send(command)
            .map_err(|e| format!("USB worker unavailable or busy: {e}"))
    }
    pub fn refresh(&self) -> Result<(), String> {
        self.send(Command::Refresh)
    }
    pub fn connect(&self, device: &DeviceInfo) -> Result<(), String> {
        self.send(Command::Connect(device.clone()))
    }
    pub fn disconnect(&self) -> Result<(), String> {
        self.cancel_operation()?;
        self.send(Command::Disconnect)
    }
    pub fn read_current(&self) -> Result<(), String> {
        self.cancel_operation()?;
        self.send(Command::ReadCurrent)
    }
    pub fn set_live(&self, binding: &LiveBinding, enabled: bool) -> Result<(), String> {
        if !enabled {
            self.cancel_live();
            return Ok(());
        }
        self.send(Command::SetLive(
            self.live_epoch.load(Ordering::Acquire),
            binding.clone(),
            true,
        ))
    }
    pub fn send_parameters(
        &self,
        binding: &LiveBinding,
        changes: Vec<openfuse_core::messages::ParameterChange>,
    ) -> Result<(), String> {
        if changes.len() > 64 {
            return Err("Live batch exceeds 64 parameters".into());
        }
        self.send(Command::Parameters(
            self.live_epoch.load(Ordering::Acquire),
            binding.clone(),
            changes,
        ))
    }
    // Out-of-band cancellation cannot be lost behind a full command channel.
    fn cancel_live(&self) {
        self.live_epoch.fetch_add(1, Ordering::AcqRel);
    }
    /// Explicit audition changes the working preset only; local XML remains owned by the UI.
    pub fn apply_preset(
        &self,
        binding: &LiveBinding,
        document: openfuse_core::preset::PresetDocument,
    ) -> Result<(), String> {
        self.cancel_live();
        self.send(Command::ApplyPreset(
            self.operation_epoch.load(Ordering::Acquire),
            binding.clone(),
            Box::new(document),
        ))
    }
    pub fn select_slot(&self, binding: &LiveBinding, slot: u16) -> Result<(), String> {
        self.cancel_live();
        self.send(Command::SelectSlot(
            self.operation_epoch.load(Ordering::Acquire),
            binding.clone(),
            slot,
        ))
    }
    pub fn simulated_superchamp() -> Self {
        Self::spawn(Box::new(simulator::Simulator::superchamp()))
    }
    pub fn simulated_floor() -> Self {
        Self::spawn(Box::new(simulator::Simulator::floor()))
    }
    pub fn apply_settings(
        &self,
        binding: &LiveBinding,
        patch: openfuse_core::controls::SettingsPatch,
    ) -> Result<(), String> {
        self.cancel_live();
        self.send(Command::Settings(
            self.operation_epoch.load(Ordering::Acquire),
            binding.clone(),
            patch,
        ))
    }
    pub fn set_tuner(
        &self,
        binding: &LiveBinding,
        enabled: bool,
        muted: bool,
    ) -> Result<(), String> {
        self.cancel_live();
        if !enabled {
            self.tuner_epoch.fetch_add(1, Ordering::AcqRel);
            *self.tuner_stop.lock().unwrap_or_else(|p| p.into_inner()) = Some(binding.clone());
            Ok(())
        } else {
            self.send(Command::Tuner(
                self.tuner_epoch.load(Ordering::Acquire),
                binding.clone(),
                muted,
            ))
        }
    }
    fn bank_request(&self, binding: &LiveBinding, request: bank::Request) -> Result<(), String> {
        self.cancel_live();
        self.send(Command::Bank(
            self.operation_epoch.load(Ordering::Acquire),
            binding.clone(),
            request,
        ))
    }
    pub fn read_auxiliary(&self, binding: &LiveBinding) -> Result<(), String> {
        self.bank_request(binding, bank::Request::AuxiliaryRead)
    }
    pub fn restore_whole(
        &self,
        binding: &LiveBinding,
        plan: openfuse_core::auxiliary::WholeRestore,
    ) -> Result<(), String> {
        self.bank_request(binding, bank::Request::WholeRestore(Box::new(plan)))
    }
    pub fn restore_auxiliary(
        &self,
        binding: &LiveBinding,
        selection: openfuse_core::auxiliary::Selection,
    ) -> Result<(), String> {
        self.bank_request(
            binding,
            bank::Request::AuxiliaryRestore(Box::new(selection)),
        )
    }
    pub fn read_bank(&self, binding: &LiveBinding) -> Result<(), String> {
        self.bank_request(binding, bank::Request::Names)
    }
    pub fn backup_bank(&self, binding: &LiveBinding) -> Result<(), String> {
        self.bank_request(binding, bank::Request::Backup)
    }
    /// Read only selected stored slots. Empty, duplicate and invalid selections fail before I/O.
    pub fn backup_slots(&self, binding: &LiveBinding, slots: Vec<u16>) -> Result<(), String> {
        self.bank_request(binding, bank::Request::BackupSlots(slots))
    }
    /// Explicit persistent save of the freshly captured amplifier working sound.
    pub fn save_slot(&self, binding: &LiveBinding, slot: u16) -> Result<(), String> {
        self.bank_request(binding, bank::Request::Save(slot))
    }
    pub fn restore_bank(
        &self,
        binding: &LiveBinding,
        backup: openfuse_core::bank::Backup,
    ) -> Result<(), String> {
        self.bank_request(binding, bank::Request::Restore(Box::new(backup)))
    }
    /// Restore an immutable source-to-destination selection with per-slot readback.
    pub fn restore_slots(
        &self,
        binding: &LiveBinding,
        selection: openfuse_core::bank::RestoreSelection,
    ) -> Result<(), String> {
        self.bank_request(
            binding,
            bank::Request::RestoreSelection(Box::new(selection)),
        )
    }
    /// Out-of-band cancellation survives a full command channel. In-flight USB writes cannot be recalled.
    pub fn cancel_operation(&self) -> Result<(), String> {
        self.cancel_live();
        self.operation_epoch.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }
    pub fn poll(&self) -> Vec<DeviceEvent> {
        self.events
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .drain(..)
            .collect()
    }
}
impl Drop for DeviceController {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}
fn emit(events: &Mailbox, event: DeviceEvent) {
    let mut q = events.lock().unwrap_or_else(|p| p.into_inner());
    // Latest inventory/state supersedes earlier values, errors retain chronology.
    q.retain(|old| {
        !matches!(
            (&event, old),
            (DeviceEvent::Devices(_), DeviceEvent::Devices(_))
                | (DeviceEvent::Status(_), DeviceEvent::Status(_))
                | (DeviceEvent::Capture(_), DeviceEvent::Capture(_))
                | (DeviceEvent::Live(_), DeviceEvent::Live(_))
                | (DeviceEvent::Operation(_), DeviceEvent::Operation(_))
                | (DeviceEvent::BankStatus(_), DeviceEvent::BankStatus(_))
                | (DeviceEvent::Tuner(_), DeviceEvent::Tuner(_))
                | (DeviceEvent::BankNames(_), DeviceEvent::BankNames(_))
                | (DeviceEvent::Backup(_), DeviceEvent::Backup(_))
                | (DeviceEvent::Auxiliary(_), DeviceEvent::Auxiliary(_))
        )
    });
    if q.len() == 32 {
        q.pop_front();
    }
    q.push_back(event);
}

/// Bounded diagnostic-file parser; does not connect or trust a file as a live binding.
pub const MAX_CAPTURE_BYTES: usize = 4 * 1024 * 1024;
pub fn parse_capture_json(text: &str) -> Result<WireCapture, String> {
    if text.len() > MAX_CAPTURE_BYTES {
        return Err("Capture exceeds 4 MiB".into());
    }
    let mut value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("Invalid capture JSON: {e}"))?;
    if let Some(capture) = value.get_mut("capture") {
        value = capture.take();
    }
    let capture: WireCapture =
        serde_json::from_value(value).map_err(|e| format!("Invalid capture fields: {e}"))?;
    if capture.reports.is_empty() || capture.reports.len() > 512 || capture.received.len() > 256 {
        return Err("Capture report count exceeds session limits".into());
    }
    if capture
        .reports
        .iter()
        .chain(&capture.received)
        .any(|r| r.len() != 64)
    {
        return Err("Every capture report must contain exactly 64 bytes".into());
    }
    if capture.device.id.len() > 4096
        || capture.device.name.len() > 1024
        || capture
            .device
            .serial
            .as_ref()
            .is_some_and(|s| s.len() > 1024)
    {
        return Err("Capture identity exceeds display limits".into());
    }
    Ok(capture)
}

#[cfg(test)]
mod capture_tests;
