use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

#[derive(Default)]
pub(super) struct DeviceHealth {
    failed: AtomicBool,
    message: Mutex<Option<String>>,
}

impl DeviceHealth {
    pub(super) fn watch(device: &wgpu::Device, wake: impl Fn() + Send + 'static) -> Arc<Self> {
        let health = Arc::new(Self::default());
        let observed = health.clone();
        device.set_device_lost_callback(move |reason, message| {
            *observed.message.lock().unwrap() = Some(format!("{reason:?}: {message}"));
            observed.failed.store(true, Ordering::Release);
            wake();
        });
        health
    }

    pub(super) fn failure(&self) -> Option<String> {
        if !self.failed.load(Ordering::Acquire) {
            return None;
        }
        self.message.lock().unwrap().clone()
    }
}
