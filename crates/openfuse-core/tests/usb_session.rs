use openfuse_core::{
    protocol::Report64,
    usb_session::{SESSION_PIDS, SessionConfig, SessionError, SessionMachine, SessionStatus},
};

fn report(id: u8, kind: u8) -> Report64 {
    let mut bytes = [0; 64];
    bytes[0] = id;
    bytes[1] = 0x81; // Preserve unknown/reserved bit 0, not a required input flag.
    bytes[2] = kind;
    Report64::from_bytes(bytes)
}
fn change(report: Report64, pairs: &[(usize, u8)]) -> Report64 {
    let mut bytes = *report.as_bytes();
    for &(index, value) in pairs {
        bytes[index] = value;
    }
    Report64::from_bytes(bytes)
}
fn version() -> Report64 {
    change(report(1, 2), &[(3, 7), (20, 0x34), (21, 0x12)])
}
fn name() -> Report64 {
    change(
        report(0x1c, 4),
        &[
            (4, 0x34),
            (5, 0x12),
            (6, 1),
            (16, b'A'),
            (17, 0xe9),
            (18, b' '),
        ],
    )
}
fn module(kind: u8) -> Report64 {
    change(report(0x1c, kind), &[(16, kind), (17, 1), (60, 0xfe)])
}

/// Simulated boundary writes are collected once; reports are fed independently
/// from hard-coded layouts rather than the implementation's encoders.
struct MockLink {
    machine: SessionMachine,
    writes: Vec<Report64>,
    now: u64,
}
impl MockLink {
    fn new() -> Self {
        Self {
            machine: SessionMachine::new(),
            writes: Vec::new(),
            now: 0,
        }
    }
    fn write(&mut self) -> Report64 {
        let report = self.machine.take_outgoing().expect("pending write");
        self.writes.push(report);
        assert!(self.machine.take_outgoing().is_none());
        report
    }
    fn recv(&mut self, report: Report64) {
        self.now += 1;
        self.machine.on_report(report, self.now);
    }
    fn bootstrap(&mut self) {
        self.machine.start(self.now).unwrap();
        assert_eq!(&self.write().as_bytes()[..2], &[0, 0xc3]);
        self.recv(report(0, 0));
        assert_eq!(&self.write().as_bytes()[..2], &[0x1a, 0xc1]);
        self.recv(version());
        assert_eq!(&self.write().as_bytes()[..2], &[0xff, 0xc5]);
    }
    fn preset(&mut self) {
        self.recv(name());
        for kind in [9, 5, 7, 6, 8] {
            self.recv(module(kind));
        }
        self.recv(change(report(0x1c, 0), &[(7, 1)]));
    }
    fn finish(&mut self) {
        self.recv(report(0xff, 0));
    }
    fn ready(&mut self) {
        self.bootstrap();
        self.preset();
        self.finish();
    }
}

#[test]
fn bootstrap_commits_only_after_complete_and_sync_end() {
    let mut link = MockLink::new();
    link.bootstrap();
    assert_eq!(link.machine.status(), &SessionStatus::Syncing);
    assert_eq!(link.machine.version().unwrap().build, 0x1234);
    link.preset();
    assert!(link.machine.snapshot().is_none());
    link.finish();
    assert_eq!(link.machine.status(), &SessionStatus::Ready);
    let snapshot = link.machine.snapshot().unwrap();
    assert_eq!(snapshot.name, "Aé");
    assert_eq!(snapshot.position, 0x1234);
    assert!(snapshot.modified);
    assert_eq!(
        snapshot
            .modules
            .iter()
            .map(|m| m.data_type)
            .collect::<Vec<_>>(),
        [5, 6, 7, 8, 9]
    );
    assert_eq!(snapshot.modules[0].model_id, 261);
    assert_eq!(snapshot.modules[0].raw, module(5));
    assert_eq!(snapshot.reports.len(), 7);
    assert_eq!(snapshot.captured_at_ms, link.now);
    assert_eq!(link.writes.len(), 3);
}

#[test]
fn wrong_ack_and_early_version_never_advance_detect() {
    let mut link = MockLink::new();
    link.machine.start(0).unwrap();
    link.write();
    link.recv(report(0, 0x1c));
    link.recv(version());
    assert_eq!(link.machine.status(), &SessionStatus::Detecting);
    assert!(link.machine.take_outgoing().is_none());
    link.recv(report(0, 0));
    assert_eq!(link.machine.status(), &SessionStatus::Version);
}

