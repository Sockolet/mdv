use std::{collections::HashMap, sync::Arc};

use eframe::egui::{self, FontId, Rect, TextStyle, Ui, UiBuilder, Vec2};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};

const TOP: f32 = 8.0;
const BOTTOM: f32 = 16.0;
const LIST_CHUNK_ITEMS: usize = 32;

struct Block {
    markdown: String,
    code: Option<Code>,
    fragments: Vec<String>,
    images: Vec<String>,
    height: f32,
    width: f32,
    gap: bool,
}

struct Code {
    text: String,
    language: String,
    galley: Option<(f32, Arc<egui::Galley>)>,
}

#[derive(PartialEq)]
struct Layout {
    width: f32,
    pixels_per_point: f32,
    fonts: [FontId; 3],
    spacing: Vec2,
    dark: bool,
}

pub struct Preview {
    blocks: Vec<Block>,
    ends: Vec<f32>,
    content_width: f32,
    headings: HashMap<String, usize>,
    cache: CommonMarkCache,
    layout: Option<Layout>,
    loading: Vec<usize>,
    correction: Option<f32>,
    #[cfg(test)]
    pub(crate) layout_passes: usize,
    #[cfg(test)]
    rendered_blocks: usize,
    #[cfg(test)]
    pub(crate) measure_calls: usize,
    #[cfg(test)]
    pub(crate) diagnostic: (f32, f32, usize),
}

impl Preview {
    pub fn new(markdown: &str) -> Result<Self, String> {
        let mut blocks = Vec::new();
        let mut headings = HashMap::new();
        let mut cache = CommonMarkCache::default();
        let mut events = Vec::new();
        let mut depth = 0;
        for event in Parser::new_ext(markdown, crate::markdown::parser_options()) {
            match event {
                Event::Start(_) => depth += 1,
                Event::End(_) => depth -= 1,
                _ => {}
            }
            events.push(event);
            if depth == 0 {
                for (part, gap) in split_list(&events) {
                    let index = blocks.len();
                    let mut text = String::new();
                    pulldown_cmark_to_cmark::cmark(part.iter(), &mut text)
                        .map_err(|err| format!("Cannot prepare Markdown block: {err}"))?;
                    let mut fragments = Vec::new();
                    let mut images = Vec::new();
                    for event in &part {
                        match event {
                            Event::Start(Tag::Heading { id: Some(id), .. }) => {
                                headings.insert(id.to_string(), index);
                            }
                            Event::Start(Tag::Link { dest_url, .. })
                                if dest_url.starts_with('#') =>
                            {
                                cache.add_link_hook(dest_url.to_string());
                                fragments.push(dest_url.to_string());
                            }
                            Event::Start(Tag::Image { dest_url, .. }) => {
                                images.push(dest_url.to_string());
                            }
                            _ => {}
                        }
                    }
                    blocks.push(Block {
                        markdown: text,
                        code: Code::from_events(&part),
                        fragments,
                        images,
                        height: 0.0,
                        width: 0.0,
                        gap,
                    });
                }
                events.clear();
            }
        }
        Ok(Self {
            ends: vec![TOP; blocks.len()],
            content_width: 0.0,
            blocks,
            headings,
            cache,
            layout: None,
            loading: Vec::new(),
            correction: None,
            #[cfg(test)]
            layout_passes: 0,
            #[cfg(test)]
            rendered_blocks: 0,
            #[cfg(test)]
            measure_calls: 0,
            #[cfg(test)]
            diagnostic: (0.0, 0.0, 0),
        })
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        anchor: Option<String>,
    ) -> egui::scroll_area::ScrollAreaOutput<()> {
        // Match ScrollArea's additional IdSalt wrapping, not just the raw string.
        let scroll_id = ui.make_persistent_id(egui::IdSalt::new("preview"));
        let old_offset =
            egui::scroll_area::State::load(ui.ctx(), scroll_id).map_or(0.0, |state| state.offset.y);
        let scroll_anchor = self.position(old_offset);
        let scroll = ui.spacing().scroll;
        let bar_width = if scroll.floating {
            0.0
        } else {
            scroll.allocated_width()
        };
        let layout = Layout {
            width: (ui.available_width() - bar_width).floor().max(1.0),
            pixels_per_point: ui.pixels_per_point(),
            fonts: [
                TextStyle::Body.resolve(ui.style()),
                TextStyle::Heading.resolve(ui.style()),
                TextStyle::Monospace.resolve(ui.style()),
            ],
            spacing: ui.spacing().item_spacing,
            dark: ui.visuals().dark_mode,
        };
        let reflowed = self.update_layout(ui, &layout);
        let mut offset = self.correction.take().map(|delta| old_offset + delta);
        if reflowed {
            offset = Some(if old_offset <= TOP {
                old_offset
            } else {
                self.offset(scroll_anchor)
            });
        }
        if let Some(index) = anchor.as_ref().and_then(|slug| self.headings.get(slug)) {
            offset = Some(self.start(*index));
        }
        let max_offset = (self.height() - ui.available_height()).max(0.0);
        if offset.is_none() && old_offset > max_offset {
            offset = Some(max_offset);
        }
        let mut area = egui::ScrollArea::both()
            .id_salt("preview")
            .auto_shrink([false, false])
            .animated(false);
        if let Some(offset) = offset {
            area = area.vertical_scroll_offset(offset.clamp(0.0, max_offset));
        }
        #[cfg(test)]
        {
            self.rendered_blocks = 0;
        }
        let mut resized = false;
        #[cfg(test)]
        let mut resized_blocks = 0;
        let output = area.show_viewport(ui, |ui, viewport| {
            let origin = ui.max_rect().min;
            let first = self
                .ends
                .partition_point(|end| *end < viewport.min.y - 80.0);
            for index in first..self.blocks.len() {
                let top = self.start(index);
                if top > viewport.max.y + 80.0 {
                    break;
                }
                let mut child = ui.new_child(
                    UiBuilder::new()
                        .id(ui.make_persistent_id(("block", index)))
                        .max_rect(Rect::from_min_size(
                            origin + egui::vec2(0.0, top),
                            egui::vec2(layout.width, f32::INFINITY),
                        )),
                );
                if anchor
                    .as_ref()
                    .is_some_and(|slug| self.headings.get(slug) == Some(&index))
                {
                    *self.cache.scroll_to_id_target_mut() = anchor.clone();
                }
                let size = self.blocks[index].show(&mut child, &mut self.cache);
                if (size.y - self.blocks[index].height).abs() > 0.5
                    || (size.x - self.blocks[index].width).abs() > 0.5
                {
                    self.blocks[index].height = size.y;
                    self.blocks[index].width = size.x;
                    resized = true;
                    #[cfg(test)]
                    {
                        resized_blocks += 1;
                    }
                }
                #[cfg(test)]
                {
                    self.rendered_blocks += 1;
                }
            }
            ui.set_min_size(egui::vec2(
                layout.width.max(self.content_width),
                self.height(),
            ));
        });
        if resized {
            let position = self.position(output.state.offset.y);
            self.reflow(layout.spacing.y);
            let delta = self.offset(position) - output.state.offset.y;
            if delta.abs() > 0.5 {
                self.correction = Some(delta);
            }
            ui.ctx().request_repaint();
        }
        self.layout = Some(layout);
        #[cfg(test)]
        {
            self.diagnostic = (output.state.offset.y, output.content_size.y, resized_blocks);
        }
        output
    }

