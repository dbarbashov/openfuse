use super::*;
use crate::{OperationPhase, bank::Request};
use openfuse_core::auxiliary::{Snapshot, WholeRestore};
fn run(w: &mut Worker, start: u64, count: u64) {
    for now in start..start + count {
        w.step(now);
    }
}
fn begin(w: &mut Worker, request: Request, now: u64) {
    w.command(
        Command::Bank(w.observed_operation_epoch, live_binding(w), request),
        now,
    );
}
fn captured(w: &Worker) -> Snapshot {
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
        .expect("capture")
}
fn prepare(w: &mut Worker) -> WholeRestore {
    begin(w, Request::AuxiliaryRead, 30);
    run(w, 30, 1000);
    assert_eq!(w.bank.status().phase, OperationPhase::Verified);
    let source = captured(w);
    WholeRestore::new(&source, &source).unwrap()
}
fn writes(state: &Arc<Mutex<State>>) -> usize {
    state
        .lock()
        .unwrap()
        .writes
        .iter()
        .filter(|r| r[0] == 0x1c || r[0] == 0x14 && r[3] != 0)
        .count()
}
#[test]
fn whole_restore_reads_before_envelope_then_verifies_all_sections_and_refreshes_current() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    let plan = prepare(&mut w);
    state.lock().unwrap().writes.clear();
    begin(&mut w, Request::WholeRestore(Box::new(plan.clone())), 1100);
    run(&mut w, 1100, 4000);
    assert_eq!(
        w.bank.status().phase,
        OperationPhase::Verified,
        "{:?}",
        w.bank.status()
    );
    assert_eq!(w.bank.status().completed, 49);
    assert!(w.snapshot_current);
    plan.matches_readback(&captured(&w)).unwrap();
    let state = state.lock().unwrap();
    let sent = &state.writes;
    let begin = sent.iter().position(|b| b[0] == 0x14 && b[3] == 1).unwrap();
    let end = sent.iter().position(|b| b[0] == 0x14 && b[3] == 2).unwrap();
    assert!(sent[..begin].iter().any(|b| b[0] == 0x14 && b[3] == 0));
    assert!(sent[end + 1..].iter().any(|b| b[0] == 0x14 && b[3] == 0));
    assert_eq!(
        sent.iter().filter(|b| b[0] == 0x1c && b[2] == 11).count(),
        1
    );
    let actual: Vec<_> = sent[begin..=end].to_vec();
    let expected: Vec<_> = plan.steps().iter().map(|s| *s.report.as_bytes()).collect();
    assert_eq!(actual, expected);
}
#[test]
fn stale_full_destination_blocks_even_begin_restore_envelope() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    let plan = prepare(&mut w);
    state.lock().unwrap().writes.clear();
    state.lock().unwrap().corrupt_readback = true;
    begin(&mut w, Request::WholeRestore(Box::new(plan)), 1100);
    run(&mut w, 1100, 1500);
    assert_eq!(w.bank.status().phase, OperationPhase::Failed);
    assert_eq!(writes(&state), 0);
}
#[test]
fn partial_whole_restore_cancel_nack_disconnect_timeout_close_without_replay() {
    for mode in 0..4 {
        let (mut w, state) = worker();
        w.command(Command::Connect(device()), 0);
        ready(&mut w, 0);
        let plan = prepare(&mut w);
        state.lock().unwrap().writes.clear();
        begin(&mut w, Request::WholeRestore(Box::new(plan)), 1100);
        let mut now = 1100;
        while !state.lock().unwrap().writes.iter().any(|r| r[0] == 0x1c) {
            w.step(now);
            now += 1;
            assert!(now < 2300);
        }
        match mode {
            0 => {
                w.operation_epoch.fetch_add(1, Ordering::AcqRel);
            }
            1 => state.lock().unwrap().operation_ack_status = Some(4),
            2 => state.lock().unwrap().fail_read = true,
            _ => state.lock().unwrap().silent = true,
        }
        run(&mut w, now, 20);
        w.step(now + 6000);
        state.lock().unwrap().silent = false;
        assert!(matches!(
            w.bank.status().phase,
            OperationPhase::Failed | OperationPhase::Cancelled
        ));
        assert_eq!(w.bank.status().completed, 0);
        let count = writes(&state);
        if let Some(due) = w.retry_at {
            run(&mut w, due, 50);
        }
        assert_eq!(writes(&state), count);
        assert!(
            !state
                .lock()
                .unwrap()
                .writes
                .iter()
                .any(|r| r[0] == 0x14 && r[3] == 2)
        );
    }
}
#[test]
fn bad_final_readback_cannot_become_verified_after_all_acks() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    let plan = prepare(&mut w);
    state.lock().unwrap().writes.clear();
    begin(&mut w, Request::WholeRestore(Box::new(plan)), 1100);
    let mut now = 1100;
    while !state
        .lock()
        .unwrap()
        .writes
        .iter()
        .any(|r| r[0] == 0x14 && r[3] == 2)
    {
        w.step(now);
        now += 1;
        assert!(now < 4000);
    }
    state.lock().unwrap().corrupt_readback = true;
    run(&mut w, now, 1500);
    assert_eq!(w.bank.status().phase, OperationPhase::Failed);
    assert_eq!(w.bank.status().completed, 0);
}
#[test]
fn begin_envelope_nack_and_two_second_timeout_prevent_every_payload() {
    for timeout in [false, true] {
        let (mut w, state) = worker();
        w.command(Command::Connect(device()), 0);
        ready(&mut w, 0);
        let plan = prepare(&mut w);
        state.lock().unwrap().writes.clear();
        if !timeout {
            state.lock().unwrap().restore_ack_status = Some(4);
        }
        begin(&mut w, Request::WholeRestore(Box::new(plan)), 1100);
        let mut now = 1100;
        while !state
            .lock()
            .unwrap()
            .writes
            .iter()
            .any(|b| b[0] == 0x14 && b[3] == 1)
        {
            w.step(now);
            now += 1;
            assert!(now < 2300);
        }
        if timeout {
            state.lock().unwrap().silent = true;
            w.step(now + 1998);
            assert!(w.bank.status().busy());
            w.step(now + 2001);
        } else {
            run(&mut w, now, 30);
        }
        assert_eq!(w.bank.status().phase, OperationPhase::Failed);
        assert!(state.lock().unwrap().writes.iter().all(|b| b[0] != 0x1c));
        assert_eq!(w.bank.status().completed, 0);
    }
}
