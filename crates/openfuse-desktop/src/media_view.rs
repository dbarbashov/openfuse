//! Local backing-track workspace; rendering does not load files or open audio devices.
use crate::{controls_view::button, media_state::Library, theme};
use eframe::egui::{self, RichText};
use openfuse_audio::{Phase, Player, Snapshot};
use openfuse_core::{
    media_library::{MediaEntry, MediaFormat},
    preset::band::Band,
};

pub struct State {
    pub open: bool,
    pub show_band: bool,
    pub library: Library,
    pub player: Player,
    pub loaded: Option<MediaEntry>,
    pub error: Option<String>,
    pub demo: u8,
    pub demo_folder: Option<tempfile::TempDir>,
    pub last_refresh: std::time::Instant,
    pub filename: String,
    pub filename_source: String,
}
impl Default for State {
    fn default() -> Self {
        Self {
            open: false,
            show_band: false,
            library: Default::default(),
            player: Player::new(),
            loaded: None,
            error: None,
            demo: 0,
            demo_folder: None,
            last_refresh: std::time::Instant::now(),
            filename: String::new(),
            filename_source: String::new(),
        }
    }
}
pub struct BandInput {
    pub source: String,
    pub product: u32,
    pub band: Band,
}
pub enum Action {
    AudioSettings,
    Close,
    Folder,
    Refresh,
    Import,
    ImportFolder,
    FindLinked(String),
    Select(String),
    Load(MediaEntry),
    Archive(MediaEntry),
    Restore(MediaEntry),
    Play,
    Pause,
    Stop,
    Seek(f64),
    Volume(f32),
    Balance(f32),
    Repeat(bool),
    Link(MediaEntry, String, Band),
    SetBand(String, Band),
    SetFilename(String, String, Band),
    UsePreset(String, Band),
    ClearBand(String, Band),
}
pub fn show(ctx: &egui::Context, state: &mut State, band: Option<BandInput>) -> Option<Action> {
    let mut action = None;
    let snapshot = state.player.snapshot();
    crate::dialog::show(ctx, "backing-tracks", 940.0, |ui| {
        theme::eyebrow(ui, "LOCAL AUDIO");
        crate::dialog::header(
            ui,
            "Backing tracks",
            "Your collection and player. Playback uses this computer’s audio output.",
        );
        egui::ScrollArea::vertical()
            .id_salt("media-workspace")
            .max_height((ctx.screen_rect().height() - 290.0).clamp(220.0, 590.0))
            .show(ui, |ui| {
                ui.columns(2, |columns| {
                    action = crate::dialog::section(&mut columns[0], "", |ui| {
                        library(ui, &mut state.library)
                    })
                    .or(action.take());
                    crate::dialog::section(&mut columns[1], "", |ui| {
                        ui.horizontal(|ui| {
                            ui.selectable_value(&mut state.show_band, false, "Player");
                            ui.selectable_value(&mut state.show_band, true, "Preset Band");
                        });
                        ui.add_space(10.0);
                        if state.show_band {
                            if let Some(input) = band {
                                action = band_controls(
                                    ui,
                                    input,
                                    state.library.selected_entry(),
                                    state.loaded.as_ref(),
                                    &state.library.entries,
                                    &mut state.filename,
                                    &mut state.filename_source,
                                )
                                .or(action.take());
                            } else {
                                crate::dialog::hint(
                                    ui,
                                    "Open a preset to link a track or edit its Band settings.",
                                );
                            }
                        } else {
                            action = transport(
                                ui,
                                &snapshot,
                                state.loaded.as_ref(),
                                state.library.busy(),
                            )
                            .or(action.take());
                        }
                    });
                });
                if let Some(error) = &state.error {
                    ui.colored_label(theme::danger(ui.ctx()), error);
                }
            });
        crate::dialog::footer(ui, |ui| {
            if button(ui, "media-audio-settings", "Audio settings…", true) {
                action = Some(Action::AudioSettings);
            }
            if button(ui, "media-close", "Stop & close", true) {
                action = Some(Action::Close);
            }
        });
    });
    action
}

