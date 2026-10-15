use std::{
    num::NonZeroU32,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use eframe::egui;
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window, WindowId},
};

use crate::{app::Mdv, raster::Painter, storage::Storage};

const FRAME_INTERVAL: Duration = Duration::from_nanos(1_000_000_000 / 60);

pub fn run(startup: Option<PathBuf>) -> Result<(), String> {
    let mut builder = EventLoop::<Instant>::with_user_event();
    #[cfg(all(test, windows))]
    {
        use winit::platform::windows::EventLoopBuilderExtWindows;
        builder.with_any_thread(true);
    }
    #[cfg(all(test, target_os = "linux"))]
    {
        use winit::platform::x11::EventLoopBuilderExtX11;
        builder.with_any_thread(true).with_x11();
    }
    let event_loop = builder
        .build()
        .map_err(|error| format!("Cannot create the event loop: {error}"))?;
    let context = egui::Context::default();
    let proxy = event_loop.create_proxy();
    context.set_request_repaint_callback(move |request| {
        // EventLoopClosed is expected if a background image finishes during shutdown.
        let _ = proxy.send_event(Instant::now() + request.delay);
    });
    let mut application = Application {
        startup,
        context,
        state: None,
        repaint_at: None,
        painted_at: None,
        close_requested: false,
        error: None,
    };
    event_loop
        .run_app(&mut application)
        .map_err(|error| format!("The native event loop failed: {error}"))?;
    application.error.map_or(Ok(()), Err)
}

struct Application {
    startup: Option<PathBuf>,
    context: egui::Context,
    state: Option<State>,
    repaint_at: Option<Instant>,
    painted_at: Option<Instant>,
    close_requested: bool,
    error: Option<String>,
}

struct State {
    // Keep clipboard/input and the surface alive until before the native window
    // is dropped; Wayland clipboard teardown depends on this ordering.
    context: egui::Context,
    surface: softbuffer::Surface<Arc<Window>, Arc<Window>>,
    input: egui_winit::State,
    window: Arc<Window>,
    _display: softbuffer::Context<Arc<Window>>,
    app: Mdv,
    painter: Painter,
    storage: Storage,
    pending: Vec<egui::Event>,
    saved_at: Instant,
    #[cfg(test)]
    profile: Option<Profile>,
}

impl Application {
    fn create(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        #[cfg(test)]
        let profile = Profile::from_env()?;
        #[cfg(test)]
        let override_path = profile
            .as_ref()
            .map(|profile| profile.output.join("state.ron"));
        #[cfg(not(test))]
        let override_path = None;
        let storage = Storage::open(override_path)?;
        let viewport = egui::ViewportBuilder::default()
            .with_title(concat!("MDV ", env!("CARGO_PKG_VERSION"), " (CPU)"))
            .with_app_id("MDV")
            .with_inner_size(
                storage
                    .placement
                    .inner_size_points
                    .as_ref()
                    .map_or(egui::vec2(1100.0, 760.0), |point| {
                        egui::vec2(point.x, point.y)
                    }),
            )
            .with_min_inner_size([600.0, 400.0])
            .with_maximized(storage.placement.maximized);
        #[cfg(test)]
        let viewport = if profile.is_some() {
            viewport.with_title("MDV local scroll diagnosis")
        } else {
            viewport
        };
        let window = Arc::new(
            egui_winit::create_window(&self.context, event_loop, &viewport)
                .map_err(|error| format!("Cannot create the native window: {error}"))?,
        );
        let display = softbuffer::Context::new(window.clone())
            .map_err(|error| format!("Cannot create the CPU display: {error}"))?;
        let surface = softbuffer::Surface::new(&display, window.clone())
            .map_err(|error| format!("Cannot create the CPU drawing surface: {error}"))?;
        let input = egui_winit::State::new(
            self.context.clone(),
            egui::ViewportId::ROOT,
            window.as_ref(),
            Some(window.scale_factor() as f32),
            None,
            None,
        );
        let app = Mdv::from_storage(&self.context, Some(&storage), self.startup.take());
        self.state = Some(State {
            context: self.context.clone(),
            surface,
            input,
            window,
            _display: display,
            app,
            painter: Painter::default(),
            storage,
            pending: Vec::new(),
            saved_at: Instant::now(),
            #[cfg(test)]
            profile,
        });
        self.state.as_ref().unwrap().window.request_redraw();
        Ok(())
    }

