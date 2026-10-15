use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

use eframe::egui::{self, Key, Modifiers};

use crate::{
    document::Document,
    links::{self, LinkTarget},
    settings::Settings,
    tree::{Tree, file_name},
};

#[derive(Clone, Copy)]
enum Pending {
    CloseTab(usize),
    Exit,
}

#[derive(Default)]
pub struct Mdv {
    settings: Settings,
    tree: Tree,
    tabs: Vec<Document>,
    active: usize,
    pending: Option<Pending>,
    error: Option<String>,
    allow_exit: bool,
    focus_editor: bool,
}

impl Mdv {
    pub fn new(cc: &eframe::CreationContext<'_>, startup: Option<PathBuf>) -> Self {
        Self::from_storage(&cc.egui_ctx, cc.storage, startup)
    }

    pub fn from_storage(
        context: &egui::Context,
        storage: Option<&dyn eframe::Storage>,
        startup: Option<PathBuf>,
    ) -> Self {
        let settings = storage
            .and_then(|storage| eframe::get_value(storage, "appearance"))
            .unwrap_or_default();
        let mut app = Self {
            settings,
            ..Default::default()
        };
        app.settings.size = app.settings.size.clamp(10.0, 32.0);
        app.settings.apply_theme(context);
        if let Some(path) = startup {
            let result = if path.is_dir() {
                app.set_folder(&path)
            } else {
                app.open_path(&path, None)
            };
            app.report(result);
        } else if let Some(path) = app.settings.folder.clone() {
            let result = app.set_folder(&path);
            app.report(result);
        } else {
            match std::env::current_dir() {
                Ok(path) => {
                    let result = app.set_folder(&path);
                    app.report(result);
                }
                Err(err) => app.error = Some(format!("Cannot read the current folder: {err}")),
            }
        }
        app
    }

    fn report(&mut self, result: Result<(), String>) {
        if let Err(error) = result {
            self.error = Some(error);
        }
    }

    fn set_folder(&mut self, path: &Path) -> Result<(), String> {
        self.tree.set_root(path)?;
        self.settings.folder = self.tree.root.as_ref().map(|root| root.path.clone());
        Ok(())
    }

    fn choose_folder(&mut self) {
        let mut dialog = rfd::FileDialog::new().set_title("Open folder");
        if let Some(root) = &self.tree.root {
            dialog = dialog.set_directory(&root.path);
        }
        if let Some(path) = dialog.pick_folder() {
            let result = self.set_folder(&path);
            self.report(result);
        }
    }

    fn open_path(&mut self, path: &Path, anchor: Option<String>) -> Result<(), String> {
        let path = dunce::canonicalize(path)
            .map_err(|err| format!("Cannot open {}: {err}", path.display()))?;
        if path.is_dir() {
            return self.set_folder(&path);
        }
        if let Some(index) = self.tabs.iter().position(|tab| tab.path == path) {
            self.active = index;
            self.tabs[index].anchor = anchor;
            return Ok(());
        }

        if links::is_external_document(&path) {
            let mut header = Vec::new();
            fs::File::open(&path)
                .and_then(|file| file.take(4).read_to_end(&mut header))
                .map_err(|err| format!("Cannot read {}: {err}", path.display()))?;
            return launch_document(&path, &header);
        }
        let bytes =
            fs::read(&path).map_err(|err| format!("Cannot read {}: {err}", path.display()))?;
        if bytes.contains(&0) || std::str::from_utf8(&bytes).is_err() {
            return launch_document(&path, &bytes);
        }
        let mut tab = Document::from_bytes(path, bytes)?;
        tab.anchor = anchor;
        if self.tree.root.is_none() {
            let parent = tab.path.parent().ok_or("The file has no parent folder.")?;
            self.set_folder(parent)?;
        }
        self.tabs.push(tab);
        self.active = self.tabs.len() - 1;
        Ok(())
    }

    fn follow_link(&mut self, href: &str) -> Result<(), String> {
        let Some(tab) = self.tabs.get(self.active) else {
            return Err("No file is open to resolve this link.".into());
        };
        match links::resolve(&tab.path, href)? {
            LinkTarget::Browser(url) => {
                open::that(&url).map_err(|err| format!("Cannot open the browser: {err}"))
            }
            LinkTarget::Local { path, anchor } => self.open_path(&path, anchor),
            LinkTarget::Anchor(anchor) => {
                self.tabs[self.active].anchor = Some(anchor);
                Ok(())
            }
        }
    }

