use super::*;
use crate::simulator::{Simulator, device};
use std::collections::VecDeque;
use std::sync::Mutex;

#[derive(Default)]
struct State {
    devices: Vec<DeviceInfo>,
    opens: usize,
    writes: Vec<[u8; 64]>,
    fail_read: bool,
    bad_frame: bool,
    fail_write: bool,
    silent: bool,
    ack_status: Option<u8>,
    operation_ack_status: Option<u8>,
    restore_ack_status: Option<u8>,
    corrupt_readback: bool,
    injected: Option<openfuse_core::protocol::Report64>,
}
struct FakeBackend(Arc<Mutex<State>>);
struct FakeConnection {
    state: Arc<Mutex<State>>,
    peer: Box<dyn Connection>,
}
impl Backend for FakeBackend {
    fn enumerate(&mut self) -> Result<Vec<DeviceInfo>, String> {
        Ok(self.0.lock().unwrap().devices.clone())
    }
    fn open(&mut self, _: &DeviceInfo) -> Result<Box<dyn Connection>, String> {
        self.0.lock().unwrap().opens += 1;
        Ok(Box::new(FakeConnection {
            state: self.0.clone(),
            peer: Simulator::default().open(&device())?,
        }))
    }
}
impl Connection for FakeConnection {
    fn write(&mut self, report: openfuse_core::protocol::Report64) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();
        state.writes.push(*report.as_bytes());
        if state.fail_write {
            state.fail_write = false;
            return Err("write failed".into());
        }
        self.peer.write(report)
    }
    fn read(&mut self) -> Result<Option<openfuse_core::protocol::Report64>, TransportError> {
        let mut state = self.state.lock().unwrap();
        if state.fail_read {
            state.fail_read = false;
            return Err(TransportError::Io("cable disconnected".into()));
        }
        if state.bad_frame {
            state.bad_frame = false;
            return Err(TransportError::Framing("63-byte truncated input".into()));
        }
        if state.silent {
            return Ok(None);
        }
        if let Some(report) = state.injected.take() {
            return Ok(Some(report));
        }
        let report = self.peer.read()?;
        Ok(report.map(|r| {
            if r.message_id() == 0
                && r.as_bytes()[2] == 5
                && let Some(status) = state.ack_status
            {
                let mut b = *r.as_bytes();
                b[3] = status;
                return openfuse_core::protocol::Report64::from_bytes(b);
            }
            if r.message_id() == 0
                && r.as_bytes()[2] == 0x1c
                && let Some(status) = state.operation_ack_status
            {
                let mut b = *r.as_bytes();
                b[3] = status;
                return openfuse_core::protocol::Report64::from_bytes(b);
            }
            if r.message_id() == 0
                && r.as_bytes()[2] == 0x14
                && let Some(status) = state.restore_ack_status
            {
                let mut b = *r.as_bytes();
                b[3] = status;
                return openfuse_core::protocol::Report64::from_bytes(b);
            }
            if state.corrupt_readback && r.message_id() == 0x1c && r.as_bytes()[2] == 5 {
                let mut b = *r.as_bytes();
                b[33] ^= 0x20;
                return openfuse_core::protocol::Report64::from_bytes(b);
            }
            r
        }))
    }
}
fn worker() -> (Worker, Arc<Mutex<State>>) {
    let state = Arc::new(Mutex::new(State {
        devices: vec![device()],
        ..Default::default()
    }));
    (
        Worker::new(
            Box::new(FakeBackend(state.clone())),
            Arc::new(Mutex::new(Default::default())),
        ),
        state,
    )
}
fn ready(w: &mut Worker, start: u64) {
    for t in start..start + 20 {
        w.step(t);
    }
    assert_eq!(w.engine.status(), &SessionStatus::Ready);
}
#[test]
fn enumeration_never_opens_and_explicit_connect_only_sends_three_read_requests() {
    let (mut w, state) = worker();
    w.command(Command::Refresh, 0);
    w.step(0);
    assert_eq!(state.lock().unwrap().opens, 0);
    assert!(state.lock().unwrap().writes.is_empty());
    w.command(Command::Connect(device()), 1);
    ready(&mut w, 1);
    let state = state.lock().unwrap();
    assert_eq!(state.opens, 1);
    assert_eq!(
        state
            .writes
            .iter()
            .map(|x| [x[0], x[1]])
            .collect::<Vec<_>>(),
        vec![[0, 0xc3], [0x1a, 0xc1], [0xff, 0xc5]]
    );
    assert_eq!(
        w.last_status
            .as_ref()
            .unwrap()
            .snapshot
            .as_ref()
            .unwrap()
            .modules,
        5
    );
}
#[test]
fn missing_forged_and_unsupported_devices_are_not_opened() {
    let (mut w, state) = worker();
    let mut d = device();
    d.id = "not-enumerated".into();
    w.command(Command::Connect(d), 0);
    let mut d = device();
    d.unavailable_reason = Some("bootloader".into());
    w.command(Command::Connect(d), 1);
    let mut d = device();
    d.product_id = 21;
    w.command(Command::Connect(d), 2);
    assert_eq!(state.lock().unwrap().opens, 0);
}
#[test]
fn unplug_reconnect_uses_new_handshake_and_never_exposes_old_snapshot() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    state.lock().unwrap().devices.clear();
    w.step(1000);
    assert!(w.last_status.as_ref().unwrap().reconnecting);
    assert!(w.last_status.as_ref().unwrap().snapshot.is_none());
    let mut moved = device();
    moved.id = "new-path-same-serial".into();
    state.lock().unwrap().devices = vec![moved];
    ready(&mut w, 2000);
    assert_eq!(state.lock().unwrap().opens, 2);
    assert_eq!(
        state
            .lock()
            .unwrap()
            .writes
            .iter()
            .map(|x| x[0])
            .collect::<Vec<_>>(),
        vec![0, 0x1a, 0xff, 0, 0x1a, 0xff]
    );
    assert_eq!(
        w.last_status
            .as_ref()
            .unwrap()
            .snapshot
            .as_ref()
            .unwrap()
            .generation,
        2
    );
}
#[test]
fn ambiguous_serial_or_different_device_never_reconnects() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    state.lock().unwrap().fail_read = true;
    w.step(30);
    let mut b = device();
    b.id = "second".into();
    state.lock().unwrap().devices.push(b);
    w.step(1030);
    assert_eq!(state.lock().unwrap().opens, 1);
    assert!(w.retry_at.is_some());
    state.lock().unwrap().devices[0].serial = Some("different".into());
    state.lock().unwrap().devices.pop();
    w.step(3030);
    assert_eq!(state.lock().unwrap().opens, 1);
}
#[test]
fn absent_serial_requires_same_path_and_interface() {
    let mut a = device();
    a.serial = None;
    let mut b = a.clone();
    b.id = "different".into();
    assert!(!reconnect_match(&a, &b));
    b.id = a.id.clone();
    assert!(reconnect_match(&a, &b));
    b.interface_number = 1;
    assert!(!reconnect_match(&a, &b));
}
#[test]
fn retry_budget_is_bounded_and_disconnect_cancels_recovery() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    state.lock().unwrap().devices.clear();
    w.step(1000);
    for _ in 0..MAX_RECONNECTS {
        let due = w.retry_at.unwrap();
        w.step(due);
    }
    assert!(w.retry_at.is_none());
    assert!(matches!(w.engine.status(), SessionStatus::Failed(_)));
    assert_eq!(state.lock().unwrap().opens, 1);
    state.lock().unwrap().devices = vec![device()];
    w.command(Command::Connect(device()), 30000);
    ready(&mut w, 30000);
    state.lock().unwrap().fail_read = true;
    w.step(30030);
    w.command(Command::Disconnect, 30031);
    w.step(50000);
    assert_eq!(w.engine.status(), &SessionStatus::Disconnected);
    assert_eq!(state.lock().unwrap().opens, 2);
}
#[test]
fn short_or_invalid_input_stops_instead_of_retrying_bad_protocol() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    state.lock().unwrap().bad_frame = true;
    w.step(0);
    assert!(matches!(w.engine.status(), SessionStatus::Failed(_)));
    assert!(w.retry_at.is_none());
    w.step(10000);
    assert_eq!(state.lock().unwrap().opens, 1);
}
#[test]
fn write_failure_discards_session_and_restarts_detection() {
    let (mut w, state) = worker();
    state.lock().unwrap().fail_write = true;
    w.command(Command::Connect(device()), 0);
    w.step(0);
    assert!(w.retry_at.is_some());
    ready(&mut w, 1000);
    assert_eq!(
        state
            .lock()
            .unwrap()
            .writes
            .iter()
            .map(|x| x[0])
            .collect::<Vec<_>>(),
        vec![0, 0, 0x1a, 0xff]
    );
}
#[test]
fn response_timeout_does_not_replay_or_reconnect() {
    let (mut w, state) = worker();
    state.lock().unwrap().silent = true;
    w.command(Command::Connect(device()), 0);
    w.step(0);
    w.step(6001);
    assert!(matches!(w.engine.status(), SessionStatus::Failed(_)));
    assert!(w.retry_at.is_none());
    assert_eq!(state.lock().unwrap().writes.len(), 1);
}
#[test]
fn explicit_refresh_read_has_new_snapshot_generation_and_no_upload() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    w.command(Command::ReadCurrent, 50);
    ready(&mut w, 50);
    assert_eq!(w.generation, 2);
    assert_eq!(
        state
            .lock()
            .unwrap()
            .writes
            .iter()
            .map(|x| x[0])
            .collect::<Vec<_>>(),
        vec![0, 0x1a, 0xff, 0xff]
    );
}

