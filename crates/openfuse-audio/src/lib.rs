//! Offline WAV/MP3 decoding and explicit, local computer playback.
//! Constructing/loading a player never opens an audio device or starts playback.
mod decode;
mod output;
mod player;
pub mod settings;
mod transport;
pub use decode::{Audio, MAX_ENCODED_BYTES, MAX_FRAMES, decode};
pub use player::Player;
pub use transport::{Phase, Snapshot, Transport};
