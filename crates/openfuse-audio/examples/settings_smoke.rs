//! Explicit, silent output selection smoke. Never opens amplifier HID or records audio.
use openfuse_audio::{Phase, Player, settings::*};
use std::time::{Duration, Instant};
fn wav() -> Vec<u8> {
    let mut b = Vec::new();
    b.extend(b"RIFF");
    b.extend((36 + 88_200u32).to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(44_100u32.to_le_bytes());
    b.extend(88_200u32.to_le_bytes());
    b.extend(2u16.to_le_bytes());
    b.extend(16u16.to_le_bytes());
    b.extend(b"data");
    b.extend(88_200u32.to_le_bytes());
    b.resize(44 + 88_200, 0);
    b
}
fn advance(player: &mut Player) -> Result<OutputConfiguration, String> {
    player.play()?;
    let actual = player
        .active_output()
        .cloned()
        .ok_or("No active output configuration")?;
    let before = player.snapshot().position;
    std::thread::sleep(Duration::from_millis(180));
    player.poll();
    if player.snapshot().phase != Phase::Playing || player.snapshot().position <= before {
        return Err(format!("Output did not advance: {:?}", player.snapshot()));
    }
    Ok(actual)
}
fn main() -> Result<(), String> {
    if std::env::args()
        .collect::<Vec<_>>()
        .as_slice()
        .get(1)
        .map(String::as_str)
        != Some("--run-muted")
    {
        return Err("Use --run-muted to explicitly enumerate and open this computer's default output with zero samples and zero volume".into());
    }
    let inventory = enumerate_outputs()?;
    let default = inventory
        .system_default
        .as_ref()
        .ok_or("No default output")?;
    let key = default.key.clone();
    let plan = plan_output(&OutputSettings::default(), &inventory)?;
    let mut p = Player::new();
    p.set_volume(0.0)?;
    p.load("muted-output-settings".into(), wav(), "wav")?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while p.snapshot().phase == Phase::Loading && Instant::now() < deadline {
        p.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
    if p.snapshot().phase != Phase::Ready {
        return Err("Decode did not become ready".into());
    }
    let first = advance(&mut p)?;
    let unique = inventory.devices.iter().filter(|d| d.key == key).count() == 1;
    let buffer = default
        .supported
        .iter()
        .find(|c| {
            c.channels == plan.channels
                && c.format == plan.format
                && plan.sample_rate >= c.min_sample_rate
                && plan.sample_rate <= c.max_sample_rate
        })
        .and_then(|c| c.min_buffer_frames.zip(c.max_buffer_frames))
        .map(|(min, max)| 128u32.clamp(min, max));
    let selected = OutputSettings {
        device: unique.then_some(key),
        sample_rate: Some(plan.sample_rate),
        buffer_frames: buffer,
    };
    p.configure_output(selected.clone())?;
    let paused = p.snapshot().position;
    std::thread::sleep(Duration::from_millis(80));
    if p.snapshot().position != paused || p.active_output().is_some() {
        return Err("Configuration failed to release/pause output".into());
    }
    let second = advance(&mut p)?;
    p.configure_output(OutputSettings {
        device: Some(DeviceKey {
            host: inventory.host.clone(),
            name: "OpenFUSE missing-output negative control 729d8c16".into(),
        }),
        ..Default::default()
    })?;
    let missing_error = p.play().err().ok_or("Missing device unexpectedly opened")?;
    if p.active_output().is_some() {
        return Err("Missing selected output fell back".into());
    }
    p.configure_output(OutputSettings::default())?;
    p.seek(0.0)?;
    let recovered = advance(&mut p)?;
    p.stop();
    println!(
        "{}",
        serde_json::json!({"passed":true,"host":inventory.host,"default_stream":first,"configured_intent":selected,"configured_stream":second,"named_default_unique":unique,"fixed_buffer_tested":buffer.is_some(),"missing_device_error":missing_error,"recovered_stream":recovered,"position_preserved_on_apply":true,"zero_samples":true,"volume":0,"hid_access":false,"acoustic_validation":false})
    );
    Ok(())
}