#[test]
fn refreshing_or_failed_read_never_labels_previous_snapshot_as_current() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    assert!(w.last_status.as_ref().unwrap().snapshot.is_some());
    w.command(Command::ReadCurrent, 50);
    assert!(w.last_status.as_ref().unwrap().snapshot.is_none());
    state.lock().unwrap().silent = true;
    w.step(50);
    w.step(16000);
    assert!(matches!(w.engine.status(), SessionStatus::Failed(_)));
    assert!(w.last_status.as_ref().unwrap().snapshot.is_none());
}

#[test]
fn event_mailbox_coalesces_captures_and_status_and_bounds_errors() {
    let mailbox = Arc::new(Mutex::new(VecDeque::new()));
    for _ in 0..100 {
        emit(&mailbox, DeviceEvent::Error("error".into()));
    }
    assert_eq!(mailbox.lock().unwrap().len(), 32);
    for n in 0..100 {
        emit(
            &mailbox,
            DeviceEvent::Status(DeviceStatus {
                phase: n.to_string(),
                ..Default::default()
            }),
        );
    }
    let q = mailbox.lock().unwrap();
    assert_eq!(q.len(), 32);
    assert_eq!(
        q.iter()
            .filter(|e| matches!(e, DeviceEvent::Status(_)))
            .count(),
        1
    );
    assert!(matches!(q.back(),Some(DeviceEvent::Status(s)) if s.phase=="99"));
}

