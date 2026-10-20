use super::*;
use crate::OperationPhase;
use openfuse_core::controls::{SettingsPatch, VolumeEndpoints};
fn run(w: &mut Worker, start: u64, count: u64) {
    for t in start..start + count {
        w.step(t);
    }
}
fn start_tuner(w: &mut Worker, now: u64) {
    w.command(Command::Tuner(0, live_binding(w), true), now);
}
fn tuner_writes(state: &Arc<Mutex<State>>) -> Vec<[u8; 64]> {
    state
        .lock()
        .unwrap()
        .writes
        .iter()
        .filter(|r| r[0] == 10)
        .copied()
        .collect()
}
#[test]
fn tuner_waits_for_state_not_ack_and_exposes_pitch_staleness() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    start_tuner(&mut w, 30);
    assert!(w.tuner.status().pending);
    assert!(!w.tuner.status().confirmed);
    w.step(30);
    assert!(w.tuner.status().pending);
    w.step(31);
    assert!(w.tuner.status().confirmed);
    w.step(32);
    assert_eq!(w.tuner.status().reading.unwrap().note.label(), "A");
    assert!(!w.tuner.status().stale);
    w.step(2000);
    assert!(w.tuner.status().stale);
    assert_eq!(tuner_writes(&state)[0][1], 1);
    w.start_tuner(live_binding(&w), false, false);
    run(&mut w, 2001, 5);
    assert!(!w.tuner.status().exclusive());
    assert!(w.tuner.status().confirmed);
    assert!(w.tuner.status().reading.is_none());
}
#[test]
fn tuner_rejects_early_pitch_wrong_state_and_recovers_only_from_observed_stop() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    start_tuner(&mut w, 30);
    w.step(30);
    state.lock().unwrap().silent = true;
    w.step(5031);
    assert!(w.tuner.status().error.is_some());
    assert!(w.tuner.status().exclusive());
    assert!(!w.tuner.status().confirmed);
    let before = tuner_writes(&state).len();
    start_tuner(&mut w, 5040);
    assert_eq!(tuner_writes(&state).len(), before);
    // Drain the late on-state, then explicitly stop; no start replay occurred.
    state.lock().unwrap().silent = false;
    run(&mut w, 5041, 3);
    w.start_tuner(live_binding(&w), false, false);
    run(&mut w, 5050, 5);
    assert!(!w.tuner.status().exclusive());
    assert_eq!(tuner_writes(&state).len(), 2);
}
#[test]
fn tuner_excludes_bank_live_audition_settings_and_current_sync() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    let binding = live_binding(&w);
    start_tuner(&mut w, 30);
    run(&mut w, 30, 5);
    let count = state.lock().unwrap().writes.len();
    w.command(
        Command::Bank(0, binding.clone(), crate::bank::Request::Save(0)),
        40,
    );
    w.command(Command::SelectSlot(0, binding.clone(), 2), 41);
    w.command(Command::SetLive(0, binding.clone(), true), 42);
    w.command(
        Command::Settings(
            0,
            binding,
            SettingsPatch {
                usb_gain: Some(99),
                ..Default::default()
            },
        ),
        43,
    );
    w.command(Command::ReadCurrent, 44);
    run(&mut w, 44, 5);
    assert_eq!(state.lock().unwrap().writes.len(), count);
    assert!(!w.live.status().enabled);
}
#[test]
fn tuner_muting_and_stop_use_state_echo_with_no_ack_requirement() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    start_tuner(&mut w, 30);
    run(&mut w, 30, 4);
    w.start_tuner(live_binding(&w), true, false);
    run(&mut w, 40, 5);
    assert!(w.tuner.status().enabled);
    assert!(!w.tuner.status().muted);
    assert!(w.tuner.status().confirmed);
    assert_eq!(tuner_writes(&state)[1][4], 0);
}
#[test]
fn stop_out_of_band_survives_full_queue_and_discards_queued_start() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    let binding = live_binding(&w);
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    tx.try_send(Command::Tuner(0, binding.clone(), true))
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
    assert!(controller.refresh().is_err());
    controller.set_tuner(&binding, false, false).unwrap();
    for command in rx.try_iter() {
        w.command(command, 30);
    }
    run(&mut w, 30, 5);
    let writes = tuner_writes(&state);
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0][2], 0);
    assert!(!w.tuner.status().exclusive());
}
#[test]
fn disconnect_attempts_tuner_off_and_reconnect_never_restarts_it() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    start_tuner(&mut w, 30);
    run(&mut w, 30, 4);
    w.command(Command::Disconnect, 40);
    assert_eq!(tuner_writes(&state).last().unwrap()[2], 0);
    assert!(!w.tuner.status().confirmed);
    w.command(Command::Connect(device()), 50);
    run(&mut w, 50, 25);
    assert_eq!(tuner_writes(&state).len(), 2);
    assert!(w.tuner.status().error.is_some());
    w.start_tuner(live_binding(&w), false, false);
    run(&mut w, 80, 5);
    assert!(!w.tuner.status().exclusive());
}
#[test]
fn gain_settings_write_only_gain_then_verify_capture_without_persistent_save() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    w.command(
        Command::Settings(
            0,
            live_binding(&w),
            SettingsPatch {
                usb_gain: Some(200),
                ..Default::default()
            },
        ),
        30,
    );
    run(&mut w, 30, 60);
    assert_eq!(
        w.operation.status().phase,
        OperationPhase::Verified,
        "{:?}",
        w.operation.status()
    );
    assert_eq!(
        openfuse_core::controls::settings(20, &w.engine.snapshot().unwrap().reports)
            .unwrap()
            .usb_gain,
        Some(200)
    );
    let writes = state
        .lock()
        .unwrap()
        .writes
        .iter()
        .filter(|r| r[0] == 0x1c)
        .map(|r| r[2])
        .collect::<Vec<_>>();
    assert_eq!(writes, vec![13, 0]);
}
#[test]
fn invalid_stale_cancelled_and_unsupported_setting_requests_do_not_write() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    for patch in [
        SettingsPatch {
            usb_gain: Some(256),
            ..Default::default()
        },
        SettingsPatch {
            fx_loop_bypassed: Some(true),
            ..Default::default()
        },
        SettingsPatch {
            first_pedal_mode: Some(1),
            ..Default::default()
        },
    ] {
        w.command(Command::Settings(0, live_binding(&w), patch), 30);
        assert_eq!(w.operation.status().phase, OperationPhase::Failed);
    }
    let mut binding = live_binding(&w);
    binding.generation += 1;
    w.command(
        Command::Settings(
            0,
            binding,
            SettingsPatch {
                usb_gain: Some(2),
                ..Default::default()
            },
        ),
        31,
    );
    w.operation_epoch.fetch_add(1, Ordering::AcqRel);
    w.command(
        Command::Settings(
            0,
            live_binding(&w),
            SettingsPatch {
                usb_gain: Some(3),
                ..Default::default()
            },
        ),
        32,
    );
    run(&mut w, 32, 10);
    assert!(state.lock().unwrap().writes.iter().all(|r| r[0] != 0x1c));
}
#[test]
fn settings_fail_on_ack_or_readback_mismatch_without_replaying() {
    for corrupt in [false, true] {
        let (mut w, state) = worker();
        w.command(Command::Connect(device()), 0);
        ready(&mut w, 0);
        if corrupt {
            state.lock().unwrap().corrupt_readback = true;
        } else {
            state.lock().unwrap().operation_ack_status = Some(4);
        }
        w.command(
            Command::Settings(
                0,
                live_binding(&w),
                SettingsPatch {
                    usb_gain: Some(222),
                    ..Default::default()
                },
            ),
            30,
        );
        run(&mut w, 30, 70);
        assert_eq!(w.operation.status().phase, OperationPhase::Failed);
        assert_eq!(
            state
                .lock()
                .unwrap()
                .writes
                .iter()
                .filter(|r| r[0] == 0x1c && r[2] == 13)
                .count(),
            1
        );
    }
}
#[test]
fn floor_settings_roundtrip_both_modes_globals_gain_and_loop() {
    let mut backend = Simulator::floor();
    let d = backend.enumerate().unwrap().remove(0);
    let mut w = Worker::new(Box::new(backend), Arc::new(Mutex::new(Default::default())));
    w.command(Command::Connect(d), 0);
    run(&mut w, 0, 30);
    assert!(w.snapshot_current);
    w.command(
        Command::Settings(
            0,
            live_binding(&w),
            SettingsPatch {
                usb_gain: Some(128),
                fx_loop_bypassed: Some(true),
                global_volume: Some(VolumeEndpoints {
                    heel: 0x1200,
                    toe: 0xab00,
                }),
                first_pedal_mode: Some(1),
                second_pedal_mode: Some(0),
            },
        ),
        40,
    );
    run(&mut w, 40, 100);
    assert_eq!(
        w.operation.status().phase,
        OperationPhase::Verified,
        "{:?}",
        w.operation.status()
    );
    let s = openfuse_core::controls::settings(0x12, &w.engine.snapshot().unwrap().reports).unwrap();
    assert_eq!(s.usb_gain, Some(128));
    assert_eq!(s.fx_loop_bypassed, Some(true));
    assert_eq!(
        s.global_volume,
        Some(VolumeEndpoints {
            heel: 0x1200,
            toe: 0xab00
        })
    );
    assert_eq!(s.first_pedal_mode, Some(1));
    assert_eq!(s.second_pedal_mode, Some(0));
}
#[test]
fn tuner_uncertainty_belongs_to_its_device_and_session_failure_clears_confirmation() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    start_tuner(&mut w, 30);
    run(&mut w, 30, 4);
    w.engine.fail_transport("forced session failure");
    w.step(40);
    assert!(w.connection.is_none());
    assert!(!w.tuner.status().confirmed);
    assert!(w.tuner.status().error.is_some());
    w.command(Command::Disconnect, 41);
    let mut other = device();
    other.id = "different-amplifier".into();
    other.serial = Some("OTHER".into());
    state.lock().unwrap().devices = vec![other.clone()];
    w.command(Command::Connect(other), 50);
    ready(&mut w, 50);
    assert!(!w.tuner.status().exclusive());
    assert!(!w.tuner.status().confirmed);
    w.command(
        Command::Settings(
            0,
            live_binding(&w),
            SettingsPatch {
                usb_gain: Some(33),
                ..Default::default()
            },
        ),
        80,
    );
    run(&mut w, 80, 40);
    assert_eq!(w.operation.status().phase, OperationPhase::Verified);
}
#[test]
fn unsolicited_tuner_activation_stops_unsent_settings() {
    let (mut w, state) = worker();
    w.command(Command::Connect(device()), 0);
    ready(&mut w, 0);
    w.command(
        Command::Settings(
            0,
            live_binding(&w),
            SettingsPatch {
                usb_gain: Some(123),
                ..Default::default()
            },
        ),
        30,
    );
    let mut b = [0; 64];
    b[0] = 10;
    b[1] = 1;
    b[2] = 1;
    b[4] = 1;
    state.lock().unwrap().injected = Some(openfuse_core::protocol::Report64::from_bytes(b));
    w.step(30);
    assert_eq!(w.operation.status().phase, OperationPhase::Failed);
    assert!(w.tuner.status().exclusive());
    assert!(state.lock().unwrap().writes.iter().all(|r| r[0] != 0x1c));
}
