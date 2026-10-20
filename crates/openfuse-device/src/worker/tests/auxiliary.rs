use super::*;
use crate::{OperationPhase, bank::Request};
use openfuse_core::auxiliary::{Key, Mapping, Selection, Snapshot};
fn run(w: &mut Worker, start: u64, steps: u64) {
    for now in start..start + steps {
        w.step(now);
    }
}
fn begin(w: &mut Worker, request: Request, now: u64) {
    w.command(
        Command::Bank(w.observed_operation_epoch, live_binding(w), request),
        now,
    );
}
fn take(w: &Worker) -> Snapshot {
    w.events
        .lock()
        .unwrap()
        .iter()
        .find_map(|e| {
            if let DeviceEvent::Auxiliary(s) = e {
                Some(s.snapshot.clone())
            } else {
                None
            }
        })
        .expect("auxiliary snapshot event")
}
fn capture(w: &mut Worker) -> Snapshot {
    begin(w, Request::AuxiliaryRead, 30);
    run(w, 30, 1000);
    assert_eq!(
        w.bank.status().phase,
        OperationPhase::Verified,
        "{:?}",
        w.bank.status()
    );
    take(w)
}
fn selection(snapshot: &Snapshot) -> Selection {
    Selection::new(
        snapshot,
        snapshot,
        &[
            Mapping {
                source: Key::Bank {
                    control: 1,
                    slot: 1,
                },
                target: Key::Bank {
                    control: 1,
                    slot: 9,
                },
            },
            Mapping {
                source: Key::Bank {
                    control: 2,
                    slot: 2,
                },
                target: Key::Bank {
                    control: 2,
                    slot: 8,
                },
            },
        ],
    )
    .unwrap()
}
fn write_count(state: &Arc<Mutex<State>>) -> usize {
    state
        .lock()
        .unwrap()
        .writes
        .iter()
        .filter(|r| r[0] == 0x1c)
        .count()
}
#[test]
fn complete_auxiliary_read_is_bound_after_fresh_current_and_never_writes() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    let original = w.engine.snapshot().unwrap().reports.clone();
    let snapshot = capture(&mut w);
    assert_eq!(snapshot.entries().len(), 25);
    assert_eq!(snapshot.presets().len(), 24);
    assert_eq!(
        snapshot.fingerprint(),
        Snapshot::parse(&snapshot.to_json().unwrap())
            .unwrap()
            .fingerprint()
    );
    assert_eq!(write_count(&state), 0);
    assert_eq!(w.engine.snapshot().unwrap().reports, original);
    let events = w.events.lock().unwrap();
    let event = events
        .iter()
        .find_map(|e| {
            if let DeviceEvent::Auxiliary(s) = e {
                Some(s)
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(event.generation, w.generation);
    assert_eq!(event.device, device());
    assert!(w.snapshot_current);
}
#[test]
fn auxiliary_restore_rechecks_destination_and_all_contents_before_verified() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    let before = capture(&mut w);
    let selection = selection(&before);
    state.lock().unwrap().writes.clear();
    begin(
        &mut w,
        Request::AuxiliaryRestore(Box::new(selection.clone())),
        1100,
    );
    run(&mut w, 1100, 2200);
    assert_eq!(
        w.bank.status().phase,
        OperationPhase::Verified,
        "{:?}",
        w.bank.status()
    );
    assert_eq!(w.bank.status().completed, 2);
    selection.matches_readback(&take(&w)).unwrap();
    let state = state.lock().unwrap();
    let reads: Vec<_> = state
        .writes
        .iter()
        .enumerate()
        .filter(|(_, r)| r[0] == 0x14)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(reads.len(), 2);
    let writes: Vec<_> = state
        .writes
        .iter()
        .enumerate()
        .filter(|(_, r)| r[0] == 0x1c)
        .collect();
    assert!(!writes.is_empty());
    assert!(reads[0] < writes[0].0);
    assert!(reads[1] > writes.last().unwrap().0);
    assert!(writes.iter().all(|(_, r)| r[3] != 0));
    assert!(state.writes.iter().all(|r| r[0] != 0x14 || r[3] == 0));
}
#[test]
fn auxiliary_destination_changed_since_preview_prevents_every_write() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    let before = capture(&mut w);
    let selection = selection(&before);
    state.lock().unwrap().corrupt_readback = true;
    begin(&mut w, Request::AuxiliaryRestore(Box::new(selection)), 1100);
    run(&mut w, 1100, 1000);
    assert_eq!(w.bank.status().phase, OperationPhase::Failed);
    assert_eq!(write_count(&state), 0);
    assert_eq!(w.bank.status().completed, 0);
}