#[test]
fn ack_before_command_is_taken_cannot_advance() {
    let mut link = MockLink::new();
    link.machine.start(0).unwrap();
    link.recv(report(0, 0));
    assert_eq!(link.machine.status(), &SessionStatus::Detecting);
    link.write();
    link.recv(report(0, 0));
    assert_eq!(link.machine.status(), &SessionStatus::Version);
}

#[test]
fn stale_detect_ack_does_not_restart_version_or_sync() {
    let mut link = MockLink::new();
    link.machine.start(0).unwrap();
    link.write();
    link.recv(report(0, 0));
    link.write();
    link.recv(report(0, 0));
    assert_eq!(link.machine.status(), &SessionStatus::Version);
    assert!(link.machine.take_outgoing().is_none());
    link.recv(version());
    link.write();
    link.recv(report(0, 0));
    link.recv(version());
    assert_eq!(link.machine.status(), &SessionStatus::Syncing);
    assert!(link.machine.take_outgoing().is_none());
}

#[test]
fn every_nonzero_detect_status_fails_without_retry_or_replay() {
    for status in [1, 2, 3, 4, 5, 0x7f, 0xff] {
        let mut link = MockLink::new();
        link.machine.start(0).unwrap();
        link.write();
        link.recv(change(report(0, 0), &[(3, status)]));
        assert_eq!(
            link.machine.status(),
            &SessionStatus::Failed(SessionError::NegativeAck(status))
        );
        link.recv(report(0, 0));
        assert!(link.machine.take_outgoing().is_none());
    }
}

#[test]
fn detect_timeout_is_absolute_including_at_boundary() {
    let mut link = MockLink::new();
    link.machine.start(0).unwrap();
    link.write();
    link.machine.on_report(report(0, 0x1c), 5_999);
    assert_eq!(link.machine.status(), &SessionStatus::Detecting);
    link.machine.on_report(report(0, 0), 6_000);
    assert_eq!(
        link.machine.status(),
        &SessionStatus::Failed(SessionError::Timeout)
    );
    assert!(link.machine.take_outgoing().is_none());
}

#[test]
fn version_and_sync_have_independent_bounded_deadlines() {
    let mut link = MockLink::new();
    link.machine.start(0).unwrap();
    link.write();
    link.recv(report(0, 0));
    link.write();
    link.machine.tick(6_001);
    assert_eq!(
        link.machine.status(),
        &SessionStatus::Failed(SessionError::Timeout)
    );
    let mut link = MockLink::new();
    link.bootstrap();
    link.machine.on_report(report(0x70, 0), 15_001);
    assert_eq!(link.machine.status(), &SessionStatus::Syncing);
    link.machine.tick(15_002);
    assert_eq!(
        link.machine.status(),
        &SessionStatus::Failed(SessionError::Timeout)
    );
}

#[test]
fn malformed_hid_input_cannot_be_padded_into_success() {
    for len in [0, 1, 63, 65] {
        let mut link = MockLink::new();
        link.bootstrap();
        assert_eq!(
            link.machine.on_bytes(&vec![0; len], 10),
            Err(SessionError::MalformedReport(len))
        );
        assert_eq!(
            link.machine.status(),
            &SessionStatus::Failed(SessionError::MalformedReport(len))
        );
        assert!(link.machine.snapshot().is_none());
    }
}

#[test]
fn unknown_and_unsolicited_reports_preserve_every_byte() {
    let mut link = MockLink::new();
    link.bootstrap();
    while link.machine.take_received().is_some() {}
    let unknown = Report64::from_bytes(std::array::from_fn(|i| (i as u8).wrapping_add(0x70)));
    link.recv(unknown);
    assert_eq!(link.machine.take_received(), Some(unknown));
    link.recv(name());
    link.recv(change(report(0x1c, 0xa7), &[(63, 0xbc)]));
    for kind in 5..=9 {
        link.recv(module(kind));
    }
    link.recv(report(0x1c, 0));
    link.finish();
    assert_eq!(
        link.machine.snapshot().unwrap().reports[1].as_bytes()[63],
        0xbc
    );
}

#[test]
fn receiving_without_active_session_does_not_create_state() {
    let mut link = MockLink::new();
    link.recv(report(0, 0));
    link.recv(version());
    link.preset();
    link.finish();
    assert_eq!(link.machine.status(), &SessionStatus::Disconnected);
    assert!(link.machine.snapshot().is_none());
    assert!(link.machine.take_outgoing().is_none());
}

