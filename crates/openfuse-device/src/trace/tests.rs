use super::*;
use crate::{
    acceptance::{Acceptance, Outcome},
    backend::Backend,
    simulator::Simulator,
};
use openfuse_core::usb_session::{SessionMachine, SessionStatus};

fn fixture() -> Trace {
    let recorder = Recorder::new(Origin::Simulator);
    let mut backend = RecordingBackend {
        inner: Box::new(Simulator::default()),
        recorder: recorder.clone(),
    };
    recorder.start().unwrap();
    let info = backend.enumerate().unwrap().remove(0);
    let mut connection = backend.open(&info).unwrap();
    let mut machine = SessionMachine::for_pid(info.product_id).unwrap();
    machine.start(0).unwrap();
    for _ in 0..100 {
        if let Some(report) = machine.take_outgoing() {
            connection.write(report).unwrap();
        }
        if let Some(report) = connection.read().unwrap() {
            machine.on_report(report, 0);
        }
        if machine.status() == &SessionStatus::Ready {
            break;
        }
    }
    assert_eq!(machine.status(), &SessionStatus::Ready);
    drop(connection);
    recorder.stop().unwrap()
}
#[test]
fn production_machine_replays_recorded_simulator_bootstrap() {
    let trace = fixture();
    let result = replay_bootstrap(&trace).unwrap();
    assert_eq!(result.tx_verified, 3);
    assert!(result.rx_consumed >= 9);
    assert_eq!(result.events_not_replayed, 1); // Close is beyond bootstrap.
    assert!(!result.hardware_validated);
    let text = serde_json::to_string(&trace).unwrap();
    assert!(!text.contains("OPENFUSE-SIMULATION"));
    assert!(!text.contains("simulated-mustang"));
    assert!(!text.contains("serial"));
}
#[test]
fn mutation_of_every_transmit_is_detected() {
    let trace = fixture();
    for i in 0..trace.entries.len() {
        let mut corrupt = trace.clone();
        if let Event::Tx { bytes, .. } = &mut corrupt.entries[i].event {
            bytes[63] ^= 1;
            assert!(replay_bootstrap(&corrupt).unwrap_err().contains("mismatch"));
        }
    }
}
#[test]
fn missing_and_reordered_transmits_do_not_pass() {
    let trace = fixture();
    let mut missing = trace.clone();
    missing.entries.remove(1);
    assert!(replay_bootstrap(&missing).is_err());
    let mut swapped = trace;
    swapped.entries.swap(1, 2);
    assert!(replay_bootstrap(&swapped).is_err());
}
#[test]
fn timeout_and_incomplete_bootstrap_do_not_pass() {
    let mut trace = fixture();
    trace.entries.truncate(2);
    trace.ended_ms += 60_000;
    assert!(replay_bootstrap(&trace).is_err());
    let mut trace = fixture();
    for entry in trace.entries.iter_mut().skip(2) {
        entry.at_ms += 60_000;
    }
    trace.ended_ms += 60_000;
    assert!(replay_bootstrap(&trace).is_err());
}
#[test]
fn truncation_bad_lengths_order_schema_and_lifecycle_are_rejected() {
    let trace = fixture();
    let mut t = trace.clone();
    t.dropped_events = 1;
    assert!(replay_bootstrap(&t).is_err());
    let mut t = trace.clone();
    t.schema = 2;
    assert!(t.validate().is_err());
    let mut t = trace.clone();
    t.entries[0].at_ms = t.ended_ms + 1;
    assert!(t.validate().is_err());
    let mut t = trace.clone();
    t.entries.remove(0);
    assert!(t.validate().is_err());
    let mut t = trace;
    if let Event::Tx { bytes, .. } = &mut t.entries[1].event {
        bytes.push(0);
    }
    assert!(t.validate().is_err());
}
#[test]
fn failed_write_or_transport_fault_does_not_pass() {
    let mut trace = fixture();
    if let Event::Tx { succeeded, .. } = &mut trace.entries[1].event {
        *succeeded = false;
    }
    assert!(replay_bootstrap(&trace).is_err());
    trace.entries[1].event = Event::Fault { framing: true };
    assert!(replay_bootstrap(&trace).is_err());
}
#[test]
fn suffix_operations_are_explicitly_outside_replay_scope() {
    let mut trace = fixture();
    trace.entries.pop(); // Keep connection open.
    trace.entries.push(Entry {
        at_ms: trace.ended_ms,
        event: Event::Tx {
            bytes: vec![0xaa; 64],
            succeeded: true,
        },
    });
    assert_eq!(replay_bootstrap(&trace).unwrap().events_not_replayed, 1);
}
#[test]
fn recording_is_opt_in_and_cannot_start_mid_connection() {
    let recorder = Recorder::new(Origin::Simulator);
    let mut backend = RecordingBackend {
        inner: Box::new(Simulator::default()),
        recorder: recorder.clone(),
    };
    let info = backend.enumerate().unwrap().remove(0);
    assert!(recorder.progress().is_none());
    let connection = backend.open(&info).unwrap();
    assert!(recorder.start().is_err());
    assert!(recorder.stop().is_none());
    drop(connection);
    recorder.start().unwrap();
    assert!(recorder.start().is_err());
    assert_eq!(recorder.stop().unwrap().entries.len(), 0);
}
#[test]
fn recording_limit_is_bounded_and_never_silently_successful() {
    let recorder = Recorder::new(Origin::Simulator);
    let mut backend = RecordingBackend {
        inner: Box::new(Simulator::default()),
        recorder: recorder.clone(),
    };
    recorder.start().unwrap();
    let info = backend.enumerate().unwrap().remove(0);
    let mut connection = backend.open(&info).unwrap();
    for _ in 0..MAX_EVENTS + 10 {
        connection
            .write(openfuse_core::protocol::Report64::device_detect())
            .unwrap();
    }
    drop(connection);
    let trace = recorder.stop().unwrap();
    assert_eq!(trace.entries.len(), MAX_EVENTS);
    assert_eq!(trace.dropped_events, 12);
    assert!(replay_bootstrap(&trace).is_err());
}
#[test]
fn bundle_digest_catches_tampering_and_checklist_cannot_be_reordered() {
    let bundle = Acceptance::new(fixture()).unwrap();
    let text = String::from_utf8(bundle.json().unwrap()).unwrap();
    assert!(Acceptance::parse(&text).is_ok());
    let mut bundle = bundle.clone();
    bundle.trace.os.push('x');
    assert!(bundle.validate().is_err());
    let mut bundle = Acceptance::new(fixture()).unwrap();
    bundle.observations.swap(0, 1);
    assert!(bundle.validate().is_err());
    let mut json: serde_json::Value = serde_json::from_str(&text).unwrap();
    json["unknown"] = true.into();
    assert!(Acceptance::parse(&json.to_string()).is_err());
}
#[test]
fn simulator_cannot_be_exported_as_owner_hardware_pass() {
    let mut bundle = Acceptance::new(fixture()).unwrap();
    bundle.amplifier_label = "Test amp".into();
    bundle.environment = "OS and setup".into();
    bundle.observer = "Owner".into();
    bundle.observations[0].outcome = Outcome::Pass;
    bundle.observations[0].notes = "Compared".into();
    assert!(bundle.validate().unwrap_err().contains("Simulator"));
}
#[test]
fn owner_claim_requires_context_and_notes_and_is_never_an_auto_pass() {
    let mut trace = fixture();
    trace.origin = Origin::HardwareTransport; // Synthetic test of input validation only.
    let mut bundle = Acceptance::new(trace).unwrap();
    bundle.observations[0].outcome = Outcome::Pass;
    assert!(bundle.validate().is_err());
    bundle.amplifier_label = "test fixture".into();
    bundle.environment = "synthetic".into();
    bundle.observer = "test".into();
    bundle.observations[0].notes = "synthetic input validation".into();
    bundle.validate().unwrap();
    assert!(!replay_bootstrap(&bundle.trace).unwrap().hardware_validated);
    assert!(bundle.html().unwrap().contains("Owner reports pass"));
}
#[test]
fn html_escapes_untrusted_owner_notes() {
    let mut bundle = Acceptance::new(fixture()).unwrap();
    bundle.amplifier_label = "<script>alert('x')</script>".into();
    bundle.observations[0].notes = "<img src=x onerror=alert(1)>".into();
    let html = bundle.html().unwrap();
    assert!(!html.contains("<script>"));
    assert!(!html.contains("<img"));
    assert!(html.contains("&lt;script&gt;"));
}
#[test]
fn create_only_export_preserves_existing_file_and_round_trips() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("session.json");
    let bundle = Acceptance::new(fixture()).unwrap();
    write_new(&path, &bundle.json().unwrap()).unwrap();
    assert!(write_new(&path, b"replace").is_err());
    let loaded = Acceptance::parse(&read_bounded(&path).unwrap()).unwrap();
    assert_eq!(loaded.trace_sha256, bundle.trace_sha256);
    assert!(read_bounded(directory.path()).is_err());
}
#[test]
fn oversized_input_is_rejected_before_deserialization() {
    assert!(Acceptance::parse(&" ".repeat(MAX_FILE_BYTES + 1)).is_err());
}