    fn update_layout(&mut self, ui: &mut Ui, layout: &Layout) -> bool {
        if self.layout.as_ref() != Some(layout) {
            for index in 0..self.blocks.len() {
                self.measure(ui, index, layout.width);
            }
            self.loading = self
                .blocks
                .iter()
                .enumerate()
                .filter_map(|(index, block)| block.loading(ui.ctx()).then_some(index))
                .collect();
            self.reflow(layout.spacing.y);
            #[cfg(test)]
            {
                self.layout_passes += 1;
            }
            return true;
        }
        let mut changed = false;
        for index in std::mem::take(&mut self.loading) {
            if self.blocks[index].loading(ui.ctx()) {
                self.loading.push(index);
            } else {
                self.measure(ui, index, layout.width);
                changed = true;
            }
        }
        if changed {
            self.reflow(layout.spacing.y);
        }
        changed
    }

    fn measure(&mut self, ui: &mut Ui, index: usize, width: f32) {
        #[cfg(test)]
        {
            self.measure_calls += 1;
        }
        if let Some(code) = &mut self.blocks[index].code {
            code.galley = None;
        }
        let mut child = ui.new_child(
            UiBuilder::new()
                .id(ui.make_persistent_id(("measure", index)))
                .invisible()
                .max_rect(Rect::from_min_size(
                    ui.max_rect().min,
                    egui::vec2(width, f32::INFINITY),
                )),
        );
        let size = self.blocks[index].show(&mut child, &mut self.cache);
        self.blocks[index].height = size.y;
        self.blocks[index].width = size.x;
    }