fn live_binding(w: &Worker) -> crate::LiveBinding {
    crate::LiveBinding {
        device_id: w.target.as_ref().unwrap().id.clone(),
        generation: w.generation,
    }
}
fn edits(w: &Worker) -> Vec<openfuse_core::messages::ParameterChange> {
    let doc = openfuse_core::wire_preset::decode(20, &w.engine.snapshot().unwrap().reports)
        .unwrap()
        .document;
    (0..2)
        .map(|index| {
            let mut p =
                openfuse_core::wire_preset::parameter_change(20, &doc, "Amplifier", index).unwrap();
            p.wire_value = 12345 + u16::from(index);
            p
        })
        .collect()
}
fn live_writes(state: &Arc<Mutex<State>>) -> usize {
    state
        .lock()
        .unwrap()
        .writes
        .iter()
        .filter(|r| r[0] == 5)
        .count()
}
#[test]
fn live_opt_in_sends_nothing_until_edit_then_ack_and_readback() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    let binding = live_binding(&w);
    let changes = edits(&w);
    w.command(Command::SetLive(0, binding.clone(), true), 30);
    w.step(30);
    assert!(w.live.status().enabled);
    assert_eq!(live_writes(&state), 0);
    w.command(Command::Parameters(0, binding.clone(), changes), 31);
    w.step(31);
    w.step(32);
    w.step(33);
    assert_eq!(w.live.status().acknowledged, 2);
    assert_eq!(w.live.status().pending, 0);
    w.command(Command::ReadCurrent, 40);
    ready(&mut w, 40);
    let doc = openfuse_core::wire_preset::decode(20, &w.engine.snapshot().unwrap().reports)
        .unwrap()
        .document;
    let value = openfuse_core::wire_preset::parameter_change(20, &doc, "Amplifier", 0)
        .unwrap()
        .wire_value;
    assert_eq!(value, 12345 / 256 * 256); // Packed snapshots quantize continuous values.
    assert!(!w.live.status().enabled);
    assert_eq!(w.generation, 2);
    w.command(Command::SetLive(0, binding, true), 70);
    assert!(!w.live.status().enabled);
    assert_eq!(live_writes(&state), 2);
}
#[test]
fn live_nack_timeout_and_late_ack_never_flush_queued_changes() {
    for status in [None, Some(1), Some(3), Some(4), Some(255)] {
        let (mut w, state) = worker();
        w.command(Command::Connect(device()), 0);
        ready(&mut w, 0);
        let binding = live_binding(&w);
        let changes = edits(&w);
        w.command(Command::SetLive(0, binding.clone(), true), 30);
        state.lock().unwrap().ack_status = status;
        state.lock().unwrap().silent = status.is_none();
        w.command(Command::Parameters(0, binding.clone(), changes), 31);
        w.step(31);
        w.step(5032);
        assert!(!w.live.status().enabled);
        assert_eq!(w.live.status().pending, 0);
        state.lock().unwrap().silent = false;
        w.step(5033); // Delayed success cannot resume.
        w.command(Command::SetLive(0, binding, true), 5034);
        w.step(5034);
        assert!(!w.live.status().enabled);
        assert_eq!(live_writes(&state), 1);
    }
}
#[test]
fn live_disconnect_or_write_failure_reconnects_read_only_without_replay() {
    for fail_write in [false, true] {
        let (mut w, state) = worker();
        w.command(Command::Connect(device()), 0);
        ready(&mut w, 0);
        let binding = live_binding(&w);
        let changes = edits(&w);
        w.command(Command::SetLive(0, binding.clone(), true), 30);
        if fail_write {
            state.lock().unwrap().fail_write = true;
        } else {
            state.lock().unwrap().silent = true;
        }
        w.command(Command::Parameters(0, binding, changes), 31);
        w.step(31);
        if !fail_write {
            state.lock().unwrap().silent = false;
            state.lock().unwrap().fail_read = true;
            w.step(32);
        }
        assert!(!w.live.status().enabled);
        assert_eq!(w.live.status().pending, 0);
        let due = w.retry_at.unwrap();
        ready(&mut w, due);
        assert_eq!(w.generation, 2);
        assert!(!w.live.status().enabled);
        assert_eq!(live_writes(&state), 1);
        assert_eq!(state.lock().unwrap().opens, 2);
    }
}
#[test]
fn disable_before_flush_and_foreign_binding_cannot_write() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    let binding = live_binding(&w);
    let changes = edits(&w);
    w.command(Command::SetLive(0, binding.clone(), true), 30);
    w.command(Command::Parameters(0, binding.clone(), changes), 31);
    w.command(Command::SetLive(0, binding.clone(), false), 31);
    w.step(31);
    assert_eq!(live_writes(&state), 0);
    w.command(Command::ReadCurrent, 40);
    ready(&mut w, 40);
    let mut foreign = live_binding(&w);
    foreign.device_id = "foreign".into();
    w.command(Command::SetLive(0, foreign, true), 70);
    w.step(70);
    assert!(!w.live.status().enabled);
    assert_eq!(live_writes(&state), 0);
}