    fn draw(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        let state = self
            .state
            .as_mut()
            .ok_or("The native window is not initialized.")?;
        let size = state.window.inner_size();
        let (Some(width), Some(height)) =
            (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
        else {
            return Ok(());
        };
        let frame_start = Instant::now();
        egui_winit::update_viewport_info(
            state
                .input
                .egui_input_mut()
                .viewports
                .entry(egui::ViewportId::ROOT)
                .or_default(),
            &state.context,
            &state.window,
            false,
        );
        let mut input = state.input.take_egui_input(&state.window);
        input.events.append(&mut state.pending);
        #[cfg(test)]
        if let Some(profile) = &mut state.profile {
            profile.wheel_events += input
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
        if self.close_requested {
            input
                .viewports
                .entry(egui::ViewportId::ROOT)
                .or_default()
                .events
                .push(egui::ViewportEvent::Close);
        }
        let mut output = state.context.run_ui(input, |ui| {
            state.app.check_close(ui.ctx());
            state.app.show(ui);
        });
        state.input.handle_platform_output_with_event_loop(
            &state.window,
            event_loop,
            std::mem::take(&mut output.platform_output),
        );
        let commands = output
            .viewport_output
            .remove(&egui::ViewportId::ROOT)
            .map_or_else(Vec::new, |viewport| viewport.commands);
        let mut actions = Vec::new();
        for command in &commands {
            match command {
                egui::ViewportCommand::Close => self.close_requested = true,
                egui::ViewportCommand::CancelClose => self.close_requested = false,
                _ => {}
            }
        }
        egui_winit::process_viewport_commands(
            &state.context,
            &mut egui::ViewportInfo::default(),
            commands,
            &state.window,
            &mut actions,
        );
        let primitives = state
            .context
            .tessellate(std::mem::take(&mut output.shapes), output.pixels_per_point);
        state
            .surface
            .resize(width, height)
            .map_err(|error| format!("Cannot resize the CPU drawing surface: {error}"))?;
        let mut buffer = state
            .surface
            .buffer_mut()
            .map_err(|error| format!("Cannot obtain the native drawing buffer: {error}"))?;
        let painted = state.painter.paint(
            &mut buffer,
            [width.get() as usize, height.get() as usize],
            &primitives,
            &output.textures_delta,
            output.pixels_per_point,
        );
        output.textures_delta.clear();
        painted?;
        for action in actions {
            match action {
                egui_winit::ActionRequested::Copy => state.pending.push(egui::Event::Copy),
                egui_winit::ActionRequested::Cut => state.pending.push(egui::Event::Cut),
                egui_winit::ActionRequested::Paste => {
                    if let Some(text) = state.input.clipboard_text() {
                        state.pending.push(egui::Event::Paste(text));
                    }
                }
                egui_winit::ActionRequested::Screenshot(user_data) => {
                    let pixels: Vec<_> = buffer
                        .iter()
                        .map(|pixel| {
                            let [b, g, r, _] = pixel.to_le_bytes();
                            egui::Color32::from_rgb(r, g, b)
                        })
                        .collect();
                    state.pending.push(egui::Event::Screenshot {
                        viewport_id: egui::ViewportId::ROOT,
                        user_data,
                        image: Arc::new(egui::ColorImage::new(
                            [width.get() as usize, height.get() as usize],
                            pixels,
                        )),
                    });
                }
            }
        }
        #[cfg(test)]
        if let Some(profile) = &mut state.profile {
            profile.capture(&buffer, size.width, size.height)?;
        }
        state.window.pre_present_notify();
        buffer
            .present()
            .map_err(|error| format!("Cannot present the native frame: {error}"))?;
        self.painted_at = Some(frame_start);
        #[cfg(test)]
        if let Some(profile) = &mut state.profile {
            profile.record(frame_start, state.app.scroll_metrics(), &state.context);
            if profile.start.elapsed() > profile.duration {
                self.close_requested = true;
            } else {
                state.context.request_repaint_after(Duration::from_secs(1));
            }
        }
        if !state.pending.is_empty() {
            self.repaint_at = Some(frame_start + FRAME_INTERVAL);
        }
        if state.saved_at.elapsed() > Duration::from_secs(30) || self.close_requested {
            eframe::App::save(&mut state.app, &mut state.storage);
            state.storage.persist(&state.window)?;
            state.saved_at = Instant::now();
        }
        if self.close_requested {
            event_loop.exit();
        } else {
            state.context.request_repaint_after(Duration::from_secs(30));
        }
        Ok(())
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: String) {
        self.error = Some(error);
        event_loop.exit();
    }

    fn schedule(&mut self, when: Instant) {
        let when = self
            .painted_at
            .map_or(when, |last| when.max(last + FRAME_INTERVAL));
        self.repaint_at = Some(self.repaint_at.map_or(when, |current| current.min(when)));
    }
}

impl ApplicationHandler<Instant> for Application {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_none()
            && let Err(error) = self.create(event_loop)
        {
            self.fail(event_loop, error);
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, when: Instant) {
        self.schedule(when);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(when) = self.repaint_at {
            if when <= Instant::now() {
                if let Some(state) = &self.state {
                    state.window.request_redraw();
                }
                self.repaint_at = None;
                event_loop.set_control_flow(ControlFlow::Wait);
            } else {
                event_loop.set_control_flow(ControlFlow::WaitUntil(when));
            }
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(state) = &mut self.state else { return };
        if window_id != state.window.id() {
            return;
        }
        match event {
            WindowEvent::RedrawRequested => {
                if self
                    .painted_at
                    .is_some_and(|last| last.elapsed() < FRAME_INTERVAL)
                {
                    self.schedule(Instant::now());
                    return;
                }
                if let Err(error) = self.draw(event_loop) {
                    self.fail(event_loop, error);
                }
            }
            WindowEvent::CloseRequested => {
                self.close_requested = true;
                if state.window.is_minimized() == Some(true) {
                    state.window.set_minimized(false);
                }
                self.schedule(Instant::now());
            }
            _ => {
                let response = state.input.on_window_event(&state.window, &event);
                if response.repaint {
                    self.schedule(Instant::now());
                }
            }
        }
    }
}

#[cfg(test)]
struct Profile {
    output: PathBuf,
    start: Instant,
    last: Instant,
    rows: String,
    captures: usize,
    wheel_events: usize,
    duration: Duration,
}

#[cfg(test)]
impl Profile {
    fn from_env() -> Result<Option<Self>, String> {
        let Some(output) = std::env::var_os("MDV_NATIVE_PROFILE_DIR") else {
            return Ok(None);
        };
        let output = PathBuf::from(output);
        let seconds = match std::env::var("MDV_NATIVE_PROFILE_SECONDS") {
            Ok(value) => value,
            Err(std::env::VarError::NotPresent) => "12".into(),
            Err(error) => return Err(format!("Cannot read native profiling duration: {error}")),
        }
        .parse::<u64>()
        .map_err(|error| format!("Invalid native profiling duration: {error}"))?;
        if seconds == 0 || seconds > 120 {
            return Err("Native profiling duration must be between 1 and 120 seconds.".into());
        }
        std::fs::create_dir_all(&output).map_err(|error| error.to_string())?;
        std::fs::write(
            output.join("adapter.txt"),
            "CPU software renderer; native bitmap presentation",
        )
        .map_err(|error| error.to_string())?;
        Ok(Some(Self {
            output,
            start: Instant::now(),
            last: Instant::now(),
            rows: "time,interval_ms,cpu_ms,offset,height,resized,wheel_events,pointer_x,pointer_y,remaining_wheel,focused\n".into(),
            captures: 0,
            wheel_events: 0,
            duration: Duration::from_secs(seconds),
        }))
    }

    fn record(&mut self, start: Instant, metrics: (f32, f32, usize), context: &egui::Context) {
        use std::fmt::Write;
        let elapsed = self.start.elapsed().as_secs_f64();
        let interval = start.duration_since(self.last).as_secs_f64() * 1000.0;
        self.last = start;
        let (pointer, wheel, focused) = context.input(|input| {
            (
                input.pointer.hover_pos().unwrap_or(egui::pos2(-1.0, -1.0)),
                input.smooth_scroll_delta().y,
                input.focused,
            )
        });
        writeln!(
            self.rows,
            "{elapsed:.6},{interval:.3},{:.3},{:.2},{:.2},{},{},{:.1},{:.1},{:.1},{}",
            start.elapsed().as_secs_f64() * 1000.0,
            metrics.0,
            metrics.1,
            metrics.2,
            self.wheel_events,
            pointer.x,
            pointer.y,
            wheel,
            focused,
        )
        .unwrap();
    }

    fn capture(&mut self, buffer: &[u32], width: u32, height: u32) -> Result<(), String> {
        if self.captures >= 3
            || self.start.elapsed().as_secs_f64() < 2.0 + self.captures as f64 * 3.0
        {
            return Ok(());
        }
        self.captures += 1;
        let pixels: Vec<_> = buffer
            .iter()
            .flat_map(|pixel| {
                let [b, g, r, _] = pixel.to_le_bytes();
                [r, g, b, 255]
            })
            .collect();
        image::save_buffer(
            self.output.join(format!("frame-{}.png", self.captures)),
            &pixels,
            width,
            height,
            image::ColorType::Rgba8,
        )
        .map_err(|error| format!("Cannot capture the native diagnostic frame: {error}"))
    }
}

#[cfg(test)]
impl Drop for Profile {
    fn drop(&mut self) {
        if let Err(error) = std::fs::write(self.output.join("frames.csv"), &self.rows) {
            eprintln!("Cannot save native diagnostics: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repaint_requests_are_capped_without_postponing_an_earlier_deadline() {
        let now = Instant::now();
        let mut app = Application {
            startup: None,
            context: egui::Context::default(),
            state: None,
            repaint_at: None,
            painted_at: Some(now),
            close_requested: false,
            error: None,
        };
        app.schedule(now);
        assert_eq!(app.repaint_at, Some(now + FRAME_INTERVAL));
        app.schedule(now + Duration::from_secs(30));
        assert_eq!(app.repaint_at, Some(now + FRAME_INTERVAL));
        app.repaint_at = None;
        app.schedule(now + Duration::from_secs(30));
        app.schedule(now + Duration::from_millis(5));
        assert_eq!(app.repaint_at, Some(now + FRAME_INTERVAL));
    }
}
