use crate::settings::{OutputConfiguration, OutputSettings};
use crate::{Phase, Transport};
use cpal::traits::{DeviceTrait, StreamTrait};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
#[derive(Clone)]
pub(crate) struct Lease {
    pub epoch: Arc<AtomicU64>,
    pub generation: u64,
}
impl Lease {
    fn valid(&self) -> bool {
        self.epoch.load(Ordering::Acquire) == self.generation
    }
}

pub(crate) fn open(
    state: Arc<Mutex<Transport>>,
    settings: &OutputSettings,
    lease: Lease,
) -> Result<(cpal::Stream, OutputConfiguration), String> {
    let (device, actual) = crate::settings::resolve(settings)?;
    let config = cpal::StreamConfig {
        channels: actual.channels,
        sample_rate: cpal::SampleRate(actual.sample_rate),
        buffer_size: actual
            .buffer_frames
            .map_or(cpal::BufferSize::Default, cpal::BufferSize::Fixed),
    };
    let stream = match actual.format.as_str() {
        "f32" => build::<f32>(&device, &config, state, lease),
        "f64" => build::<f64>(&device, &config, state, lease),
        "i8" => build::<i8>(&device, &config, state, lease),
        "i16" => build::<i16>(&device, &config, state, lease),
        "i32" => build::<i32>(&device, &config, state, lease),
        "i64" => build::<i64>(&device, &config, state, lease),
        "u8" => build::<u8>(&device, &config, state, lease),
        "u16" => build::<u16>(&device, &config, state, lease),
        "u32" => build::<u32>(&device, &config, state, lease),
        "u64" => build::<u64>(&device, &config, state, lease),
        other => return Err(format!("Unsupported audio device sample format: {other}")),
    }
    .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok((stream, actual))
}
fn build<T: cpal::SizedSample + cpal::FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    state: Arc<Mutex<Transport>>,
    lease: Lease,
) -> Result<cpal::Stream, cpal::BuildStreamError> {
    let channels = config.channels as usize;
    let rate = config.sample_rate.0;
    let errors = state.clone();
    let error_lease = lease.clone();
    device.build_output_stream(
        config,
        move |data: &mut [T], _| {
            if !lease.valid() {
                data.fill(T::EQUILIBRIUM);
                return;
            }
            // GUI controls never block the real-time callback. Contention produces silence.
            let Ok(mut state) = state.try_lock() else {
                data.fill(T::EQUILIBRIUM);
                return;
            };
            if !lease.valid() {
                data.fill(T::EQUILIBRIUM);
                return;
            }
            for frame in data.chunks_mut(channels) {
                let [left, right] = state.next_frame(rate);
                frame.fill(T::EQUILIBRIUM);
                if channels == 1 {
                    frame[0] = T::from_sample((left + right) * 0.5);
                } else {
                    frame[0] = T::from_sample(left);
                    frame[1] = T::from_sample(right);
                }
            }
        },
        move |error| report_error(&errors, &error_lease, error.to_string()),
        None,
    )
}

fn report_error(state: &Mutex<Transport>, lease: &Lease, error: String) {
    if !lease.valid() {
        return;
    }
    if let Ok(mut state) = state.lock()
        && lease.valid()
    {
        state.phase = Phase::Error;
        state.error = Some(format!("Audio output: {error}"));
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retired_output_error_cannot_poison_replacement_transport() {
        let epoch = Arc::new(AtomicU64::new(7));
        let old = Lease {
            epoch: epoch.clone(),
            generation: 7,
        };
        let state = Mutex::new(Transport::default());
        epoch.store(8, Ordering::Release);
        report_error(&state, &old, "old cable removed".into());
        assert_eq!(state.lock().unwrap().phase, Phase::Empty);
        let current = Lease {
            epoch,
            generation: 8,
        };
        report_error(&state, &current, "current output lost".into());
        assert_eq!(state.lock().unwrap().phase, Phase::Error);
        assert!(
            state
                .lock()
                .unwrap()
                .error
                .as_ref()
                .unwrap()
                .contains("current output lost")
        );
    }
}
