use openfuse_audio::{Audio, Phase, Player, Transport, decode};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
fn wav(samples: &[i16], channels: u16, rate: u32) -> Vec<u8> {
    let mut v = Vec::new();
    let bytes = (samples.len() * 2) as u32;
    v.extend(b"RIFF");
    v.extend((36 + bytes).to_le_bytes());
    v.extend(b"WAVEfmt ");
    v.extend(16u32.to_le_bytes());
    v.extend(1u16.to_le_bytes());
    v.extend(channels.to_le_bytes());
    v.extend(rate.to_le_bytes());
    v.extend((rate * channels as u32 * 2).to_le_bytes());
    v.extend((channels * 2).to_le_bytes());
    v.extend(16u16.to_le_bytes());
    v.extend(b"data");
    v.extend(bytes.to_le_bytes());
    for s in samples {
        v.extend(s.to_le_bytes());
    }
    v
}
fn transport() -> Transport {
    let mut t = Transport::default();
    t.set_audio(
        "test".into(),
        Audio {
            sample_rate: 8_000,
            channels: 2,
            frames: Arc::new(vec![[0.25, -0.5], [0.5, -0.25], [0.75, 0.25], [1.0, 0.5]]),
        },
    )
    .unwrap();
    t.set_volume(1.0).unwrap();
    t
}
#[test]
fn pcm_decoder_preserves_stereo_and_duplicates_mono() {
    let a = decode(wav(&[8192, -16384, 16384, -8192], 2, 48_000), "wav").unwrap();
    assert_eq!(&**a.frames, &[[0.25, -0.5], [0.5, -0.25]]);
    assert_eq!(a.channels, 2);
    assert_eq!(a.sample_rate, 48_000);
    let mono = decode(wav(&[8192, -8192], 1, 44_100), "WAV").unwrap();
    assert_eq!(&**mono.frames, &[[0.25, 0.25], [-0.25, -0.25]]);
}
#[test]
fn mp3_decoder_reads_real_independently_encoded_fixture() {
    let a = decode(include_bytes!("fixtures/sine.mp3").to_vec(), "mp3").unwrap();
    assert_eq!(a.sample_rate, 44_100);
    assert_eq!(a.channels, 2);
    assert!((a.duration() - 0.1).abs() < 0.001);
    assert!(a.frames.iter().any(|f| f[0].abs() > 0.05));
    assert!(a.frames.iter().all(|f| (f[0] - f[1]).abs() < 1e-5));
}
#[test]
fn decoder_rejects_malformed_unsupported_and_truncated_inputs() {
    assert!(decode(vec![0; 30], "wav").is_err());
    assert!(decode(vec![0; 30], "mid").is_err());
    let mut truncated = wav(&[0; 100], 2, 8_000);
    truncated.truncate(truncated.len() - 60);
    assert!(decode(truncated, "wav").is_err());
    assert!(decode(wav(&[0; 12], 3, 8_000), "wav").is_err());
}
#[test]
fn transport_pause_seek_stop_end_replay_are_deterministic() {
    let mut t = transport();
    assert_eq!(t.next_frame(8_000), [0.0; 2]);
    t.play().unwrap();
    assert_eq!(t.next_frame(8_000), [0.25, -0.5]);
    t.pause();
    let at = t.snapshot().position;
    assert_eq!(t.next_frame(8_000), [0.0; 2]);
    assert_eq!(t.snapshot().position, at);
    t.seek(2.0 / 8_000.0).unwrap();
    t.play().unwrap();
    assert_eq!(t.next_frame(8_000), [0.75, 0.25]);
    assert_eq!(t.next_frame(8_000), [1.0, 0.5]);
    assert_eq!(t.snapshot().phase, Phase::Ended);
    assert_eq!(t.next_frame(8_000), [0.0; 2]);
    t.play().unwrap();
    assert_eq!(t.next_frame(8_000), [0.25, -0.5]);
    t.stop();
    assert_eq!(t.snapshot().position, 0.0);
    assert_eq!(t.snapshot().phase, Phase::Ready);
}
#[test]
fn balance_volume_repeat_and_rate_adaptation_render_expected_samples() {
    let mut t = transport();
    t.set_balance(-1.0).unwrap();
    t.set_volume(0.5).unwrap();
    t.set_repeat(true);
    t.play().unwrap();
    assert_eq!(t.next_frame(16_000), [0.125, 0.0]);
    assert_eq!(t.next_frame(16_000), [0.1875, 0.0]);
    for _ in 0..6 {
        t.next_frame(16_000);
    }
    assert_eq!(t.next_frame(16_000), [0.125, 0.0]);
    assert_eq!(t.snapshot().phase, Phase::Playing);
    t.set_balance(1.0).unwrap();
    t.seek(0.0).unwrap();
    assert_eq!(t.next_frame(8_000), [0.0, -0.25]);
}
#[test]
fn rendering_is_partition_invariant_and_invalid_controls_do_not_mutate() {
    let mut a = transport();
    let mut b = transport();
    a.set_repeat(true);
    b.set_repeat(true);
    a.play().unwrap();
    b.play().unwrap();
    let expected: Vec<_> = (0..100).map(|_| a.next_frame(44_100)).collect();
    let mut actual = Vec::new();
    for size in [3, 17, 1, 39, 40] {
        actual.extend((0..size).map(|_| b.next_frame(44_100)));
    }
    assert_eq!(expected, actual);
    let s = b.snapshot();
    assert!(b.seek(f64::NAN).is_err());
    assert!(b.seek(10.0).is_err());
    assert!(b.set_volume(f32::NAN).is_err());
    assert!(b.set_balance(1.1).is_err());
    assert_eq!(b.snapshot().position, s.position);
}
fn wait(player: &mut Player) {
    let end = Instant::now() + Duration::from_secs(5);
    while player.snapshot().phase == Phase::Loading && Instant::now() < end {
        player.poll();
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_ne!(player.snapshot().phase, Phase::Loading);
}
#[test]
fn async_load_never_autoplays_or_opens_output_and_clear_rejects_stale_results() {
    let mut p = Player::headless();
    p.load("first".into(), wav(&[0; 100], 1, 8_000), "wav")
        .unwrap();
    p.clear();
    std::thread::sleep(Duration::from_millis(10));
    p.poll();
    assert_eq!(p.snapshot().phase, Phase::Empty);
    p.load("second".into(), wav(&[8192; 100], 1, 8_000), "wav")
        .unwrap();
    wait(&mut p);
    assert_eq!(p.snapshot().id.as_deref(), Some("second"));
    assert_eq!(p.snapshot().phase, Phase::Ready);
    assert!(p.play().is_err());
    assert_eq!(p.snapshot().position, 0.0);
}
#[test]
fn latest_load_wins_and_decode_failure_is_visible() {
    let mut p = Player::headless();
    for n in 0..50 {
        p.load(n.to_string(), wav(&[0; 100], 1, 8_000), "wav")
            .unwrap();
    }
    wait(&mut p);
    assert_eq!(p.snapshot().id.as_deref(), Some("49"));
    assert_eq!(p.snapshot().phase, Phase::Ready);
    p.load("bad".into(), vec![0; 30], "mp3").unwrap();
    wait(&mut p);
    assert_eq!(p.snapshot().phase, Phase::Error);
    assert!(p.snapshot().error.is_some());
}

#[test]
fn stopping_an_async_load_cancels_without_late_ready_state() {
    let mut p = Player::headless();
    p.load("cancel".into(), wav(&[0; 10_000], 1, 8_000), "wav")
        .unwrap();
    p.stop();
    std::thread::sleep(Duration::from_millis(15));
    p.poll();
    assert_eq!(p.snapshot().phase, Phase::Empty);
    assert!(p.snapshot().id.is_none());
}
