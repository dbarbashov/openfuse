use std::{io::Cursor, sync::Arc};
use symphonia::core::{
    audio::SampleBuffer, codecs::DecoderOptions, errors::Error, formats::FormatOptions,
    io::MediaSourceStream, meta::MetadataOptions, probe::Hint,
};

pub const MAX_ENCODED_BYTES: usize = 256 * 1024 * 1024;
/// At most 256 MiB of stereo float PCM (about 12 minutes at 48 kHz).
pub const MAX_FRAMES: usize = 32 * 1024 * 1024;
#[derive(Clone, Debug)]
pub struct Audio {
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: Arc<Vec<[f32; 2]>>,
}
impl Audio {
    pub fn duration(&self) -> f64 {
        self.frames.len() as f64 / self.sample_rate as f64
    }
}
pub fn decode(bytes: Vec<u8>, extension: &str) -> Result<Audio, String> {
    decode_cancel(bytes, extension, || false)
}
pub(crate) fn decode_cancel(
    bytes: Vec<u8>,
    extension: &str,
    cancelled: impl Fn() -> bool,
) -> Result<Audio, String> {
    if bytes.is_empty() || bytes.len() > MAX_ENCODED_BYTES {
        return Err("Audio input is empty or exceeds 256 MiB".into());
    }
    let extension = extension.to_ascii_lowercase();
    if !matches!(extension.as_str(), "wav" | "mp3") {
        return Err(
            "Local playback supports WAV and MP3; MIDI requires an external instrument".into(),
        );
    }
    let source = MediaSourceStream::new(Box::new(Cursor::new(bytes)), Default::default());
    let mut hint = Hint::new();
    hint.with_extension(&extension);
    let mut format = symphonia::default::get_probe()
        .format(
            &hint,
            source,
            &FormatOptions {
                enable_gapless: true,
                ..Default::default()
            },
            &MetadataOptions::default(),
        )
        .map_err(|e| e.to_string())?
        .format;
    let track = format.default_track().ok_or("No audio track")?;
    let track_id = track.id;
    let expected_frames = track.codec_params.n_frames;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| e.to_string())?;
    let mut rate = 0;
    let mut channels = 0;
    let mut frames = Vec::new();
    loop {
        if cancelled() {
            return Err("Audio loading cancelled".into());
        }
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(format!("Audio packet: {e}")),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = decoder
            .decode(&packet)
            .map_err(|e| format!("Audio decode: {e}"))?;
        let spec = *decoded.spec();
        let count = spec.channels.count();
        if !matches!(count, 1 | 2) || !(8_000..=192_000).contains(&spec.rate) {
            return Err("Only mono/stereo audio at 8–192 kHz is supported".into());
        }
        if rate != 0 && (rate != spec.rate || channels != count as u16) {
            return Err("Audio format changes within the track".into());
        }
        rate = spec.rate;
        channels = count as u16;
        if frames.len().saturating_add(decoded.frames()) > MAX_FRAMES {
            return Err("Decoded track exceeds 256 MiB PCM limit".into());
        }
        let mut samples = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        samples.copy_interleaved_ref(decoded);
        for frame in samples.samples().chunks_exact(count) {
            if frame.iter().any(|v| !v.is_finite()) {
                return Err("Audio contains non-finite samples".into());
            }
            frames.push([frame[0].clamp(-1.0, 1.0), frame[count - 1].clamp(-1.0, 1.0)]);
        }
    }
    if frames.is_empty() {
        return Err("Audio track contains no decoded frames".into());
    }
    // WAV declares a precise PCM frame count; partial/truncated input must not appear complete.
    if extension == "wav" && expected_frames.is_some_and(|n| n != frames.len() as u64) {
        return Err("Truncated WAV data or inconsistent frame count".into());
    }
    Ok(Audio {
        sample_rate: rate,
        channels,
        frames: Arc::new(frames),
    })
}
