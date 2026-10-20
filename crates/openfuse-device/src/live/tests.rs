use super::*;
fn binding() -> LiveBinding {
    LiveBinding {
        device_id: "test".into(),
        generation: 1,
    }
}
fn document() -> PresetDocument {
    PresetDocument::parse("<Preset ProductId=\"13\"><Amplifier ID=\"0\"><Module ID=\"117\" POS=\"0\" BypassState=\"1\"><Param ControlIndex=\"0\">1000</Param><Param ControlIndex=\"1\">2000</Param></Module></Amplifier></Preset>").unwrap()
}
fn change(index: u8, value: u16) -> ParameterChange {
    let mut c = wire_preset::parameter_change(20, &document(), "Amplifier", index).unwrap();
    c.wire_value = value;
    c
}
fn queue() -> LiveQueue {
    let mut q = LiveQueue::default();
    q.enable(binding(), 20, document()).unwrap();
    q
}
fn ack(command: u8, status: u8) -> Report64 {
    let mut b = [0; 64];
    b[1] = 0xc1;
    b[2] = command;
    b[3] = status;
    Report64::from_bytes(b)
}
#[test]
fn only_one_write_is_inflight_and_only_its_command_ack_advances() {
    let mut q = queue();
    q.enqueue(&binding(), vec![change(0, 3000), change(1, 4000)])
        .unwrap();
    assert_eq!(q.outgoing(10), Some(change(0, 3000).encode()));
    assert!(q.outgoing(11).is_none());
    q.on_report(ack(6, 0), 12);
    assert_eq!(q.status().pending, 2);
    assert!(q.outgoing(13).is_none());
    q.on_report(ack(5, 0), 14);
    assert_eq!(q.status().acknowledged, 1);
    assert_eq!(q.outgoing(15), Some(change(1, 4000).encode()));
    q.on_report(ack(5, 0), 16);
    assert_eq!(q.status().pending, 0);
    assert_eq!(q.status().acknowledged, 2);
}
#[test]
fn slider_coalescing_replaces_unsent_value_but_not_inflight() {
    let mut q = queue();
    q.enqueue(&binding(), vec![change(0, 100)]).unwrap();
    q.outgoing(0);
    for value in 200..400 {
        q.enqueue(&binding(), vec![change(0, value)]).unwrap();
    }
    assert_eq!(q.status().pending, 2);
    q.on_report(ack(5, 0), 1);
    assert_eq!(q.outgoing(2), Some(change(0, 399).encode()));
}
#[test]
fn invalid_batches_are_atomic_and_cannot_change_models_or_controls() {
    let mut q = queue();
    q.enqueue(&binding(), vec![change(0, 11)]).unwrap();
    for bad in [
        ParameterChange {
            model_id: 65535,
            ..change(1, 22)
        },
        ParameterChange {
            control_index: 255,
            ..change(1, 22)
        },
        ParameterChange {
            parameter_type: 65535,
            ..change(1, 22)
        },
        ParameterChange {
            parameter_index: 200,
            ..change(1, 22)
        },
        ParameterChange {
            module_type: 0,
            ..change(1, 22)
        },
    ] {
        assert!(q.enqueue(&binding(), vec![change(0, 33), bad]).is_err());
        assert_eq!(q.status().pending, 1);
    }
    assert_eq!(q.outgoing(0), Some(change(0, 11).encode()));
}
#[test]
fn stale_binding_and_oversized_input_do_not_send() {
    let mut q = queue();
    let mut old = binding();
    old.generation = 0;
    assert!(q.enqueue(&old, vec![change(0, 1)]).is_err());
    assert!(q.enqueue(&binding(), vec![change(0, 1); 65]).is_err());
    assert!(q.outgoing(0).is_none());
}
#[test]
fn timeout_and_negative_ack_clear_queue_and_require_new_read() {
    for status in [None, Some(1), Some(3), Some(4), Some(255)] {
        let mut q = queue();
        q.enqueue(&binding(), vec![change(0, 1), change(1, 2)])
            .unwrap();
        q.outgoing(0);
        if let Some(status) = status {
            q.on_report(ack(5, status), 1);
        } else {
            q.tick(5000);
        }
        assert!(!q.status().enabled);
        assert_eq!(q.status().pending, 0);
        assert!(q.status().error.is_some());
        assert!(q.enable(binding(), 20, document()).is_err());
        q.on_report(ack(5, 0), 5001);
        assert!(q.outgoing(5002).is_none());
        q.fresh_read();
        q.enable(
            LiveBinding {
                generation: 2,
                ..binding()
            },
            20,
            document(),
        )
        .unwrap();
        assert!(q.status().enabled);
    }
}
#[test]
fn cancelling_pending_delivery_requires_refresh_but_idle_pause_does_not() {
    let mut q = queue();
    q.disable();
    q.enable(binding(), 20, document()).unwrap();
    q.enqueue(&binding(), vec![change(0, 1)]).unwrap();
    q.outgoing(0);
    q.disable();
    assert!(q.enable(binding(), 20, document()).is_err());
}
#[test]
fn parameter_echo_is_accepted_but_external_change_disarms() {
    let mut q = queue();
    q.enqueue(&binding(), vec![change(0, 1)]).unwrap();
    q.outgoing(0);
    q.on_report(change(0, 1).encode(), 1);
    assert!(q.status().enabled);
    q.on_report(ack(5, 0), 2);
    q.on_report(change(0, 1).encode(), 3);
    assert!(q.status().enabled);
    q.on_report(change(0, 2).encode(), 4);
    assert!(!q.status().enabled);
}
#[test]
fn remote_preset_or_knob_change_cancels_queued_writes() {
    for id in [6, 8, 19, 25, 27, 28, 29, 32, 255] {
        let mut q = queue();
        q.enqueue(&binding(), vec![change(0, 1)]).unwrap();
        let mut b = [0; 64];
        b[0] = id;
        q.on_report(Report64::from_bytes(b), 0);
        assert!(q.outgoing(1).is_none());
        assert!(!q.status().enabled);
    }
}

#[test]
fn external_changes_while_disabled_invalidate_capture_for_later_enable() {
    for id in [5, 6, 8, 19, 25, 27, 28, 29, 32, 255] {
        let mut q = queue();
        q.disable();
        let mut b = [0; 64];
        b[0] = id;
        q.on_report(Report64::from_bytes(b), 0);
        assert!(q.enable(binding(), 20, document()).is_err());
        q.fresh_read();
        assert!(
            q.enable(
                LiveBinding {
                    generation: 2,
                    ..binding()
                },
                20,
                document()
            )
            .is_ok()
        );
    }
}
