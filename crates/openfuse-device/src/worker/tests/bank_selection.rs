use super::*;
use openfuse_core::bank::{Backup, RestoreSelection, SlotMapping};

fn read_selected(w: &mut Worker, slots: Vec<u16>) -> Backup {
    begin(w, Request::BackupSlots(slots), 30);
    run(w, 30, 150);
    assert_eq!(w.bank.status().phase, OperationPhase::Verified);
    take_backup(w)
}
fn targets(state: &Arc<Mutex<State>>, id: u8, kind: u8) -> Vec<u16> {
    state
        .lock()
        .unwrap()
        .writes
        .iter()
        .filter(|b| b[0] == id && b[2] == kind)
        .map(|b| u16::from_le_bytes([b[4], b[5]]))
        .collect()
}

#[test]
fn noncontiguous_and_single_slot_backups_read_exact_selection_without_writes() {
    for slots in [vec![23], vec![23, 2, 9]] {
        let (mut w, state) = worker();
        w.command(Command::Connect(device()), 0);
        ready(&mut w, 0);
        let original = w.engine.snapshot().unwrap().reports.clone();
        let archive = read_selected(&mut w, slots.clone());
        let mut expected = slots;
        expected.sort_unstable();
        let reads: Vec<_> = state
            .lock()
            .unwrap()
            .writes
            .iter()
            .filter(|r| r[0] == 0x14)
            .map(|r| u16::from(r[2]))
            .collect();
        assert_eq!(reads, expected);
        assert_eq!(
            archive.slots().iter().map(|s| s.slot()).collect::<Vec<_>>(),
            expected
        );
        assert!(!archive.is_complete());
        assert_eq!(
            Backup::parse(&archive.to_json().unwrap())
                .unwrap()
                .slots()
                .len(),
            expected.len()
        );
        assert_eq!(w.bank.status().completed, expected.len());
        assert_eq!(w.engine.snapshot().unwrap().reports, original);
        assert!(state.lock().unwrap().writes.iter().all(|r| r[0] != 0x1c));
    }
}

#[test]
fn invalid_backup_selection_emits_no_requests_or_partial_results() {
    for slots in [vec![], vec![0, 0], vec![0, 24], vec![65535]] {
        let (mut w, state) = worker();
        w.command(Command::Connect(device()), 0);
        ready(&mut w, 0);
        let count = state.lock().unwrap().writes.len();
        begin(&mut w, Request::BackupSlots(slots), 30);
        run(&mut w, 30, 10);
        assert_eq!(w.bank.status().phase, OperationPhase::Failed);
        assert_eq!(state.lock().unwrap().writes.len(), count);
        assert!(
            !w.events
                .lock()
                .unwrap()
                .iter()
                .any(|e| matches!(e, DeviceEvent::Backup(_)))
        );
    }
}

#[test]
fn mapped_restore_writes_only_destinations_and_verifies_each_before_next() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    let backup = read_selected(&mut w, vec![2, 9]);
    let selection = RestoreSelection::new(
        &backup,
        device().product_id,
        &[
            SlotMapping {
                source: 9,
                target: 22,
            },
            SlotMapping {
                source: 2,
                target: 3,
            },
        ],
    )
    .unwrap();
    state.lock().unwrap().writes.clear();
    begin(&mut w, Request::RestoreSelection(Box::new(selection)), 200);
    run(&mut w, 200, 160);
    assert_eq!(
        w.bank.status().phase,
        OperationPhase::Verified,
        "{:?}",
        w.bank.status()
    );
    assert_eq!(w.bank.status().total, 2);
    assert_eq!(w.bank.status().completed, 2);
    assert_eq!(targets(&state, 0x1c, 3), vec![22, 3]);
    let ordered: Vec<_> = state
        .lock()
        .unwrap()
        .writes
        .iter()
        .filter_map(|r| {
            if r[0] == 0x1c && r[2] == 3 {
                Some(("save", r[4]))
            } else if r[0] == 0x14 {
                Some(("read", r[2]))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        ordered,
        vec![("save", 22), ("read", 22), ("save", 3), ("read", 3)]
    );
}

#[test]
fn partial_archive_never_implicitly_enters_full_restore() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    let backup = read_selected(&mut w, vec![9]);
    let count = state.lock().unwrap().writes.len();
    begin(&mut w, Request::Restore(Box::new(backup)), 200);
    run(&mut w, 200, 10);
    assert_eq!(w.bank.status().phase, OperationPhase::Failed);
    assert_eq!(state.lock().unwrap().writes.len(), count);
}

