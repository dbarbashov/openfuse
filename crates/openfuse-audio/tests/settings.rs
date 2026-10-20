use openfuse_audio::{Phase, Player, settings::*};
use std::time::{Duration, Instant};
fn inventory() -> OutputInventory {
    let key = DeviceKey {
        host: "Synthetic".into(),
        name: "Test speakers".into(),
    };
    let device = OutputDevice {
        key: key.clone(),
        error: None,
        default_config: Some(OutputConfiguration {
            device: key,
            channels: 2,
            sample_rate: 48_000,
            format: "f32".into(),
            buffer_frames: None,
        }),
        supported: vec![OutputRange {
            channels: 2,
            min_sample_rate: 44_100,
            max_sample_rate: 96_000,
            format: "f32".into(),
            min_buffer_frames: Some(64),
            max_buffer_frames: Some(1024),
        }],
    };
    OutputInventory {
        host: "Synthetic".into(),
        system_default: Some(device.clone()),
        devices: vec![device],
    }
}
#[test]
fn defaults_and_explicit_rate_buffer_follow_advertised_capabilities() {
    let inv = inventory();
    let default = plan_output(&OutputSettings::default(), &inv).unwrap();
    assert_eq!(default.sample_rate, 48_000);
    assert_eq!(default.buffer_frames, None);
    let explicit = OutputSettings {
        device: Some(inv.devices[0].key.clone()),
        sample_rate: Some(96_000),
        buffer_frames: Some(128),
    };
    let chosen = plan_output(&explicit, &inv).unwrap();
    assert_eq!(chosen.sample_rate, 96_000);
    assert_eq!(chosen.buffer_frames, Some(128));
    assert_eq!(chosen.channels, 2);
    assert_eq!(chosen.format, "f32");
}
#[test]
fn missing_named_device_never_falls_back_to_available_default() {
    let inv = inventory();
    let saved = OutputSettings {
        device: Some(DeviceKey {
            host: inv.host.clone(),
            name: "Removed interface".into(),
        }),
        ..Default::default()
    };
    assert!(
        plan_output(&saved, &inv)
            .unwrap_err()
            .contains("unavailable")
    );
    assert!(plan_output(&OutputSettings::default(), &inv).is_ok());
}
#[test]
fn duplicate_display_names_block_named_selection_but_not_system_default() {
    let mut inv = inventory();
    inv.devices.push(inv.devices[0].clone());
    assert!(
        plan_output(
            &OutputSettings {
                device: Some(inv.devices[0].key.clone()),
                ..Default::default()
            },
            &inv
        )
        .unwrap_err()
        .contains("Several")
    );
    assert!(plan_output(&OutputSettings::default(), &inv).is_ok());
}
#[test]
fn other_host_and_absent_default_are_not_guessed() {
    let mut inv = inventory();
    let mut key = inv.devices[0].key.clone();
    key.host = "Old OS".into();
    assert!(
        plan_output(
            &OutputSettings {
                device: Some(key),
                ..Default::default()
            },
            &inv
        )
        .is_err()
    );
    inv.system_default = None;
    assert!(plan_output(&OutputSettings::default(), &inv).is_err());
    assert!(
        plan_output(
            &OutputSettings {
                device: Some(inv.devices[0].key.clone()),
                ..Default::default()
            },
            &inv
        )
        .is_ok()
    );
}
#[test]
fn explicit_rate_or_buffer_does_not_silently_change_to_supported_value() {
    let inv = inventory();
    for (rate, buffer) in [
        (Some(192_000), None),
        (None, Some(2048)),
        (Some(8_000), Some(128)),
        (None, Some(63)),
    ] {
        assert!(
            plan_output(
                &OutputSettings {
                    sample_rate: rate,
                    buffer_frames: buffer,
                    ..Default::default()
                },
                &inv
            )
            .is_err()
        );
    }
    for rate in [44_100, 96_000] {
        for buffer in [64, 1024] {
            let actual = plan_output(
                &OutputSettings {
                    sample_rate: Some(rate),
                    buffer_frames: Some(buffer),
                    ..Default::default()
                },
                &inv,
            )
            .unwrap();
            assert_eq!(actual.sample_rate, rate);
            assert_eq!(actual.buffer_frames, Some(buffer));
        }
    }
}
#[test]
fn unknown_buffer_caps_allow_driver_default_only() {
    let mut inv = inventory();
    let device = inv.system_default.as_mut().unwrap();
    device.supported[0].min_buffer_frames = None;
    device.supported[0].max_buffer_frames = None;
    assert!(plan_output(&OutputSettings::default(), &inv).is_ok());
    assert!(
        plan_output(
            &OutputSettings {
                buffer_frames: Some(128),
                ..Default::default()
            },
            &inv
        )
        .is_err()
    );
}
#[test]
fn query_errors_and_invalid_capability_ranges_are_visible() {
    let mut inv = inventory();
    inv.system_default.as_mut().unwrap().error = Some("No permission".into());
    assert!(
        plan_output(&OutputSettings::default(), &inv)
            .unwrap_err()
            .contains("No permission")
    );
    inv.system_default.as_mut().unwrap().error = None;
    inv.system_default.as_mut().unwrap().supported[0].channels = 0;
    assert!(plan_output(&OutputSettings::default(), &inv).is_err());
}
#[test]
fn supported_alternative_format_is_selected_for_explicit_rate() {
    let mut inv = inventory();
    let d = inv.system_default.as_mut().unwrap();
    d.default_config.as_mut().unwrap().format = "i16".into();
    let mut limited = d.supported[0].clone();
    limited.format = "i16".into();
    limited.max_sample_rate = 48_000;
    d.supported.push(limited);
    assert_eq!(
        plan_output(&OutputSettings::default(), &inv)
            .unwrap()
            .format,
        "i16"
    );
    assert_eq!(
        plan_output(
            &OutputSettings {
                sample_rate: Some(96_000),
                ..Default::default()
            },
            &inv
        )
        .unwrap()
        .format,
        "f32"
    );
}
#[test]
fn persisted_intent_is_strict_and_bounded() {
    let good = OutputSettings {
        device: Some(inventory().devices[0].key.clone()),
        sample_rate: Some(44_100),
        buffer_frames: Some(64),
    };
    let json = serde_json::to_string(&good).unwrap();
    let decoded: OutputSettings = serde_json::from_str(&json).unwrap();
    decoded.validate().unwrap();
    assert_eq!(decoded, good);
    assert!(
        serde_json::from_str::<OutputSettings>(
            r#"{"device":null,"sample_rate":null,"buffer_frames":null,"future":true}"#
        )
        .is_err()
    );
    for bad in [
        OutputSettings {
            sample_rate: Some(0),
            ..Default::default()
        },
        OutputSettings {
            sample_rate: Some(1_000_001),
            ..Default::default()
        },
        OutputSettings {
            buffer_frames: Some(0),
            ..Default::default()
        },
        OutputSettings {
            device: Some(DeviceKey {
                host: "a".into(),
                name: "a".repeat(1025),
            }),
            ..Default::default()
        },
    ] {
        assert!(bad.validate().is_err());
    }
}
fn wav() -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend(b"RIFF");
    bytes.extend((36 + 16_000u32).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16u32.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(8_000u32.to_le_bytes());
    bytes.extend(16_000u32.to_le_bytes());
    bytes.extend(2u16.to_le_bytes());
    bytes.extend(16u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend(16_000u32.to_le_bytes());
    bytes.resize(44 + 16_000, 0);
    bytes
}
fn ready(player: &mut Player) {
    let end = Instant::now() + Duration::from_secs(5);
    while player.snapshot().phase == Phase::Loading && Instant::now() < end {
        player.poll();
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(player.snapshot().phase, Phase::Ready);
}
#[test]
fn configuration_keeps_track_position_and_never_opens_headless_output() {
    let mut player = Player::headless();
    player.load("track".into(), wav(), "wav").unwrap();
    ready(&mut player);
    player.seek(0.25).unwrap();
    player.set_volume(0.2).unwrap();
    player.set_repeat(true);
    let settings = OutputSettings {
        device: Some(inventory().devices[0].key.clone()),
        sample_rate: Some(96_000),
        buffer_frames: Some(128),
    };
    player.configure_output(settings.clone()).unwrap();
    let state = player.snapshot();
    assert_eq!(state.position, 0.25);
    assert_eq!(state.phase, Phase::Paused);
    assert_eq!(state.volume, 0.2);
    assert!(state.repeat);
    assert_eq!(player.output_settings(), &settings);
    assert!(player.active_output().is_none());
    assert!(player.play().is_err());
    assert!(
        player
            .configure_output(OutputSettings {
                sample_rate: Some(0),
                ..Default::default()
            })
            .is_err()
    );
    assert_eq!(player.output_settings(), &settings);
    assert_eq!(player.snapshot().position, 0.25);
}
#[test]
fn configuration_during_decode_preserves_load_and_intent() {
    let mut player = Player::headless();
    player.load("track".into(), wav(), "wav").unwrap();
    let settings = OutputSettings {
        sample_rate: Some(44_100),
        ..Default::default()
    };
    player.configure_output(settings.clone()).unwrap();
    ready(&mut player);
    assert_eq!(player.output_settings(), &settings);
    assert!(player.active_output().is_none());
    assert_eq!(player.snapshot().id.as_deref(), Some("track"));
}
