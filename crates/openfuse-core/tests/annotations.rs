use openfuse_core::preset::{
    PresetAnnotations, PresetDocument,
    annotations::{DEFAULT_PEDAL_COLORS, GENRES, PEDAL_COLORS},
};

#[test]
fn annotation_snapshots_preserve_unedited_spelling_entities_and_extensions() {
    let xml = "<Preset xmlns:x='urn:future'><FUSE><Info name='A' rating='04.5000' genre1='&#54;' genre2='-1' genre3='2147483647' fenderid='+0017' x:rating='opaque'>untouched<!--keep--></Info><x:Info/></FUSE></Preset>";
    let mut preset = PresetDocument::parse(xml).unwrap();
    let mut fields = preset.annotations().unwrap();
    assert_eq!(fields.rating.as_deref(), Some("04.5000"));
    assert_eq!(fields.genres, [Some(6), Some(-1), Some(i32::MAX)]);
    assert_eq!(fields.fender_id, Some(17));
    preset.set_annotations(&fields).unwrap();
    assert_eq!(preset.to_xml().unwrap(), xml);
    fields.genres[1] = Some(25);
    preset.set_annotations(&fields).unwrap();
    assert_eq!(
        preset.to_xml().unwrap(),
        xml.replace("genre2='-1'", "genre2='25'")
    );
    fields.rating = Some("3.125".into());
    fields.fender_id = Some(i32::MIN);
    preset.set_annotations(&fields).unwrap();
    assert_eq!(preset.annotations().unwrap(), fields);
    assert!(preset.to_xml().unwrap().contains("genre1='&#54;'"));
    assert!(
        preset
            .to_xml()
            .unwrap()
            .contains("x:rating='opaque'>untouched<!--keep-->")
    );
}

#[test]
fn creates_absent_annotations_and_distinguishes_absent_from_zero() {
    for source in [
        "<Preset/>",
        "<Preset></Preset>",
        "<Preset><FUSE/></Preset>",
        "<Preset><FUSE><Info/></FUSE></Preset>",
    ] {
        let mut preset = PresetDocument::parse(source).unwrap();
        assert_eq!(preset.annotations().unwrap(), PresetAnnotations::default());
        preset
            .set_annotations(&PresetAnnotations::default())
            .unwrap();
        assert_eq!(preset.to_xml().unwrap(), source);
        let fields = PresetAnnotations {
            rating: Some("0".into()),
            genres: [Some(-1), Some(0), Some(127)],
            fender_id: Some(0),
        };
        preset.set_annotations(&fields).unwrap();
        assert_eq!(preset.annotations().unwrap(), fields);
        preset
            .set_annotations(&PresetAnnotations::default())
            .unwrap();
        assert_eq!(preset.annotations().unwrap(), PresetAnnotations::default());
    }
}

#[test]
fn removing_attributes_does_not_remove_namespaced_extensions_or_description() {
    let xml = "<Preset xmlns:x='urn:future'><FUSE><Info rating = '3' x:rating='leave' genre1='25' fenderid='7'>keep<!--yes--></Info></FUSE></Preset>";
    let mut preset = PresetDocument::parse(xml).unwrap();
    let mut values = preset.annotations().unwrap();
    values.rating = None;
    values.fender_id = None;
    values.genres[1] = Some(8);
    preset.set_annotations(&values).unwrap();
    assert_eq!(preset.annotations().unwrap(), values);
    let result = preset.to_xml().unwrap();
    assert!(result.contains("x:rating='leave' genre1='25'"));
    assert!(result.contains(">keep<!--yes--></Info>"));
}

