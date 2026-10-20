use crate::{Audio, Phase, Snapshot, Transport, decode::decode_cancel, output};
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicU64, Ordering},
};
struct Job {
    generation: u64,
    id: String,
    bytes: Vec<u8>,
    extension: String,
}
#[derive(Default)]
struct Pending {
    closed: bool,
    job: Option<Job>,
}
struct Loaded {
    generation: u64,
    id: String,
    result: Result<Audio, String>,
}
/// A single decoder worker and a replaceable one-job queue bound memory and cancel stale loads.
pub struct Player {
    output_settings: crate::settings::OutputSettings,
    active_output: Option<crate::settings::OutputConfiguration>,
    output_epoch: Arc<AtomicU64>,
    state: Arc<Mutex<Transport>>,
    pending: Arc<(Mutex<Pending>, Condvar)>,
    epoch: Arc<AtomicU64>,
    loaded: Arc<Mutex<Option<Loaded>>>,
    stream: Option<cpal::Stream>,
    headless: bool,
}
impl Default for Player {
    fn default() -> Self {
        Self::new()
    }
}
impl Player {
    pub fn new() -> Self {
        Self::create(false)
    }
    pub fn headless() -> Self {
        Self::create(true)
    }
    fn create(headless: bool) -> Self {
        let state = Arc::new(Mutex::new(Transport::default()));
        let pending = Arc::new((Mutex::new(Pending::default()), Condvar::new()));
        let epoch = Arc::new(AtomicU64::new(0));
        let loaded = Arc::new(Mutex::new(None));
        let worker_loaded = loaded.clone();
        let worker_pending = pending.clone();
        let worker_epoch = epoch.clone();
        std::thread::spawn(move || {
            loop {
                let (mutex, wake) = &*worker_pending;
                let mut pending = mutex.lock().unwrap();
                while pending.job.is_none() && !pending.closed {
                    pending = wake.wait(pending).unwrap();
                }
                if pending.closed {
                    break;
                }
                let job = pending.job.take().unwrap();
                drop(pending);
                let result = decode_cancel(job.bytes, &job.extension, || {
                    worker_epoch.load(Ordering::Acquire) != job.generation
                });
                let mut slot = worker_loaded.lock().unwrap();
                if worker_epoch.load(Ordering::Acquire) == job.generation {
                    *slot = Some(Loaded {
                        generation: job.generation,
                        id: job.id,
                        result,
                    });
                }
            }
        });
        Self {
            output_settings: Default::default(),
            active_output: None,
            output_epoch: Arc::new(AtomicU64::new(0)),
            state,
            pending,
            epoch,
            loaded,
            stream: None,
            headless,
        }
    }
    /// Applies intent without enumeration or stream creation. Explicit Play
    /// resolves fresh capabilities; changing settings never reroutes live sound.
    pub fn configure_output(
        &mut self,
        settings: crate::settings::OutputSettings,
    ) -> Result<(), String> {
        settings.validate()?;
        self.pause();
        let mut state = self.state.lock().unwrap();
        if state.audio.is_some() {
            state.phase = Phase::Paused;
            state.error = None;
        }
        self.output_settings = settings;
        Ok(())
    }
    pub fn output_settings(&self) -> &crate::settings::OutputSettings {
        &self.output_settings
    }
    pub fn active_output(&self) -> Option<&crate::settings::OutputConfiguration> {
        self.active_output.as_ref()
    }
    pub fn load(&mut self, id: String, bytes: Vec<u8>, extension: &str) -> Result<u64, String> {
        if bytes.is_empty() || bytes.len() > crate::MAX_ENCODED_BYTES {
            return Err("Audio input is empty or exceeds 256 MiB".into());
        }
        if !matches!(extension.to_ascii_lowercase().as_str(), "wav" | "mp3") {
            return Err("Local playback supports WAV and MP3".into());
        }
        self.clear();
        let generation = self.epoch.load(Ordering::Acquire);
        {
            let mut state = self.state.lock().unwrap();
            state.id = Some(id.clone());
            state.phase = Phase::Loading;
        }
        let (pending, wake) = &*self.pending;
        pending.lock().unwrap().job = Some(Job {
            generation,
            id,
            bytes,
            extension: extension.to_owned(),
        });
        wake.notify_one();
        Ok(generation)
    }
    pub fn poll(&mut self) {
        if let Some(loaded) = self.loaded.lock().unwrap().take() {
            let mut state = self.state.lock().unwrap();
            if loaded.generation != state.generation {
                return;
            }
            match loaded.result {
                Ok(audio) => {
                    if let Err(e) = state.set_audio(loaded.id, audio) {
                        state.phase = Phase::Error;
                        state.error = Some(e);
                    }
                }
                Err(error) => {
                    state.phase = Phase::Error;
                    state.error = Some(error);
                }
            }
        }
        if matches!(
            self.state.lock().unwrap().phase,
            Phase::Error | Phase::Ended
        ) {
            self.release_output();
        }
    }
    fn release_output(&mut self) {
        self.output_epoch.fetch_add(1, Ordering::AcqRel);
        self.stream = None;
        self.active_output = None;
    }
    pub fn clear(&mut self) {
        let generation = self.epoch.fetch_add(1, Ordering::AcqRel) + 1;
        self.pending.0.lock().unwrap().job = None;
        self.release_output();
        {
            let mut state = self.state.lock().unwrap();
            state.clear();
            state.generation = generation;
        }
        *self.loaded.lock().unwrap() = None;
    }
    pub fn play(&mut self) -> Result<(), String> {
        if self.state.lock().unwrap().audio.is_none() {
            return Err("Load an audio track first".into());
        }
        if self.headless {
            return Err("Audio output disabled in headless mode".into());
        }
        if self.stream.is_none() {
            {
                let mut state = self.state.lock().unwrap();
                state.phase = Phase::Paused;
                state.error = None;
            }
            match output::open(
                self.state.clone(),
                &self.output_settings,
                output::Lease {
                    epoch: self.output_epoch.clone(),
                    generation: self.output_epoch.load(Ordering::Acquire),
                },
            ) {
                Ok((stream, actual)) => {
                    self.stream = Some(stream);
                    self.active_output = Some(actual);
                }
                Err(error) => {
                    let mut state = self.state.lock().unwrap();
                    state.phase = Phase::Error;
                    state.error = Some(error.clone());
                    return Err(error);
                }
            }
        }
        let mut state = self.state.lock().unwrap();
        if let Some(error) = &state.error {
            return Err(error.clone());
        }
        state.play()
    }
    pub fn pause(&mut self) {
        self.release_output();
        self.state.lock().unwrap().pause();
    }
    pub fn stop(&mut self) {
        if self.snapshot().phase == Phase::Loading {
            self.clear();
            return;
        }
        self.release_output();
        self.state.lock().unwrap().stop();
    }
    pub fn seek(&mut self, seconds: f64) -> Result<(), String> {
        self.state.lock().unwrap().seek(seconds)
    }
    pub fn set_volume(&mut self, volume: f32) -> Result<(), String> {
        self.state.lock().unwrap().set_volume(volume)
    }
    pub fn set_balance(&mut self, balance: f32) -> Result<(), String> {
        self.state.lock().unwrap().set_balance(balance)
    }
    pub fn set_repeat(&mut self, repeat: bool) {
        self.state.lock().unwrap().set_repeat(repeat);
    }
    pub fn snapshot(&self) -> Snapshot {
        self.state.lock().unwrap().snapshot()
    }
}
impl Drop for Player {
    fn drop(&mut self) {
        self.clear();
        let (pending, wake) = &*self.pending;
        pending.lock().unwrap().closed = true;
        wake.notify_one();
    }
}