    fn begin_edit(&mut self) {
        if let Some(tab) = self.tabs.get_mut(self.active) {
            let result = tab.begin_edit();
            self.focus_editor = result.is_ok();
            self.report(result);
        }
    }

    fn save_active(&mut self) {
        if let Some(tab) = self.tabs.get_mut(self.active) {
            let result = tab.save();
            self.report(result);
        }
    }

    fn request_close(&mut self, index: usize) {
        if self.tabs[index].is_dirty() {
            self.pending = Some(Pending::CloseTab(index));
        } else {
            self.close_tab(index);
        }
    }

    fn close_tab(&mut self, index: usize) {
        self.tabs.remove(index);
        if index < self.active {
            self.active -= 1;
        }
        self.active = self.active.min(self.tabs.len().saturating_sub(1));
    }

    pub(crate) fn check_close(&mut self, ctx: &egui::Context) {
        if !self.allow_exit
            && ctx.input(|input| input.viewport().close_requested())
            && self.tabs.iter().any(Document::is_dirty)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending = Some(Pending::Exit);
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.pending.is_some() || self.error.is_some() {
            return;
        }
        let (save, edit, close, folder) = ctx.input_mut(|input| {
            (
                input.consume_key(Modifiers::CTRL, Key::S),
                input.consume_key(Modifiers::CTRL, Key::E),
                input.consume_key(Modifiers::CTRL, Key::W),
                input.consume_key(Modifiers::CTRL, Key::O),
            )
        });
        if save {
            self.save_active();
        }
        if edit {
            self.begin_edit();
        }
        if close && !self.tabs.is_empty() {
            self.request_close(self.active);
        }
        if folder {
            self.choose_folder();
        }
    }

