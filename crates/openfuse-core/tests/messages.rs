use openfuse_core::{hex, messages::*, protocol::Report64};

#[test]
fn matches_original_cil_for_all_probe_vectors() {
    let vectors: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/message_vectors.json")).unwrap();
    assert_eq!(vectors.as_array().unwrap().len(), 30);
    for vector in vectors.as_array().unwrap() {
        let v = &vector["values"];
        let n = |key: &str| v[key].as_u64().unwrap() as u16;
        let report = match vector["name"].as_str().unwrap() {
            "DeviceDetectMessage" => Report64::device_detect(),
            "VersionRequestMessage" => Report64::version_request(),
            "FxParamAdjustMessage" => ParameterChange {
                module_type: n("FxType") as u8,
                model_id: n("FxId"),
                parameter_index: n("ParameterIndex") as u8,
                control_index: n("ControlIndex") as u8,
                parameter_type: n("ParameterType"),
                wire_value: n("ParameterValue"),
            }
            .encode(),
            "FxPositionMessage" => effect_positions(
                n("PosStomp") as u8,
                n("PosMod") as u8,
                n("PosDly") as u8,
                n("PosRvb") as u8,
            ),
            "FxBypassModeMessage" => {
                bypass_mode(n("FxType") as u8, n("IsActive") != 0, n("BypassMode") as u8)
            }
            "FxKillMessage" => {
                effect_state(n("FxType") as u8, n("IsActive") != 0, n("Position") as u8)
            }
            "PresetLoadMessage" => preset_load(vector["arguments"][0].as_u64().unwrap() as u8),
            "TunerControlMessage" => tuner_control(
                n("IsTunerOn") != 0,
                n("SnapToState") != 0,
                n("IsMuted") != 0,
            ),
            "KnobPositionMessage" => knob_position(n("PositionIndex") as u8, n("KnobIndex") as u8),
            unknown => panic!("missing encoder {unknown}"),
        };
        assert_eq!(
            hex::encode(report.as_bytes())
                .replace(' ', "")
                .to_lowercase(),
            vector["hex"].as_str().unwrap(),
            "{}",
            vector["name"]
        );
    }
}

#[test]
fn inbound_fields_preserve_unknown_values_and_signed_tuner_offset() {
    let mut b = [0xaa; 64];
    b[..6].copy_from_slice(&[0, 0xc3, 0xfe, 0xee, 2, 7]);
    let report = Report64::from_bytes(b);
    assert_eq!(
        decode_inbound(&report),
        MessageFields::Handshake {
            command: 254,
            status: 238,
            major: 2,
            minor: 7
        }
    );
    assert_eq!(report.as_bytes(), &b);
    b[0] = 11;
    b[2] = 255;
    b[3] = 0xfd;
    assert_eq!(
        decode_inbound(&Report64::from_bytes(b)),
        MessageFields::TunerInfo {
            tone: 255,
            offset: -3
        }
    );
    b[0] = 0xf8;
    assert_eq!(
        decode_inbound(&Report64::from_bytes(b)),
        MessageFields::Unknown { id: 248 }
    );
}

#[test]
fn version_date_is_opaque_and_build_little_endian() {
    let mut b = [0; 64];
    b[0] = 1;
    b[2] = 3;
    b[3] = 11;
    b[4..20].fill(0xff);
    b[20] = 0x34;
    b[21] = 0x12;
    assert_eq!(
        decode_inbound(&Report64::from_bytes(b)),
        MessageFields::Version {
            major: 3,
            minor: 11,
            build: 0x1234,
            build_date: [255; 16]
        }
    );
}

#[test]
fn module_capacity_reserves_separate_index_and_value_regions() {
    let mut load = ModuleLoad {
        module_type: 3,
        model_id: 0x1234,
        position: 7,
        parameters: (0..19).map(|i| (i, 0xab00 + i as u16)).collect(),
    };
    let [first, end] = load.encode().unwrap();
    assert_eq!(&first.as_bytes()[..7], &[6, 0x43, 3, 0x34, 0x12, 19, 7]);
    assert_eq!(&first.as_bytes()[7..26], &(0..19).collect::<Vec<_>>());
    assert_eq!(&first.as_bytes()[62..64], &[18, 0xab]);
    assert_eq!(&end.as_bytes()[..3], &[7, 0x83, 3]);
    load.parameters.push((19, 0));
    assert!(load.encode().is_err());
    load.parameters = vec![(1, 0), (1, 1)];
    assert!(load.encode().is_err());
    load.parameters = vec![(0, 1)];
    load.model_id = 0;
    assert!(load.encode().is_err());
    load.parameters.clear();
    assert!(load.encode().is_ok());
}

#[test]
fn sync_scope_and_parameter_endianness_are_explicit() {
    assert_eq!(&sync_request(false).as_bytes()[..2], &[255, 0xc1]);
    assert_eq!(&sync_request(true).as_bytes()[..2], &[255, 0xc5]);
    let p = ParameterChange {
        module_type: 255,
        model_id: 0xfedc,
        parameter_index: 9,
        control_index: 17,
        parameter_type: 0xabcd,
        wire_value: 0x1234,
    };
    assert_eq!(decode_inbound(&p.encode()), MessageFields::Parameter(p));
    assert_eq!(
        &p.encode().as_bytes()[3..11],
        &[0xdc, 0xfe, 9, 17, 0xcd, 0xab, 0x34, 0x12]
    );
    assert_eq!(bypass_mode(4, true, 3).as_bytes()[3], 0);
    assert_eq!(effect_state(4, false, 2).as_bytes()[3], 1);
}
