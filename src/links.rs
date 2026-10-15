use std::path::{Path, PathBuf};

use percent_encoding::percent_decode_str;
use url::Url;

#[derive(Debug, PartialEq, Eq)]
pub enum LinkTarget {
    Browser(String),
    Local {
        path: PathBuf,
        anchor: Option<String>,
    },
    Anchor(String),
}

pub fn resolve(base_file: &Path, href: &str) -> Result<LinkTarget, String> {
    let href = href.trim();
    if href.is_empty() {
        return Err("The link has no destination.".into());
    }
    if let Some(anchor) = href.strip_prefix('#') {
        return Ok(LinkTarget::Anchor(decode_anchor(anchor)?));
    }

    // A drive letter is a filesystem prefix, not a URL scheme.
    #[cfg(windows)]
    if (href.as_bytes().get(1) == Some(&b':')
        && href.as_bytes().first().is_some_and(u8::is_ascii_alphabetic))
        || href.starts_with("\\\\")
    {
        let (path, anchor) = href.split_once('#').unwrap_or((href, ""));
        return Ok(LinkTarget::Local {
            path: PathBuf::from(path),
            anchor: (!anchor.is_empty())
                .then(|| decode_anchor(anchor))
                .transpose()?,
        });
    }

    let url = if href.starts_with("//") {
        Url::parse(&format!("https:{href}"))
    } else {
        let base = Url::from_file_path(base_file)
            .map_err(|()| format!("Cannot resolve links from {}.", base_file.display()))?;
        base.join(href)
    }
    .map_err(|err| format!("Invalid link: {err}"))?;

    match url.scheme() {
        "http" | "https" | "mailto" => Ok(LinkTarget::Browser(url.into())),
        "file" => Ok(LinkTarget::Local {
            path: url
                .to_file_path()
                .map_err(|()| "This file link is not valid on this operating system.".to_owned())?,
            anchor: url.fragment().map(decode_anchor).transpose()?,
        }),
        scheme => Err(format!("Links using '{scheme}:' are not supported.")),
    }
}

fn decode_anchor(anchor: &str) -> Result<String, String> {
    percent_decode_str(anchor)
        .decode_utf8()
        .map(|text| text.into_owned())
        .map_err(|err| format!("Invalid UTF-8 in link anchor: {err}"))
}

pub fn is_external_document(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some(
            "pdf"
                | "png"
                | "jpg"
                | "jpeg"
                | "gif"
                | "webp"
                | "svg"
                | "bmp"
                | "ico"
                | "avif"
                | "tif"
                | "tiff"
                | "mp3"
                | "wav"
                | "ogg"
                | "mp4"
                | "webm"
                | "mkv"
                | "doc"
                | "docx"
                | "xls"
                | "xlsx"
                | "ppt"
                | "pptx"
                | "odt"
                | "ods"
                | "odp"
        )
    )
}

pub fn can_launch(path: &Path, bytes: &[u8]) -> bool {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    !matches!(
        extension.as_str(),
        "exe"
            | "com"
            | "bat"
            | "cmd"
            | "ps1"
            | "vbs"
            | "vbe"
            | "js"
            | "jse"
            | "wsf"
            | "wsh"
            | "msi"
            | "msp"
            | "scr"
            | "pif"
            | "cpl"
            | "dll"
            | "lnk"
            | "url"
            | "appref-ms"
            | "desktop"
            | "sh"
            | "bash"
            | "run"
            | "appimage"
    ) && !bytes.starts_with(b"MZ")
        && !bytes.starts_with(b"\x7fELF")
        && !bytes.starts_with(b"#!")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> PathBuf {
        std::env::current_dir()
            .unwrap()
            .join("docs")
            .join("start.md")
    }

    #[test]
    fn relative_links_decode_names_and_keep_fragments() {
        let LinkTarget::Local { path, anchor } =
            resolve(&base(), "../other%20file.md#hello%20world").unwrap()
        else {
            panic!("Expected a local file");
        };
        assert_eq!(path, std::env::current_dir().unwrap().join("other file.md"));
        assert_eq!(anchor.as_deref(), Some("hello world"));
    }

    #[test]
    fn handles_absolute_file_urls_and_unicode() {
        let path = std::env::current_dir().unwrap().join("notes #1.md");
        let url = Url::from_file_path(&path).unwrap();
        assert_eq!(
            resolve(&base(), url.as_str()).unwrap(),
            LinkTarget::Local { path, anchor: None }
        );
        assert_eq!(
            resolve(&base(), "#caf%C3%A9").unwrap(),
            LinkTarget::Anchor("caf\u{e9}".into())
        );
    }

    #[test]
    fn internet_links_are_not_local_files() {
        for href in [
            "https://example.com/a?q=1#two",
            "http://example.com",
            "mailto:test@example.com",
            "//example.com/path",
        ] {
            assert!(matches!(
                resolve(&base(), href).unwrap(),
                LinkTarget::Browser(_)
            ));
        }
    }

    #[test]
    fn refuses_active_url_schemes_and_executable_documents() {
        for href in [
            "javascript:alert(1)",
            "data:text/html,hi",
            "shell:AppsFolder",
        ] {
            assert!(resolve(&base(), href).is_err());
        }
        assert!(!can_launch(Path::new("program.exe"), b"anything"));
        assert!(!can_launch(Path::new("disguised.pdf"), b"MZpayload"));
        assert!(!can_launch(Path::new("program"), b"\x7fELFpayload"));
        assert!(can_launch(Path::new("document.pdf"), b"%PDF-1.7"));
    }

    #[cfg(windows)]
    #[test]
    fn accepts_windows_drive_paths() {
        assert_eq!(
            resolve(&base(), r"C:\docs\other.md#section").unwrap(),
            LinkTarget::Local {
                path: PathBuf::from(r"C:\docs\other.md"),
                anchor: Some("section".into()),
            }
        );
    }
}