#[test]
fn edited_current_effect_is_saved_to_one_bank_and_verified_without_changing_current() {
    use openfuse_core::control_preset::ControlPreset;
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    let before = capture(&mut w);
    let current = w.engine.snapshot().unwrap().reports.clone();
    let mut document = ControlPreset::from_reports(20, 1, &current).unwrap();
    document.set_name("Edited current chorus").unwrap();
    document.set_module_active("Modulation", true).unwrap();
    let selection = Selection::from_control_preset(&document, &before, 9).unwrap();
    state.lock().unwrap().writes.clear();
    begin(
        &mut w,
        Request::AuxiliaryRestore(Box::new(selection.clone())),
        1100,
    );
    run(&mut w, 1100, 2200);
    assert_eq!(
        w.bank.status().phase,
        OperationPhase::Verified,
        "{:?}",
        w.bank.status()
    );
    assert_eq!(w.bank.status().completed, 1);
    let after = take(&w);
    selection.matches_readback(&after).unwrap();
    assert_eq!(w.engine.snapshot().unwrap().reports, current);
    let entry = after
        .entries()
        .iter()
        .find(|e| {
            e.key()
                == Key::Bank {
                    control: 1,
                    slot: 9,
                }
        })
        .unwrap();
    assert_eq!(entry.label(), "MOD 9 · Edited current chorus");
    let writes = state.lock().unwrap();
    let packets: Vec<_> = writes.writes.iter().filter(|r| r[0] == 0x1c).collect();
    assert_eq!(packets.len(), 3);
    assert!(packets.iter().all(|r| r[3] == 1));
    assert!(packets.iter().all(|r| !matches!(r[2], 3 | 11 | 21)));
}