fn library(ui: &mut egui::Ui, library: &mut Library) -> Option<Action> {
    let mut action = None;
    ui.heading("Track library");
    if let Some(folder) = &library.folder {
        ui.add(
            egui::Label::new(
                RichText::new(folder.file_name().unwrap_or_default().to_string_lossy())
                    .small()
                    .color(theme::muted(ui.ctx())),
            )
            .truncate(),
        )
        .on_hover_text(folder.display().to_string());
    } else {
        crate::dialog::hint(ui, "Choose a folder for your WAV, MP3 and MIDI collection.");
    }
    ui.horizontal_wrapped(|ui| {
        if button(ui, "media-folder", "Folder…", true) {
            action = Some(Action::Folder);
        }
        if button(
            ui,
            "media-import",
            "Import…",
            library.folder.is_some() && !library.busy(),
        ) {
            action = Some(Action::Import);
        }
        if button(
            ui,
            "media-import-folder",
            "Import folder…",
            library.folder.is_some() && !library.busy(),
        ) {
            action = Some(Action::ImportFolder);
        }
        if button(
            ui,
            "media-refresh",
            "Refresh",
            library.folder.is_some() && !library.busy(),
        ) {
            action = Some(Action::Refresh);
        }
    });
    ui.horizontal(|ui| {
        ui.selectable_value(&mut library.deleted, false, "Tracks");
        ui.selectable_value(&mut library.deleted, true, "Archived");
    });
    ui.add(
        egui::TextEdit::singleline(&mut library.query)
            .hint_text("Search tracks or formats")
            .desired_width(f32::INFINITY),
    );
    let entries = if library.deleted {
        &library.trash
    } else {
        &library.entries
    };
    let filtered: Vec<_> = entries
        .iter()
        .filter(|entry| entry.matches_query(&library.query))
        .collect();
    ui.label(
        RichText::new(format!(
            "{} tracks{}",
            filtered.len(),
            if library.busy() { " · working…" } else { "" }
        ))
        .color(theme::muted(ui.ctx())),
    );
    egui::ScrollArea::vertical()
        .id_salt("media-entries")
        .max_height(230.0)
        .show_rows(ui, 46.0, filtered.len(), |ui, range| {
            for index in range {
                let entry = filtered[index];
                let response = ui.add_sized(
                    [ui.available_width(), 42.0],
                    egui::Button::new(format!(
                        "{} · {}",
                        short_name(&entry.name),
                        entry.format.extension().to_uppercase()
                    ))
                    .selected(library.selected.as_deref() == Some(&entry.id))
                    .wrap(),
                );
                crate::audition_view::remember(&response, &format!("media-track-{}", entry.id));
                if response.clicked() {
                    action = Some(Action::Select(entry.id.clone()));
                }
            }
        });
    if let Some(entry) = library.selected_entry() {
        ui.add(
            egui::Label::new(
                RichText::new(format!(
                    "{} · {:.1} MiB",
                    entry.name,
                    entry.size as f64 / 1_048_576.0
                ))
                .strong(),
            )
            .wrap(),
        );
        ui.add(
            egui::Label::new(
                RichText::new(format!("Imported from {}", entry.original_path.display()))
                    .small()
                    .color(theme::muted(ui.ctx())),
            )
            .wrap(),
        );
        ui.horizontal_wrapped(|ui| {
            if library.deleted {
                if button(ui, "media-restore", "Restore", !library.busy()) {
                    action = Some(Action::Restore(entry.clone()));
                }
            } else {
                if button(
                    ui,
                    "media-load",
                    "Load player",
                    !library.busy() && entry.format != MediaFormat::Midi,
                ) {
                    action = Some(Action::Load(entry.clone()));
                }
                if button(ui, "media-archive", "Archive", !library.busy()) {
                    action = Some(Action::Archive(entry.clone()));
                }
            }
        });
        if entry.format == MediaFormat::Midi {
            ui.add(
                egui::Label::new(
                    "MIDI is preserved in the catalog. Local playback supports WAV and MP3.",
                )
                .wrap(),
            );
        }
    }
    if let Some(notice) = &library.notice {
        ui.add(
            egui::Label::new(RichText::new(notice).small().color(theme::muted(ui.ctx()))).wrap(),
        );
    }
    if let Some(error) = &library.error {
        ui.add(egui::Label::new(RichText::new(error).color(theme::danger(ui.ctx()))).wrap());
    }
    if !library.diagnostics.is_empty() {
        egui::CollapsingHeader::new(format!("{} library notices", library.diagnostics.len())).show(
            ui,
            |ui| {
                for message in library.diagnostics.iter().take(50) {
                    ui.add(egui::Label::new(message).wrap());
                }
            },
        );
    }
    action
}
fn time(seconds: f64) -> String {
    let seconds = seconds.max(0.0) as u64;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}