#[test]
fn signed_unknown_identifiers_are_preserved_without_enum_coercion() {
    let mut preset = PresetDocument::parse("<Preset/>").unwrap();
    let values = PresetAnnotations {
        rating: None,
        genres: [Some(i32::MIN), Some(500), Some(i32::MAX)],
        fender_id: Some(-90),
    };
    preset.set_annotations(&values).unwrap();
    preset
        .set_pedal_colors([i32::MIN, -2, 16, i32::MAX])
        .unwrap();
    assert_eq!(preset.annotations().unwrap(), values);
    assert_eq!(
        preset.pedal_colors().unwrap(),
        Some([i32::MIN, -2, 16, i32::MAX])
    );
}

#[test]
fn decimal_precision_is_exact_without_floating_point() {
    let mut preset = PresetDocument::parse("<Preset/>").unwrap();
    for rating in [
        "0",
        " -0.000 ",
        "+.5",
        "5.",
        "79228162514264337593543950335",
        "0.1234567890123456789012345678",
        "79228162514264337593543950335.000",
        "00000000000000000000000000000000000000000001",
    ] {
        let values = PresetAnnotations {
            rating: Some(rating.into()),
            ..Default::default()
        };
        preset.set_annotations(&values).unwrap();
        assert_eq!(preset.annotations().unwrap(), values);
    }
    let before = preset.to_xml().unwrap();
    for rating in [
        "",
        ".",
        "NaN",
        "inf",
        "1e0",
        "1,2.5",
        "1.2.3",
        "--1",
        "1 2",
        "79228162514264337593543950336",
        "0.12345678901234567890123456789",
        "\0",
    ] {
        let values = PresetAnnotations {
            rating: Some(rating.into()),
            ..Default::default()
        };
        assert!(
            preset.set_annotations(&values).is_err(),
            "accepted {rating}"
        );
        assert_eq!(preset.to_xml().unwrap(), before);
    }
}

#[test]
fn malformed_present_annotations_are_explicit_errors_and_writes_stay_atomic() {
    for attributes in [
        "rating='invalid'",
        "genre1=''",
        "genre2='2147483648'",
        "genre3='1.0'",
        "fenderid='-2147483649'",
        "rating='1,2,5'",
    ] {
        let source = format!("<Preset><FUSE><Info {attributes}/></FUSE></Preset>");
        let mut preset = PresetDocument::parse(&source).unwrap();
        assert!(preset.annotations().is_err(), "accepted {attributes}");
        assert!(
            preset
                .set_annotations(&PresetAnnotations::default())
                .is_err()
        );
        assert_eq!(preset.to_xml().unwrap(), source);
    }
}

#[test]
fn palette_uses_ordinal_order_and_edits_only_requested_text() {
    let source = "<Preset xmlns:x='urn:future'><FUSE><PedalColors custom='keep'><Color ID='4'>014</Color><x:Color>opaque</x:Color><Color ID='3'><![CDATA[1]]><!--note--></Color><Color ID='2'> 2 </Color><Color ID='1'>10</Color></PedalColors></FUSE></Preset>";
    let mut preset = PresetDocument::parse(source).unwrap();
    assert_eq!(preset.pedal_colors().unwrap(), Some(DEFAULT_PEDAL_COLORS));
    preset.set_pedal_colors(DEFAULT_PEDAL_COLORS).unwrap();
    assert_eq!(preset.to_xml().unwrap(), source);
    preset.set_pedal_colors([14, 7, 2, 10]).unwrap();
    assert_eq!(
        preset.to_xml().unwrap(),
        source.replace("<![CDATA[1]]>", "7")
    );
    assert_eq!(preset.pedal_colors().unwrap(), Some([14, 7, 2, 10]));
}

