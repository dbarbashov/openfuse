//! Serializable user intent and metadata-only output discovery. Planning never
//! creates an audio stream. Named devices never silently fall back to default.
use cpal::traits::{DeviceTrait, HostTrait};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceKey {
    pub host: String,
    pub name: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputSettings {
    pub device: Option<DeviceKey>,
    pub sample_rate: Option<u32>,
    pub buffer_frames: Option<u32>,
}
impl OutputSettings {
    pub fn validate(&self) -> Result<(), String> {
        if self.device.as_ref().is_some_and(|key| {
            key.host.is_empty()
                || key.name.is_empty()
                || key.host.len() > 1024
                || key.name.len() > 1024
        }) {
            return Err("Invalid audio device identity".into());
        }
        if self.sample_rate.is_some_and(|v| v == 0 || v > 1_000_000) {
            return Err("Sample rate must be 1–1000000 Hz".into());
        }
        if self.buffer_frames.is_some_and(|v| v == 0 || v > 1_048_576) {
            return Err("Buffer must be 1–1048576 frames".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputConfiguration {
    pub device: DeviceKey,
    pub sample_rate: u32,
    pub channels: u16,
    pub format: String,
    /// Requested frame count; the driver can use a different callback quantum.
    pub buffer_frames: Option<u32>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputRange {
    pub channels: u16,
    pub min_sample_rate: u32,
    pub max_sample_rate: u32,
    pub format: String,
    pub min_buffer_frames: Option<u32>,
    pub max_buffer_frames: Option<u32>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputDevice {
    pub key: DeviceKey,
    pub default_config: Option<OutputConfiguration>,
    pub supported: Vec<OutputRange>,
    pub error: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputInventory {
    pub host: String,
    /// Kept separately: the system default remains usable even when multiple
    /// endpoints have the same display name and named selection is ambiguous.
    pub system_default: Option<OutputDevice>,
    pub devices: Vec<OutputDevice>,
}

pub fn enumerate_outputs() -> Result<OutputInventory, String> {
    let host = cpal::default_host();
    let name = host.id().name().to_owned();
    let system_default = host.default_output_device().map(|d| describe(&d, &name));
    let devices = host
        .output_devices()
        .map_err(|e| e.to_string())?
        .take(256)
        .map(|d| describe(&d, &name))
        .collect();
    Ok(OutputInventory {
        host: name,
        system_default,
        devices,
    })
}

pub(crate) fn describe(device: &cpal::Device, host: &str) -> OutputDevice {
    let name = device.name();
    let mut error = name.as_ref().err().map(ToString::to_string);
    let key = DeviceKey {
        host: host.into(),
        name: name.unwrap_or_else(|_| "Unnamed output (unavailable)".into()),
    };
    let default_config = match device.default_output_config() {
        Ok(c) => Some(OutputConfiguration {
            device: key.clone(),
            sample_rate: c.sample_rate().0,
            channels: c.channels(),
            format: format_name(c.sample_format()).into(),
            buffer_frames: None,
        }),
        Err(e) => {
            error.get_or_insert_with(|| e.to_string());
            None
        }
    };
    let supported = match device.supported_output_configs() {
        Ok(configs) => configs
            .take(512)
            .map(|c| {
                let (min, max) = match c.buffer_size() {
                    cpal::SupportedBufferSize::Range { min, max } => (Some(*min), Some(*max)),
                    cpal::SupportedBufferSize::Unknown => (None, None),
                };
                OutputRange {
                    channels: c.channels(),
                    min_sample_rate: c.min_sample_rate().0,
                    max_sample_rate: c.max_sample_rate().0,
                    format: format_name(c.sample_format()).into(),
                    min_buffer_frames: min,
                    max_buffer_frames: max,
                }
            })
            .collect(),
        Err(e) => {
            error.get_or_insert_with(|| e.to_string());
            Vec::new()
        }
    };
    OutputDevice {
        key,
        default_config,
        supported,
        error,
    }
}

pub fn plan_output(
    settings: &OutputSettings,
    inventory: &OutputInventory,
) -> Result<OutputConfiguration, String> {
    settings.validate()?;
    let device = match &settings.device {
        None => inventory
            .system_default
            .as_ref()
            .ok_or("No system default audio output")?,
        Some(key) => {
            if key.host != inventory.host {
                return Err(format!(
                    "Saved audio host {} is unavailable; choose an output on {}",
                    key.host, inventory.host
                ));
            }
            let matches: Vec<_> = inventory.devices.iter().filter(|d| &d.key == key).collect();
            match matches.as_slice() {
                [one] => *one,
                [] => {
                    return Err(format!(
                        "Audio output '{}' is unavailable; choose an output explicitly",
                        key.name
                    ));
                }
                _ => {
                    return Err(format!(
                        "Several audio outputs are named '{}'; use system default or give devices distinct names",
                        key.name
                    ));
                }
            }
        }
    };
    plan_device(settings, device)
}

pub(crate) fn plan_device(
    settings: &OutputSettings,
    device: &OutputDevice,
) -> Result<OutputConfiguration, String> {
    settings.validate()?;
    if let Some(error) = &device.error {
        return Err(format!("Cannot query audio output: {error}"));
    }
    let default = device
        .default_config
        .as_ref()
        .ok_or("Output has no default configuration")?;
    let rate = settings.sample_rate.unwrap_or(default.sample_rate);
    let mut candidates: Vec<_> = device
        .supported
        .iter()
        .filter(|c| {
            (1..=32).contains(&c.channels)
                && c.format != "unsupported"
                && rate >= c.min_sample_rate
                && rate <= c.max_sample_rate
                && settings.buffer_frames.is_none_or(|b| {
                    match (c.min_buffer_frames, c.max_buffer_frames) {
                        (Some(min), Some(max)) => b >= min && b <= max,
                        _ => false,
                    }
                })
        })
        .collect();
    candidates.sort_by_key(|c| {
        (
            c.channels != default.channels,
            c.format != default.format,
            c.channels != 2,
            c.format != "f32",
            c.channels,
        )
    });
    let chosen = candidates.first().ok_or_else(|| format!("Output does not advertise {} Hz with the requested buffer. Use supported values or driver-default buffer.", rate))?;
    Ok(OutputConfiguration {
        device: device.key.clone(),
        sample_rate: rate,
        channels: chosen.channels,
        format: chosen.format.clone(),
        buffer_frames: settings.buffer_frames,
    })
}

pub(crate) fn format_name(format: cpal::SampleFormat) -> &'static str {
    match format {
        cpal::SampleFormat::F32 => "f32",
        cpal::SampleFormat::F64 => "f64",
        cpal::SampleFormat::I8 => "i8",
        cpal::SampleFormat::I16 => "i16",
        cpal::SampleFormat::I32 => "i32",
        cpal::SampleFormat::I64 => "i64",
        cpal::SampleFormat::U8 => "u8",
        cpal::SampleFormat::U16 => "u16",
        cpal::SampleFormat::U32 => "u32",
        cpal::SampleFormat::U64 => "u64",
        _ => "unsupported",
    }
}

pub(crate) fn resolve(
    settings: &OutputSettings,
) -> Result<(cpal::Device, OutputConfiguration), String> {
    settings.validate()?;
    let host = cpal::default_host();
    let host_name = host.id().name();
    let device = match &settings.device {
        None => host
            .default_output_device()
            .ok_or("No system default audio output")?,
        Some(key) => {
            if key.host != host_name {
                return Err(format!("Saved audio host {} is unavailable", key.host));
            }
            let mut matches = host
                .output_devices()
                .map_err(|e| e.to_string())?
                .filter(|d| d.name().ok().as_deref() == Some(key.name.as_str()));
            let device = matches.next().ok_or_else(|| {
                format!(
                    "Audio output '{}' is unavailable; no fallback was opened",
                    key.name
                )
            })?;
            if matches.next().is_some() {
                return Err(format!("Audio output name '{}' is ambiguous", key.name));
            }
            device
        }
    };
    let actual = plan_device(settings, &describe(&device, host_name))?;
    Ok((device, actual))
}
