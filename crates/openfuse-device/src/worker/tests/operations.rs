use super::*;
use crate::OperationPhase;
use openfuse_core::{audition, preset::PresetDocument};
fn desired(w: &Worker) -> PresetDocument {
    let mut doc = openfuse_core::wire_preset::decode(20, &w.engine.snapshot().unwrap().reports)
        .unwrap()
        .document;
    let mut metadata = doc.metadata();
    metadata.name = "Audition change".into();
    doc.set_metadata(&metadata).unwrap();
    for (key, category) in [
        (
            "Amplifier",
            openfuse_core::model_catalog::ModuleCategory::Amplifier,
        ),
        (
            "Stompbox",
            openfuse_core::model_catalog::ModuleCategory::Stompbox,
        ),
    ] {
        let old = doc
            .modules()
            .iter()
            .find(|m| m.key == key)
            .unwrap()
            .model_id;
        let model = openfuse_core::model_catalog::models(13, category)
            .find(|m| m.id != 0 && Some(m.id) != old)
            .unwrap();
        let parameters = model
            .parameters
            .iter()
            .map(|p| openfuse_core::preset::PresetParameter {
                index: p.index,
                value: p.default_raw,
            })
            .collect::<Vec<_>>();
        doc.replace_module_model(key, model.id, &parameters, Some(model.bypass_mode))
            .unwrap();
    }
    doc.move_module("Delay", 2).unwrap();
    doc.set_parameter("Amplifier", 0, 12345).unwrap();
    doc.set_module_active("Stompbox", true).unwrap();
    doc
}
fn start(w: &mut Worker) {
    let doc = desired(w);
    w.command(Command::ApplyPreset(0, live_binding(w), Box::new(doc)), 30);
    assert_eq!(w.operation.status().phase, OperationPhase::Sending);
}
fn complete(w: &mut Worker, start: u64) {
    for t in start..start + 80 {
        w.step(t);
    }
}
fn working_writes(state: &Arc<Mutex<State>>) -> usize {
    state
        .lock()
        .unwrap()
        .writes
        .iter()
        .filter(|b| b[0] == 0x1c)
        .count()
}
#[test]
fn whole_audition_requires_final_ack_and_verified_fresh_read() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    let doc = desired(&w);
    let plan = audition::preset(20, &doc, &w.engine.snapshot().unwrap().reports).unwrap();
    start(&mut w);
    assert!(w.last_status.as_ref().unwrap().snapshot.is_none());
    complete(&mut w, 31);
    assert_eq!(
        w.operation.status().phase,
        OperationPhase::Verified,
        "{:?}",
        w.operation.status()
    );
    assert_eq!(w.operation.status().readback_generation, Some(2));
    assert_eq!(working_writes(&state), plan.steps().len());
    assert_eq!(w.engine.snapshot().unwrap().name, "Audition change");
    assert!(!w.live.status().enabled);
    let writes = &state.lock().unwrap().writes;
    assert!(writes.iter().all(|b| b[0] != 0x1c || b[2] != 3)); // Never SaveName/persistent save.
    assert_eq!(writes.last().unwrap()[..2], [0xff, 0xc5]);
}
#[test]
fn slot_selection_uses_mustang_change_and_confirms_read_position() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    w.command(Command::SelectSlot(0, live_binding(&w), 23), 30);
    complete(&mut w, 31);
    assert_eq!(
        w.operation.status().phase,
        OperationPhase::Verified,
        "{:?}",
        w.operation.status()
    );
    assert_eq!(w.engine.snapshot().unwrap().position, 23);
    assert_eq!(working_writes(&state), 1);
    let state = state.lock().unwrap();
    let packet = state.writes.iter().find(|b| b[0] == 0x1c).unwrap();
    assert_eq!(packet[..8], [0x1c, 0xc3, 1, 0, 23, 0, 1, 0]);
    assert!(!state.writes.iter().any(|b| b[0] == 0x13));
}
#[test]
fn malformed_stale_and_invalid_slot_requests_transmit_nothing() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    w.command(Command::SelectSlot(0, live_binding(&w), 24), 30);
    assert_eq!(w.operation.status().phase, OperationPhase::Failed);
    let mut old = live_binding(&w);
    old.generation = 0;
    let doc = desired(&w);
    w.command(Command::ApplyPreset(0, old, Box::new(doc)), 31);
    w.step(32);
    assert_eq!(working_writes(&state), 0);
    assert_eq!(w.operation.status().phase, OperationPhase::Failed);
}
#[test]
fn failed_or_missing_ack_stops_batch_and_late_ack_never_resumes() {
    for status in [None, Some(1), Some(3), Some(4), Some(255)] {
        let (mut w, state) = worker();
        w.command(Command::Connect(device()), 0);
        ready(&mut w, 0);
        start(&mut w);
        state.lock().unwrap().operation_ack_status = status;
        state.lock().unwrap().silent = status.is_none();
        w.step(31);
        w.step(32);
        w.step(33);
        w.step(6000);
        assert_eq!(w.operation.status().phase, OperationPhase::Failed);
        assert_eq!(working_writes(&state), 2);
        state.lock().unwrap().silent = false;
        w.step(6001);
        w.step(6002);
        assert_eq!(working_writes(&state), 2);
        assert!(!w.snapshot_current);
    }
}
#[test]
fn mismatch_is_not_reported_verified_and_capture_is_available_for_review() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    start(&mut w);
    state.lock().unwrap().corrupt_readback = true;
    complete(&mut w, 31);
    assert_eq!(w.operation.status().phase, OperationPhase::Failed);
    assert_eq!(w.operation.status().readback_generation, Some(2));
    assert!(w.operation.status().detail.contains("Readback differs"));
    assert_eq!(w.generation, 2);
}
#[test]
fn read_timeout_never_reports_verified_after_successful_acks() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    start(&mut w);
    for t in 31..80 {
        w.step(t);
        if w.operation.status().phase == OperationPhase::Reading {
            break;
        }
    }
    assert_eq!(w.operation.status().phase, OperationPhase::Reading);
    state.lock().unwrap().silent = true;
    w.step(100);
    w.step(16000);
    assert_eq!(w.operation.status().phase, OperationPhase::Failed);
    assert_eq!(w.generation, 1);
}
#[test]
fn reconnect_after_partial_audition_is_read_only_and_never_replays() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    start(&mut w);
    w.step(31);
    w.step(32);
    state.lock().unwrap().fail_read = true;
    w.step(33);
    let count = working_writes(&state);
    let due = w.retry_at.unwrap();
    ready(&mut w, due);
    assert_eq!(w.operation.status().phase, OperationPhase::Failed);
    assert_eq!(working_writes(&state), count);
    assert_eq!(w.generation, 2);
}
#[test]
fn cancellation_epoch_discards_queued_or_inflight_transactions() {
    for begun in [false, true] {
        let (mut w, state) = worker();
        w.command(Command::Connect(device()), 0);
        ready(&mut w, 0);
        let binding = live_binding(&w);
        let doc = desired(&w);
        if begun {
            start(&mut w);
            w.step(31);
            w.step(32);
        }
        let before = working_writes(&state);
        w.operation_epoch.fetch_add(1, Ordering::AcqRel);
        w.command(Command::ApplyPreset(0, binding, Box::new(doc)), 33);
        complete(&mut w, 34);
        assert_eq!(working_writes(&state), before);
        assert_eq!(w.operation.status().phase, OperationPhase::Cancelled);
    }
}
#[test]
fn arrived_external_change_prevents_first_or_next_transaction_write() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    start(&mut w);
    let mut b = [0; 64];
    b[0] = 0x19;
    b[1] = 0xc1;
    state.lock().unwrap().injected = Some(openfuse_core::protocol::Report64::from_bytes(b));
    w.step(31);
    assert_eq!(working_writes(&state), 0);
    assert_eq!(w.operation.status().phase, OperationPhase::Failed);
}
#[test]
fn saturated_command_channel_cannot_prevent_operation_cancellation() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    start(&mut w);
    w.step(31);
    let binding = live_binding(&w);
    let doc = desired(&w);
    let (tx, rx) = std::sync::mpsc::sync_channel(2);
    tx.try_send(Command::ApplyPreset(0, binding.clone(), Box::new(doc)))
        .unwrap();
    tx.try_send(Command::SelectSlot(0, binding, 3)).unwrap();
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
    assert!(controller.refresh().is_err());
    controller.cancel_operation().unwrap();
    let before = working_writes(&state);
    for command in rx.try_iter() {
        w.command(command, 32);
    }
    complete(&mut w, 33);
    assert_eq!(working_writes(&state), before);
    assert_eq!(w.operation.status().phase, OperationPhase::Cancelled);
}
#[test]
fn live_enable_and_second_operation_cannot_interleave_with_audition() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    let binding = live_binding(&w);
    start(&mut w);
    w.command(Command::SetLive(0, binding.clone(), true), 31);
    w.command(Command::SelectSlot(0, binding, 4), 32);
    assert!(!w.live.status().enabled);
    complete(&mut w, 33);
    assert_eq!(w.operation.status().phase, OperationPhase::Verified);
    assert_eq!(live_writes(&state), 0);
    assert_eq!(w.engine.snapshot().unwrap().position, 7);
    assert!(
        !state
            .lock()
            .unwrap()
            .writes
            .iter()
            .any(|b| b[0] == 0x1c && b[2] == 1)
    );
}
#[test]
fn cancelled_unacknowledged_live_delivery_blocks_audition_until_fresh_read() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    let binding = live_binding(&w);
    let changes = edits(&w);
    w.command(Command::SetLive(0, binding.clone(), true), 30);
    w.command(Command::Parameters(0, binding.clone(), changes), 31);
    state.lock().unwrap().silent = true;
    w.step(31);
    w.live_epoch.fetch_add(1, Ordering::AcqRel);
    let doc = desired(&w);
    w.command(Command::ApplyPreset(0, binding, Box::new(doc)), 32);
    w.step(32);
    assert_eq!(w.operation.status().phase, OperationPhase::Failed);
    assert_eq!(working_writes(&state), 0);
}
#[test]
fn interrupted_transfer_reopens_read_only_without_synthetic_complete() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    start(&mut w);
    w.step(31);
    w.step(32);
    w.operation_epoch.fetch_add(1, Ordering::AcqRel);
    w.step(33);
    assert_eq!(w.operation.status().phase, OperationPhase::Cancelled);
    assert!(w.connection.is_none());
    let count = working_writes(&state);
    let due = w.retry_at.unwrap();
    ready(&mut w, due);
    assert_eq!(w.operation.status().phase, OperationPhase::Cancelled);
    assert_eq!(w.generation, 2);
    assert_eq!(working_writes(&state), count);
    let state = state.lock().unwrap();
    assert!(!state.writes.iter().any(|b| b[0] == 0x1c && b[2] == 0));
    assert_eq!(state.opens, 2);
}
#[test]
fn simulated_audition_is_working_state_and_reselecting_slot_restores_bank_copy() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    let original = w.engine.snapshot().unwrap().name.clone();
    start(&mut w);
    complete(&mut w, 31);
    assert_eq!(w.engine.snapshot().unwrap().name, "Audition change");
    w.command(Command::SelectSlot(0, live_binding(&w), 7), 120);
    complete(&mut w, 121);
    assert_eq!(w.operation.status().phase, OperationPhase::Verified);
    assert_eq!(w.engine.snapshot().unwrap().name, original);
    assert_eq!(w.generation, 3);
    assert!(
        !state
            .lock()
            .unwrap()
            .writes
            .iter()
            .any(|b| b[0] == 0x1c && b[2] == 3)
    );
}
