use softbuffer::{Context, Surface};
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy},
    window::{Window, WindowAttributes, WindowId},
};

mod confirm;
pub(super) use confirm::{confirm_update, show_notice};

const WIDTH: u32 = 480;
const HEIGHT: u32 = 148;
const BG: u32 = 0xFF_16_16_18;
const TRACK: u32 = 0xFF_2A_2A_2E;
const FILL: u32 = 0xFF_E8_E8_EA;
const TEXT: u32 = 0xFF_F4_F4_F5;
const MUTED: u32 = 0xFF_A1_A1_AA;
const MIN_PROGRESS_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Default)]
pub(super) struct ProgressState {
    pub(super) message: String,
    pub(super) fraction: Option<f32>,
    pub(super) done: Option<Result<(), String>>,
    pub(super) dirty: bool,
}

pub(super) fn run_progress_window(
    title: &str,
    work: impl FnOnce(Arc<Mutex<ProgressState>>, EventLoopProxy) + Send + 'static,
) -> Result<(), String> {
    let event_loop = EventLoop::new().map_err(|error| error.to_string())?;
    let proxy = event_loop.create_proxy();
    let state = Arc::new(Mutex::new(ProgressState {
        message: title.to_string(),
        dirty: true,
        ..ProgressState::default()
    }));
    let worker_state = Arc::clone(&state);
    let worker_proxy = proxy.clone();
    thread::spawn(move || work(worker_state, worker_proxy));

    let app = ProgressApp {
        state: Arc::clone(&state),
        window: None,
        context: None,
        surface: None,
        title: title.to_string(),
    };
    event_loop.run_app(app).map_err(|error| error.to_string())?;

    match state.lock() {
        Ok(guard) => guard
            .done
            .clone()
            .unwrap_or_else(|| Err("Sabine setup did not complete".to_string())),
        Err(_) => Err("Sabine setup did not complete".to_string()),
    }
}

static LAST_PROGRESS_MS: AtomicU64 = AtomicU64::new(0);

pub(super) fn set_progress(
    state: &Mutex<ProgressState>,
    proxy: &EventLoopProxy,
    message: impl Into<String>,
    fraction: Option<f32>,
) {
    let message = message.into();
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0);
    let last = LAST_PROGRESS_MS.load(Ordering::Relaxed);
    let force = fraction.is_some_and(|value| value >= 0.999);
    if let Ok(mut guard) = state.lock() {
        guard.message = message;
        guard.fraction = fraction;
        guard.dirty = true;
    }
    if !force && now_ms.saturating_sub(last) < MIN_PROGRESS_INTERVAL.as_millis() as u64 {
        return;
    }
    LAST_PROGRESS_MS.store(now_ms, Ordering::Relaxed);
    proxy.wake_up();
}

pub(super) fn finish(
    state: &Mutex<ProgressState>,
    proxy: &EventLoopProxy,
    result: Result<(), String>,
) {
    if let Ok(mut guard) = state.lock() {
        guard.done = Some(result);
        guard.dirty = true;
    }
    proxy.wake_up();
}

struct ProgressApp {
    state: Arc<Mutex<ProgressState>>,
    window: Option<Arc<dyn Window>>,
    context: Option<Context<Arc<dyn Window>>>,
    surface: Option<Surface<Arc<dyn Window>, Arc<dyn Window>>>,
    title: String,
}

impl ApplicationHandler for ProgressApp {
    fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::Wait);
        self.resumed(event_loop);
    }

    fn resumed(&mut self, event_loop: &dyn ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::Wait);
        if self.window.is_some() {
            return;
        }
        let attributes = WindowAttributes::default()
            .with_title(&self.title)
            .with_surface_size(LogicalSize::new(f64::from(WIDTH), f64::from(HEIGHT)))
            .with_resizable(false)
            .with_decorations(true);
        let window: Arc<dyn Window> = match event_loop.create_window(attributes) {
            Ok(window) => Arc::from(window),
            Err(error) => {
                eprintln!("failed to open Sabine setup window: {error}");
                event_loop.exit();
                return;
            }
        };
        let context = match Context::new(window.clone()) {
            Ok(context) => context,
            Err(error) => {
                eprintln!("failed to create Sabine setup context: {error}");
                event_loop.exit();
                return;
            }
        };
        let surface = match Surface::new(&context, window.clone()) {
            Ok(surface) => surface,
            Err(error) => {
                eprintln!("failed to create Sabine setup surface: {error}");
                event_loop.exit();
                return;
            }
        };
        self.context = Some(context);
        self.surface = Some(surface);
        self.window = Some(window);
        self.paint(true);
    }

    fn window_event(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        _id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::KeyboardInput { event, .. }
                if event.state == winit::event::ElementState::Pressed
                    && matches!(
                        event.logical_key,
                        winit::keyboard::Key::Named(
                            winit::keyboard::NamedKey::Escape | winit::keyboard::NamedKey::Enter
                        )
                    )
                    && self.state.lock().is_ok_and(|state| state.done.is_some()) =>
            {
                event_loop.exit()
            }
            WindowEvent::RedrawRequested => self.paint(false),
            WindowEvent::SurfaceResized(_) => self.paint(true),
            _ => {}
        }
    }

    fn proxy_wake_up(&mut self, event_loop: &dyn ActiveEventLoop) {
        let outcome = self.state.lock().ok().and_then(|state| state.done.clone());
        match outcome {
            Some(Ok(())) => {
                event_loop.exit();
                return;
            }
            Some(Err(error)) => {
                if let Some(window) = &self.window {
                    window.set_title("Sabine setup failed");
                    let _ = window.request_surface_size(LogicalSize::new(640.0, 360.0).into());
                }
                if let Ok(mut state) = self.state.lock() {
                    state.message = format!(
                        "{error}\n\nClose this window to finish. Details: {}",
                        sabine_runtime::diagnostic_path("setup").display()
                    );
                    state.fraction = None;
                    state.dirty = true;
                }
                self.paint(true);
                return;
            }
            None => {}
        }
        self.paint(false);
    }
}

