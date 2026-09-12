use std::time::{Duration, Instant};

use super::native::OsrNativeHost;
use crate::render::GpuRenderer;

impl OsrNativeHost {
    pub(super) fn recover_gpu(&mut self) {
        if self.closing_deadline.is_some() || self.failure.is_some() {
            return;
        }
        let Some(reason) = self.renderer.as_ref().and_then(GpuRenderer::device_loss) else {
            return;
        };
        let now = Instant::now();
        while self
            .gpu_recoveries
            .front()
            .is_some_and(|at| now.duration_since(*at) >= Duration::from_secs(60))
        {
            self.gpu_recoveries.pop_front();
        }
        if self.gpu_recoveries.len() >= 3 {
            self.fail(format!("The graphics device stopped repeatedly. Restart the application. GPU details: {reason}"));
            return;
        }
        self.gpu_recoveries.push_back(now);
        sabine_runtime::report_error("gpu", format!("Recreating the graphics device: {reason}"));
        let Some(window) = self.window.clone() else {
            return;
        };
        #[cfg(windows)]
        let previous_adapter = crate::osr::accel::adapter_luid(self.renderer.as_ref().unwrap());
        self.renderer = None;
        let proxy = self.proxy.clone();
        let renderer = match pollster::block_on(GpuRenderer::new(
            window.clone(),
            self.config.transparent,
            move || proxy.wake_up(),
        )) {
            Ok(renderer) => renderer,
            Err(error) => {
                self.fail(format!("Could not restore GPU rendering: {error}"));
                return;
            }
        };
        #[cfg(windows)]
        if previous_adapter != crate::osr::accel::adapter_luid(&renderer) {
            self.fail("The active graphics adapter is no longer available. Restart the application to connect Chromium to the replacement adapter.".to_string());
            return;
        }
        self.renderer = Some(renderer);
        if let Err(error) = self.upload_cached_textures() {
            self.fail(format!("Could not restore window textures: {error}"));
            return;
        }
        self.send_control("repaint\n");
        window.request_redraw();
    }
}
