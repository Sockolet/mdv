#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod document;
mod links;
mod markdown;
mod preview;
mod settings;
mod tree;

use std::path::PathBuf;

fn main() -> eframe::Result {
    let mut arguments = std::env::args_os().skip(1);
    let path = arguments.next().map(PathBuf::from);
    if arguments.next().is_some() {
        rfd::MessageDialog::new()
            .set_title("MDV")
            .set_description("Usage: mdv [file or folder]")
            .set_level(rfd::MessageLevel::Error)
            .show();
        return Ok(());
    }

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("MDV")
            .with_inner_size([1100.0, 760.0])
            .with_min_inner_size([600.0, 400.0])
            .with_icon(eframe::egui::IconData::default()),
        #[cfg(windows)]
        renderer: eframe::Renderer::Wgpu,
        #[cfg(not(windows))]
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "MDV",
        options,
        Box::new(move |cc| Ok(Box::new(app::Mdv::new(cc, path)))),
    )
}
