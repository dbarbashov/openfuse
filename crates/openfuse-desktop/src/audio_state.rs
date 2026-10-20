//! Explicit audio metadata discovery and versioned preferences. No stream is opened here.
use openfuse_audio::settings::{self, OutputInventory, OutputSettings};
use std::sync::mpsc::{self, Receiver, TryRecvError};

pub const SETTINGS_KEY: &str = "confuse.audio_output.v1";
const MAX_SETTINGS_BYTES: usize = 8192;

enum JobResult {
    Inventory(Result<Box<OutputInventory>, String>),
    Panel(Result<(), String>),
}
#[derive(Default)]
pub struct State {
    pub open: bool,
    pub demo: bool,
    pub draft: OutputSettings,
    pub inventory: Option<OutputInventory>,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub storage_warning: Option<String>,
    persisted: Option<String>,
    job: Option<Receiver<JobResult>>,
}
impl State {
    pub fn with_demo(demo: bool) -> Self {
        Self {
            demo,
            open: demo,
            inventory: demo.then(demo_inventory),
            ..Default::default()
        }
    }
    pub fn restore(&mut self, text: &str) -> Option<OutputSettings> {
        match parse_preferences(text) {
            Ok(settings) => {
                self.draft = settings.clone();
                Some(settings)
            }
            Err(error) => {
                self.storage_warning = Some(format!(
                    "Saved audio preferences were not loaded: {error}. They remain untouched until you explicitly apply a replacement."
                ));
                None
            }
        }
    }
    pub fn committed(&mut self, settings: &OutputSettings) {
        self.persisted = Some(serde_json::json!({"schema":1,"output":settings}).to_string());
        self.storage_warning = None;
        self.error = None;
        self.notice = Some(
            "Output preferences applied. Playback is paused; press Play to use this output.".into(),
        );
    }
    pub fn save(&self, storage: &mut dyn eframe::Storage) {
        if !self.demo
            && let Some(value) = &self.persisted
        {
            storage.set_string(SETTINGS_KEY, value.clone());
        }
    }
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }
    fn start(&mut self, task: impl FnOnce() -> JobResult + Send + 'static) {
        if self.busy() {
            return;
        }
        let (sender, receiver) = mpsc::sync_channel(1);
        match std::thread::Builder::new()
            .name("openfuse-audio-settings".into())
            .spawn(move || {
                let _ = sender.send(task());
            }) {
            Ok(_) => {
                self.job = Some(receiver);
                self.error = None;
            }
            Err(error) => self.error = Some(format!("Cannot start audio settings task: {error}")),
        }
    }
    pub fn refresh(&mut self) {
        if self.busy() {
            return;
        }
        if self.demo {
            self.inventory = Some(demo_inventory());
            self.notice = Some("Synthetic device inventory; no audio device was accessed.".into());
        } else {
            self.inventory = None;
            self.start(|| JobResult::Inventory(settings::enumerate_outputs().map(Box::new)));
        }
    }
    pub fn open_system_panel(&mut self) {
        if self.demo {
            self.notice = Some("Demo: the system panel launcher is disabled.".into());
        } else {
            self.start(|| JobResult::Panel(launch_system_panel()));
        }
    }
    pub fn poll(&mut self) {
        let Some(job) = &self.job else {
            return;
        };
        let result = match job.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.job = None;
                self.error = Some("Audio settings task ended without a result".into());
                return;
            }
        };
        self.job = None;
        match result {
            JobResult::Inventory(Ok(inventory)) => {
                self.notice = Some(format!(
                    "{} output devices found. Playback starts only with Play.",
                    inventory.devices.len()
                ));
                self.inventory = Some(*inventory);
            }
            JobResult::Panel(Ok(())) => {
                self.notice = Some(
                    "System audio panel opened. Refresh devices after changing system settings."
                        .into(),
                )
            }
            JobResult::Inventory(Err(error)) | JobResult::Panel(Err(error)) => {
                self.error = Some(error)
            }
        }
    }
    pub fn validate_draft(&self) -> Result<(), String> {
        self.draft.validate()?;
        if let Some(inventory) = &self.inventory {
            settings::plan_output(&self.draft, inventory)?;
        } else if self.draft != OutputSettings::default() {
            return Err(
                "Refresh devices to verify this output configuration before applying it.".into(),
            );
        }
        Ok(())
    }
}
fn parse_preferences(text: &str) -> Result<OutputSettings, String> {
    if text.len() > MAX_SETTINGS_BYTES {
        return Err("preference data exceeds 8 KiB".into());
    }
    let value: serde_json::Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let object = value.as_object().ok_or("expected an object")?;
    if object.len() != 2 || value.get("schema").and_then(|v| v.as_u64()) != Some(1) {
        return Err("unknown preference schema".into());
    }
    let output = object.get("output").ok_or("missing output settings")?;
    let fields = output.as_object().ok_or("invalid output settings")?;
    if fields
        .keys()
        .any(|key| !matches!(key.as_str(), "device" | "sample_rate" | "buffer_frames"))
    {
        return Err("unknown output setting".into());
    }
    if let Some(device) = fields.get("device").filter(|value| !value.is_null()) {
        let device = device.as_object().ok_or("invalid device identity")?;
        if device
            .keys()
            .any(|key| !matches!(key.as_str(), "host" | "name"))
        {
            return Err("unknown device field".into());
        }
    }
    let settings: OutputSettings =
        serde_json::from_value(output.clone()).map_err(|error| error.to_string())?;
    settings.validate()?;
    Ok(settings)
}
fn launch_system_panel() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("/usr/bin/open")
        .args(["-a", "Audio MIDI Setup"])
        .status();
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("control.exe")
        .arg("mmsys.cpl")
        .status();
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    return result
        .map_err(|error| format!("Cannot open system audio settings: {error}"))?
        .success()
        .then_some(())
        .ok_or_else(|| "System audio settings launcher failed".into());
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    Err("Open your desktop environment’s Sound settings manually. No portable system audio panel launcher is configured on this platform.".into())
}
pub fn demo_inventory() -> OutputInventory {
    use settings::{DeviceKey, OutputConfiguration, OutputDevice, OutputRange};
    let key = DeviceKey {
        host: "Synthetic demo".into(),
        name: "Studio output · demo".into(),
    };
    let device = OutputDevice {
        key: key.clone(),
        default_config: Some(OutputConfiguration {
            device: key,
            sample_rate: 48000,
            channels: 2,
            format: "f32".into(),
            buffer_frames: None,
        }),
        supported: vec![OutputRange {
            channels: 2,
            min_sample_rate: 44100,
            max_sample_rate: 96000,
            format: "f32".into(),
            min_buffer_frames: Some(64),
            max_buffer_frames: Some(2048),
        }],
        error: None,
    };
    OutputInventory {
        host: "Synthetic demo".into(),
        system_default: Some(device.clone()),
        devices: vec![device],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::Storage as _;
    #[derive(Default)]
    struct Storage(std::collections::HashMap<String, String>);
    impl eframe::Storage for Storage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.into(), value);
        }
        fn flush(&mut self) {}
    }
    #[test]
    fn render_startup_state_is_offline_and_does_not_persist_defaults() {
        let state = State::default();
        let mut storage = Storage::default();
        state.save(&mut storage);
        assert!(!state.busy());
        assert!(state.inventory.is_none());
        assert!(storage.0.is_empty());
    }
    #[test]
    fn future_malformed_and_oversized_preferences_remain_untouched() {
        for text in [
            "{\"schema\":2,\"output\":{}}".to_string(),
            "garbage".into(),
            "x".repeat(8193),
            "{\"schema\":1,\"output\":{\"future\":true}}".into(),
        ] {
            let mut storage = Storage::default();
            storage.set_string(SETTINGS_KEY, text.clone());
            let mut state = State::default();
            assert!(state.restore(&text).is_none());
            state.save(&mut storage);
            assert_eq!(storage.get_string(SETTINGS_KEY).unwrap(), text);
            assert!(state.storage_warning.is_some());
            state.committed(&OutputSettings::default());
            state.save(&mut storage);
            assert!(parse_preferences(&storage.get_string(SETTINGS_KEY).unwrap()).is_ok());
        }
    }
    #[test]
    fn selected_output_roundtrips_without_inventory_or_stream() {
        let mut state = State::default();
        let output = OutputSettings {
            device: Some(demo_inventory().devices[0].key.clone()),
            sample_rate: Some(48000),
            buffer_frames: Some(256),
        };
        state.committed(&output);
        let mut storage = Storage::default();
        state.save(&mut storage);
        let mut restored = State::default();
        assert_eq!(
            restored.restore(&storage.get_string(SETTINGS_KEY).unwrap()),
            Some(output)
        );
        assert!(!restored.busy());
        assert!(restored.inventory.is_none());
    }
    #[test]
    fn explicit_demo_refresh_and_unsupported_draft_are_visible() {
        let mut state = State {
            demo: true,
            ..Default::default()
        };
        state.refresh();
        assert!(!state.busy());
        state.draft.sample_rate = Some(192000);
        assert!(state.validate_draft().is_err());
        state.draft.sample_rate = Some(48000);
        assert!(state.validate_draft().is_ok());
        state.draft.device = Some(settings::DeviceKey {
            host: "gone".into(),
            name: "missing".into(),
        });
        assert!(state.validate_draft().is_err());
    }
    #[test]
    fn refresh_does_not_accumulate_workers_while_a_job_is_pending() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let mut state = State {
            demo: true,
            job: Some(receiver),
            ..Default::default()
        };
        state.refresh();
        assert!(state.inventory.is_none());
        assert!(state.busy());
        sender
            .send(JobResult::Inventory(Ok(Box::new(demo_inventory()))))
            .unwrap();
        state.poll();
        assert!(!state.busy());
        assert!(state.inventory.is_some());
    }
}
