//! Browser file operations use explicit JS calls; no device or filesystem access.
use crate::{session::Session, theme, views};
use eframe::egui;
use wasm_bindgen::prelude::*;

struct BrowserEditor {
    session: Session,
    selected: String,
    inspector: views::Inspector,
    error: Option<String>,
    context: egui::Context,
}
impl eframe::App for BrowserEditor {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::bg(ctx)).inner_margin(24))
            .show(ctx, |ui| {
                egui::ScrollArea::both().show(ui, |ui| {
                    ui.set_min_width(840.0);
                    if let Some(error) = &self.error {
                        ui.colored_label(theme::danger(ctx), error);
                        ui.add_space(12.0);
                    }
                    views::preset(
                        ui,
                        &mut self.session,
                        &mut self.selected,
                        &mut self.error,
                        false,
                        &mut self.inspector,
                    );
                });
            });
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[wasm_bindgen]
pub struct WebEditor {
    runner: eframe::WebRunner,
}
#[wasm_bindgen]
impl WebEditor {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        console_error_panic_hook::set_once();
        Self {
            runner: eframe::WebRunner::new(),
        }
    }
    pub async fn start(&self, canvas: web_sys::HtmlCanvasElement) -> Result<(), JsValue> {
        self.runner
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(|cc| {
                    theme::install(&cc.egui_ctx);
                    cc.egui_ctx.set_theme(egui::ThemePreference::System);
                    Ok(Box::new(BrowserEditor {
                        session: Session::example()?,
                        selected: "Amplifier".into(),
                        inspector: views::Inspector::Sound,
                        error: None,
                        context: cc.egui_ctx.clone(),
                    }))
                }),
            )
            .await
    }
    pub fn open(&self, xml: &str) -> Result<(), JsValue> {
        let session = Session::parse(xml, None).map_err(|e| JsValue::from_str(&e))?;
        self.replace(session)
    }
    pub fn create(&self, product: u32) -> Result<(), JsValue> {
        self.replace(Session::new_preset(product).map_err(|e| JsValue::from_str(&e))?)
    }
    pub fn example(&self) -> Result<(), JsValue> {
        self.replace(Session::example().map_err(|e| JsValue::from_str(&e))?)
    }
    pub fn export_xml(&self) -> Result<String, JsValue> {
        self.runner
            .app_mut::<BrowserEditor>()
            .ok_or_else(|| JsValue::from_str("Editor not ready"))?
            .session
            .document
            .to_xml()
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }
    pub fn dirty(&self) -> bool {
        self.runner
            .app_mut::<BrowserEditor>()
            .is_some_and(|app| app.session.dirty())
    }
    pub fn undo(&self) -> Result<(), JsValue> {
        self.history(false)
    }
    pub fn redo(&self) -> Result<(), JsValue> {
        self.history(true)
    }
    pub fn appearance(&self, dark: bool) {
        if let Some(app) = self.runner.app_mut::<BrowserEditor>() {
            app.context.set_theme(if dark {
                egui::Theme::Dark
            } else {
                egui::Theme::Light
            });
        }
    }
    pub fn products(&self) -> Result<String, JsValue> {
        let products: Vec<_> = openfuse_core::model_catalog::products()
            .iter()
            .map(|p| (p.id, p.name))
            .collect();
        serde_json::to_string(&products).map_err(|e| JsValue::from_str(&e.to_string()))
    }
}
impl WebEditor {
    fn replace(&self, session: Session) -> Result<(), JsValue> {
        let mut app = self
            .runner
            .app_mut::<BrowserEditor>()
            .ok_or_else(|| JsValue::from_str("Editor not ready"))?;
        app.session = session;
        app.selected = "Amplifier".into();
        app.error = None;
        app.context.request_repaint();
        Ok(())
    }
    fn history(&self, redo: bool) -> Result<(), JsValue> {
        let mut app = self
            .runner
            .app_mut::<BrowserEditor>()
            .ok_or_else(|| JsValue::from_str("Editor not ready"))?;
        let result = if redo {
            app.session.redo()
        } else {
            app.session.undo()
        };
        app.context.request_repaint();
        result.map_err(|e| JsValue::from_str(&e))
    }
}
impl Default for WebEditor {
    fn default() -> Self {
        Self::new()
    }
}
