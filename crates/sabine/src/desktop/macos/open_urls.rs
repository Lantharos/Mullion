use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained,
    runtime::ProtocolObject,
};
use objc2_app_kit::{NSApplication, NSApplicationDelegate};
use objc2_foundation::{NSArray, NSObject, NSObjectProtocol, NSURL};
use sabine_platform::PlatformEvent;

use super::EventQueue;

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "SabineApplicationDelegate"]
    #[ivars = EventQueue]
    struct ApplicationDelegate;

    unsafe impl NSObjectProtocol for ApplicationDelegate {}

    unsafe impl NSApplicationDelegate for ApplicationDelegate {
        #[unsafe(method(application:openURLs:))]
        #[allow(non_snake_case)]
        fn application_openURLs(&self, _application: &NSApplication, urls: &NSArray<NSURL>) {
            let urls = urls
                .iter()
                .filter_map(|url| url.absoluteString().map(|url| url.to_string()))
                .collect::<Vec<_>>();
            if !urls.is_empty() {
                let _ = self.ivars().send(PlatformEvent::OpenUrls(urls));
            }
        }
    }
);

pub(super) struct OpenUrlEvents {
    application: Retained<NSApplication>,
    _delegate: Retained<ApplicationDelegate>,
}

impl OpenUrlEvents {
    pub(super) fn install(events: EventQueue) -> Result<Self, String> {
        let main_thread =
            MainThreadMarker::new().ok_or("macOS URL events require the main thread")?;
        let application = NSApplication::sharedApplication(main_thread);
        if application.delegate().is_some() {
            return Err("macOS application already has an application delegate".into());
        }
        let delegate: Retained<ApplicationDelegate> = unsafe {
            msg_send![
                super(ApplicationDelegate::alloc(main_thread).set_ivars(events)),
                init
            ]
        };
        application.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        Ok(Self {
            application,
            _delegate: delegate,
        })
    }
}

impl Drop for OpenUrlEvents {
    fn drop(&mut self) {
        self.application.setDelegate(None);
    }
}
