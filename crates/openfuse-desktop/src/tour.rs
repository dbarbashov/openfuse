//! Offline native input recorder for reproducible product footage.
//! Pointer and keyboard events pass through the actual egui controls.
use crate::{session::Session, theme};
use eframe::egui::{self, Event, Modifiers, PointerButton, Pos2};
use std::path::PathBuf;

pub struct Capture {
    folder: PathBuf,
    scene: String,
    frame: usize,
    started: bool,
    awaiting: bool,
    injected: bool,
    cursor: Option<Pos2>,
    pressed: bool,
    events: Vec<serde_json::Value>,
    states: Vec<serde_json::Value>,
    failure: Option<String>,
}
impl Capture {
    pub fn new(folder: PathBuf, scene: &str) -> Self {
        let failure = std::fs::create_dir_all(&folder)
            .err()
            .map(|e| e.to_string());
        Self {
            folder,
            scene: if scene.is_empty() {
                "edit".into()
            } else {
                scene.into()
            },
            frame: 0,
            started: false,
            awaiting: false,
            injected: false,
            cursor: None,
            pressed: false,
            events: Vec::new(),
            states: Vec::new(),
            failure,
        }
    }
    fn target(&mut self, ctx: &egui::Context, key: &str) -> Option<egui::Rect> {
        let r = ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("tour-control", key))));
        if r.is_none() {
            self.failure = Some(format!(
                "Missing native control {key} at frame {}",
                self.frame
            ));
        }
        r
    }
    fn pointer(&mut self, input: &mut egui::RawInput, p: Pos2, pressed: Option<bool>) {
        self.cursor = Some(p);
        input.events.push(Event::PointerMoved(p));
        if let Some(pressed) = pressed {
            self.pressed = pressed;
            input.events.push(Event::PointerButton {
                pos: p,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            });
        }
    }
    fn click(&mut self, ctx: &egui::Context, input: &mut egui::RawInput, at: usize, key: &str) {
        if (at.saturating_sub(10)..=at + 1).contains(&self.frame)
            && let Some(r) = self.target(ctx, key)
        {
            let target = r.center();
            let from = self.cursor.unwrap_or(target);
            let p = from.lerp(target, 0.35);
            self.pointer(
                input,
                if self.frame >= at { target } else { p },
                if self.frame == at {
                    Some(true)
                } else if self.frame == at + 1 {
                    Some(false)
                } else {
                    None
                },
            );
            if self.frame == at {
                self.events.push(serde_json::json!({"frame":self.frame,"action":"click","target":key,"x":target.x,"y":target.y}));
            }
        }
    }
    fn drag(
        &mut self,
        ctx: &egui::Context,
        input: &mut egui::RawInput,
        frames: std::ops::RangeInclusive<usize>,
        key: &str,
        values: std::ops::RangeInclusive<f32>,
    ) {
        let (start, end) = (*frames.start(), *frames.end());
        let (from, to) = (*values.start(), *values.end());
        if (start..=end + 1).contains(&self.frame)
            && let Some(r) = self.target(ctx, key)
        {
            let t = ((self.frame - start) as f32 / (end - start) as f32).min(1.0);
            let x = egui::lerp(r.left() + 8.0..=r.right() - 8.0, egui::lerp(from..=to, t));
            self.pointer(
                input,
                egui::pos2(x, r.center().y),
                if self.frame == start {
                    Some(true)
                } else if self.frame == end + 1 {
                    Some(false)
                } else {
                    None
                },
            );
            if self.frame == start {
                self.events.push(serde_json::json!({"frame":self.frame,"action":"drag","target":key,"from":from,"to":to}));
            }
        }
    }
    pub fn input(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        input
            .events
            .retain(|event| matches!(event, Event::Screenshot { .. }));
        if !self.started || self.awaiting || self.injected || self.failure.is_some() {
            return;
        }
        self.injected = true;
        match self.scene.as_str() {
            "expression" => {
                self.click(ctx, input, 20, "tab-expression");
                self.click(ctx, input, 45, "expression-Pedal 1-add");
                self.click(ctx, input, 70, "expression-Pedal 1-target");
                self.click(ctx, input, 90, "expression-Pedal 1-target-1");
                self.drag(ctx, input, 115..=155, "expression-Pedal 1-Heel", 0.0..=0.72);
                self.drag(ctx, input, 170..=210, "expression-Pedal 1-Toe", 1.0..=0.3);
            }
            "library" => {
                self.click(ctx, input, 25, "library-query");
                if (40..=64).contains(&self.frame) && (self.frame - 40).is_multiple_of(6) {
                    let i = (self.frame - 40) / 6;
                    let text = ["G", "l", "a", "s", "s"][i];
                    input.events.push(Event::Text(text.into()));
                }
                self.click(ctx, input, 95, "library-entry:05-Glass room.fuse");
                self.click(ctx, input, 135, "tab-details");
                self.click(ctx, input, 170, "tab-sound");
            }
            _ => {
                self.click(ctx, input, 12, "module-Amplifier");
                self.drag(ctx, input, 25..=72, "parameter-Gain", 0.4..=0.86);
                self.click(ctx, input, 95, "module-Delay");
                self.click(ctx, input, 120, "model-picker");
                self.click(ctx, input, 145, "model-choice-demo");
                self.click(ctx, input, 178, "effect-enabled");
                self.click(ctx, input, 205, "undo");
            }
        }
    }
    pub fn capture(&mut self, ctx: &egui::Context, ready: bool, session: Option<&Session>) -> bool {
        ctx.request_repaint();
        if let Some(error) = &self.failure {
            let _ = std::fs::write(self.folder.join("failure.txt"), error);
            eprintln!("Tour capture failed: {error}");
            return true;
        }
        if !self.started {
            if ready {
                self.started = true;
            }
            return false;
        }
        if let Some(p) = self.cursor {
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Tooltip,
                egui::Id::new("tour-cursor"),
            ));
            if self.pressed {
                painter.circle_stroke(
                    p,
                    15.0,
                    egui::Stroke::new(2.0_f32, theme::accent(ctx).gamma_multiply(0.6)),
                );
            }
            painter.add(egui::Shape::convex_polygon(
                vec![
                    p,
                    p + egui::vec2(0.0, 18.0),
                    p + egui::vec2(5.0, 13.0),
                    p + egui::vec2(9.0, 21.0),
                    p + egui::vec2(12.0, 19.0),
                    p + egui::vec2(8.0, 11.0),
                    p + egui::vec2(15.0, 11.0),
                ],
                egui::Color32::BLACK,
                egui::Stroke::new(1.0_f32, egui::Color32::WHITE),
            ));
        }
        let screenshot = ctx.input(|i| {
            i.events.iter().find_map(|event| {
                if let Event::Screenshot { image, .. } = event {
                    Some(image.clone())
                } else {
                    None
                }
            })
        });
        if let Some(image) = screenshot {
            let path = self.folder.join(format!("frame-{:04}.png", self.frame));
            let rgba: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
            if let Err(error) = image::save_buffer(
                &path,
                &rgba,
                image.width() as u32,
                image.height() as u32,
                image::ColorType::Rgba8,
            ) {
                self.failure = Some(error.to_string());
                return false;
            }
            if self.frame.is_multiple_of(15)
                && let Some(session) = session
            {
                self.states.push(serde_json::json!({"frame":self.frame,"dirty":session.dirty(),"xml":session.document.to_xml().ok()}));
            }
            self.frame += 1;
            self.awaiting = false;
            self.injected = false;
            if self.frame == 240 {
                let manifest = serde_json::json!({"scene":self.scene,"frames":self.frame,"fps":30,"width":image.width(),"height":image.height(),"method":"Native egui pointer/keyboard events; local example only; no HID or audio stream","events":self.events,"states":self.states});
                if let Err(error) = std::fs::write(
                    self.folder.join("capture.json"),
                    serde_json::to_vec_pretty(&manifest).unwrap(),
                ) {
                    self.failure = Some(error.to_string());
                    return false;
                }
                println!("Native tour captured: {}", self.folder.display());
                return true;
            }
        } else if self.injected && !self.awaiting {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            self.awaiting = true;
        }
        false
    }
}