#[test]
fn palettes_can_be_added_to_minimal_presets_without_touching_info() {
    for source in [
        "<Preset/>",
        "<Preset><FUSE/></Preset>",
        "<Preset><FUSE><Info rating='4'><!--keep--></Info></FUSE></Preset>",
    ] {
        let mut preset = PresetDocument::parse(source).unwrap();
        assert_eq!(preset.pedal_colors().unwrap(), None);
        preset.set_pedal_colors(DEFAULT_PEDAL_COLORS).unwrap();
        assert_eq!(preset.pedal_colors().unwrap(), Some(DEFAULT_PEDAL_COLORS));
        let before = preset.to_xml().unwrap();
        preset.set_pedal_colors(DEFAULT_PEDAL_COLORS).unwrap();
        assert_eq!(preset.to_xml().unwrap(), before);
        assert!(before.contains("<Color ID=\"1\">14</Color>"));
        if source.contains("<!--keep-->") {
            assert!(before.contains("<Info rating='4'><!--keep--></Info>"));
        }
    }
}

#[test]
fn incomplete_or_malformed_palette_does_not_silently_reset_colors() {
    for body in [
        "",
        "<Color>1</Color>",
        "<Color>1</Color><Color>2</Color><Color>3</Color><Color>4</Color><Color>5</Color>",
        "<Color>1</Color><Color>2</Color><Color>3</Color><Color>abc</Color>",
        "<Color>1</Color><Color>2</Color><Color>3</Color><Color>2147483648</Color>",
        "<Color>1</Color><Color>2</Color><Color>3</Color><Color><Nested>4</Nested></Color>",
    ] {
        let xml = format!("<Preset><FUSE><PedalColors>{body}</PedalColors></FUSE></Preset>");
        let mut preset = PresetDocument::parse(&xml).unwrap();
        assert!(preset.pedal_colors().is_err(), "accepted {body}");
        assert!(preset.set_pedal_colors(DEFAULT_PEDAL_COLORS).is_err());
        assert_eq!(preset.to_xml().unwrap(), xml);
    }
    let mut preset =
        PresetDocument::parse("<Preset><FUSE><PedalColors/><PedalColors/></FUSE></Preset>")
            .unwrap();
    assert!(preset.pedal_colors().is_err());
    assert!(preset.set_pedal_colors(DEFAULT_PEDAL_COLORS).is_err());
}

#[test]
fn vocabulary_has_fuse_ids_not_presentation_order() {
    assert_eq!(GENRES.len(), 129);
    assert_eq!(GENRES[0].id, -1);
    assert_eq!(GENRES[26].label, "Jazz");
    assert_eq!(GENRES[128].id, 127);
    assert_eq!(PEDAL_COLORS.len(), 16);
    assert_eq!(PEDAL_COLORS[0].label, "Aquamarine");
    assert_eq!(PEDAL_COLORS[14].rgb, [255, 244, 0]);
    for (index, color) in PEDAL_COLORS.iter().enumerate() {
        assert_eq!(color.id, index as i32);
    }
}

#[test]
fn comma_decimal_projection_and_edits_preserve_locale_spelling_exactly() {
    let source =
        "<Preset><FUSE><Info rating='04,1234567890123456789012345678' genre1='6'/></FUSE></Preset>";
    let mut preset = PresetDocument::parse(source).unwrap();
    let mut annotations = preset.annotations().unwrap();
    assert_eq!(
        annotations.rating.as_deref(),
        Some("04,1234567890123456789012345678")
    );
    preset.set_annotations(&annotations).unwrap();
    assert_eq!(preset.to_xml().unwrap(), source);
    annotations.genres[0] = Some(25);
    preset.set_annotations(&annotations).unwrap();
    assert_eq!(
        preset.to_xml().unwrap(),
        source.replace("genre1='6'", "genre1='25'")
    );
    annotations.rating = Some("0,1234567890123456789012345678".into());
    preset.set_annotations(&annotations).unwrap();
    assert_eq!(preset.annotations().unwrap(), annotations);
    let before = preset.to_xml().unwrap();
    for rating in [
        "0,12345678901234567890123456789",
        "1,2.3",
        "1.2,3",
        "1,000,000",
        "1.000.000",
    ] {
        annotations.rating = Some(rating.into());
        assert!(preset.set_annotations(&annotations).is_err());
        assert_eq!(preset.to_xml().unwrap(), before);
    }
}
