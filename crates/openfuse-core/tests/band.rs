use openfuse_core::preset::{PresetDocument, band::Band};
fn document(band: &Band) -> PresetDocument {
    PresetDocument::parse(&format!(
        "<Preset ProductId=\"10\">{}</Preset>",
        band.to_xml().unwrap()
    ))
    .unwrap()
}
#[test]
fn defaults_and_filename_resets_follow_original() {
    let mut band = Band::default();
    assert_eq!(
        (band.balance, band.speed, band.pitch, band.tempo),
        (29127, 100, 0, 0)
    );
    band.tempo = 120;
    band.transpose = 12;
    band.repeat = 255;
    band.set_filename("SONG.MiD").unwrap();
    assert_eq!(band.band_type, 1);
    assert_eq!((band.tempo, band.transpose), (0, 0));
    band.set_filename("song.midi").unwrap();
    assert_eq!(band.band_type, 0);
    band.band_type = 254;
    band.set_filename("").unwrap();
    assert_eq!(band.band_type, 254);
    band.link_pc_audio("A & B.wav").unwrap();
    assert_eq!(
        (band.location, band.audio_mix, band.speed, band.pitch),
        (9, 1, 32767, 32767)
    );
    assert_eq!(band.repeat, 255);
    let before = band.clone();
    assert!(band.link_pc_audio("../x.mp3").is_err());
    assert_eq!(band, before);
    band.clear();
    assert_eq!(band.filename, "No Band");
    assert_eq!((band.location, band.repeat), (6, 255));
    assert!(Band::allows_midi(10));
    assert!(!Band::allows_midi(13));
}
#[test]
fn surgical_edit_preserves_unknowns_inactive_values_and_exact_raw_encoding() {
    let raw = "<Preset ProductId='13'><Band Type='1' Repeat='001' ext='keep'><SongFile Location='250'>Rock&amp;Roll.wav</SongFile><AudioMix>-19</AudioMix><Balance>0007</Balance><Speed>32767</Speed><Pitch>0</Pitch><Tempo>12345</Tempo><!--kept--><Extension value='opaque'/></Band></Preset>";
    let mut doc = PresetDocument::parse(raw).unwrap();
    let mut band = doc.band().unwrap().unwrap();
    assert_eq!(
        (band.band_type, band.repeat, band.location, band.audio_mix),
        (0, 1, 250, 0)
    );
    doc.set_band(&band).unwrap();
    assert_eq!(doc.to_xml().unwrap(), raw);
    band.repeat = 0;
    doc.set_band(&band).unwrap();
    assert_eq!(
        doc.to_xml().unwrap(),
        raw.replace("Repeat='001'", "Repeat='0'")
    );
}
#[test]
fn inserting_link_and_clear_are_atomic_and_xml_safe() {
    let original = "<Preset ProductId='13'><Unknown x='keep'/></Preset>";
    let mut doc = PresetDocument::parse(original).unwrap();
    assert!(doc.band().unwrap().is_none());
    let mut band = Band::default();
    band.link_pc_audio("A & B.mp3").unwrap();
    doc.set_band(&band).unwrap();
    assert_eq!(doc.band().unwrap(), Some(band.clone()));
    assert!(doc.to_xml().unwrap().contains("<Unknown x='keep'/>"));
    let before = doc.to_xml().unwrap();
    band.band_type = 1;
    assert!(doc.set_band(&band).is_err());
    assert_eq!(doc.to_xml().unwrap(), before);
    band.clear();
    doc.set_band(&band).unwrap();
    assert_eq!(doc.band().unwrap().unwrap().location, 6);
}
#[test]
fn unknown_enums_and_boundary_values_round_trip_semantically() {
    let band = Band {
        band_type: 255,
        repeat: 255,
        location: 255,
        filename: String::new(),
        tempo: 65535,
        transpose: -128,
        count_in: 127,
        drum_solo: true,
        ..Default::default()
    };
    assert_eq!(document(&band).band().unwrap(), Some(band));
    let band = Band {
        audio_mix: 511,
        balance: 65535,
        speed: 65535,
        pitch: 65535,
        ..Default::default()
    };
    assert_eq!(document(&band).band().unwrap().unwrap().audio_mix, 255);
}
#[test]
fn malformed_values_and_duplicate_fields_refuse_edit_without_mutation() {
    for inner in [
        "<Band/>",
        "<Band Type='0' Repeat='0'><SongFile Location='9'>a.wav</SongFile><AudioMix>0</AudioMix><Balance>65536</Balance><Speed>1</Speed><Pitch>0</Pitch></Band>",
        "<Band/><Band/>",
    ] {
        let raw = format!("<Preset>{inner}</Preset>");
        let mut doc = PresetDocument::parse(&raw).unwrap();
        assert!(doc.band().is_err());
        assert!(doc.set_band(&Band::default()).is_err());
        assert_eq!(doc.to_xml().unwrap(), raw);
    }
}
#[test]
fn branch_changes_retain_extensions_and_refuse_nested_scalar_overwrite() {
    let raw = "<Preset><Band Type='0' Repeat='0'><SongFile Location='9'>a.wav</SongFile><AudioMix>0</AudioMix><Balance>0</Balance><Speed>0</Speed><Pitch>0</Pitch><Tempo><Future/></Tempo></Band></Preset>";
    let mut doc = PresetDocument::parse(raw).unwrap();
    let mut band = doc.band().unwrap().unwrap();
    band.set_filename("a.mid").unwrap();
    assert!(doc.set_band(&band).is_err());
    assert_eq!(doc.to_xml().unwrap(), raw);
}