    fn reflow(&mut self, spacing: f32) {
        let mut y = TOP;
        self.content_width = 0.0;
        for (block, end) in self.blocks.iter().zip(&mut self.ends) {
            y += block.height + if block.gap { spacing * 2.0 } else { 0.0 };
            self.content_width = self.content_width.max(block.width);
            *end = y;
        }
    }

    fn start(&self, index: usize) -> f32 {
        if index == 0 {
            TOP
        } else {
            self.ends[index - 1]
        }
    }

    fn height(&self) -> f32 {
        self.ends.last().copied().unwrap_or(TOP) + BOTTOM
    }

    fn position(&self, y: f32) -> (usize, f32) {
        let index = self
            .ends
            .partition_point(|end| *end <= y)
            .min(self.blocks.len().saturating_sub(1));
        let fraction = self.ends.get(index).map_or(0.0, |end| {
            ((y - self.start(index)) / (end - self.start(index)).max(1.0)).clamp(0.0, 1.0)
        });
        (index, fraction)
    }

    fn offset(&self, (index, fraction): (usize, f32)) -> f32 {
        self.start(index)
            + self
                .ends
                .get(index)
                .map_or(0.0, |end| (end - self.start(index)) * fraction)
    }
}

impl Block {
    fn show(&mut self, ui: &mut Ui, cache: &mut CommonMarkCache) -> Vec2 {
        if let Some(code) = &mut self.code {
            return code.show(ui);
        }
        let response = CommonMarkViewer::new()
            .enable_scroll_to_heading(true)
            .explicit_image_uri_scheme(true)
            .show(ui, cache, &self.markdown);
        for fragment in &self.fragments {
            if cache.get_link_hook(fragment) == Some(true) {
                ui.ctx().open_url(egui::OpenUrl::same_tab(fragment));
            }
        }
        response.response.rect.size()
    }

    fn loading(&self, ctx: &egui::Context) -> bool {
        self.images.iter().any(|uri| {
            matches!(
                ctx.try_load_texture(
                    uri,
                    egui::TextureOptions::default(),
                    egui::load::SizeHint::default(),
                ),
                Ok(egui::load::TexturePoll::Pending { .. })
            )
        })
    }
}

