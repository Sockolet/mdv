use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use crate::{markdown, preview::Preview};

pub struct Document {
    pub path: PathBuf,
    pub text: String,
    pub draft: Option<String>,
    pub is_markdown: bool,
    pub view: Preview,
    pub anchor: Option<String>,
    disk_bytes: Vec<u8>,
    bom: bool,
    crlf: bool,
}

impl Document {
    pub fn open(path: &Path) -> Result<Self, String> {
        let path = dunce::canonicalize(path)
            .map_err(|err| format!("Cannot open {}: {err}", path.display()))?;
        let disk_bytes =
            fs::read(&path).map_err(|err| format!("Cannot read {}: {err}", path.display()))?;
        Self::from_bytes(path, disk_bytes)
    }

    pub fn from_bytes(path: PathBuf, disk_bytes: Vec<u8>) -> Result<Self, String> {
        let bom = disk_bytes.starts_with(b"\xef\xbb\xbf");
        let bytes = if bom { &disk_bytes[3..] } else { &disk_bytes };
        let decoded =
            std::str::from_utf8(bytes).map_err(|_| "This file is not UTF-8 text.".to_owned())?;
        if decoded.contains('\0') {
            return Err("This file contains binary data.".into());
        }
        let crlf = decoded.contains("\r\n");
        let text = decoded.replace("\r\n", "\n");
        let is_markdown = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                matches!(
                    extension.to_ascii_lowercase().as_str(),
                    "md" | "markdown" | "mdown" | "mkd" | "mdwn" | "mkdn"
                )
            });
        let preview = if is_markdown {
            markdown::prepare(&text, &path)?
        } else {
            String::new()
        };
        let view = Preview::new(&preview)?;
        Ok(Self {
            path,
            text,
            draft: None,
            is_markdown,
            view,
            anchor: None,
            disk_bytes,
            bom,
            crlf,
        })
    }

    pub fn begin_edit(&mut self) -> Result<(), String> {
        if self.draft.is_none() {
            *self = Self::open(&self.path)?;
            self.draft = Some(self.text.clone());
        }
        Ok(())
    }

    pub fn is_dirty(&self) -> bool {
        self.draft.as_ref().is_some_and(|draft| draft != &self.text)
    }

    pub fn discard(&mut self) {
        self.draft = None;
    }

    pub fn save(&mut self) -> Result<(), String> {
        let Some(draft) = &self.draft else {
            return Ok(());
        };
        self.check_disk()?;
        if draft == &self.text {
            self.draft = None;
            return Ok(());
        }
        let preview = if self.is_markdown {
            markdown::prepare(draft, &self.path)?
        } else {
            String::new()
        };
        let view = Preview::new(&preview)?;
        let metadata = fs::metadata(&self.path).map_err(|err| format!("Cannot save: {err}"))?;
        if metadata.permissions().readonly() {
            return Err("This file is read-only. Your edits have not been lost.".into());
        }
        let parent = self.path.parent().ok_or("The file has no parent folder.")?;
        let encoded = if self.crlf {
            draft.replace('\n', "\r\n")
        } else {
            draft.clone()
        };
        let mut bytes = Vec::with_capacity(encoded.len() + 3);
        if self.bom {
            bytes.extend_from_slice(b"\xef\xbb\xbf");
        }
        bytes.extend_from_slice(encoded.as_bytes());
        let mut temporary = tempfile::NamedTempFile::new_in(parent)
            .map_err(|err| format!("Cannot create a temporary file for saving: {err}"))?;
        temporary
            .write_all(&bytes)
            .and_then(|()| temporary.as_file().set_permissions(metadata.permissions()))
            .and_then(|()| temporary.as_file().sync_all())
            .map_err(|err| format!("Cannot write the file: {err}"))?;
        self.check_disk()?;
        temporary
            .persist(&self.path)
            .map_err(|err| format!("Cannot replace {}: {err}", self.path.display()))?;
        self.text = self.draft.take().expect("The draft was checked above");
        self.disk_bytes = bytes;
        self.view = view;
        Ok(())
    }

    fn check_disk(&self) -> Result<(), String> {
        let current = fs::read(&self.path)
            .map_err(|err| format!("Cannot check the file before saving: {err}"))?;
        if current != self.disk_bytes {
            return Err(
                "The file changed on disk. It was not overwritten. Copy your edits before \
                 discarding, then click Edit again to load the latest version."
                    .into(),
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_updates_disk_and_returns_to_preview_in_the_same_document() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("test.md");
        fs::write(&path, "# Before\n").unwrap();
        let mut document = Document::open(&path).unwrap();
        document.begin_edit().unwrap();
        document.draft = Some("# After\n".into());
        assert!(document.is_dirty());
        document.save().unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "# After\n");
        assert_eq!(document.text, "# After\n");
        assert!(document.draft.is_none());
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn discard_never_writes_to_disk() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("test.md");
        fs::write(&path, "Original").unwrap();
        let mut document = Document::open(&path).unwrap();
        document.begin_edit().unwrap();
        document.draft = Some("Unsaved".into());
        document.discard();
        assert_eq!(document.text, "Original");
        assert!(document.draft.is_none());
        assert_eq!(fs::read_to_string(path).unwrap(), "Original");
    }

    #[test]
    fn saving_preserves_utf8_bom_and_windows_line_endings() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("test.md");
        fs::write(&path, b"\xef\xbb\xbf# Before\r\nLine\r\n").unwrap();
        let mut document = Document::open(&path).unwrap();
        document.begin_edit().unwrap();
        document.draft.as_mut().unwrap().push_str("New\n");
        document.save().unwrap();
        assert_eq!(
            fs::read(&path).unwrap(),
            b"\xef\xbb\xbf# Before\r\nLine\r\nNew\r\n"
        );
    }

    #[test]
    fn external_changes_block_save_and_keep_the_draft() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("test.md");
        fs::write(&path, "Original").unwrap();
        let mut document = Document::open(&path).unwrap();
        document.begin_edit().unwrap();
        document.draft = Some("My edits".into());
        fs::write(&path, "External edits").unwrap();
        assert!(document.save().unwrap_err().contains("changed on disk"));
        assert_eq!(document.draft.as_deref(), Some("My edits"));
        assert_eq!(fs::read_to_string(path).unwrap(), "External edits");
    }

    #[test]
    fn entering_edit_reloads_external_changes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("test.md");
        fs::write(&path, "Original").unwrap();
        let mut document = Document::open(&path).unwrap();
        fs::write(&path, "Updated").unwrap();
        document.begin_edit().unwrap();
        assert_eq!(document.draft.as_deref(), Some("Updated"));
    }

    #[test]
    fn text_files_are_editable_but_binary_files_are_not() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("notes.txt");
        fs::write(&path, "Some text").unwrap();
        assert!(!Document::open(&path).unwrap().is_markdown);
        fs::write(&path, b"some\0binary").unwrap();
        assert!(Document::open(&path).is_err());
        fs::write(&path, [0xff, 0xfe]).unwrap();
        assert!(Document::open(&path).is_err());
    }
}