fn transport(
    ui: &mut egui::Ui,
    snapshot: &Snapshot,
    entry: Option<&MediaEntry>,
    busy: bool,
) -> Option<Action> {
    let mut action = None;
    ui.heading("Local player");
    ui.add(
        egui::Label::new(
            RichText::new(
                entry
                    .map(|e| e.name.as_str())
                    .unwrap_or("Choose a track, then load the player"),
            )
            .strong(),
        )
        .wrap(),
    );
    ui.label(
        RichText::new(if snapshot.sample_rate == 0 {
            format!("{:?}", snapshot.phase)
        } else {
            format!(
                "{:?} · {} Hz · Channels: {}",
                snapshot.phase, snapshot.sample_rate, snapshot.channels
            )
        })
        .small()
        .color(theme::muted(ui.ctx())),
    );
    let ready = (snapshot.phase == Phase::Error && snapshot.duration > 0.0)
        || matches!(
            snapshot.phase,
            Phase::Ready | Phase::Playing | Phase::Paused | Phase::Ended
        );
    ui.horizontal_wrapped(|ui| {
        if snapshot.phase == Phase::Playing {
            if button(ui, "media-pause", "Pause", true) {
                action = Some(Action::Pause);
            }
        } else if button(ui, "media-play", "Play", ready) {
            action = Some(Action::Play);
        }
        if button(
            ui,
            "media-stop",
            "Stop",
            ready || busy || snapshot.phase == Phase::Loading,
        ) {
            action = Some(Action::Stop);
        }
    });
    ui.label(
        RichText::new(format!(
            "{} / {}",
            time(snapshot.position),
            time(snapshot.duration)
        ))
        .monospace(),
    );
    let mut position = snapshot.position;
    let seek = ui.add_enabled(
        ready,
        egui::Slider::new(&mut position, 0.0..=snapshot.duration.max(0.01)).show_value(false),
    );
    crate::audition_view::remember(&seek, "media-seek");
    if seek.changed() {
        action = Some(Action::Seek(position));
    }
    let mut volume = snapshot.volume;
    let volume_control = ui.add(egui::Slider::new(&mut volume, 0.0..=1.0).text("Volume"));
    crate::audition_view::remember(&volume_control, "media-volume");
    if volume_control.changed() {
        action = Some(Action::Volume(volume));
    }
    let mut balance = snapshot.balance;
    let balance_control = ui.add(egui::Slider::new(&mut balance, -1.0..=1.0).text("L / R balance"));
    crate::audition_view::remember(&balance_control, "media-balance");
    if balance_control.changed() {
        action = Some(Action::Balance(balance));
    }
    let mut repeat = snapshot.repeat;
    let response = ui.checkbox(&mut repeat, "Repeat this track");
    crate::audition_view::remember(&response, "media-repeat");
    if response.changed() {
        action = Some(Action::Repeat(repeat));
    }
    if let Some(error) = &snapshot.error {
        ui.add(egui::Label::new(RichText::new(error).color(theme::danger(ui.ctx()))).wrap());
    }
    action
}
fn band_controls(
    ui: &mut egui::Ui,
    mut input: BandInput,
    entry: Option<&MediaEntry>,
    loaded: Option<&MediaEntry>,
    entries: &[MediaEntry],
    filename: &mut String,
    filename_source: &mut String,
) -> Option<Action> {
    if *filename_source != input.source {
        *filename = input.band.filename.clone();
        *filename_source = input.source.clone();
    }
    let mut action = None;
    ui.heading("Preset track link");
    ui.add(
        egui::Label::new(
            if input.band.location == 6 || input.band.filename.is_empty() {
                "No linked track".into()
            } else {
                format!("Stored track: {}", input.band.filename)
            },
        )
        .wrap(),
    );
    let kind = match input.band.band_type {
        0 => "Audio".into(),
        1 => "MIDI".into(),
        raw => format!("Unknown type {raw}"),
    };
    let locations = [
        "Internal audio",
        "SD audio",
        "Internal phrase",
        "SD phrase",
        "Internal MIDI",
        "SD MIDI",
        "No Band",
        "SD FUSE",
        "Internal root",
        "PC audio",
        "SD root",
    ];
    let location = locations
        .get(input.band.location as usize)
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("Unknown location {}", input.band.location));
    ui.add(
        egui::Label::new(
            RichText::new(format!("Stored source: {kind} · {location}"))
                .small()
                .color(theme::muted(ui.ctx())),
        )
        .wrap(),
    );
    ui.horizontal_wrapped(|ui| {
        let playable =
            entry.is_some_and(|entry| !entry.trashed && entry.format != MediaFormat::Midi);
        if button(ui, "media-link", "Link selected track", playable) {
            action = entry
                .map(|entry| Action::Link(entry.clone(), input.source.clone(), input.band.clone()));
        }
        if button(
            ui,
            "media-find-linked",
            "Find linked track",
            input.band.location == 9,
        ) {
            action = Some(Action::FindLinked(input.band.filename.clone()));
        }
        if button(
            ui,
            "media-unlink",
            "Clear track link",
            input.band.location != 6,
        ) {
            action = Some(Action::ClearBand(input.source.clone(), input.band.clone()));
        }
    });
    ui.add(egui::Label::new(RichText::new("Linking changes this preset and supports Undo. Player volume, balance and repeat remain independent.").small().color(theme::muted(ui.ctx()))).wrap());
    let mut changed = false;
    egui::ComboBox::from_id_salt("band-repeat")
        .selected_text(format!(
            "Preset repeat: {}",
            match input.band.repeat {
                0 => "off".into(),
                1 => "on".into(),
                v => format!("unknown ({v})"),
            }
        ))
        .show_ui(ui, |ui| {
            changed |= ui
                .selectable_value(&mut input.band.repeat, 0, "Off")
                .changed();
            changed |= ui
                .selectable_value(&mut input.band.repeat, 1, "On")
                .changed();
        });
    if input.band.location == 9 && input.band.band_type == 0 {
        changed |= stored_balance(ui, &mut input.band.balance, input.band.audio_mix == 1);
        let unique = entries
            .iter()
            .filter(|entry| entry.name == input.band.filename && !entry.trashed)
            .count()
            == 1;
        let matches_loaded = loaded.is_some_and(|entry| {
            entry.name == input.band.filename
                && entries.iter().any(|candidate| candidate.id == entry.id)
        });
        if button(
            ui,
            "media-use-preset",
            "Use preset playback settings",
            unique && matches_loaded,
        ) {
            action = Some(Action::UsePreset(input.source.clone(), input.band.clone()));
        }
        ui.add(egui::Label::new(RichText::new("Applies stored repeat and balance to the matching loaded track. Does not start playback.").small().color(theme::muted(ui.ctx()))).wrap());
    }
    if input.band.band_type > 1 {
        ui.add(
            egui::Label::new(format!(
                "Unknown Band type {} is preserved. Branch-specific controls are unavailable.",
                input.band.band_type
            ))
            .wrap(),
        );
    }
    if input.product == 10 {
        egui::CollapsingHeader::new("G-DEC Band file settings").default_open(true).show(ui,|ui| {
            ui.add(egui::Label::new("These are stored preset values. Speed, pitch, mix and MIDI settings do not process local audio or control an amplifier.").wrap());
            let locations=["Internal audio","SD audio","Internal phrase","SD phrase","Internal MIDI","SD MIDI","No Band","SD FUSE","Internal root","PC audio","SD root"];
            egui::ComboBox::from_id_salt("band-location").selected_text(locations.get(input.band.location as usize).map(|s|s.to_string()).unwrap_or_else(||format!("Unknown location {}",input.band.location))).show_ui(ui,|ui|{for (index,label) in locations.iter().enumerate(){changed|=ui.selectable_value(&mut input.band.location,index as u8,*label).changed();}});
            let response = ui.add(egui::TextEdit::singleline(filename).hint_text("Stored filename"));
            ui.add(egui::Label::new(RichText::new("Set filename commits the edit and resets that track type’s defaults.").small().color(theme::muted(ui.ctx()))).wrap());
            let commit = response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            if button(ui, "media-band-filename", "Set filename", *filename != input.band.filename) || commit {
                action = Some(Action::SetFilename(input.source.clone(), filename.clone(), input.band.clone()));
            }
            if input.band.location==9 || input.band.location==6 {ui.add(egui::Label::new("PC audio / No Band: onboard controls are preserved and hidden.").wrap());}
            else if input.band.band_type==1 {
                let mut auto_tempo = input.band.tempo == 0;
                if ui.checkbox(&mut auto_tempo, "Use track tempo").changed() { input.band.tempo = if auto_tempo { 0 } else { 120 }; changed = true; }
                changed |= ui.add_enabled(!auto_tempo, egui::DragValue::new(&mut input.band.tempo).range(30..=240).clamp_existing_to_range(false).prefix("Tempo BPM ")).changed();
                changed|=ui.add(egui::DragValue::new(&mut input.band.transpose).range(-12..=12).clamp_existing_to_range(false).prefix("Transpose ")).changed();
                changed|=ui.add(egui::DragValue::new(&mut input.band.count_in).range(0..=12).clamp_existing_to_range(false).prefix("Count in ")).changed();
                changed|=ui.checkbox(&mut input.band.drum_solo,"Drum solo").changed();
            } else if input.band.band_type == 0 {
                egui::ComboBox::from_id_salt("band-mix").selected_text(match input.band.audio_mix {0=>"Stereo".into(),1=>"Left / right mix to mono".into(),2=>"Center cancel".into(),v=>format!("Unknown mix ({v})")}).show_ui(ui,|ui|{for (value,label) in [(0,"Stereo"),(1,"Left / right mix to mono"),(2,"Center cancel")] {changed|=ui.selectable_value(&mut input.band.audio_mix,value,label).changed();}});
                changed |= stored_balance(ui, &mut input.band.balance, input.band.audio_mix == 1);
                changed|=ui.add(egui::DragValue::new(&mut input.band.speed).range(50..=150).clamp_existing_to_range(false).prefix("Speed % ")).changed();
                let mut pitch=input.band.pitch as i16;
                if ui.add(egui::DragValue::new(&mut pitch).range(-5..=5).clamp_existing_to_range(false).prefix("Pitch ")).changed(){input.band.pitch=pitch as u16;changed=true;}
            }
        });
    }
    if changed {
        action = Some(Action::SetBand(input.source, input.band));
    }
    action
}

fn stored_balance(ui: &mut egui::Ui, raw: &mut u16, enabled: bool) -> bool {
    let mut percent = *raw as f64 * 100.0 / 65535.0;
    let response = ui
        .add_enabled(
            enabled,
            egui::Slider::new(&mut percent, 0.0..=100.0)
                .clamping(egui::SliderClamping::Edits)
                .step_by(0.1)
                .suffix("%")
                .text("Preset L / R"),
        )
        .on_hover_text(format!(
            "Stored value: {raw}. Unchanged until you move this control."
        ));
    ui.add(
        egui::Label::new(
            RichText::new("0% left · 50% center · 100% right")
                .small()
                .color(theme::muted(ui.ctx())),
        )
        .wrap(),
    );
    if response.changed() {
        *raw = (percent * 65535.0 / 100.0).round() as u16;
        true
    } else {
        false
    }
}
fn short_name(name: &str) -> String {
    let mut chars = name.chars();
    let text: String = chars.by_ref().take(64).collect();
    if chars.next().is_some() {
        format!("{text}…")
    } else {
        text
    }
}