#[test]
fn diagnostic_queue_is_bounded_with_explicit_drop_count() {
    let mut machine = SessionMachine::with_config(
        0x14,
        SessionConfig {
            max_received_reports: 2,
            ..Default::default()
        },
    )
    .unwrap();
    for kind in 0..5 {
        machine.on_report(report(0x70, kind), 0);
    }
    assert_eq!(machine.dropped_reports(), 3);
    assert_eq!(machine.take_received(), Some(report(0x70, 3)));
    assert_eq!(machine.take_received(), Some(report(0x70, 4)));
    assert!(machine.take_received().is_none());
}

#[test]
fn sync_flood_is_bounded_and_never_publishes_partial_state() {
    let mut link = MockLink {
        machine: SessionMachine::with_config(
            0x14,
            SessionConfig {
                max_sync_reports: 3,
                ..Default::default()
            },
        )
        .unwrap(),
        ..MockLink::new()
    };
    link.bootstrap();
    link.recv(name());
    link.recv(module(5));
    link.recv(report(0x70, 0));
    link.recv(report(0x70, 1));
    assert_eq!(
        link.machine.status(),
        &SessionStatus::Failed(SessionError::SyncOverflow)
    );
    assert!(link.machine.snapshot().is_none());
}

#[test]
fn missing_name_complete_or_module_rejects_sync_end() {
    for omission in [4, 0, 5, 6, 7, 8, 9] {
        let mut link = MockLink::new();
        link.bootstrap();
        if omission != 4 {
            link.recv(name());
        }
        for kind in 5..=9 {
            if kind != omission {
                link.recv(module(kind));
            }
        }
        if omission != 0 {
            link.recv(report(0x1c, 0));
        }
        link.finish();
        assert_eq!(
            link.machine.status(),
            &SessionStatus::Failed(SessionError::IncompletePreset)
        );
        assert!(link.machine.snapshot().is_none());
    }
}

#[test]
fn wrong_phase_modules_cannot_satisfy_later_preset() {
    let mut link = MockLink::new();
    link.bootstrap();
    for kind in 5..=9 {
        link.recv(module(kind));
    }
    link.recv(name());
    link.recv(report(0x1c, 0));
    link.finish();
    assert_eq!(
        link.machine.status(),
        &SessionStatus::Failed(SessionError::IncompletePreset)
    );
}

#[test]
fn unrelated_slot_names_and_knob_packets_never_replace_current() {
    let mut link = MockLink::new();
    link.bootstrap();
    link.recv(change(name(), &[(6, 0), (16, b'Z')]));
    link.recv(change(name(), &[(3, 1), (16, b'X')]));
    link.preset();
    link.recv(change(name(), &[(6, 0), (16, b'Z')]));
    link.finish();
    assert_eq!(link.machine.snapshot().unwrap().name, "Aé");
}

#[test]
fn duplicate_current_name_or_model_is_rejected() {
    let mut link = MockLink::new();
    link.bootstrap();
    link.recv(name());
    link.recv(name());
    assert_eq!(
        link.machine.status(),
        &SessionStatus::Failed(SessionError::DuplicateCurrentPreset)
    );
    let mut link = MockLink::new();
    link.bootstrap();
    link.recv(name());
    link.recv(module(5));
    link.recv(module(5));
    assert_eq!(
        link.machine.status(),
        &SessionStatus::Failed(SessionError::DuplicateModule(5))
    );
}

#[test]
fn two_expression_payloads_remain_preserved_without_guessing_their_identity() {
    let mut link = MockLink::new();
    link.bootstrap();
    link.recv(name());
    link.recv(report(0x1c, 10));
    link.recv(change(report(0x1c, 10), &[(16, 1)]));
    for kind in 5..=9 {
        link.recv(module(kind));
    }
    link.recv(report(0x1c, 0));
    link.finish();
    assert_eq!(link.machine.snapshot().unwrap().reports.len(), 9);
}

#[test]
fn refresh_retains_previous_snapshot_until_atomic_success_or_failure() {
    let mut link = MockLink::new();
    link.ready();
    let before = link.machine.snapshot().unwrap().clone();
    link.machine.request_sync(link.now).unwrap();
    assert_eq!(link.write().message_id(), 0xff);
    link.recv(name());
    assert_eq!(link.machine.snapshot(), Some(&before));
    link.finish();
    assert_eq!(
        link.machine.status(),
        &SessionStatus::Failed(SessionError::IncompletePreset)
    );
    assert_eq!(link.machine.snapshot(), Some(&before));
    assert!(link.machine.request_sync(link.now).is_err());
}

