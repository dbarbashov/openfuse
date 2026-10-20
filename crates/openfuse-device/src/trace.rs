//! Opt-in, bounded recording at the normalized Connection boundary.
mod recorder;
mod replay;
#[cfg(test)]
mod tests;

pub use recorder::Recorder;
pub(crate) use recorder::RecordingBackend;
pub use replay::{ReplayResult, replay_bootstrap};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{io::Read, path::Path};

pub const MAX_EVENTS: usize = 8192;
pub const MAX_FILE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    Simulator,
    HardwareTransport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub product_id: u16,
    pub interface_number: i32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Event {
    Open { profile: Profile },
    OpenFailed { profile: Profile },
    Tx { bytes: Vec<u8>, succeeded: bool },
    Rx { bytes: Vec<u8> },
    Fault { framing: bool },
    Close,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub at_ms: u64,
    pub event: Event,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Trace {
    pub schema: u32,
    pub origin: Origin,
    pub app_version: String,
    pub os: String,
    pub architecture: String,
    pub started_unix_ms: u64,
    pub ended_ms: u64,
    pub dropped_events: u64,
    pub entries: Vec<Entry>,
}
impl Trace {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != 1 || self.entries.len() > MAX_EVENTS {
            return Err("Unsupported trace schema or event limit".into());
        }
        if [&self.app_version, &self.os, &self.architecture]
            .iter()
            .any(|s| s.len() > 128)
        {
            return Err("Trace metadata exceeds limit".into());
        }
        let mut previous = 0;
        let mut open = false;
        for entry in &self.entries {
            if entry.at_ms < previous || entry.at_ms > self.ended_ms {
                return Err("Trace timestamps are not ordered or exceed its end".into());
            }
            previous = entry.at_ms;
            match &entry.event {
                Event::Open { .. } if !open => open = true,
                Event::Close if open => open = false,
                Event::OpenFailed { .. } if !open => (),
                Event::Rx { bytes } | Event::Tx { bytes, .. } if open => {
                    if bytes.len() != 64 {
                        return Err("Trace report must contain exactly 64 bytes".into());
                    }
                }
                Event::Fault { .. } if open => (),
                _ => return Err("Invalid trace connection lifecycle".into()),
            }
        }
        Ok(())
    }
    /// Digest of the canonical serialized trace, independent of JSON whitespace.
    pub fn digest(&self) -> Result<String, String> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }
}

pub fn read_bounded(path: &Path) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Choose a regular JSON file".into());
    }
    let mut text = String::new();
    file.take(MAX_FILE_BYTES as u64 + 1)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    if text.len() > MAX_FILE_BYTES {
        return Err("Trace bundle exceeds 16 MiB".into());
    }
    Ok(text)
}

/// Atomic create-only publication; never replaces an existing artifact.
pub fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    file.write_all(bytes).map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist_noclobber(path)
        .map_err(|e| format!("Export did not overwrite an existing file: {e}"))?;
    Ok(())
}