    pub(crate) fn show(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.shortcuts(&ctx);
        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.strong("MDV");
                ui.separator();
                if ui.button("Open folder").on_hover_text("Ctrl+O").clicked() {
                    self.choose_folder();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    self.settings.menu(ui);
                });
            });
        });
        let selected = self.tabs.get(self.active).map(|tab| tab.path.clone());
        egui::Panel::left("files")
            .default_size(230.0)
            .size_range(140.0..=500.0)
            .show(ui, |ui| {
                if let Some(path) = self.tree.ui(ui, selected.as_deref()) {
                    let result = self.open_path(&path, None);
                    self.report(result);
                }
            });
        egui::CentralPanel::default().show(ui, |ui| {
            if self.tabs.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(100.0);
                    ui.heading("Markdown, simply.");
                    ui.weak("Choose a file from the tree, or open a folder.");
                });
                return;
            }
            self.tab_bar(ui);
            self.document_ui(ui);
        });

        // Consume URL commands so local links never reach the browser integration.
        let mut urls = Vec::new();
        ctx.output_mut(|output| {
            output.commands.retain(|command| {
                if let egui::OutputCommand::OpenUrl(url) = command {
                    urls.push(url.url.clone());
                    false
                } else {
                    true
                }
            });
        });
        for url in urls {
            let result = self.follow_link(&url);
            self.report(result);
        }
        self.confirmation(&ctx);
        self.error_dialog(&ctx);
    }

    fn tab_bar(&mut self, ui: &mut egui::Ui) {
        let mut close = None;
        egui::ScrollArea::horizontal()
            .id_salt("tabs")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (index, tab) in self.tabs.iter().enumerate() {
                        ui.push_id(&tab.path, |ui| {
                            let name = file_name(&tab.path);
                            let title = if tab.is_dirty() {
                                format!("{name} *")
                            } else {
                                name
                            };
                            if ui
                                .selectable_label(index == self.active, title)
                                .on_hover_text(tab.path.to_string_lossy())
                                .clicked()
                            {
                                self.active = index;
                            }
                            if ui.small_button("x").on_hover_text("Close tab").clicked() {
                                close = Some(index);
                            }
                            ui.separator();
                        });
                    }
                });
            });
        if let Some(index) = close {
            self.request_close(index);
        }
        ui.separator();
    }

    fn document_ui(&mut self, ui: &mut egui::Ui) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let editing = tab.draft.is_some();
        let path = tab.path.clone();
        ui.horizontal(|ui| {
            if editing {
                if ui.button("Save").on_hover_text("Ctrl+S").clicked() {
                    self.save_active();
                }
                if ui.button("Discard").clicked() {
                    self.tabs[self.active].discard();
                }
            } else if ui.button("Edit").on_hover_text("Ctrl+E").clicked() {
                self.begin_edit();
            }
            ui.add(egui::Label::new(egui::RichText::new(path.to_string_lossy()).weak()).truncate())
                .on_hover_text(path.to_string_lossy());
        });
        ui.separator();
        let tab = &mut self.tabs[self.active];
        ui.push_id(&tab.path, |ui| {
            self.settings.style_document(ui);
            if let Some(draft) = &mut tab.draft {
                egui::ScrollArea::both().id_salt("editor").show(ui, |ui| {
                    let response = ui.add_sized(
                        ui.available_size(),
                        egui::TextEdit::multiline(draft)
                            .id_salt("source")
                            .code_editor()
                            .font(egui::FontId::new(
                                self.settings.size,
                                self.settings.font.family(),
                            ))
                            .desired_width(f32::INFINITY),
                    );
                    if self.focus_editor {
                        response.request_focus();
                        self.focus_editor = false;
                    }
                });
            } else if tab.is_markdown {
                tab.view.show(ui, tab.anchor.take());
            } else {
                egui::ScrollArea::both().id_salt("preview").show(ui, |ui| {
                    ui.add_space(8.0);
                    ui.add(egui::Label::new(&tab.text).wrap().selectable(true));
                    ui.add_space(16.0);
                });
            }
        });
    }

    fn confirmation(&mut self, ctx: &egui::Context) {
        let Some(pending) = self.pending else {
            return;
        };
        if self.error.is_some() {
            return;
        }
        let mut choice = None;
        let response = egui::Modal::new(egui::Id::new("unsaved")).show(ctx, |ui| {
            ui.set_max_width(430.0);
            ui.heading("Unsaved changes");
            match pending {
                Pending::CloseTab(index) => {
                    ui.label(format!(
                        "Save changes to {} before closing?",
                        file_name(&self.tabs[index].path)
                    ));
                }
                Pending::Exit => {
                    ui.label("Save changes in all edited tabs before quitting?");
                }
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.button("Save and close").clicked() {
                    choice = Some(true);
                }
                if ui.button("Discard and close").clicked() {
                    choice = Some(false);
                }
                if ui.button("Cancel").clicked() {
                    self.pending = None;
                }
            });
        });
        if response.should_close() {
            self.pending = None;
        }
        if let Some(save) = choice {
            if save {
                let result = match pending {
                    Pending::CloseTab(index) => self.tabs[index].save(),
                    Pending::Exit => self
                        .tabs
                        .iter_mut()
                        .filter(|tab| tab.is_dirty())
                        .try_for_each(Document::save),
                };
                if result.is_err() {
                    self.report(result);
                    return;
                }
            }
            self.pending = None;
            match pending {
                Pending::CloseTab(index) => self.close_tab(index),
                Pending::Exit => {
                    self.allow_exit = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }

    fn error_dialog(&mut self, ctx: &egui::Context) {
        let Some(error) = self.error.clone() else {
            return;
        };
        let response = egui::Modal::new(egui::Id::new("error")).show(ctx, |ui| {
            ui.set_max_width(480.0);
            ui.heading("Unable to complete the action");
            ui.add_space(8.0);
            ui.label(error);
            ui.add_space(12.0);
            if ui.button("OK").clicked() {
                self.error = None;
            }
        });
        if response.should_close() {
            self.error = None;
        }
    }

    #[cfg(test)]
    pub(crate) fn scroll_metrics(&self) -> (f32, f32, usize) {
        self.tabs
            .get(self.active)
            .map_or((0.0, 0.0, 0), |tab| tab.view.diagnostic)
    }
}

impl eframe::App for Mdv {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.check_close(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.show(ui);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, "appearance", &self.settings);
    }
}

fn launch_document(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if !links::can_launch(path, bytes) {
        return Err("Executable files and shortcuts are not launched by MDV.".into());
    }
    open::that(path).map_err(|err| format!("Cannot open {}: {err}", path.display()))
}

#[cfg(test)]
mod tests {
    use egui_kittest::{Harness, kittest::Queryable};

    use super::*;

