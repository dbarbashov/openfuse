//! Explicit muted default-output smoke: no amplifier access, no audible signal.
use openfuse_audio::{Phase, Player};
use std::time::{Duration, Instant};
fn main() -> Result<(), String> {
    if !std::env::args().any(|a| a == "--run") {
        return Err(
            "Use --run to explicitly open the computer's default audio output (muted)".into(),
        );
    }
    let frames = 44_100u32;
    let mut wav = Vec::new();
    wav.extend(b"RIFF");
    wav.extend((36 + frames * 2).to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(frames.to_le_bytes());
    wav.extend((frames * 2).to_le_bytes());
    wav.extend(2u16.to_le_bytes());
    wav.extend(16u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend((frames * 2).to_le_bytes());
    wav.resize(wav.len() + frames as usize * 2, 0);
    let mut p = Player::new();
    p.set_volume(0.0)?;
    p.load("muted-smoke".into(), wav, "wav")?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while p.snapshot().phase == Phase::Loading && Instant::now() < deadline {
        p.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
    if p.snapshot().phase != Phase::Ready {
        return Err(format!("Decode failed: {:?}", p.snapshot()));
    }
    p.play()?;
    std::thread::sleep(Duration::from_millis(200));
    p.poll();
    let position = p.snapshot().position;
    if position <= 0.0 || p.snapshot().phase != Phase::Playing {
        return Err(format!("Output did not advance: {:?}", p.snapshot()));
    }
    p.pause();
    let paused = p.snapshot().position;
    std::thread::sleep(Duration::from_millis(80));
    if p.snapshot().position != paused {
        return Err("Paused output advanced".into());
    }
    p.seek(0.95)?;
    p.set_repeat(true);
    p.play()?;
    std::thread::sleep(Duration::from_millis(150));
    p.poll();
    if p.snapshot().phase != Phase::Playing || p.snapshot().position >= 0.95 {
        return Err("Loop did not wrap".into());
    }
    p.stop();
    if p.snapshot().position != 0.0 || p.snapshot().phase != Phase::Ready {
        return Err("Stop did not rewind".into());
    }
    println!(
        "{{\"passed\":true,\"default_output_opened\":true,\"muted\":true,\"initial_advance_seconds\":{position},\"pause_seek_loop_stop\":true,\"amplifier_access\":false,\"acoustic_validation\":false}}"
    );
    Ok(())
}
