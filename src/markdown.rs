use std::{collections::HashSet, path::Path};

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

use crate::links::{self, LinkTarget};

pub fn parser_options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_HEADING_ATTRIBUTES
        | Options::ENABLE_GFM
}

pub fn prepare(source: &str, path: &Path) -> Result<String, String> {
    let mut events: Vec<_> = Parser::new_ext(source, parser_options()).collect();
    let mut used_ids: HashSet<String> = events
        .iter()
        .filter_map(|event| match event {
            Event::Start(Tag::Heading { id: Some(id), .. }) => Some(id.to_string()),
            _ => None,
        })
        .collect();
    let mut generated_ids = Vec::new();

    for (index, event) in events.iter().enumerate() {
        if matches!(event, Event::Start(Tag::Heading { id: None, .. })) {
            let mut title = String::new();
            for event in &events[index + 1..] {
                match event {
                    Event::End(TagEnd::Heading(_)) => break,
                    Event::Text(text) | Event::Code(text) => title.push_str(text),
                    Event::SoftBreak | Event::HardBreak => title.push(' '),
                    _ => {}
                }
            }
            let slug: String = title
                .to_lowercase()
                .chars()
                .filter_map(|character| {
                    if character.is_whitespace() {
                        Some('-')
                    } else if character.is_alphanumeric() || matches!(character, '-' | '_') {
                        Some(character)
                    } else {
                        None
                    }
                })
                .collect();
            let mut unique = slug.clone();
            let mut suffix = 1;
            while !used_ids.insert(unique.clone()) {
                unique = format!("{slug}-{suffix}");
                suffix += 1;
            }
            generated_ids.push((index, unique));
        }
    }
    for (index, slug) in generated_ids {
        if let Event::Start(Tag::Heading { id, .. }) = &mut events[index] {
            *id = Some(slug.into());
        }
    }
    for event in &mut events {
        if let Event::Start(Tag::Link { dest_url, .. }) = event
            && dest_url.starts_with('#')
            && let Ok(LinkTarget::Anchor(anchor)) = links::resolve(path, dest_url)
        {
            *dest_url = format!("#{anchor}").into();
        }
        if let Event::Start(Tag::Image { dest_url, .. }) = event {
            match links::resolve(path, dest_url) {
                Ok(LinkTarget::Local { path, .. }) => {
                    // egui's file loader expects a filesystem path, not URL encoding.
                    *dest_url = format!("file://{}", path.display()).into();
                }
                Ok(LinkTarget::Browser(url)) if url.starts_with("http") => *dest_url = url.into(),
                // Unsupported image URIs get the renderer's visible load-error placeholder.
                _ => {}
            }
        }
    }

    let mut preview = String::new();
    pulldown_cmark_to_cmark::cmark(events.into_iter(), &mut preview)
        .map_err(|err| format!("Cannot prepare Markdown: {err}"))?;
    Ok(preview)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path() -> std::path::PathBuf {
        std::env::current_dir()
            .unwrap()
            .join("docs")
            .join("test.md")
    }

    #[test]
    fn generates_heading_anchors_without_altering_the_source() {
        let source = "# Hello, *world*!\n\n# Hello, world!\n\nTitle\n=====\n";
        let rendered = prepare(source, &path()).unwrap();
        let ids: Vec<_> = Parser::new_ext(&rendered, Options::ENABLE_HEADING_ATTRIBUTES)
            .filter_map(|event| match event {
                Event::Start(Tag::Heading { id, .. }) => id.map(|id| id.to_string()),
                _ => None,
            })
            .collect();
        assert_eq!(ids, ["hello-world", "hello-world-1", "title"]);
        assert!(!source.contains("{#"));
    }

    #[test]
    fn resolves_reference_images_relative_to_the_document() {
        let rendered =
            prepare("![A picture][pic]\n\n[pic]: ../image%20one.png\n", &path()).unwrap();
        let expected = format!(
            "file://{}",
            std::env::current_dir()
                .unwrap()
                .join("image one.png")
                .display()
        );
        assert!(Parser::new(&rendered).any(|event| matches!(
            event,
            Event::Start(Tag::Image { dest_url, .. }) if dest_url.as_ref() == expected
        )));
    }

    #[test]
    fn preserves_tables_tasks_links_and_code_as_markdown() {
        let source = "| A | B |\n|---|---|\n| 1 | 2 |\n\n- [x] Done\n\n[Next](other.md)\n\n```rust\nlet x = 1;\n```\n";
        let rendered = prepare(source, &path()).unwrap();
        let options = Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS;
        let before: Vec<_> = Parser::new_ext(source, options).collect();
        let after: Vec<_> = Parser::new_ext(&rendered, options).collect();
        assert_eq!(before, after);
    }

    #[test]
    fn encoded_anchors_work_and_unsupported_images_do_not_block_the_document() {
        let rendered = prepare(
            "# Caf\u{e9}\n\n[Jump](#caf%C3%A9)\n\n![Unsupported](custom:picture)\n",
            &path(),
        )
        .unwrap();
        assert!(Parser::new(&rendered).any(|event| matches!(
            event,
            Event::Start(Tag::Link { dest_url, .. }) if dest_url.as_ref() == "#caf\u{e9}"
        )));
        assert!(rendered.contains("custom:picture"));
    }
}
