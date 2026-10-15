#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod document;
mod links;
mod markdown;
mod preview;
mod raster;
mod settings;
mod software;
mod storage;
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

    let software = match std::env::var("MDV_RENDERER") {
        Ok(value) if value == "software" => true,
        Ok(value) if value != "auto" => {
            return Err(native_error(format!(
                "Unsupported MDV_RENDERER value: {value}"
            )));
        }
        Ok(_) | Err(std::env::VarError::NotPresent) => needs_software(),
        Err(error) => {
            return Err(native_error(format!(
                "Cannot read renderer settings: {error}"
            )));
        }
    };
    if software {
        return software::run(path).map_err(native_error);
    }

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title(concat!("MDV ", env!("CARGO_PKG_VERSION")))
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

fn native_error(message: String) -> eframe::Error {
    rfd::MessageDialog::new()
        .set_title("MDV")
        .set_description(&message)
        .set_level(rfd::MessageLevel::Error)
        .show();
    eframe::Error::AppCreation(std::io::Error::other(message).into())
}

#[cfg(windows)]
fn needs_software() -> bool {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::DX12,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    !pollster::block_on(instance.enumerate_adapters(wgpu::Backends::DX12))
        .iter()
        .any(|adapter| adapter.get_info().device_type != wgpu::DeviceType::Cpu)
}

#[cfg(not(windows))]
fn needs_software() -> bool {
    false
}
