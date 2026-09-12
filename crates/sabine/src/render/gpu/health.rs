use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

pub(super) struct DeviceHealth {
    failed: AtomicBool,
    message: Mutex<Option<String>>,
    wake: Box<dyn Fn() + Send + Sync>,
}

impl DeviceHealth {
    pub(super) fn watch(
        device: &wgpu::Device,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Arc<Self> {
        let health = Arc::new(Self {
            failed: AtomicBool::new(false),
            message: Mutex::new(None),
            wake: Box::new(wake),
        });
        let observed = health.clone();
        device.set_device_lost_callback(move |reason, message| {
            observed.fail(format!("{reason:?}: {message}"));
        });
        health
    }

    pub(super) fn fail(&self, message: String) {
        let mut current = self.message.lock().unwrap();
        if current.is_some() {
            return;
        }
        *current = Some(message);
        self.failed.store(true, Ordering::Release);
        drop(current);
        (self.wake)();
    }

    pub(super) fn failure(&self) -> Option<String> {
        if !self.failed.load(Ordering::Acquire) {
            return None;
        }
        self.message.lock().unwrap().clone()
    }
}
