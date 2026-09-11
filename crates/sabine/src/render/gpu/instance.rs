use std::sync::OnceLock;

pub(super) fn shared() -> wgpu::Instance {
    static INSTANCE: OnceLock<wgpu::Instance> = OnceLock::new();
    INSTANCE
        .get_or_init(|| {
            let descriptor = wgpu::InstanceDescriptor {
                backends: preferred_backends(),
                ..wgpu::InstanceDescriptor::new_without_display_handle()
            };
            #[cfg(target_os = "windows")]
            let descriptor = {
                let mut descriptor = descriptor;
                descriptor.backend_options.dx12.presentation_system =
                    wgpu::Dx12SwapchainKind::DxgiFromVisual;
                descriptor
            };
            wgpu::Instance::new(descriptor)
        })
        .clone()
}

fn preferred_backends() -> wgpu::Backends {
    #[cfg(target_os = "windows")]
    {
        wgpu::Backends::DX12
    }
    #[cfg(target_os = "linux")]
    {
        wgpu::Backends::VULKAN | wgpu::Backends::GL
    }
    #[cfg(target_os = "macos")]
    {
        wgpu::Backends::METAL
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    {
        wgpu::Backends::all()
    }
}
