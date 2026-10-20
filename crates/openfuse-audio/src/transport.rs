use crate::Audio;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Empty,
    Loading,
    Ready,
    Playing,
    Paused,
    Ended,
    Error,
}
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub id: Option<String>,
    pub generation: u64,
    pub phase: Phase,
    pub duration: f64,
    pub position: f64,
    pub sample_rate: u32,
    pub channels: u16,
    pub volume: f32,
    pub balance: f32,
    pub repeat: bool,
    pub error: Option<String>,
}
/// Deterministic transport shared by offline tests and the physical output callback.
/// Rendering does not allocate. Device-rate adaptation uses linear interpolation.
pub struct Transport {
    pub(crate) audio: Option<Audio>,
    pub(crate) id: Option<String>,
    pub(crate) generation: u64,
    pub(crate) phase: Phase,
    cursor: f64,
    volume: f32,
    balance: f32,
    repeat: bool,
    pub(crate) error: Option<String>,
}
impl Default for Transport {
    fn default() -> Self {
        Self {
            audio: None,
            id: None,
            generation: 0,
            phase: Phase::Empty,
            cursor: 0.0,
            volume: 0.5,
            balance: 0.0,
            repeat: false,
            error: None,
        }
    }
}
impl Transport {
    pub fn set_audio(&mut self, id: String, audio: Audio) -> Result<(), String> {
        if audio.frames.is_empty() || audio.sample_rate == 0 || !matches!(audio.channels, 1 | 2) {
            return Err("Invalid decoded audio".into());
        }
        self.audio = Some(audio);
        self.id = Some(id);
        self.cursor = 0.0;
        self.phase = Phase::Ready;
        self.error = None;
        Ok(())
    }
    pub fn clear(&mut self) {
        self.audio = None;
        self.id = None;
        self.cursor = 0.0;
        self.phase = Phase::Empty;
        self.error = None;
    }
    pub fn play(&mut self) -> Result<(), String> {
        let audio = self.audio.as_ref().ok_or("Load an audio track first")?;
        if self.cursor >= audio.frames.len() as f64 {
            self.cursor = 0.0;
        }
        self.phase = Phase::Playing;
        self.error = None;
        Ok(())
    }
    pub fn pause(&mut self) {
        if self.phase == Phase::Playing {
            self.phase = Phase::Paused;
        }
    }
    pub fn stop(&mut self) {
        if self.audio.is_some() {
            self.phase = Phase::Ready;
            self.cursor = 0.0;
        }
    }
    pub fn seek(&mut self, seconds: f64) -> Result<(), String> {
        let audio = self.audio.as_ref().ok_or("Load an audio track first")?;
        if !seconds.is_finite() || !(0.0..=audio.duration()).contains(&seconds) {
            return Err("Seek is outside track duration".into());
        }
        self.cursor = seconds * audio.sample_rate as f64;
        if self.phase == Phase::Ended {
            self.phase = Phase::Paused;
        }
        Ok(())
    }
    pub fn set_volume(&mut self, value: f32) -> Result<(), String> {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err("Volume must be 0–1".into());
        }
        self.volume = value;
        Ok(())
    }
    pub fn set_balance(&mut self, value: f32) -> Result<(), String> {
        if !value.is_finite() || !(-1.0..=1.0).contains(&value) {
            return Err("Balance must be −1–1".into());
        }
        self.balance = value;
        Ok(())
    }
    pub fn set_repeat(&mut self, value: bool) {
        self.repeat = value;
    }
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            id: self.id.clone(),
            generation: self.generation,
            phase: self.phase,
            duration: self.audio.as_ref().map_or(0.0, Audio::duration),
            position: self
                .audio
                .as_ref()
                .map_or(0.0, |a| self.cursor / a.sample_rate as f64),
            sample_rate: self.audio.as_ref().map_or(0, |a| a.sample_rate),
            channels: self.audio.as_ref().map_or(0, |a| a.channels),
            volume: self.volume,
            balance: self.balance,
            repeat: self.repeat,
            error: self.error.clone(),
        }
    }
    pub fn next_frame(&mut self, output_rate: u32) -> [f32; 2] {
        if self.phase != Phase::Playing || output_rate == 0 {
            return [0.0; 2];
        }
        let Some(audio) = self.audio.as_ref() else {
            return [0.0; 2];
        };
        let length = audio.frames.len();
        if self.cursor >= length as f64 {
            if self.repeat {
                self.cursor %= length as f64;
            } else {
                self.cursor = length as f64;
                self.phase = Phase::Ended;
                return [0.0; 2];
            }
        }
        let i = self.cursor.floor() as usize;
        let next = if i + 1 < length {
            i + 1
        } else if self.repeat {
            0
        } else {
            i
        };
        let fraction = (self.cursor - i as f64) as f32;
        let left = (audio.frames[i][0] + fraction * (audio.frames[next][0] - audio.frames[i][0]))
            * self.volume
            * (1.0 - self.balance.max(0.0));
        let right = (audio.frames[i][1] + fraction * (audio.frames[next][1] - audio.frames[i][1]))
            * self.volume
            * (1.0 + self.balance.min(0.0));
        self.cursor += audio.sample_rate as f64 / output_rate as f64;
        if self.cursor >= length as f64 {
            if self.repeat {
                self.cursor %= length as f64;
            } else {
                self.cursor = length as f64;
                self.phase = Phase::Ended;
            }
        }
        [left, right]
    }
}