#[test]
fn mapped_restore_stops_after_first_corrupt_readback() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    let backup = read_selected(&mut w, vec![1, 9]);
    let selection = RestoreSelection::new(
        &backup,
        device().product_id,
        &[
            SlotMapping {
                source: 1,
                target: 6,
            },
            SlotMapping {
                source: 9,
                target: 7,
            },
        ],
    )
    .unwrap();
    state.lock().unwrap().corrupt_readback = true;
    begin(&mut w, Request::RestoreSelection(Box::new(selection)), 200);
    run(&mut w, 200, 150);
    assert_eq!(w.bank.status().phase, OperationPhase::Failed);
    assert_eq!(w.bank.status().completed, 0);
    assert_eq!(targets(&state, 0x1c, 3), vec![6]);
}

#[test]
fn mapped_restore_cancellation_retains_verified_count_without_replay() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    let backup = read_selected(&mut w, vec![1, 9]);
    let selection = RestoreSelection::new(
        &backup,
        device().product_id,
        &[
            SlotMapping {
                source: 1,
                target: 6,
            },
            SlotMapping {
                source: 9,
                target: 7,
            },
        ],
    )
    .unwrap();
    begin(&mut w, Request::RestoreSelection(Box::new(selection)), 200);
    let mut now = 200;
    while w.bank.status().completed == 0 {
        w.step(now);
        now += 1;
        assert!(now < 300);
    }
    w.operation_epoch.fetch_add(1, Ordering::AcqRel);
    w.step(now);
    assert_eq!(w.bank.status().phase, OperationPhase::Cancelled);
    assert_eq!(w.bank.status().completed, 1);
    assert_eq!(targets(&state, 0x1c, 3), vec![6]);
    if let Some(due) = w.retry_at {
        run(&mut w, due, 40);
    }
    assert_eq!(targets(&state, 0x1c, 3), vec![6]);
    assert_eq!(w.bank.status().phase, OperationPhase::Cancelled);
}

#[test]
fn incomplete_selected_backup_does_not_publish_successful_partial_archive() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    begin(&mut w, Request::BackupSlots(vec![1, 9]), 30);
    let mut now = 30;
    while w.bank.status().completed == 0 {
        w.step(now);
        now += 1;
        assert!(now < 100);
    }
    state.lock().unwrap().fail_read = true;
    w.step(now);
    assert_eq!(w.bank.status().phase, OperationPhase::Failed);
    assert_eq!(w.bank.status().completed, 1);
    assert!(
        !w.events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, DeviceEvent::Backup(_)))
    );
}

#[test]
fn legacy_xml_restore_preflights_all_sources_before_any_persistent_write() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    let backup = read_selected(&mut w, vec![1, 9]);
    let good = backup.slots()[0].document().clone();
    let mut bad = good.clone();
    let mut metadata = bad.metadata();
    metadata.name = "Cannot encode 🎸".into();
    bad.set_metadata(&metadata).unwrap();
    let selection = RestoreSelection::from_documents(
        device().product_id,
        vec![(1, good), (9, bad)],
        device().product_id,
        &[
            SlotMapping {
                source: 1,
                target: 6,
            },
            SlotMapping {
                source: 9,
                target: 7,
            },
        ],
    )
    .unwrap();
    let writes = state.lock().unwrap().writes.len();
    begin(&mut w, Request::RestoreSelection(Box::new(selection)), 200);
    run(&mut w, 200, 50);
    assert_eq!(w.bank.status().phase, OperationPhase::Failed);
    assert_eq!(state.lock().unwrap().writes.len(), writes);
    assert_eq!(saved_writes(&state), 0);
}

#[test]
fn mapped_restore_rejects_stale_binding_and_cancelled_queued_command() {
    for cancel in [false, true] {
        let (mut w, state) = worker();
        w.command(Command::Connect(device()), 0);
        ready(&mut w, 0);
        let backup = read_selected(&mut w, vec![1]);
        let selection = RestoreSelection::new(
            &backup,
            device().product_id,
            &[SlotMapping {
                source: 1,
                target: 6,
            }],
        )
        .unwrap();
        let mut binding = live_binding(&w);
        if cancel {
            w.operation_epoch.fetch_add(1, Ordering::AcqRel);
        } else {
            binding.generation += 1;
        }
        w.command(
            Command::Bank(0, binding, Request::RestoreSelection(Box::new(selection))),
            200,
        );
        run(&mut w, 200, 40);
        assert_eq!(saved_writes(&state), 0);
        assert_eq!(
            w.bank.status().phase,
            if cancel {
                OperationPhase::Cancelled
            } else {
                OperationPhase::Failed
            }
        );
    }
}