#[test]
fn largest_bundle_round_trips_within_file_limit() {
    let mut trace = fixture();
    trace.entries.truncate(1);
    trace.entries.resize(
        MAX_EVENTS,
        Entry {
            at_ms: trace.ended_ms,
            event: Event::Rx {
                bytes: vec![255; 64],
            },
        },
    );
    let bundle = Acceptance::new(trace).unwrap();
    let json = bundle.json().unwrap();
    assert!(json.len() <= MAX_FILE_BYTES, "{}", json.len());
    Acceptance::parse(std::str::from_utf8(&json).unwrap()).unwrap();
}
#[test]
fn mixed_device_trace_cannot_support_owner_pass() {
    let mut trace = fixture();
    trace.origin = Origin::HardwareTransport;
    trace.entries.push(Entry {
        at_ms: trace.ended_ms,
        event: Event::Open {
            profile: Profile {
                product_id: 18,
                interface_number: 0,
            },
        },
    });
    let mut bundle = Acceptance::new(trace).unwrap();
    bundle.amplifier_label = "synthetic".into();
    bundle.environment = "synthetic".into();
    bundle.observer = "synthetic".into();
    bundle.observations[0].notes = "synthetic".into();
    bundle.observations[0].outcome = Outcome::Pass;
    assert!(bundle.validate().is_err());
}
#[test]
fn threaded_controller_recording_preserves_user_session_and_live_events() {
    use crate::{DeviceController, DeviceEvent};
    use std::time::{Duration, Instant};
    let controller = DeviceController::simulated();
    let recorder = controller.recorder();
    recorder.start().unwrap();
    controller.connect(&crate::simulator::device()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut ready = false;
    while Instant::now() < deadline {
        for event in controller.poll() {
            if let DeviceEvent::Status(s) = event {
                ready |= s.can_sync;
            }
        }
        if ready {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(ready);
    assert!(recorder.start().is_err());
    let trace = recorder.stop().unwrap();
    assert_eq!(replay_bootstrap(&trace).unwrap().product_id, 20);
    assert!(recorder.progress().is_none());
    controller.read_current().unwrap(); // Stopping tracing does not disconnect.
}

#[test]
fn failed_open_keeps_attempted_profile_without_private_error_text() {
    use crate::{DeviceInfo, backend::Connection};
    struct Failing;
    impl Backend for Failing {
        fn enumerate(&mut self) -> Result<Vec<DeviceInfo>, String> {
            Ok(vec![])
        }
        fn open(&mut self, _: &DeviceInfo) -> Result<Box<dyn Connection>, String> {
            Err("private serial and path".into())
        }
    }
    let recorder = Recorder::new(Origin::HardwareTransport);
    let mut backend = RecordingBackend {
        inner: Box::new(Failing),
        recorder: recorder.clone(),
    };
    recorder.start().unwrap();
    assert!(backend.open(&crate::simulator::device()).is_err());
    let trace = recorder.stop().unwrap();
    trace.validate().unwrap();
    assert!(
        matches!(&trace.entries[0].event, Event::OpenFailed { profile } if profile.product_id == 20)
    );
    let bundle = Acceptance::new(trace).unwrap();
    let json = String::from_utf8(bundle.json().unwrap()).unwrap();
    assert!(!json.contains("private serial"));
    assert!(bundle.html().unwrap().contains("USB 0014 / interface 0"));
    assert!(replay_bootstrap(&bundle.trace).is_err());
    recorder.start().unwrap(); // Failed open released the active reservation.
}
