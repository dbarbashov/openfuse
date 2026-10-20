use crate::{
    Command, DeviceEvent, DeviceInfo, DeviceStatus, Mailbox, SnapshotSummary,
    backend::{Backend, Connection, TransportError},
    emit,
};
use openfuse_core::usb_session::{SessionMachine, SessionStatus};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc::{Receiver, TryRecvError},
};
use std::time::{Duration, Instant};

const POLL_MS: u64 = 1000;
const MAX_RECONNECTS: u8 = 5;
struct Worker {
    backend: Box<dyn Backend>,
    connection: Option<Box<dyn Connection>>,
    target: Option<DeviceInfo>,
    engine: SessionMachine,
    events: Mailbox,
    retry_at: Option<u64>,
    attempts: u8,
    next_poll: u64,
    generation: u64,
    last_capture: Option<u64>,
    last_status: Option<DeviceStatus>,
    live: crate::live::LiveQueue,
    last_live: Option<crate::LiveStatus>,
    live_epoch: Arc<AtomicU64>,
    observed_epoch: u64,
    operation: crate::operation::OperationQueue,
    last_operation: Option<crate::OperationStatus>,
    operation_epoch: Arc<AtomicU64>,
    observed_operation_epoch: u64,
    snapshot_current: bool,
    bank: crate::bank::BankQueue,
    last_bank: Option<crate::BankStatus>,
    bank_refresh_pending: bool,
    tuner: crate::tuner::Tuner,
    last_tuner: Option<crate::TunerStatus>,
    tuner_device: Option<DeviceInfo>,
    tuner_epoch: Arc<AtomicU64>,
    tuner_stop: Arc<std::sync::Mutex<Option<crate::LiveBinding>>>,
}
pub(crate) fn run(
    backend: Box<dyn Backend>,
    commands: Receiver<Command>,
    events: Mailbox,
    stop: Arc<AtomicBool>,
    live_epoch: Arc<AtomicU64>,
    operation_epoch: Arc<AtomicU64>,
    tuner_commands: (
        Arc<AtomicU64>,
        Arc<std::sync::Mutex<Option<crate::LiveBinding>>>,
    ),
) {
    let start = Instant::now();
    let mut w = Worker::new(backend, events);
    w.live_epoch = live_epoch;
    w.operation_epoch = operation_epoch;
    w.tuner_epoch = tuner_commands.0;
    w.tuner_stop = tuner_commands.1;
    while !stop.load(Ordering::Acquire) {
        let now = start.elapsed().as_millis().min(u64::MAX as u128) as u64;
        // Drain commands before any report is sent: Disconnect supersedes pending work.
        for _ in 0..16 {
            match commands.try_recv() {
                Ok(command) => w.command(command, now),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    w.shutdown_tuner();
                    return;
                }
            }
        }
        if stop.load(Ordering::Acquire) {
            break;
        }
        w.step(now);
        std::thread::sleep(Duration::from_millis(5));
    }
    w.shutdown_tuner();
}
impl Worker {
    fn new(backend: Box<dyn Backend>, events: Mailbox) -> Self {
        Self {
            backend,
            connection: None,
            target: None,
            engine: SessionMachine::new(),
            events,
            retry_at: None,
            attempts: 0,
            next_poll: 0,
            generation: 0,
            last_capture: None,
            last_status: None,
            live: crate::live::LiveQueue::default(),
            last_live: None,
            live_epoch: Arc::new(AtomicU64::new(0)),
            observed_epoch: 0,
            operation: crate::operation::OperationQueue::default(),
            last_operation: None,
            operation_epoch: Arc::new(AtomicU64::new(0)),
            observed_operation_epoch: 0,
            snapshot_current: false,
            bank: Default::default(),
            last_bank: None,
            bank_refresh_pending: false,
            tuner: Default::default(),
            last_tuner: None,
            tuner_device: None,
            tuner_epoch: Arc::new(AtomicU64::new(0)),
            tuner_stop: Arc::new(std::sync::Mutex::new(None)),
        }
    }
    fn error(&self, error: impl Into<String>) {
        emit(&self.events, DeviceEvent::Error(error.into()));
    }
    fn publish(&mut self, detail: Option<String>) {
        let reconnecting = self.retry_at.is_some();
        let phase = if reconnecting {
            "Waiting for amplifier".into()
        } else {
            match self.engine.status() {
                SessionStatus::Disconnected => "Disconnected".into(),
                SessionStatus::Detecting => "Detecting".into(),
                SessionStatus::Version => "Reading firmware".into(),
                SessionStatus::Syncing => "Reading current preset".into(),
                SessionStatus::Ready => "Connected".into(),
                SessionStatus::Failed(_) => "Connection failed".into(),
            }
        };
        let version = self
            .engine
            .version()
            .map(|v| format!("{}.{} build {}", v.major, v.minor, v.build));
        let snapshot = self
            .engine
            .snapshot()
            .filter(|_| {
                matches!(self.engine.status(), SessionStatus::Ready)
                    && !self.operation.status().busy()
                    && !self.bank.status().busy()
                    && !self.tuner.status().exclusive()
                    && self.snapshot_current
            })
            .map(|s| SnapshotSummary {
                generation: self.generation,
                reports: s.reports.len(),
                name: Some(s.name.clone()),
                slot: Some(s.position),
                modules: s.modules.len(),
            });
        let status = DeviceStatus {
            phase,
            detail: detail.unwrap_or_else(|| match self.engine.status() {
                SessionStatus::Failed(e) => e.to_string(),
                _ => self
                    .target
                    .as_ref()
                    .map(|d| d.name.clone())
                    .unwrap_or_else(|| "Local editor remains offline".into()),
            }),
            version,
            snapshot,
            can_sync: matches!(self.engine.status(), SessionStatus::Ready)
                && self.connection.is_some()
                && !self.operation.status().busy()
                && !self.bank.status().busy()
                && !self.tuner.status().exclusive(),
            reconnecting,
        };
        if self.last_status.as_ref() != Some(&status) {
            emit(&self.events, DeviceEvent::Status(status.clone()));
            self.last_status = Some(status);
        }
    }
    fn publish_live(&mut self) {
        let status = self.live.status();
        if self.last_live.as_ref() != Some(&status) {
            emit(&self.events, DeviceEvent::Live(status.clone()));
            self.last_live = Some(status);
        }
    }
    fn set_live(&mut self, binding: crate::LiveBinding, enabled: bool) {
        if !enabled {
            self.live.disable();
            self.publish_live();
            return;
        }
        let result = (|| {
            let target = self.target.as_ref().ok_or("No amplifier selected")?;
            if !matches!(self.engine.status(), SessionStatus::Ready)
                || self.operation.status().busy()
                || self.bank.status().busy()
                || self.tuner.status().exclusive()
                || !self.snapshot_current
                || self.connection.is_none()
                || binding.device_id != target.id
                || binding.generation != self.generation
            {
                return Err(
                    "Live binding is stale; read and import the selected amplifier again".into(),
                );
            }
            let snapshot = self
                .engine
                .snapshot()
                .ok_or("No complete device snapshot")?;
            let decoded = openfuse_core::wire_preset::decode(target.product_id, &snapshot.reports)
                .map_err(|e| e.to_string())?;
            self.live
                .enable(binding, target.product_id, decoded.document)
        })();
        if let Err(error) = result {
            self.live.fail(error);
        }
        self.publish_live();
    }
    fn cancel_stale_live(&mut self) {
        let epoch = self.live_epoch.load(Ordering::Acquire);
        if epoch != self.observed_epoch {
            self.observed_epoch = epoch;
            self.live.disable();
            self.publish_live();
        }
    }
    fn command(&mut self, command: Command, now: u64) {
        self.cancel_stale_live();
        self.cancel_stale_operation();
        self.stop_requested_tuner();
        match command {
            Command::Settings(epoch, binding, patch) => {
                if epoch == self.observed_operation_epoch {
                    self.start_settings(binding, patch, now);
                } else {
                    self.operation.cancelled_before_start();
                    self.publish_operation();
                }
            }
            Command::Tuner(epoch, binding, muted) => {
                if epoch == self.tuner_epoch.load(Ordering::Acquire) {
                    self.start_tuner(binding, true, muted);
                }
            }

            Command::Bank(epoch, binding, request) => {
                if epoch == self.observed_operation_epoch {
                    self.start_bank(binding, request, now);
                } else {
                    self.bank.cancelled_before_start();
                    self.publish_bank();
                }
            }
            Command::ApplyPreset(epoch, binding, document) => {
                if epoch == self.observed_operation_epoch {
                    self.start_operation(binding, Some(*document), None, now);
                } else {
                    self.operation.cancelled_before_start();
                    self.publish_operation();
                }
            }
            Command::SelectSlot(epoch, binding, slot) => {
                if epoch == self.observed_operation_epoch {
                    self.start_operation(binding, None, Some(slot), now);
                } else {
                    self.operation.cancelled_before_start();
                    self.publish_operation();
                }
            }
            Command::SetLive(epoch, binding, enabled) => {
                if epoch == self.observed_epoch {
                    self.set_live(binding, enabled);
                }
            }
            Command::Parameters(epoch, binding, changes) => {
                if epoch != self.observed_epoch {
                    return;
                }
                if let Err(error) = self.live.enqueue(&binding, changes) {
                    self.live.fail(error);
                }
                self.publish_live();
            }
            Command::Refresh => match self.backend.enumerate() {
                Ok(devices) => emit(&self.events, DeviceEvent::Devices(devices)),
                Err(e) => self.error(e),
            },
            Command::Disconnect => {
                self.shutdown_tuner();
                self.tuner.lost();
                self.publish_tuner();
                self.bank.cancel();
                self.bank.clear_results();
                self.bank.take_transport_reset();
                self.bank_refresh_pending = false;
                self.publish_bank();
                self.operation.cancel();
                self.publish_operation();
                self.snapshot_current = false;
                self.live
                    .fail("Disconnected; read and import again before live editing");
                self.publish_live();
                self.connection = None;
                self.target = None;
                self.retry_at = None;
                self.attempts = 0;
                self.last_capture = None;
                self.engine.disconnect();
                self.publish(None);
            }
            Command::Connect(device) => {
                if self.connection.is_some() || self.retry_at.is_some() {
                    self.error("Disconnect the current session before selecting another amplifier");
                    return;
                }
                if let Some(reason) = &device.unavailable_reason {
                    self.error(reason.clone());
                    self.engine.fail_transport(reason.clone());
                    self.publish(None);
                    return;
                }
                // Validate against fresh discovery, not caller-provided labels/capabilities.
                match self.backend.enumerate() {
                    Ok(devices) if devices.contains(&device) => {}
                    Ok(_) => {
                        self.error("Selected interface is no longer present; refresh the list");
                        self.engine
                            .fail_transport("Selected interface is no longer present");
                        self.publish(None);
                        return;
                    }
                    Err(e) => {
                        self.engine.fail_transport(e.clone());
                        self.bank.fail(e.clone());
                        self.publish_bank();
                        self.error(e);
                        self.publish(None);
                        return;
                    }
                }
                if self.tuner_device.as_ref().is_some_and(|old| old != &device) {
                    self.tuner = Default::default();
                    self.tuner_device = None;
                    self.publish_tuner();
                }
                self.target = Some(device);
                self.attempts = 0;
                if let Err(e) = self.open(now) {
                    self.engine.fail_transport(e.clone());
                    self.bank.fail(e.clone());
                    self.publish_bank();
                    self.error(e);
                    self.publish(None);
                }
            }
            Command::ReadCurrent => {
                if self.tuner.status().exclusive() {
                    self.error("Stop the tuner before reading the current preset");
                    return;
                }
                self.bank.cancel();
                self.publish_bank();
                self.recover_uncertain_bank(now);
                self.operation.cancel();
                self.publish_operation();
                self.snapshot_current = false;
                self.live.disable();
                self.publish_live();
                if self.connection.is_none() {
                    self.error("No active amplifier session");
                } else if let Err(e) = self.engine.request_sync(now) {
                    self.error(e.to_string());
                }
                self.publish(None);
            }
        }
    }
    fn open(&mut self, now: u64) -> Result<(), String> {
        self.bank.clear_results();
        self.bank_refresh_pending = false;
        let target = self.target.as_ref().ok_or("No selected device")?;
        let mut engine = SessionMachine::for_pid(target.product_id).map_err(|e| e.to_string())?;
        let connection = self.backend.open(target)?;
        engine.start(now).map_err(|e| e.to_string())?;
        self.engine = engine;
        self.connection = Some(connection);
        self.retry_at = None;
        self.next_poll = now + POLL_MS;
        self.last_capture = None;
        self.snapshot_current = false;
        self.publish(None);
        Ok(())
    }
    fn lost(&mut self, error: String, now: u64) {
        self.tuner.lost();
        self.publish_tuner();
        self.bank.fail("Connection lost during bank operation");
        self.bank.clear_results();
        self.bank.take_transport_reset();
        self.bank_refresh_pending = false;
        self.publish_bank();
        self.snapshot_current = false;
        if self.operation.status().busy() {
            self.operation
                .fail("Connection lost; working-preset operation discarded without replay");
        }
        self.publish_operation();
        self.live
            .fail("Connection lost; live edits were discarded, not replayed");
        self.publish_live();
        self.connection = None;
        self.engine.disconnect();
        self.last_capture = None;
        self.error(error.clone());
        if self.attempts < MAX_RECONNECTS && self.target.is_some() {
            self.retry_at = Some(now + (1000u64 << self.attempts.min(3)));
        } else {
            self.retry_at = None;
            self.engine.fail_transport(format!(
                "Reconnect stopped after {MAX_RECONNECTS} attempts; connect explicitly to retry"
            ));
        }
        self.publish(Some(error));
    }
    fn reconnect(&mut self, now: u64) {
        if self.retry_at.is_none_or(|t| now < t) {
            return;
        }
        self.attempts += 1;
        let Some(target) = self.target.clone() else {
            self.retry_at = None;
            return;
        };
        let devices = match self.backend.enumerate() {
            Ok(v) => v,
            Err(e) => {
                self.lost(e, now);
                return;
            }
        };
        emit(&self.events, DeviceEvent::Devices(devices.clone()));
        // A stable serial may resolve a changed OS path, but ambiguity is rejected.
        // Without serial, only the exact path/interface is ever eligible.
        let matches: Vec<_> = devices
            .into_iter()
            .filter(|d| reconnect_match(&target, d))
            .collect();
        if matches.len() != 1 {
            self.lost(
                format!(
                    "Selected amplifier absent or ambiguous (attempt {}/{MAX_RECONNECTS})",
                    self.attempts
                ),
                now,
            );
            return;
        }
        self.target = matches.into_iter().next();
        if let Err(e) = self.open(now) {
            self.lost(e, now);
        }
    }
    fn step(&mut self, now: u64) {
        self.stop_requested_tuner();
        self.cancel_stale_live();
        self.cancel_stale_operation();
        self.recover_uncertain_operation(now);
        self.recover_uncertain_bank(now);
        let cycle_started = Instant::now();
        self.reconnect(now);
        if self.connection.is_none() {
            return;
        }
        if now >= self.next_poll {
            self.next_poll = now + POLL_MS;
            match self.backend.enumerate() {
                Ok(devices) => {
                    let present = self.target.as_ref().is_some_and(|t| devices.contains(t));
                    emit(&self.events, DeviceEvent::Devices(devices));
                    if !present {
                        self.lost("USB interface disconnected".into(), now);
                        return;
                    }
                }
                Err(e) => {
                    self.lost(format!("USB discovery failed: {e}"), now);
                    return;
                }
            }
        }
        self.engine.tick(now);
        if matches!(self.engine.status(), SessionStatus::Failed(_)) {
            self.tuner.lost();
            self.publish_tuner();
            self.connection = None;
            self.retry_at = None;
            self.fail_readback();
            self.publish(None);
            return;
        }
        while let Some(report) = self.engine.take_outgoing() {
            if let Err(e) = self.connection.as_mut().expect("connected").write(report) {
                self.lost(e, now);
                return;
            }
        }
        let read = self.connection.as_mut().expect("connected").read();
        let now =
            now.saturating_add(cycle_started.elapsed().as_millis().min(u64::MAX as u128) as u64);
        self.live.tick(now);
        self.operation.tick(now);
        self.bank.tick(now);
        self.tuner.tick(now);
        self.engine.tick(now);
        match read {
            Ok(Some(report)) => {
                if report.message_id() == 0x0a {
                    self.tuner_device = self.target.clone();
                }
                self.tuner.on_report(report, now);
                if self.tuner.status().enabled && report.message_id() == 0x0a {
                    if self.operation.status().busy() {
                        self.operation.fail("Tuner activated during operation");
                    }
                    self.bank.fail("Tuner activated during bank operation");
                    self.live.disable();
                }
                if self.bank.status().busy() {
                    self.bank.on_report(report, now);
                } else {
                    self.operation.on_report(report, now);
                    self.live.on_report(report, now);
                    if self.live.status().error.is_some()
                        && !self.operation.status().busy()
                        && !self.bank.status().busy()
                        && !self.tuner.status().exclusive()
                    {
                        self.snapshot_current = false;
                    }
                    self.engine.on_report(report, now);
                }
            }
            Ok(None) => {}
            Err(TransportError::Io(e)) => {
                self.lost(e, now);
                return;
            }
            Err(TransportError::Framing(e)) => {
                self.tuner.lost();
                self.publish_tuner();
                self.connection = None;
                self.retry_at = None;
                self.engine.fail_transport(e.clone());
                self.bank.fail(e.clone());
                self.publish_bank();
                self.live.fail(e.clone());
                if self.operation.status().busy() {
                    self.operation.fail(e.clone());
                }
                self.snapshot_current = false;
                self.error(e);
            }
        }
        if matches!(self.engine.status(), SessionStatus::Failed(_)) {
            self.tuner.lost();
            self.publish_tuner();
            self.connection = None;
            self.retry_at = None;
            self.fail_readback();
        }
        if let Some(snapshot) = self
            .engine
            .snapshot()
            .filter(|_| matches!(self.engine.status(), SessionStatus::Ready))
            && self.last_capture != Some(snapshot.captured_at_ms)
        {
            self.last_capture = Some(snapshot.captured_at_ms);
            self.generation += 1;
            self.live.fresh_read();
            self.snapshot_current = true;
            self.operation.readback(&snapshot.reports, self.generation);
            self.attempts = 0;
            if let Some(device) = self.target.clone() {
                emit(
                    &self.events,
                    DeviceEvent::Capture(crate::WireCapture {
                        device,
                        generation: self.generation,
                        version: self
                            .engine
                            .version()
                            .map(|v| format!("{}.{} build {}", v.major, v.minor, v.build)),
                        reports: snapshot
                            .reports
                            .iter()
                            .map(|r| r.as_bytes().to_vec())
                            .collect(),
                        received: std::iter::from_fn(|| self.engine.take_received())
                            .map(|r| r.as_bytes().to_vec())
                            .collect(),
                        dropped_received: self.engine.dropped_reports(),
                    }),
                );
            }
        }
        self.finish_bank(now);
        self.stop_requested_tuner();
        // Cancellation can arrive while HID read is blocked. Recheck before dispatch.
        self.cancel_stale_live();
        self.cancel_stale_operation();
        self.recover_uncertain_operation(now);
        self.recover_uncertain_bank(now);
        if matches!(self.engine.status(), SessionStatus::Ready) && self.connection.is_some() {
            if let Some(report) = self.tuner.take_outgoing(now)
                && let Err(error) = self.connection.as_mut().expect("connected").write(report)
            {
                self.lost(error, now);
                return;
            }
            if let Some(report) = self.bank.outgoing(now)
                && let Err(error) = self.connection.as_mut().expect("connected").write(report)
            {
                self.lost(error, now);
                return;
            }
            if let Some(report) = self.operation.outgoing(now)
                && let Err(error) = self.connection.as_mut().expect("connected").write(report)
            {
                self.lost(error, now);
                return;
            }
            if self.operation.needs_read() {
                match self.engine.request_sync(now) {
                    Ok(()) => self.operation.reading(),
                    Err(error) => self.operation.fail(error.to_string()),
                }
            }
        }
        // Process already-arrived device changes before sending another live write.
        if matches!(self.engine.status(), SessionStatus::Ready)
            && !self.operation.status().busy()
            && !self.bank.status().busy()
            && !self.tuner.status().exclusive()
            && let Some(report) = self.live.outgoing(now)
            && let Err(error) = self.connection.as_mut().expect("connected").write(report)
        {
            self.lost(error, now);
            return;
        }
        self.publish(None);
        self.publish_live();
        self.publish_operation();
        self.publish_bank();
        self.publish_tuner();
    }
}
fn reconnect_match(old: &DeviceInfo, new: &DeviceInfo) -> bool {
    new.unavailable_reason.is_none()
        && old.product_id == new.product_id
        && old.interface_number == new.interface_number
        && match &old.serial {
            Some(serial) => new.serial.as_ref() == Some(serial),
            None => old.id == new.id && new.serial.is_none(),
        }
}

mod bank;
mod controls;
mod operation;
#[cfg(test)]
mod tests;
