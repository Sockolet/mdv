use std::{collections::BTreeMap, fs, io::Write, path::PathBuf};

use serde::{Deserialize, Serialize};
use winit::window::Window;

pub struct Storage {
    pub placement: Placement,
    path: PathBuf,
    values: BTreeMap<String, String>,
}

impl Storage {
    pub fn open(override_path: Option<PathBuf>) -> Result<Self, String> {
        let path = eframe::storage_dir("MDV")
            .ok_or("Cannot locate the application settings folder.")?
            .join("app.ron");
        let mut storage = Self::read(path)?;
        if let Some(path) = override_path {
            storage.path = path;
        }
        Ok(storage)
    }

    fn read(path: PathBuf) -> Result<Self, String> {
        let values = match fs::read_to_string(&path) {
            Ok(text) => ron::from_str(&text)
                .map_err(|error| format!("Cannot read application settings: {error}"))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(error) => return Err(format!("Cannot read application settings: {error}")),
        };
        let mut storage = Self {
            placement: Placement::default(),
            path,
            values,
        };
        storage.placement = eframe::get_value(&storage, "window").unwrap_or_default();
        Ok(storage)
    }

    pub fn persist(&mut self, window: &Window) -> Result<(), String> {
        let scale = window.scale_factor();
        let size = window.inner_size().to_logical::<f32>(scale);
        self.placement = Placement {
            inner_position_pixels: window.inner_position().ok().map(|position| Point {
                x: position.x,
                y: position.y,
            }),
            outer_position_pixels: window.outer_position().ok().map(|position| Point {
                x: position.x,
                y: position.y,
            }),
            fullscreen: window.fullscreen().is_some(),
            maximized: window.is_maximized(),
            inner_size_points: Some(Point {
                x: size.width,
                y: size.height,
            }),
        };
        let placement = self.placement.clone();
        eframe::set_value(self, "window", &placement);
        self.write()
    }

    fn write(&mut self) -> Result<(), String> {
        let parent = self
            .path
            .parent()
            .ok_or("The settings path has no parent folder.")?;
        fs::create_dir_all(parent)
            .map_err(|error| format!("Cannot create the settings folder: {error}"))?;
        let text = ron::ser::to_string_pretty(&self.values, ron::ser::PrettyConfig::default())
            .map_err(|error| format!("Cannot encode application settings: {error}"))?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)
            .map_err(|error| format!("Cannot save application settings: {error}"))?;
        temporary
            .write_all(text.as_bytes())
            .and_then(|()| temporary.as_file().sync_all())
            .map_err(|error| format!("Cannot write application settings: {error}"))?;
        temporary
            .persist(&self.path)
            .map_err(|error| format!("Cannot replace application settings: {error}"))?;
        Ok(())
    }
}

impl eframe::Storage for Storage {
    fn get_string(&self, key: &str) -> Option<String> {
        self.values.get(key).cloned()
    }

    fn set_string(&mut self, key: &str, value: String) {
        self.values.insert(key.to_owned(), value);
    }

    fn remove_string(&mut self, key: &str) {
        self.values.remove(key);
    }

    fn flush(&mut self) {
        if let Err(error) = self.write() {
            eprintln!("MDV: {error}");
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Point<T> {
    pub x: T,
    pub y: T,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Placement {
    pub inner_position_pixels: Option<Point<i32>>,
    pub outer_position_pixels: Option<Point<i32>>,
    pub fullscreen: bool,
    pub maximized: bool,
    pub inner_size_points: Option<Point<f32>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{Font, Settings};
    use eframe::Storage as _;

    #[test]
    fn appearance_and_legacy_window_settings_survive_atomic_round_trip() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("app.ron");
        let mut storage = Storage::read(path.clone()).unwrap();
        let settings = Settings {
            dark: true,
            font: Font::Monospace,
            size: 23.0,
            folder: None,
        };
        eframe::set_value(&mut storage, "appearance", &settings);
        storage.set_string("unrelated-state", "preserve this".into());
        let placement = Placement {
            maximized: true,
            inner_size_points: Some(Point {
                x: 1200.0,
                y: 800.0,
            }),
            ..Default::default()
        };
        eframe::set_value(&mut storage, "window", &placement);
        storage.write().unwrap();
        let reloaded = Storage::read(path).unwrap();
        let appearance: Settings = eframe::get_value(&reloaded, "appearance").unwrap();
        assert!(appearance.dark);
        assert!(appearance.font == Font::Monospace);
        assert_eq!(appearance.size, 23.0);
        assert!(reloaded.placement.maximized);
        assert_eq!(
            reloaded.placement.inner_size_points.as_ref().unwrap().x,
            1200.0
        );
        assert_eq!(
            reloaded.get_string("unrelated-state").as_deref(),
            Some("preserve this")
        );
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn corrupt_settings_are_reported_instead_of_silently_replaced() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("app.ron");
        fs::write(&path, "not a settings map").unwrap();
        assert!(Storage::read(path.clone()).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "not a settings map");
    }
}