impl Code {
    fn from_events(events: &[Event<'_>]) -> Option<Self> {
        let Event::Start(Tag::CodeBlock(kind)) = events.first()? else {
            return None;
        };
        let text = events
            .iter()
            .filter_map(|event| match event {
                Event::Text(text) => Some(text.as_ref()),
                _ => None,
            })
            .collect();
        Some(Self {
            text,
            language: match kind {
                CodeBlockKind::Fenced(language) => language.to_string(),
                CodeBlockKind::Indented => String::new(),
            },
            galley: None,
        })
    }

    fn show(&mut self, ui: &mut Ui) -> Vec2 {
        let max_width = ui.available_width();
        let mut layouter = |ui: &Ui, text: &dyn egui::TextBuffer, width: f32| {
            if let Some((cached_width, galley)) = &self.galley
                && *cached_width == width
            {
                return galley.clone();
            }
            let mut job = egui_extras::syntax_highlighting::highlight(
                ui.ctx(),
                ui.style(),
                &egui_extras::syntax_highlighting::CodeTheme::from_style(ui.style()),
                text.as_str(),
                &self.language,
            );
            job.wrap.max_width = width;
            let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
            self.galley = Some((width, galley.clone()));
            galley
        };
        ui.scope(|ui| {
            egui_commonmark_backend::elements::code_block(ui, max_width, &self.text, &mut layouter);
        })
        .response
        .rect
        .size()
    }
}

fn split_list<'a>(events: &[Event<'a>]) -> Vec<(Vec<Event<'a>>, bool)> {
    let Some(Event::Start(Tag::List(start))) = events.first() else {
        return vec![(events.to_vec(), true)];
    };
    let mut result = Vec::new();
    let mut chunk = vec![events[0].clone()];
    let mut depth = 1;
    let mut count = 0;
    for event in &events[1..events.len() - 1] {
        if count > 0 && count % LIST_CHUNK_ITEMS == 0 && depth == 1 {
            chunk.push(Event::End(TagEnd::List(start.is_some())));
            result.push((std::mem::take(&mut chunk), false));
            chunk.push(Event::Start(Tag::List(
                start.map(|number| number.saturating_add(count as u64)),
            )));
        }
        match event {
            Event::Start(_) => depth += 1,
            Event::End(_) => depth -= 1,
            _ => {}
        }
        chunk.push(event.clone());
        if depth == 1 && matches!(event, Event::End(TagEnd::Item)) {
            count += 1;
        }
    }
    chunk.push(Event::End(TagEnd::List(start.is_some())));
    result.push((chunk, true));
    result
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use super::*;
    use crate::settings::{Font, Settings};

    fn frame(
        preview: &mut Preview,
        ctx: &egui::Context,
        size: Vec2,
        settings: &Settings,
        offset: Option<f32>,
        anchor: Option<&str>,
    ) -> egui::scroll_area::ScrollAreaOutput<()> {
        let mut result = None;
        settings.apply_theme(ctx);
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)),
                ..Default::default()
            },
            |ui| {
                settings.style_document(ui);
                if let Some(offset) = offset {
                    let id = ui.make_persistent_id(egui::IdSalt::new("preview"));
                    let mut state = egui::scroll_area::State::load(ctx, id).unwrap_or_default();
                    state.offset.y = offset;
                    state.store(ctx, id);
                }
                result = Some(preview.show(ui, anchor.map(str::to_owned)));
            },
        );
        output.drop_without_applying_deltas();
        result.unwrap()
    }

    fn paragraphs(count: usize) -> String {
        (0..count)
            .map(|index| {
                format!(
                    "Paragraph {index}: **bold**, *italic*, and [a link](other.md). \
                     Enough text to wrap when the reading pane becomes narrow.\n\n"
                )
            })
            .collect()
    }

    #[test]
    fn scrolling_renders_only_visible_blocks_without_remeasuring_the_document() {
        let mut preview = Preview::new(&paragraphs(2_000)).unwrap();
        let ctx = egui::Context::default();
        let settings = Settings::default();
        let size = egui::vec2(900.0, 600.0);
        let initial = frame(&mut preview, &ctx, size, &settings, None, None);
        let height = initial.content_size.y;
        assert_eq!(initial.state.offset.y, 0.0);
        for offset in [1_000.0, height * 0.5, height - 600.0, 400.0, 0.0] {
            let output = frame(&mut preview, &ctx, size, &settings, Some(offset), None);
            assert_eq!(preview.layout_passes, 1);
            assert!(preview.rendered_blocks > 0 && preview.rendered_blocks < 40);
            assert!((output.content_size.y - height).abs() < 0.5);
            assert!(
                (output.state.offset.y - offset).abs() < 1.0,
                "requested {offset}, got {}",
                output.state.offset.y
            );
        }
    }

    #[test]
    fn resizing_and_appearance_changes_reflow_once_and_preserve_the_reading_position() {
        let mut preview = Preview::new(&paragraphs(500)).unwrap();
        let ctx = egui::Context::default();
        let mut settings = Settings::default();
        let wide = egui::vec2(900.0, 600.0);
        frame(&mut preview, &ctx, wide, &settings, None, None);
        let output = frame(&mut preview, &ctx, wide, &settings, Some(8_000.0), None);
        let position = preview.position(output.state.offset.y);
        let narrow = egui::vec2(500.0, 600.0);
        let output = frame(&mut preview, &ctx, narrow, &settings, None, None);
        assert_eq!(preview.layout_passes, 2);
        let after = preview.position(output.state.offset.y);
        assert_eq!(position.0, after.0);
        assert!((position.1 - after.1).abs() < 0.02);

        frame(
            &mut preview,
            &ctx,
            egui::vec2(500.0, 800.0),
            &settings,
            None,
            None,
        );
        assert_eq!(preview.layout_passes, 2);
        settings.size = 24.0;
        frame(&mut preview, &ctx, narrow, &settings, None, None);
        assert_eq!(preview.layout_passes, 3);
        settings.font = Font::Monospace;
        frame(&mut preview, &ctx, narrow, &settings, None, None);
        assert_eq!(preview.layout_passes, 4);
        settings.dark = true;
        frame(&mut preview, &ctx, narrow, &settings, None, None);
        frame(&mut preview, &ctx, narrow, &settings, None, None);
        assert_eq!(preview.layout_passes, 5);
    }

    #[test]
    fn offscreen_headings_and_out_of_range_offsets_never_produce_a_blank_view() {
        let source = format!(
            "# First {{#first}}\n\n{}\n# Last {{#last}}\n\nEnd of document.",
            paragraphs(500)
        );
        let mut preview = Preview::new(&source).unwrap();
        let ctx = egui::Context::default();
        let settings = Settings::default();
        let size = egui::vec2(900.0, 600.0);
        frame(&mut preview, &ctx, size, &settings, None, None);
        let bottom = frame(&mut preview, &ctx, size, &settings, None, Some("last"));
        assert!(bottom.state.offset.y > 10_000.0);
        assert!(preview.rendered_blocks > 0);
        let beyond = frame(&mut preview, &ctx, size, &settings, Some(1e9), None);
        assert!(preview.rendered_blocks > 0);
        assert!((beyond.state.offset.y - bottom.state.offset.y).abs() < 1.0);
        let top = frame(&mut preview, &ctx, size, &settings, None, Some("first"));
        assert!(top.state.offset.y < 16.0);
        assert_eq!(preview.layout_passes, 1);
    }

    #[test]
    fn long_lists_are_bounded_and_keep_numbering_nested_items_and_links() {
        let mut source = String::new();
        for number in 5..5_005 {
            source.push_str(&format!("{number}. Item {number} [jump](#target)\n"));
            if number == 9 {
                source.push_str("    - Nested item\n");
            }
        }
        let mut preview = Preview::new(&source).unwrap();
        assert_eq!(preview.blocks.len(), 5_000_usize.div_ceil(LIST_CHUNK_ITEMS));
        for (index, block) in preview.blocks.iter().enumerate() {
            let first = Parser::new(&block.markdown).next().unwrap();
            assert_eq!(
                first,
                Event::Start(Tag::List(Some((5 + index * LIST_CHUNK_ITEMS) as u64)))
            );
            assert!(!block.fragments.is_empty());
        }
        assert!(preview.blocks[0].markdown.contains("Nested item"));
        let ctx = egui::Context::default();
        let settings = Settings::default();
        let size = egui::vec2(900.0, 600.0);
        let initial = frame(&mut preview, &ctx, size, &settings, None, None);
        let height = initial.content_size.y;
        frame(
            &mut preview,
            &ctx,
            size,
            &settings,
            Some(height * 0.75),
            None,
        );
        assert!(preview.rendered_blocks <= 3);
        assert_eq!(preview.layout_passes, 1);
    }

    struct DelayedImage(Arc<AtomicBool>);

    impl egui::load::ImageLoader for DelayedImage {
        fn id(&self) -> &str {
            "mdv-delayed-test-image"
        }

        fn load(
            &self,
            _ctx: &egui::Context,
            uri: &str,
            _hint: egui::load::SizeHint,
        ) -> egui::load::ImageLoadResult {
            if uri != "delay://picture" {
                return Err(egui::load::LoadError::NotSupported);
            }
            if self.0.load(Ordering::SeqCst) {
                Ok(egui::load::ImagePoll::Ready {
                    image: Arc::new(egui::ColorImage::new(
                        [64, 128],
                        vec![egui::Color32::LIGHT_BLUE; 64 * 128],
                    )),
                })
            } else {
                Ok(egui::load::ImagePoll::Pending { size: None })
            }
        }

        fn forget(&self, _uri: &str) {}

        fn forget_all(&self) {}

        fn byte_size(&self) -> usize {
            0
        }
    }

    #[test]
    fn delayed_images_update_only_their_block_without_moving_the_reading_position() {
        let mut preview =
            Preview::new(&format!("![Later](delay://picture)\n\n{}", paragraphs(500))).unwrap();
        let ctx = egui::Context::default();
        let ready = Arc::new(AtomicBool::new(false));
        ctx.run_ui(egui::RawInput::default(), |ui| {
            CommonMarkViewer::new().show(ui, &mut preview.cache, "");
        })
        .drop_without_applying_deltas();
        ctx.add_image_loader(Arc::new(DelayedImage(ready.clone())));
        let settings = Settings::default();
        let size = egui::vec2(900.0, 600.0);
        frame(&mut preview, &ctx, size, &settings, None, None);
        let measured = preview.measure_calls;
        let before = frame(&mut preview, &ctx, size, &settings, Some(3_000.0), None);
        let position = preview.position(before.state.offset.y);
        assert_eq!(preview.measure_calls, measured);
        assert_eq!(preview.loading, [0]);
        ready.store(true, Ordering::SeqCst);
        let after = frame(&mut preview, &ctx, size, &settings, None, None);
        assert_eq!(preview.measure_calls, measured + 1);
        assert!(preview.loading.is_empty());
        assert!(after.content_size.y > before.content_size.y + 50.0);
        let after_position = preview.position(after.state.offset.y);
        assert_eq!(position.0, after_position.0);
        assert!((position.1 - after_position.1).abs() < 0.02);
        frame(&mut preview, &ctx, size, &settings, None, None);
        assert_eq!(preview.measure_calls, measured + 1);
    }
}
