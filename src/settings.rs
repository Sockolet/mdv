use std::path::PathBuf;

use eframe::egui::{self, FontFamily, FontId, TextStyle};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub enum Font {
    #[default]
    Sans,
    Monospace,
}

impl Font {
    pub fn family(self) -> FontFamily {
        match self {
            Self::Sans => FontFamily::Proportional,
            Self::Monospace => FontFamily::Monospace,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub dark: bool,
    pub font: Font,
    pub size: f32,
    pub folder: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            dark: false,
            font: Font::Sans,
            size: 17.0,
            folder: None,
        }
    }
}

impl Settings {
    pub fn apply_theme(&self, ctx: &egui::Context) {
        ctx.set_theme(if self.dark {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        });
    }

    pub fn menu(&mut self, ui: &mut egui::Ui) {
        egui::containers::menu::MenuButton::new("Options")
            .config(
                egui::containers::menu::MenuConfig::new()
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside),
            )
            .ui(ui, |ui| {
                ui.set_min_width(210.0);
                ui.label("Appearance");
                ui.horizontal(|ui| {
                    let light = ui.selectable_value(&mut self.dark, false, "Light");
                    let dark = ui.selectable_value(&mut self.dark, true, "Dark");
                    if light.changed() || dark.changed() {
                        self.apply_theme(ui.ctx());
                    }
                });
                ui.separator();
                ui.label("Font");
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.font, Font::Sans, "Sans serif");
                    ui.selectable_value(&mut self.font, Font::Monospace, "Monospace");
                });
                ui.add(egui::Slider::new(&mut self.size, 10.0..=32.0).text("Size"));
            });
    }

    pub fn style_document(&self, ui: &mut egui::Ui) {
        let style = ui.style_mut();
        style.url_in_tooltip = true;
        for text_style in [TextStyle::Body, TextStyle::Button] {
            style
                .text_styles
                .insert(text_style, FontId::new(self.size, self.font.family()));
        }
        style.text_styles.insert(
            TextStyle::Heading,
            FontId::new(self.size * 1.9, self.font.family()),
        );
        style.text_styles.insert(
            TextStyle::Monospace,
            FontId::new(self.size * 0.9, FontFamily::Monospace),
        );
    }
}
