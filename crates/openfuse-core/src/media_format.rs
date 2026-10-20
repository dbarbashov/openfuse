//! Portable media extension recognition.
use serde::{Deserialize, Serialize};
use std::path::Path;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaFormat {
    Wav,
    Mp3,
    Midi,
}
impl MediaFormat {
    pub fn from_path(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "wav" => Some(Self::Wav),
            "mp3" => Some(Self::Mp3),
            "mid" | "midi" => Some(Self::Midi),
            _ => None,
        }
    }
    pub fn extension(self) -> &'static str {
        match self {
            Self::Wav => "wav",
            Self::Mp3 => "mp3",
            Self::Midi => "mid",
        }
    }
}
