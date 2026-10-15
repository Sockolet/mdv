use std::{
    fs,
    path::{Path, PathBuf},
};

use eframe::egui;

#[derive(Default)]
pub struct Tree {
    pub root: Option<Node>,
}

pub struct Node {
    pub path: PathBuf,
    pub name: String,
    directory: bool,
    children: Option<Result<Vec<Node>, String>>,
}

impl Tree {
    pub fn set_root(&mut self, path: &Path) -> Result<(), String> {
        let path = dunce::canonicalize(path)
            .map_err(|err| format!("Cannot open folder {}: {err}", path.display()))?;
        if !path.is_dir() {
            return Err(format!("{} is not a folder.", path.display()));
        }
        self.root = Some(Node {
            name: file_name(&path),
            path,
            directory: true,
            children: None,
        });
        Ok(())
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, selected: Option<&Path>) -> Option<PathBuf> {
        let mut clicked = None;
        let Some(root) = &mut self.root else {
            ui.weak("Open a folder to browse its files.");
            return None;
        };
        ui.horizontal(|ui| {
            ui.strong(&root.name)
                .on_hover_text(root.path.to_string_lossy());
            if ui.small_button("Refresh").clicked() {
                root.children = None;
            }
        });
        ui.separator();
        egui::ScrollArea::both().show(ui, |ui| {
            root.show_children(ui, selected, &mut clicked);
        });
        clicked
    }
}

impl Node {
    fn show_children(
        &mut self,
        ui: &mut egui::Ui,
        selected: Option<&Path>,
        clicked: &mut Option<PathBuf>,
    ) {
        let children = self.children.get_or_insert_with(|| read_folder(&self.path));
        match children {
            Ok(children) if children.is_empty() => {
                ui.weak("Empty folder");
            }
            Ok(children) => {
                for child in children {
                    if child.directory {
                        egui::CollapsingHeader::new(&child.name)
                            .id_salt(&child.path)
                            .show(ui, |ui| child.show_children(ui, selected, clicked));
                    } else {
                        ui.push_id(&child.path, |ui| {
                            if ui
                                .selectable_label(selected == Some(&child.path), &child.name)
                                .on_hover_text(child.path.to_string_lossy())
                                .clicked()
                            {
                                *clicked = Some(child.path.clone());
                            }
                        });
                    }
                }
            }
            Err(error) => {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
        }
    }
}

fn read_folder(path: &Path) -> Result<Vec<Node>, String> {
    let mut nodes = Vec::new();
    for entry in fs::read_dir(path).map_err(|err| format!("Cannot read folder: {err}"))? {
        let entry = entry.map_err(|err| format!("Cannot read folder entry: {err}"))?;
        let kind = entry
            .file_type()
            .map_err(|err| format!("Cannot inspect {}: {err}", entry.path().display()))?;
        nodes.push(Node {
            name: entry.file_name().to_string_lossy().into_owned(),
            path: entry.path(),
            // Directory symlinks are opened on click rather than recursively expanded.
            directory: kind.is_dir(),
            children: None,
        });
    }
    nodes.sort_by_cached_key(|node| (!node.directory, node.name.to_lowercase(), node.name.clone()));
    Ok(nodes)
}

pub fn file_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_are_sorted_first_and_only_loaded_on_expansion() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("b.md"), "B").unwrap();
        fs::write(directory.path().join("A.md"), "A").unwrap();
        fs::create_dir(directory.path().join("z-folder")).unwrap();
        let entries = read_folder(directory.path()).unwrap();
        let names: Vec<_> = entries.iter().map(|node| node.name.as_str()).collect();
        assert_eq!(names, ["z-folder", "A.md", "b.md"]);
        assert!(entries.iter().all(|entry| entry.children.is_none()));
    }
}