#[test]
fn edited_document_save_cannot_use_a_changed_destination() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    let before = capture(&mut w);
    let document = openfuse_core::control_preset::ControlPreset::new(20, 1, "Local edit").unwrap();
    let selection = document.plan(&before, 9).unwrap();
    state.lock().unwrap().corrupt_readback = true;
    begin(&mut w, Request::AuxiliaryRestore(Box::new(selection)), 1100);
    run(&mut w, 1100, 1000);
    assert_eq!(w.bank.status().phase, OperationPhase::Failed);
    assert_eq!(w.bank.status().completed, 0);
    assert_eq!(write_count(&state), 0);
}
#[test]
fn auxiliary_cancel_and_transport_failure_never_replay_or_complete_pending_writes() {
    for mode in 0..4 {
        let (mut w, state) = worker();
        w.command(Command::Connect(device()), 0);
        ready(&mut w, 0);
        let before = capture(&mut w);
        begin(
            &mut w,
            Request::AuxiliaryRestore(Box::new(selection(&before))),
            1100,
        );
        let mut now = 1100;
        while write_count(&state) == 0 {
            w.step(now);
            now += 1;
            assert!(now < 2200);
        }
        match mode {
            0 => {
                w.operation_epoch.fetch_add(1, Ordering::AcqRel);
            }
            1 => state.lock().unwrap().fail_read = true,
            2 => state.lock().unwrap().operation_ack_status = Some(4),
            _ => state.lock().unwrap().silent = true,
        }
        run(&mut w, now, 20);
        w.step(now + 6000);
        state.lock().unwrap().silent = false;
        let count = write_count(&state);
        if let Some(due) = w.retry_at {
            run(&mut w, due, 40);
        }
        assert_eq!(write_count(&state), count);
        assert_eq!(w.bank.status().completed, 0);
        assert!(matches!(
            w.bank.status().phase,
            OperationPhase::Failed | OperationPhase::Cancelled
        ));
    }
}
#[test]
fn auxiliary_stale_binding_and_queued_cancel_send_nothing() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    let count = state.lock().unwrap().writes.len();
    let mut binding = live_binding(&w);
    binding.generation += 1;
    w.command(Command::Bank(0, binding, Request::AuxiliaryRead), 30);
    run(&mut w, 30, 10);
    assert_eq!(state.lock().unwrap().writes.len(), count);
    w.operation_epoch.fetch_add(1, Ordering::AcqRel);
    w.command(
        Command::Bank(0, live_binding(&w), Request::AuxiliaryRead),
        50,
    );
    run(&mut w, 50, 10);
    assert_eq!(state.lock().unwrap().writes.len(), count);
    assert_eq!(w.bank.status().phase, OperationPhase::Cancelled);
}
#[test]
fn auxiliary_results_are_not_rebound_after_disconnect_during_followup() {
    let (mut w, _state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    begin(&mut w, Request::AuxiliaryRead, 30);
    let mut now = 30;
    while w.bank.status().busy() {
        w.step(now);
        now += 1;
        assert!(now < 1000);
    }
    assert!(!w.snapshot_current);
    w.command(Command::Disconnect, now);
    w.command(Command::Connect(device()), now + 1);
    run(&mut w, now + 1, 40);
    assert!(
        !w.events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, DeviceEvent::Auxiliary(_)))
    );
}

#[test]
fn superchamp_has_voice_clean_and_fx_without_full_preset_slots() {
    let mut backend = Simulator::superchamp();
    let device = backend.enumerate().unwrap().remove(0);
    let mut w = Worker::new(Box::new(backend), Arc::new(Mutex::new(Default::default())));
    w.command(Command::Connect(device), 0);
    run(&mut w, 0, 30);
    assert!(w.snapshot_current);
    let snapshot = capture(&mut w);
    assert_eq!(snapshot.entries().len(), 32);
    assert!(snapshot.presets().is_empty());
    assert!(snapshot.entries().iter().any(|e| e.key()
        == Key::Bank {
            control: 1,
            slot: 16
        }));
    assert!(snapshot.entries().iter().all(|e| e.key() != Key::System));
    let selection = Selection::new(
        &snapshot,
        &snapshot,
        &[
            Mapping {
                source: Key::Bank {
                    control: 1,
                    slot: 1,
                },
                target: Key::Bank {
                    control: 1,
                    slot: 3,
                },
            },
            Mapping {
                source: Key::Bank {
                    control: 2,
                    slot: 1,
                },
                target: Key::Bank {
                    control: 2,
                    slot: 2,
                },
            },
        ],
    )
    .unwrap();
    begin(&mut w, Request::AuxiliaryRestore(Box::new(selection)), 1100);
    run(&mut w, 1100, 1500);
    assert_eq!(
        w.bank.status().phase,
        OperationPhase::Verified,
        "{:?}",
        w.bank.status()
    );
    assert_eq!(w.bank.status().completed, 2);
}

#[test]
fn corrupt_postwrite_readback_never_claims_verified() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    let before = capture(&mut w);
    begin(
        &mut w,
        Request::AuxiliaryRestore(Box::new(selection(&before))),
        1100,
    );
    let mut now = 1100;
    while write_count(&state) == 0 {
        w.step(now);
        now += 1;
        assert!(now < 2200);
    }
    state.lock().unwrap().corrupt_readback = true;
    run(&mut w, now, 1000);
    assert_eq!(w.bank.status().phase, OperationPhase::Failed);
    assert_eq!(w.bank.status().completed, 0);
}