#[test]
fn arrived_external_changes_cancel_before_live_dispatch_and_invalidate_idle_capture() {
    for enabled in [false, true] {
        let (mut w, state) = worker();
        w.command(Command::Connect(device()), 0);
        ready(&mut w, 0);
        let binding = live_binding(&w);
        let changes = edits(&w);
        if enabled {
            w.command(Command::SetLive(0, binding.clone(), true), 30);
            w.command(Command::Parameters(0, binding.clone(), changes), 31);
        }
        let mut b = [0; 64];
        b[0] = 6;
        b[1] = 0xc1;
        state.lock().unwrap().injected = Some(openfuse_core::protocol::Report64::from_bytes(b));
        w.step(31);
        assert_eq!(live_writes(&state), 0);
        assert!(!w.live.status().enabled);
        w.command(Command::SetLive(0, binding, true), 32);
        assert!(!w.live.status().enabled);
        w.command(Command::ReadCurrent, 40);
        ready(&mut w, 40);
        w.command(Command::SetLive(0, live_binding(&w), true), 70);
        assert!(w.live.status().enabled);
    }
}

#[test]
fn full_command_channel_cannot_drop_cancellation_or_reenable_stale_commands() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    let binding = live_binding(&w);
    let changes = edits(&w);
    w.command(Command::SetLive(0, binding.clone(), true), 30);
    w.command(Command::Parameters(0, binding.clone(), changes.clone()), 31);
    let (tx, rx) = std::sync::mpsc::sync_channel(2);
    tx.try_send(Command::SetLive(0, binding.clone(), true))
        .unwrap();
    tx.try_send(Command::Parameters(0, binding.clone(), changes))
        .unwrap();
    let controller = crate::DeviceController {
        recorder: crate::trace::Recorder::new(crate::trace::Origin::Simulator),
        commands: tx,
        events: w.events.clone(),
        stop: Arc::new(AtomicBool::new(false)),
        live_epoch: w.live_epoch.clone(),
        operation_epoch: w.operation_epoch.clone(),
        tuner_epoch: w.tuner_epoch.clone(),
        tuner_stop: w.tuner_stop.clone(),
    };
    assert!(controller.refresh().is_err()); // Queue is genuinely full.
    controller.set_live(&binding, false).unwrap();
    for command in rx.try_iter() {
        w.command(command, 32);
    }
    w.step(32);
    assert!(!w.live.status().enabled);
    assert_eq!(live_writes(&state), 0);
}

mod operations;

mod bank;

mod controls;

mod auxiliary;

mod whole;