#[test]
fn successful_refresh_atomically_replaces_snapshot() {
    let mut link = MockLink::new();
    link.ready();
    let captured = link.machine.snapshot().unwrap().captured_at_ms;
    link.machine.request_sync(link.now).unwrap();
    link.write();
    link.recv(change(name(), &[(16, b'B')]));
    for kind in 5..=9 {
        link.recv(module(kind));
    }
    link.recv(report(0x1c, 0));
    link.finish();
    assert_eq!(link.machine.snapshot().unwrap().name, "Bé");
    assert!(link.machine.snapshot().unwrap().captured_at_ms > captured);
    assert_eq!(link.writes.len(), 4);
}

#[test]
fn disconnect_resets_pending_output_snapshot_and_version_without_replay() {
    let mut link = MockLink::new();
    link.ready();
    link.machine.request_sync(link.now).unwrap();
    link.machine.disconnect();
    assert!(link.machine.take_outgoing().is_none());
    assert!(link.machine.snapshot().is_none());
    assert!(link.machine.version().is_none());
    link.machine.tick(u64::MAX);
    assert_eq!(link.machine.status(), &SessionStatus::Disconnected);
    link.machine.start(100).unwrap();
    assert_eq!(link.write(), Report64::device_detect());
    assert_eq!(link.machine.dropped_reports(), 0);
    assert!(link.machine.take_received().is_none());
}

#[test]
fn write_failure_clears_pending_commands_and_requires_disconnect() {
    let mut link = MockLink::new();
    link.machine.start(0).unwrap();
    link.machine.fail_transport("unplugged");
    assert!(link.machine.take_outgoing().is_none());
    assert!(matches!(
        link.machine.status(),
        SessionStatus::Failed(SessionError::Transport(_))
    ));
    assert_eq!(link.machine.start(1), Err(SessionError::Busy));
    link.machine.disconnect();
    link.machine.start(2).unwrap();
    assert_eq!(link.write(), Report64::device_detect());
}

#[test]
fn only_evidenced_pid_profiles_can_start() {
    for &pid in SESSION_PIDS {
        assert!(SessionMachine::for_pid(pid).is_ok());
    }
    for pid in [0, 2, 3, 6, 7, 0x0b, 0x0c, 0x0f, 0x18, 0x19, 0xffff] {
        assert!(
            matches!(SessionMachine::for_pid(pid), Err(SessionError::UnsupportedPid(p)) if p == pid)
        );
    }
}

#[test]
fn old_firmware_gate_is_limited_to_original_two_families() {
    for (pid, major, minor, allowed) in [
        (4, 1, 1, false),
        (4, 1, 2, true),
        (5, 0, 9, false),
        (5, 1, 0, true),
        (0x14, 0, 1, true),
    ] {
        let mut link = MockLink {
            machine: SessionMachine::for_pid(pid).unwrap(),
            ..MockLink::new()
        };
        link.machine.start(0).unwrap();
        link.write();
        link.recv(report(0, 0));
        link.write();
        link.recv(change(version(), &[(2, major), (3, minor)]));
        if allowed {
            assert_eq!(link.machine.status(), &SessionStatus::Syncing);
        } else {
            assert_eq!(
                link.machine.status(),
                &SessionStatus::Failed(SessionError::UnsupportedFirmware)
            );
            assert!(link.machine.take_outgoing().is_none());
        }
    }
}

#[test]
fn interleaved_noncurrent_preset_cannot_supply_current_modules() {
    let mut link = MockLink::new();
    link.bootstrap();
    link.recv(name());
    link.recv(module(5));
    link.recv(change(name(), &[(6, 0), (16, b'Z')]));
    for kind in 6..=9 {
        link.recv(module(kind));
    }
    link.recv(report(0x1c, 0));
    link.finish();
    assert_eq!(
        link.machine.status(),
        &SessionStatus::Failed(SessionError::InterleavedPreset)
    );
    assert!(link.machine.snapshot().is_none());
}

#[test]
fn completed_candidate_is_frozen_against_later_unsolicited_module_updates() {
    let mut link = MockLink::new();
    link.bootstrap();
    link.preset();
    let unsolicited = change(module(5), &[(16, 0xff), (17, 0xff)]);
    link.recv(unsolicited);
    link.finish();
    assert_eq!(link.machine.snapshot().unwrap().modules[0].model_id, 261);
    let mut observed = false;
    while let Some(report) = link.machine.take_received() {
        observed |= report == unsolicited;
    }
    assert!(observed);
}