impl ProgressApp {
    fn paint(&mut self, force: bool) {
        let Some(window) = &self.window else {
            return;
        };
        let Some(surface) = self.surface.as_mut() else {
            return;
        };
        let (message, fraction, dirty) = match self.state.lock() {
            Ok(mut guard) => {
                let dirty = guard.dirty;
                guard.dirty = false;
                (guard.message.clone(), guard.fraction, dirty)
            }
            Err(_) => return,
        };
        if !force && !dirty {
            return;
        }

        let status = if message.is_empty() {
            self.title.as_str()
        } else {
            message.as_str()
        };

        let size = window.surface_size();
        let width = size.width.max(1);
        let height = size.height.max(1);
        let Ok(width_nz) = NonZeroU32::try_from(width) else {
            return;
        };
        let Ok(height_nz) = NonZeroU32::try_from(height) else {
            return;
        };
        if surface.resize(width_nz, height_nz).is_err() {
            return;
        }
        let Ok(mut buffer) = surface.buffer_mut() else {
            return;
        };
        buffer.fill(BG);

        let scale = (width as f32 / WIDTH as f32).max(1.0);
        let pad = (20.0 * scale) as i32;
        let bar_y = (height as i32 * 2) / 3;
        let failed = self
            .state
            .lock()
            .is_ok_and(|state| matches!(state.done, Some(Err(_))));
        let bar_h = (10.0 * scale).round().max(6.0) as i32;
        let bar_w = width as i32 - pad * 2;
        if !failed {
            fill_rect(
                &mut buffer,
                (width, height),
                (pad, bar_y, bar_w, bar_h),
                TRACK,
            );
        }
        let filled = (fraction.unwrap_or(0.0).clamp(0.0, 1.0) * bar_w as f32).round() as i32;
        if filled > 0 {
            fill_rect(
                &mut buffer,
                (width, height),
                (pad, bar_y, filled, bar_h),
                FILL,
            );
        }

        draw_text(
            &mut buffer,
            (width, height),
            (pad, pad + (8.0 * scale) as i32),
            status,
            TEXT,
            scale,
        );
        if let Some(value) = fraction {
            draw_text(
                &mut buffer,
                (width, height),
                (pad, bar_y - (22.0 * scale) as i32),
                &format!("{}%", (value * 100.0).round() as u8),
                MUTED,
                scale,
            );
        }

        let _ = buffer.present();
    }
}

fn fill_rect(buffer: &mut [u32], surface: (u32, u32), rect: (i32, i32, i32, i32), color: u32) {
    let (width, height) = surface;
    let (x, y, w, h) = rect;
    let x0 = x.max(0) as u32;
    let y0 = y.max(0) as u32;
    let x1 = ((x + w).max(0) as u32).min(width);
    let y1 = ((y + h).max(0) as u32).min(height);
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let span = (x1 - x0) as usize;
    for py in y0..y1 {
        let start = (py * width + x0) as usize;
        buffer[start..start + span].fill(color);
    }
}

fn draw_text(
    buffer: &mut [u32],
    surface: (u32, u32),
    position: (i32, i32),
    text: &str,
    color: u32,
    scale: f32,
) {
    thread_local! {
        static TEXT_RENDERER: std::cell::RefCell<crate::render::raster_text::RasterText> =
            std::cell::RefCell::new(crate::render::raster_text::RasterText::new());
    }
    TEXT_RENDERER.with(|renderer| {
        renderer.borrow_mut().draw_wrapped(
            bytemuck::cast_slice_mut(buffer),
            surface,
            (
                position.0,
                position.1,
                surface.0.saturating_sub(position.0.max(0) as u32 + 20),
                surface.1.saturating_sub(position.1.max(0) as u32),
            ),
            text,
            13.0 * scale,
            [
                ((color >> 16) & 255) as u8,
                ((color >> 8) & 255) as u8,
                (color & 255) as u8,
                255,
            ],
        );
    });
}
