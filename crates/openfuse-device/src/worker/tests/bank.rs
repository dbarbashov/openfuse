use super::*;
use crate::{OperationPhase, bank::Request};
fn run(w: &mut Worker, start: u64, steps: u64) {
    for t in start..start + steps {
        w.step(t);
    }
}
fn begin(w: &mut Worker, request: Request, now: u64) {
    w.command(Command::Bank(0, live_binding(w), request), now);
}
fn saved_writes(state: &Arc<Mutex<State>>) -> usize {
    state
        .lock()
        .unwrap()
        .writes
        .iter()
        .filter(|b| b[0] == 0x1c && b[2] == 3)
        .count()
}
fn take_backup(w: &Worker) -> openfuse_core::bank::Backup {
    w.events
        .lock()
        .unwrap()
        .iter()
        .find_map(|e| {
            if let DeviceEvent::Backup(b) = e {
                Some(b.backup.clone())
            } else {
                None
            }
        })
        .expect("complete backup event")
}
#[test]
fn named_bank_is_complete_freshly_bound_and_does_not_select_slots() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    begin(&mut w, Request::Names, 30);
    run(&mut w, 30, 100);
    assert_eq!(w.bank.status().phase, OperationPhase::Verified);
    let events = w.events.lock().unwrap();
    let names = events
        .iter()
        .find_map(|e| {
            if let DeviceEvent::BankNames(n) = e {
                Some(n)
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(names.entries.len(), 24);
    assert_eq!(names.generation, w.generation);
    assert_eq!(w.generation, 2);
    assert_eq!(names.entries[&7], "Simulated clean preset");
    assert!(state.lock().unwrap().writes.iter().all(|b| b[0] != 0x1c));
}
#[test]
fn backup_is_complete_without_mutating_working_sound_and_roundtrips() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    let before = w.engine.snapshot().unwrap().reports.clone();
    begin(&mut w, Request::Backup, 30);
    run(&mut w, 30, 400);
    assert_eq!(
        w.bank.status().phase,
        OperationPhase::Verified,
        "{:?}",
        w.bank.status()
    );
    let backup = take_backup(&w);
    assert_eq!(backup.slots().len(), 24);
    assert_eq!(
        openfuse_core::bank::Backup::parse(&backup.to_json().unwrap())
            .unwrap()
            .slots()
            .len(),
        24
    );
    assert_eq!(w.engine.snapshot().unwrap().reports, before);
    assert!(state.lock().unwrap().writes.iter().all(|b| b[0] != 0x1c));
}
#[test]
fn save_reads_actual_stored_target_before_reporting_verified() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    begin(&mut w, Request::Save(23), 30);
    run(&mut w, 30, 80);
    assert_eq!(
        w.bank.status().phase,
        OperationPhase::Verified,
        "{:?}",
        w.bank.status()
    );
    assert_eq!(saved_writes(&state), 1);
    assert_eq!(w.bank.status().completed, 1);
    assert!(
        state
            .lock()
            .unwrap()
            .writes
            .iter()
            .any(|b| b[0] == 0x14 && b[2] == 23 && b[4] == 0)
    );
    w.command(Command::SelectSlot(0, live_binding(&w), 23), 120);
    run(&mut w, 120, 70);
    assert_eq!(w.engine.snapshot().unwrap().name, "Simulated clean preset");
}
#[test]
fn restore_verifies_all_slots_and_stops_on_readback_corruption() {
    for corrupt in [false, true] {
        let (mut w, state) = worker();
        w.command(Command::Connect(device()), 0);
        ready(&mut w, 0);
        begin(&mut w, Request::Backup, 30);
        run(&mut w, 30, 400);
        let backup = take_backup(&w);
        state.lock().unwrap().corrupt_readback = corrupt;
        begin(&mut w, Request::Restore(Box::new(backup)), 500);
        run(&mut w, 500, 1000);
        if corrupt {
            assert_eq!(w.bank.status().phase, OperationPhase::Failed);
            assert_eq!(saved_writes(&state), 1);
            assert_eq!(w.bank.status().completed, 0);
        } else {
            assert_eq!(
                w.bank.status().phase,
                OperationPhase::Verified,
                "{:?}",
                w.bank.status()
            );
            assert_eq!(saved_writes(&state), 24);
            assert_eq!(w.bank.status().completed, 24);
        }
    }
}
#[test]
fn stale_or_invalid_save_and_concurrent_actions_cannot_write() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    begin(&mut w, Request::Save(24), 30);
    assert_eq!(w.bank.status().phase, OperationPhase::Failed);
    let mut binding = live_binding(&w);
    binding.generation += 1;
    w.command(Command::Bank(0, binding, Request::Save(0)), 31);
    assert_eq!(saved_writes(&state), 0);
    let binding = live_binding(&w);
    begin(&mut w, Request::Backup, 40);
    w.command(Command::Bank(0, binding.clone(), Request::Save(0)), 41);
    w.command(Command::SelectSlot(0, binding.clone(), 0), 42);
    w.command(Command::SetLive(0, binding, true), 43);
    run(&mut w, 40, 400);
    assert_eq!(saved_writes(&state), 0);
    assert!(!w.live.status().enabled);
    assert_eq!(w.bank.status().phase, OperationPhase::Verified);
}
#[test]
fn failed_ack_timeout_and_disconnect_never_replay_persistent_save() {
    for mode in 0..3 {
        let (mut w, state) = worker();
        w.command(Command::Connect(device()), 0);
        ready(&mut w, 0);
        begin(&mut w, Request::Save(0), 30);
        w.step(30);
        w.step(31);
        match mode {
            0 => state.lock().unwrap().operation_ack_status = Some(4),
            1 => state.lock().unwrap().silent = true,
            _ => state.lock().unwrap().fail_read = true,
        };
        w.step(32);
        w.step(5033);
        state.lock().unwrap().silent = false;
        if let Some(due) = w.retry_at {
            run(&mut w, due, 30);
        }
        assert_eq!(w.bank.status().phase, OperationPhase::Failed);
        assert_eq!(saved_writes(&state), 1);
        assert_eq!(w.bank.status().completed, 0);
    }
}
#[test]
fn cancel_read_or_write_discards_stream_and_requires_fresh_handshake() {
    for request in [Request::Names, Request::Backup, Request::Save(2)] {
        let (mut w, state) = worker();
        w.command(Command::Connect(device()), 0);
        ready(&mut w, 0);
        begin(&mut w, request, 30);
        w.step(30);
        w.operation_epoch.fetch_add(1, Ordering::AcqRel);
        w.step(31);
        assert_eq!(w.bank.status().phase, OperationPhase::Cancelled);
        let due = w.retry_at.unwrap();
        run(&mut w, due, 30);
        assert_eq!(state.lock().unwrap().opens, 2);
        assert!(w.snapshot_current);
        assert_eq!(w.bank.status().phase, OperationPhase::Cancelled);
        assert!(
            state
                .lock()
                .unwrap()
                .writes
                .iter()
                .all(|b| !(b[0] == 0x1c && b[2] == 0))
        );
    }
}
#[test]
fn queued_bank_cancel_and_external_change_before_dispatch_emit_no_write() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    let binding = live_binding(&w);
    w.operation_epoch.fetch_add(1, Ordering::AcqRel);
    w.command(Command::Bank(0, binding.clone(), Request::Save(1)), 30);
    w.step(30);
    assert_eq!(w.bank.status().phase, OperationPhase::Cancelled);
    assert_eq!(saved_writes(&state), 0);
    w.command(Command::Bank(1, binding, Request::Save(1)), 31);
    let mut b = [0; 64];
    b[0] = 5;
    b[1] = 0xc1;
    state.lock().unwrap().injected = Some(openfuse_core::protocol::Report64::from_bytes(b));
    w.step(31);
    assert_eq!(w.bank.status().phase, OperationPhase::Failed);
    assert_eq!(saved_writes(&state), 0);
}
#[test]
fn acknowledged_live_edit_requires_fresh_capture_before_persistent_save() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    let binding = live_binding(&w);
    let changes = edits(&w);
    w.command(Command::SetLive(0, binding.clone(), true), 30);
    w.command(Command::Parameters(0, binding, changes), 31);
    run(&mut w, 31, 10);
    assert_eq!(w.live.status().pending, 0);
    assert!(w.live.status().acknowledged > 0);
    begin(&mut w, Request::Save(2), 50);
    assert_eq!(w.bank.status().phase, OperationPhase::Failed);
    assert_eq!(saved_writes(&state), 0);
    w.command(Command::ReadCurrent, 60);
    ready(&mut w, 60);
    begin(&mut w, Request::Save(2), 90);
    run(&mut w, 90, 80);
    assert_eq!(w.bank.status().phase, OperationPhase::Verified);
    assert_eq!(saved_writes(&state), 1);
}
#[test]
fn deferred_result_cannot_be_relabelled_after_disconnect_or_loss() {
    for disconnect in [true, false] {
        let (mut w, state) = worker();
        w.command(Command::Connect(device()), 0);
        ready(&mut w, 0);
        begin(&mut w, Request::Names, 30);
        let mut now = 30;
        while w.bank.status().busy() {
            w.step(now);
            now += 1;
            assert!(now < 100);
        }
        assert!(w.bank.names.is_some());
        assert!(!w.snapshot_current);
        if disconnect {
            w.command(Command::Disconnect, now);
            w.command(Command::Connect(device()), now + 1);
            run(&mut w, now + 1, 30);
        } else {
            state.lock().unwrap().fail_read = true;
            w.step(now);
            let due = w.retry_at.unwrap();
            run(&mut w, due, 30);
        }
        assert!(w.bank.names.is_none());
        assert!(
            !w.events
                .lock()
                .unwrap()
                .iter()
                .any(|e| matches!(e, DeviceEvent::BankNames(_)))
        );
    }
}
#[test]
fn complete_archive_is_delivered_even_when_followup_current_read_fails() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    begin(&mut w, Request::Backup, 30);
    let mut now = 30;
    while w.bank.status().busy() {
        w.step(now);
        now += 1;
        assert!(now < 500);
    }
    assert!(!w.snapshot_current);
    assert_eq!(take_backup(&w).slots().len(), 24);
    state.lock().unwrap().fail_read = true;
    w.step(now);
    assert_eq!(take_backup(&w).slots().len(), 24);
    let e = w.events.lock().unwrap();
    let source = e
        .iter()
        .find_map(|event| {
            if let DeviceEvent::Backup(b) = event {
                Some(&b.device)
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(source, &device());
}

#[path = "bank_selection.rs"]
mod selection;
