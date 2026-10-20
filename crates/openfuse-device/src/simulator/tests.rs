use super::*;
use openfuse_core::{
    audition,
    model_catalog::{self, ModuleCategory},
    preset::PresetParameter,
    protocol::FLAG_ACK,
};
fn peer() -> Box<dyn Connection> {
    Simulator::default().open(&device()).unwrap()
}
fn drain(peer: &mut dyn Connection) -> Vec<Report64> {
    let mut reports = Vec::new();
    while let Some(report) = peer.read().unwrap() {
        reports.push(report);
        assert!(reports.len() < 128);
    }
    reports
}
fn read(peer: &mut dyn Connection) -> Vec<Report64> {
    peer.write(openfuse_core::messages::sync_request(true))
        .unwrap();
    drain(peer)
}
fn apply(peer: &mut dyn Connection, plan: &audition::Plan) -> Vec<Report64> {
    for &report in plan.steps() {
        peer.write(report).unwrap();
        let replies = drain(peer);
        if report.has_flag(FLAG_ACK) {
            assert_eq!(replies.len(), 1);
            assert_eq!(
                &replies[0].as_bytes()[..4],
                &[0, 0xc1, report.message_id(), 0]
            );
        } else {
            assert!(replies.is_empty());
        }
    }
    let current = read(peer);
    plan.matches_readback(&current).unwrap();
    current
}
#[test]
fn full_temporary_audition_does_not_modify_the_stored_bank() {
    let mut peer = peer();
    let original = read(peer.as_mut());
    let mut desired = wire_preset::decode(20, &original).unwrap().document;
    let mut info = desired.metadata();
    info.name = "Temporary rehearsal".into();
    desired.set_metadata(&info).unwrap();
    desired.set_parameter("Amplifier", 1, 0x6600).unwrap();
    let plan = audition::preset(20, &desired, &original).unwrap();
    let after = apply(peer.as_mut(), &plan);
    assert_eq!(
        wire_preset::decode(20, &after)
            .unwrap()
            .document
            .metadata()
            .name,
        "Temporary rehearsal"
    );
    let selected = apply(peer.as_mut(), &audition::slot_select(20, 7).unwrap());
    let restored = wire_preset::decode(20, &selected).unwrap().document;
    let original = wire_preset::decode(20, &original).unwrap().document;
    assert_eq!(restored.modules(), original.modules());
    assert_eq!(restored.metadata().name, "Simulated clean preset");
}
#[test]
fn module_bypass_and_routing_packets_change_only_working_state() {
    let mut peer = peer();
    let current = read(peer.as_mut());
    let mut desired = wire_preset::decode(20, &current).unwrap().document;
    let model = model_catalog::models(13, ModuleCategory::Delay)
        .find(|m| m.id != 0 && m.id != 43)
        .unwrap();
    let params = model
        .parameters
        .iter()
        .map(|p| PresetParameter {
            index: p.index,
            value: p.default_raw,
        })
        .collect::<Vec<_>>();
    desired
        .replace_module_model("Delay", model.id, &params, Some(model.bypass_mode))
        .unwrap();
    let current = apply(
        peer.as_mut(),
        &audition::module(20, &desired, &current, "Delay").unwrap(),
    );
    let current = apply(
        peer.as_mut(),
        &audition::bypass(20, &current, "Delay", false).unwrap(),
    );
    let current = apply(
        peer.as_mut(),
        &audition::routing(20, &current, [1, 0, 5, 4]).unwrap(),
    );
    let doc = wire_preset::decode(20, &current).unwrap().document;
    let delay = doc
        .modules()
        .into_iter()
        .find(|m| m.key == "Delay")
        .unwrap();
    assert_eq!(delay.model_id, Some(model.id));
    assert_eq!(delay.active, Some(false));
    assert_eq!(delay.position, Some(5));
}
#[test]
fn slot_selection_changes_sound_and_global_volume_endpoints_survive() {
    let mut peer = peer();
    let original = read(peer.as_mut());
    let selected = apply(peer.as_mut(), &audition::slot_select(20, 23).unwrap());
    assert_eq!(
        wire_preset::decode(20, &selected)
            .unwrap()
            .document
            .metadata()
            .name,
        "Simulated slot 23"
    );
    let globals = |reports: &[Report64]| {
        let b = reports
            .iter()
            .find(|r| r.as_bytes()[2] == 10)
            .unwrap()
            .as_bytes();
        (b[17], b[18])
    };
    assert_eq!(globals(&original), globals(&selected));
}
#[test]
fn malformed_save_unknown_control_and_firmware_commands_are_refused() {
    let mut peer = peer();
    let before = read(peer.as_mut());
    for report in [
        frame(&[(0, 0x1c), (1, 1), (2, 2)]),
        frame(&[(0, 0x1c), (1, 1), (2, 3)]),
        frame(&[(0, 0x1c), (1, 3), (2, 5), (3, 1)]),
        frame(&[(0, 2)]),
        frame(&[(0, 0x22)]),
    ] {
        assert!(peer.write(report).is_err());
        assert!(drain(peer.as_mut()).is_empty());
    }
    assert_eq!(read(peer.as_mut()), before);
}
#[test]
fn an_incomplete_temporary_transaction_never_commits_working_state() {
    let mut peer = Peer {
        reports: VecDeque::new(),
        document: document(),
        slot: 7,
        globals: (4, 250),
        staged: None,
        bank: Simulator::default().bank,
        save: None,
        pid: 20,
        tuner: false,
        last_tone: std::time::Instant::now(),
        auxiliary: Arc::new(Mutex::new(auxiliary::fixtures(20, &document()))),
        staged_auxiliary: None,
        whole_restore: false,
        stored_globals: Default::default(),
    };
    let current = peer.snapshot();
    let plan = audition::preset(20, &peer.document, &current).unwrap();
    let before = peer.document.to_xml().unwrap();
    peer.write(plan.steps()[0]).unwrap();
    peer.write(plan.steps()[1]).unwrap();
    assert_eq!(peer.document.to_xml().unwrap(), before);
    assert!(peer.write(*plan.steps().last().unwrap()).is_err());
    assert_eq!(peer.document.to_xml().unwrap(), before);
    assert!(peer.staged.is_none());
}
#[test]
fn explicit_save_survives_simulator_reopen_and_backup_reads_without_selecting() {
    let mut backend = Simulator::default();
    let mut peer = backend.open(&device()).unwrap();
    let before = read(peer.as_mut());
    let plan = openfuse_core::bank::save_current(20, &before, 23).unwrap();
    for report in plan.steps() {
        peer.write(*report).unwrap();
        drain(peer.as_mut());
    }
    drop(peer);
    let mut peer = backend.open(&device()).unwrap();
    let working = read(peer.as_mut());
    peer.write(openfuse_core::bank::stored_request(20, 23).unwrap())
        .unwrap();
    let mut reader = openfuse_core::bank::StoredRead::new(20, 23).unwrap();
    let mut stored = None;
    for report in drain(peer.as_mut()) {
        if let Some(slot) = reader.observe(report).unwrap() {
            stored = Some(slot);
        }
    }
    plan.matches_stored_readback(stored.unwrap().raw_reports())
        .unwrap();
    assert_eq!(read(peer.as_mut()), working);
}
