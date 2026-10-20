//! Backing-track metadata. Host playback and amplifier transport are separate.
//! Static contract: PresetLib BandModule file constructor and AsFileOutput.
use super::{
    PresetDocument, Result, append_child, attribute, child, direct_text, edit_attributes,
    edit_text, error, escape, parse_xml,
};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Band {
    pub band_type: u8,
    pub repeat: u8,
    pub location: u8,
    pub filename: String,
    pub audio_mix: i32,
    pub balance: u16,
    pub speed: u16,
    pub pitch: u16,
    pub tempo: u16,
    pub transpose: i8,
    pub drum_solo: bool,
    pub count_in: i8,
}
impl Default for Band {
    fn default() -> Self {
        Self {
            band_type: 0,
            repeat: 0,
            location: 6,
            filename: "No Band".into(),
            audio_mix: 0,
            balance: 29127,
            speed: 100,
            pitch: 0,
            tempo: 0,
            transpose: 0,
            drum_solo: false,
            count_in: 0,
        }
    }
}
impl Band {
    pub fn allows_midi(product: u32) -> bool {
        product == 10
    }
    fn reset_audio(&mut self) {
        self.audio_mix = 0;
        self.balance = 29127;
        self.speed = 100;
        self.pitch = 0;
    }
    fn reset_midi(&mut self) {
        self.tempo = 0;
        self.transpose = 0;
        self.drum_solo = false;
        self.count_in = 0;
    }
    /// Original filename setter selects the effective branch (only `.MID`).
    pub fn set_filename(&mut self, filename: &str) -> Result<()> {
        validate_filename(filename)?;
        self.filename = filename.into();
        if !filename.is_empty() {
            if filename != "No Band"
                && filename
                    .rsplit('.')
                    .next()
                    .is_some_and(|s| s.eq_ignore_ascii_case("mid"))
            {
                self.band_type = 1;
                self.reset_midi();
            } else {
                self.band_type = 0;
                self.reset_audio();
            }
        }
        Ok(())
    }
    /// Original PC audio selection uses basename and location 9, not a path.
    pub fn link_pc_audio(&mut self, filename: &str) -> Result<()> {
        if filename.is_empty()
            || filename.contains(['/', '\\'])
            || !matches!(
                crate::media_format::MediaFormat::from_path(std::path::Path::new(filename)),
                Some(crate::media_format::MediaFormat::Wav | crate::media_format::MediaFormat::Mp3)
            )
        {
            return Err(error("Select a WAV or MP3 basename for PC backing track"));
        }
        self.set_filename(filename)?;
        self.location = 9;
        self.audio_mix = 1;
        self.speed = 32767;
        self.pitch = 32767;
        Ok(())
    }
    pub fn clear(&mut self) {
        self.filename = "No Band".into();
        self.location = 6;
        self.band_type = 0;
        self.reset_audio();
    }
    /// Canonical original branch serialization, for fresh Band elements only.
    pub fn to_xml(&self) -> Result<String> {
        validate_filename(&self.filename)?;
        let mut xml = format!(
            "<Band Type=\"{}\" Repeat=\"{}\"><SongFile Location=\"{}\">{}</SongFile>",
            self.band_type,
            self.repeat,
            self.location,
            escape(&self.filename, false)
        );
        for (key, value) in self.fields() {
            xml.push_str(&format!("<{key}>{value}</{key}>"));
        }
        xml.push_str("</Band>");
        Ok(xml)
    }
    fn fields(&self) -> [(&'static str, String); 8] {
        let audio = self.band_type == 0;
        [
            (
                "AudioMix",
                if audio {
                    (self.audio_mix as u8).to_string()
                } else {
                    String::new()
                },
            ),
            (
                "Balance",
                if audio {
                    self.balance.to_string()
                } else {
                    String::new()
                },
            ),
            (
                "Speed",
                if audio {
                    self.speed.to_string()
                } else {
                    String::new()
                },
            ),
            (
                "Pitch",
                if audio {
                    self.pitch.to_string()
                } else {
                    String::new()
                },
            ),
            (
                "Tempo",
                if !audio {
                    self.tempo.to_string()
                } else {
                    String::new()
                },
            ),
            (
                "Transpose",
                if !audio {
                    self.transpose.to_string()
                } else {
                    String::new()
                },
            ),
            (
                "DrumSolo",
                if !audio {
                    u8::from(self.drum_solo).to_string()
                } else {
                    String::new()
                },
            ),
            (
                "CountIn",
                if !audio {
                    self.count_in.to_string()
                } else {
                    String::new()
                },
            ),
        ]
    }
}
fn validate_filename(name: &str) -> Result<()> {
    if name.len() > 4096
        || name
            .chars()
            .any(|c| !matches!(c, '\t' | '\n' | '\r') && c < '\u{20}')
    {
        return Err(error("Invalid or oversized Band filename"));
    }
    Ok(())
}
impl PresetDocument {
    pub fn band(&self) -> Result<Option<Band>> {
        let document = parse_xml(&self.xml)?;
        let Some(node) = child(document.root_element(), "Band")? else {
            return Ok(None);
        };
        let attr =
            |node, key| attribute(node, key).ok_or_else(|| error(format!("Band is missing {key}")));
        let mut band = Band {
            band_type: parse(attr(node, "Type")?, "Type")?,
            repeat: parse(attr(node, "Repeat")?, "Repeat")?,
            ..Default::default()
        };
        let song = child(node, "SongFile")?.ok_or_else(|| error("Band is missing SongFile"))?;
        band.location = parse(attr(song, "Location")?, "Location")?;
        if song.children().any(|n| n.is_element()) {
            return Err(error("SongFile cannot contain nested elements"));
        }
        band.set_filename(&direct_text(song))?;
        let scalar = |key| -> Result<String> {
            let leaf = child(node, key)?.ok_or_else(|| error(format!("Band is missing {key}")))?;
            if leaf.children().any(|n| n.is_element()) {
                return Err(error("Band scalar cannot contain nested elements"));
            }
            Ok(direct_text(leaf))
        };
        if band.band_type == 0 {
            band.audio_mix = parse::<i32>(&scalar("AudioMix")?, "AudioMix")?.max(0);
            band.balance = parse(&scalar("Balance")?, "Balance")?;
            band.speed = parse(&scalar("Speed")?, "Speed")?;
            band.pitch = parse(&scalar("Pitch")?, "Pitch")?;
        } else {
            band.tempo = parse(&scalar("Tempo")?, "Tempo")?;
            band.transpose = parse(&scalar("Transpose")?, "Transpose")?;
            band.drum_solo = parse::<u8>(&scalar("DrumSolo")?, "DrumSolo")? != 0;
            band.count_in = parse(&scalar("CountIn")?, "CountIn")?;
        }
        Ok(Some(band))
    }
    /// Apply atomically; unchanged known fields and opaque extensions keep bytes.
    pub fn set_band(&mut self, value: &Band) -> Result<()> {
        validate_filename(&value.filename)?;
        let current = self.band()?;
        if current.as_ref() == Some(value) {
            return Ok(());
        }
        let document = parse_xml(&self.xml)?;
        let root = document.root_element();
        let mut edits = Vec::new();
        if let Some(node) = child(root, "Band")? {
            let previous = current.expect("present Band was parsed");
            let mut attrs = Vec::new();
            if previous.band_type != value.band_type {
                attrs.push(("Type", value.band_type.to_string()));
            }
            if previous.repeat != value.repeat {
                attrs.push(("Repeat", value.repeat.to_string()));
            }
            edit_attributes(&self.xml, node, &attrs, &mut edits)?;
            let song = child(node, "SongFile")?.expect("parsed SongFile");
            if previous.location != value.location {
                edit_attributes(
                    &self.xml,
                    song,
                    &[("Location", value.location.to_string())],
                    &mut edits,
                )?;
            }
            if previous.filename != value.filename {
                edit_text(&self.xml, song, &value.filename, &mut edits)?;
            }
            let mut additions = String::new();
            let previous_fields = previous.fields();
            for ((key, next), (_, old)) in value.fields().into_iter().zip(previous_fields) {
                if next == old && previous.band_type == value.band_type {
                    continue;
                }
                if let Some(leaf) = child(node, key)? {
                    if leaf.children().any(|n| n.is_element()) {
                        return Err(error("Cannot overwrite nested Band scalar extension"));
                    }
                    edit_text(&self.xml, leaf, &next, &mut edits)?;
                } else {
                    additions.push_str(&format!("<{key}>{next}</{key}>"));
                }
            }
            if !additions.is_empty() {
                append_child(&self.xml, node, &additions, &mut edits)?;
            }
        } else {
            append_child(&self.xml, root, &value.to_xml()?, &mut edits)?;
        }
        let mut candidate = self.clone();
        candidate.apply(edits)?;
        let actual = candidate
            .band()?
            .ok_or_else(|| error("Band edit did not produce Band"))?;
        // A mismatched filename and explicit type would read back differently.
        if actual.band_type != value.band_type {
            return Err(error("Band type conflicts with filename extension"));
        }
        *self = candidate;
        Ok(())
    }
}
fn parse<T: std::str::FromStr>(value: &str, key: &str) -> Result<T> {
    value
        .trim()
        .parse()
        .map_err(|_| error(format!("Invalid Band {key}")))
}
