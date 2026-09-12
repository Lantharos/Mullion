use std::{
    sync::{Arc, mpsc},
    thread,
    time::Duration,
};

use super::{RendererError, health::DeviceHealth};

pub(super) struct SubmissionPoller(mpsc::SyncSender<()>);

impl SubmissionPoller {
    pub(super) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        health: Arc<DeviceHealth>,
    ) -> Result<Self, RendererError> {
        let (sender, receiver) = mpsc::sync_channel(1);
        let device = device.clone();
        let queue = queue.clone();
        thread::Builder::new()
            .name("sabine-gpu-retire".into())
            .spawn(move || {
                while receiver.recv().is_ok() {
                    while let Err(error) = device.poll(wgpu::PollType::Wait {
                        submission_index: None,
                        timeout: Some(Duration::from_secs(5)),
                    }) {
                        health.fail(format!("GPU completion failed: {error}"));
                    }
                }
                drop(queue);
            })
            .map_err(|error| RendererError::Device(error.to_string()))?;
        Ok(Self(sender))
    }

    pub(super) fn notify(&self) {
        let _ = self.0.try_send(());
    }
}