    fn app_with_file() -> (tempfile::TempDir, Mdv) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("first.md");
        fs::write(&path, "# First\n\n[Second](second.md)\n").unwrap();
        fs::write(directory.path().join("second.md"), "# Second\n").unwrap();
        let mut app = Mdv::default();
        app.open_path(&path, None).unwrap();
        (directory, app)
    }

    #[test]
    fn edit_save_and_discard_use_the_same_tab_and_correct_buttons() {
        let (directory, app) = app_with_file();
        let mut harness = Harness::new_ui_state(|ui, app| app.show(ui), app);
        harness.get_by_label("Edit").click();
        harness.run();
        assert!(harness.query_by_label("Edit").is_none());
        assert!(harness.query_by_label("Save").is_some());
        assert!(harness.query_by_label("Discard").is_some());
        harness.state_mut().tabs[0].draft = Some("# Edited\n".into());
        harness.get_by_label("Save").click();
        harness.run();
        assert_eq!(harness.state().tabs.len(), 1);
        assert!(harness.query_by_label("Save").is_none());
        assert!(harness.query_by_label("Discard").is_none());
        assert!(harness.query_by_label("Edit").is_some());
        assert!(harness.query_by_label("Edited").is_some());
        assert_eq!(
            fs::read_to_string(directory.path().join("first.md")).unwrap(),
            "# Edited\n"
        );
        harness.get_by_label("Edit").click();
        harness.run();
        harness.state_mut().tabs[0].draft = Some("Discard this".into());
        harness.get_by_label("Discard").click();
        harness.run();
        assert_eq!(harness.state().tabs[0].text, "# Edited\n");
        assert!(harness.state().tabs[0].draft.is_none());
        assert_eq!(harness.state().tabs.len(), 1);
        assert_eq!(
            fs::read_to_string(directory.path().join("first.md")).unwrap(),
            "# Edited\n"
        );
    }

    #[test]
    fn clicking_a_local_markdown_link_opens_a_new_tab_and_reuses_existing_tabs() {
        let (_directory, app) = app_with_file();
        let mut harness = Harness::new_ui_state(|ui, app| app.show(ui), app);
        harness.get_by_label("Second").click();
        harness.run();
        assert_eq!(harness.state().tabs.len(), 2);
        assert_eq!(harness.state().active, 1);
        harness.state_mut().active = 0;
        harness.state_mut().begin_edit();
        harness.state_mut().tabs[0].draft = Some("Keep these edits".into());
        let path = harness.state().tabs[0].path.clone();
        harness.state_mut().open_path(&path, None).unwrap();
        assert_eq!(harness.state().tabs.len(), 2);
        assert_eq!(
            harness.state().tabs[0].draft.as_deref(),
            Some("Keep these edits")
        );
    }

    #[test]
    fn fragment_links_reach_headings_outside_the_visible_blocks() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("anchors.md");
        let source = format!(
            "# Top {{#top}}\n\n[Jump to the end](#last)\n\n{}\n\n\
             # Last heading {{#last}}\n\n[Back to top](#top)\n",
            "A paragraph between the link and its target.\n\n".repeat(300)
        );
        fs::write(&path, source).unwrap();
        let mut app = Mdv::default();
        app.open_path(&path, None).unwrap();
        let mut harness = Harness::new_ui_state(|ui, app| app.show(ui), app);
        assert!(harness.query_by_label("Last heading").is_none());
        harness.get_by_label("Jump to the end").click();
        harness.run();
        assert!(harness.query_by_label("Last heading").is_some());
        assert_eq!(harness.state().tabs.len(), 1);
    }

    #[test]
    fn cached_code_remains_selectable_copyable_and_read_only() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("code.md");
        let code = (0..100)
            .map(|index| format!("let value_{index} = \"selectable code\";"))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(
            &path,
            format!("Introductory paragraph.\n\n```rust\n{code}\n```\n"),
        )
        .unwrap();
        let mut app = Mdv::default();
        app.open_path(&path, None).unwrap();
        let mut harness = Harness::new_ui_state(|ui, app| app.show(ui), app);
        harness
            .get_by_role(egui::accesskit::Role::MultilineTextInput)
            .focus();
        harness.run();
        harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
        harness.run();
        harness.event(egui::Event::Copy);
        harness.step();
        assert!(harness.output().platform_output.commands.iter().any(
            |command| matches!(command, egui::OutputCommand::CopyText(text) if text == &code)
        ));
        harness
            .get_by_role(egui::accesskit::Role::MultilineTextInput)
            .type_text("Not editable");
        harness.run();
        assert_eq!(
            harness
                .get_by_role(egui::accesskit::Role::MultilineTextInput)
                .value()
                .as_deref(),
            Some(code.as_str())
        );
        assert!(harness.state().tabs[0].draft.is_none());
    }

    #[test]
    fn closing_a_dirty_tab_requires_confirmation_and_cancel_keeps_edits() {
        let (_directory, mut app) = app_with_file();
        app.begin_edit();
        app.tabs[0].draft = Some("Unsaved".into());
        app.request_close(0);
        let mut harness = Harness::new_ui_state(|ui, app| app.show(ui), app);
        harness.get_by_label("Cancel").click();
        harness.run();
        assert_eq!(harness.state().tabs.len(), 1);
        assert!(harness.state().pending.is_none());
        harness.state_mut().request_close(0);
        harness.run();
        harness.get_by_label("Discard and close").click();
        harness.run();
        assert!(harness.state().tabs.is_empty());
    }

    #[test]
    fn typing_into_the_editor_changes_the_draft_and_saves_exactly_that_text() {
        let (directory, app) = app_with_file();
        let mut harness = Harness::new_ui_state(|ui, app| app.show(ui), app);
        harness.get_by_label("Edit").click();
        harness.run();
        harness
            .get_by_role(egui::accesskit::Role::MultilineTextInput)
            .type_text("Typed in the editor");
        harness.run();
        let edited = harness.state().tabs[0].draft.clone().unwrap();
        assert!(edited.contains("Typed in the editor"));
        assert!(harness.state().tabs[0].is_dirty());
        harness.get_by_label("Save").click();
        harness.run();
        assert_eq!(
            fs::read_to_string(directory.path().join("first.md")).unwrap(),
            edited
        );
    }

    #[test]
    fn appearance_menu_changes_theme_font_and_size() {
        let (_directory, app) = app_with_file();
        let mut harness = Harness::new_ui_state(|ui, app| app.show(ui), app);
        harness.get_by_label("Options").click();
        harness.run();
        harness.get_by_label("Dark").click();
        harness.run();
        assert!(harness.state().settings.dark);
        assert_eq!(harness.ctx.theme(), egui::Theme::Dark);
        harness.get_by_label("Monospace").click();
        harness.run();
        assert!(harness.state().settings.font == crate::settings::Font::Monospace);
        let previous_size = harness.state().settings.size;
        harness
            .get_by_role_and_label(egui::accesskit::Role::Slider, "Size")
            .click();
        harness.run();
        assert_ne!(harness.state().settings.size, previous_size);
    }

    #[test]
    fn quitting_can_save_all_dirty_tabs() {
        let (directory, mut app) = app_with_file();
        app.begin_edit();
        app.tabs[0].draft = Some("First changed".into());
        app.follow_link("second.md").unwrap();
        app.begin_edit();
        app.tabs[1].draft = Some("Second changed".into());
        app.pending = Some(Pending::Exit);
        let mut harness = Harness::new_ui_state(|ui, app| app.show(ui), app);
        harness.get_by_label("Save and close").click();
        harness.run();
        assert!(harness.state().allow_exit);
        assert!(harness.state().tabs.iter().all(|tab| !tab.is_dirty()));
        assert_eq!(
            fs::read_to_string(directory.path().join("first.md")).unwrap(),
            "First changed"
        );
        assert_eq!(
            fs::read_to_string(directory.path().join("second.md")).unwrap(),
            "Second changed"
        );
    }

    #[test]
    #[ignore = "Run in release mode to measure real scroll-frame cost"]
    fn scroll_frame_benchmark() {
        let mut source = String::new();
        for section in 0..1_000 {
            source.push_str(&format!(
                "## Section {section}\n\nA paragraph with **bold text**, \
                 *emphasis*, `inline code`, and [a link](other.md). \
                 Scrolling should not lay out the entire document again.\n\n\
                 - First item\n- Second item\n- Third item\n\n\
                 ```rust\nfn example() {{\n    println!(\"section {section}\");\n}}\n```\n\n\
                 | Name | Value |\n| --- | --- |\n| Example | {section} |\n\n"
            ));
        }
        benchmark_scroll_document("Mixed Markdown", &source);
        let list: String = (0..10_000)
            .map(|index| format!("1. Item {index} with **bold** and [a link](other.md).\n"))
            .collect();
        benchmark_scroll_document("Long ordered list", &list);
        let code: String = (0..10_000)
            .map(|index| format!("let variable_{index} = \"a line of code\";\n"))
            .collect();
        benchmark_scroll_document("Long code block", &format!("```rust\n{code}```\n"));
    }

    #[test]
    #[ignore = "Local-only diagnosis; requires an explicitly supplied fixture folder"]
    fn local_scroll_diagnostic() {
        let directory = PathBuf::from(
            std::env::var_os("MDV_SCROLL_FIXTURE_DIR")
                .expect("Set MDV_SCROLL_FIXTURE_DIR to the local Markdown folder"),
        );
        let mut paths: Vec<_> = fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "md"))
            .collect();
        paths.sort();
        for path in paths {
            let mut app = Mdv::default();
            app.open_path(&path, None).unwrap();
            let ctx = egui::Context::default();
            let mut times = Vec::new();
            let mut backwards = 0;
            let mut resized = 0;
            let mut previous = 0.0;
            let mut stuck = 0;
            let mut max_height = 0.0_f32;
            for frame in 0..240 {
                let mut input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1100.0, 760.0),
                    )),
                    time: Some(frame as f64 / 60.0),
                    ..Default::default()
                };
                input
                    .events
                    .push(egui::Event::PointerMoved(egui::pos2(700.0, 400.0)));
                input.events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Line,
                    delta: egui::vec2(0.0, -3.0),
                    phase: egui::TouchPhase::Move,
                    modifiers: Modifiers::NONE,
                });
                let start = std::time::Instant::now();
                let mut output = ctx.run_ui(input, |ui| app.show(ui));
                let _ = ctx.tessellate(std::mem::take(&mut output.shapes), output.pixels_per_point);
                output.drop_without_applying_deltas();
                let (offset, height, changes) = app.tabs[0].view.diagnostic;
                if frame > 15 {
                    times.push(start.elapsed().as_secs_f64() * 1_000.0);
                    backwards += usize::from(offset < previous - 1.0);
                    stuck +=
                        usize::from((offset - previous).abs() < 0.01 && offset < height - 800.0);
                    resized += changes;
                }
                max_height = max_height.max(height);
                previous = offset;
            }
            times.sort_by(f64::total_cmp);
            eprintln!(
                "{}: p50 {:.2} ms p95 {:.2} ms; layout={} measure={} resized={} backwards={} stuck={} offset={:.0}/{:.0}",
                path.file_name().unwrap().to_string_lossy(),
                times[times.len() / 2],
                times[times.len() * 95 / 100],
                app.tabs[0].view.layout_passes,
                app.tabs[0].view.measure_calls,
                resized,
                backwards,
                stuck,
                previous,
                max_height,
            );
        }
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "Local native profiling; requires a document and output directory"]
    fn native_scroll_diagnostic() {
        use std::time::{Duration, Instant};
        use winit::platform::windows::EventLoopBuilderExtWindows;

        struct Probe {
            inner: Mdv,
            start: Instant,
            last: Instant,
            rows: String,
            output: PathBuf,
            captures: usize,
            wheel_events: usize,
        }

        impl eframe::App for Probe {
            fn raw_input_hook(&mut self, _ctx: &egui::Context, input: &mut egui::RawInput) {
                self.wheel_events += input
                    .events
                    .iter()
                    .filter(|event| matches!(event, egui::Event::MouseWheel { .. }))
                    .count();
                if std::env::var_os("MDV_NATIVE_VIRTUAL_HOVER").is_some() {
                    input
                        .events
                        .push(egui::Event::PointerMoved(egui::pos2(700.0, 400.0)));
                }
            }

            fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
                use std::fmt::Write;
                let now = Instant::now();
                let elapsed = now.duration_since(self.start).as_secs_f64();
                let interval = now.duration_since(self.last).as_secs_f64() * 1000.0;
                self.last = now;
                self.inner.show(ui);
                let (offset, height, resized) = self.inner.tabs[0].view.diagnostic;
                writeln!(
                    self.rows,
                    "{elapsed:.6},{interval:.3},{:.3},{offset:.2},{height:.2},{resized},{}",
                    frame.info().cpu_usage.unwrap_or(0.0) * 1000.0,
                    self.wheel_events,
                )
                .unwrap();
                let screenshots = ui.input(|input| {
                    input
                        .events
                        .iter()
                        .filter_map(|event| match event {
                            egui::Event::Screenshot { image, .. } => Some(image.clone()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                });
                for screenshot in screenshots {
                    let pixels: Vec<_> = screenshot
                        .pixels
                        .iter()
                        .flat_map(|pixel| pixel.to_srgba_unmultiplied())
                        .collect();
                    image::save_buffer(
                        self.output.join(format!("frame-{}.png", self.captures)),
                        &pixels,
                        screenshot.size[0] as u32,
                        screenshot.size[1] as u32,
                        image::ColorType::Rgba8,
                    )
                    .unwrap();
                }
                if elapsed > 2.0 + self.captures as f64 * 3.0 && self.captures < 3 {
                    self.captures += 1;
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::Screenshot(
                            egui::UserData::default(),
                        ));
                }
                if elapsed > 12.0 {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                } else {
                    ui.ctx()
                        .request_repaint_after(Duration::from_secs_f64(12.1 - elapsed));
                }
            }
        }

        impl Drop for Probe {
            fn drop(&mut self) {
                fs::write(self.output.join("frames.csv"), &self.rows).unwrap();
            }
        }

        let document = PathBuf::from(
            std::env::var_os("MDV_NATIVE_DOCUMENT").expect("Set MDV_NATIVE_DOCUMENT"),
        );
        let output = PathBuf::from(
            std::env::var_os("MDV_NATIVE_PROFILE_DIR").expect("Set MDV_NATIVE_PROFILE_DIR"),
        );
        fs::create_dir_all(&output).unwrap();
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_title("MDV local scroll diagnosis")
                .with_inner_size([1100.0, 760.0]),
            renderer: eframe::Renderer::Wgpu,
            event_loop_builder: Some(Box::new(|builder| {
                builder.with_any_thread(true);
            })),
            persistence_path: Some(output.join("state")),
            ..Default::default()
        };
        eframe::run_native(
            "MDV scroll diagnosis",
            options,
            Box::new(move |cc| {
                if let Some(state) = &cc.wgpu_render_state {
                    fs::write(
                        output.join("adapter.txt"),
                        format!("{:#?}", state.adapter.get_info()),
                    )?;
                } else {
                    fs::write(output.join("adapter.txt"), "OpenGL/EGL renderer")?;
                }
                Ok(Box::new(Probe {
                    inner: Mdv::new(cc, Some(document)),
                    start: Instant::now(),
                    last: Instant::now(),
                    rows: "time,interval_ms,cpu_ms,offset,height,resized,wheel_events\n".into(),
                    output,
                    captures: 0,
                    wheel_events: 0,
                }))
            }),
        )
        .unwrap();
    }

    #[test]
    #[ignore = "Local native CPU profiling; requires a document and output directory"]
    fn native_software_scroll_diagnostic() {
        assert!(
            std::env::var_os("MDV_NATIVE_PROFILE_DIR").is_some(),
            "Set MDV_NATIVE_PROFILE_DIR"
        );
        let document = PathBuf::from(
            std::env::var_os("MDV_NATIVE_DOCUMENT").expect("Set MDV_NATIVE_DOCUMENT"),
        );
        crate::software::run(Some(document)).unwrap();
    }

    fn benchmark_scroll_document(name: &str, source: &str) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("large.md");
        fs::write(&path, source).unwrap();
        let mut app = Mdv::default();
        app.open_path(&path, None).unwrap();
        let ctx = egui::Context::default();
        let mut milliseconds = Vec::new();
        for frame in 0..135 {
            let mut input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100.0, 760.0),
                )),
                time: Some(frame as f64 / 60.0),
                ..Default::default()
            };
            input
                .events
                .push(egui::Event::PointerMoved(egui::pos2(800.0, 400.0)));
            input.events.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -120.0),
                phase: egui::TouchPhase::Move,
                modifiers: Modifiers::NONE,
            });
            let start = std::time::Instant::now();
            let mut output = ctx.run_ui(input, |ui| app.show(ui));
            let _primitives =
                ctx.tessellate(std::mem::take(&mut output.shapes), output.pixels_per_point);
            output.drop_without_applying_deltas();
            if frame >= 15 {
                milliseconds.push(start.elapsed().as_secs_f64() * 1_000.0);
            }
        }
        milliseconds.sort_by(f64::total_cmp);
        eprintln!(
            "{name}: {} bytes, {} lines; scroll frames: p50 {:.2} ms, p95 {:.2} ms, max {:.2} ms",
            source.len(),
            source.lines().count(),
            milliseconds[60],
            milliseconds[114],
            milliseconds[119],
        );
    }
}
