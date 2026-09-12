use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use sabine_platform::DeepLinkRegistration;

#[derive(Clone, Debug, Default)]
pub(crate) struct OpenUrls(Arc<Mutex<Vec<String>>>);

impl OpenUrls {
    pub(crate) fn receive(&self, urls: impl IntoIterator<Item = String>) -> bool {
        let mut pending = self.0.lock().unwrap_or_else(|error| error.into_inner());
        let mut added = false;
        for url in urls {
            if pending.len() == 128 || url.len() > 32768 {
                let _ = sabine_runtime::record_diagnostic(
                    "desktop",
                    "URL opening queue is full or the URL exceeds 32 KiB",
                );
                continue;
            }
            pending.push(url);
            added = true;
        }
        added
    }

    pub(crate) fn receive_arguments(
        &self,
        arguments: &[String],
        directory: Option<&Path>,
        registrations: &[DeepLinkRegistration],
    ) -> bool {
        self.receive(arguments.iter().filter_map(|argument| {
            if let Ok(url) = url::Url::parse(argument)
                && (url.scheme() == "file"
                    || registrations.iter().any(|registration| {
                        registration
                            .schemes
                            .iter()
                            .any(|scheme| scheme.eq_ignore_ascii_case(url.scheme()))
                    }))
            {
                return Some(url.into());
            }
            let path = directory?.join(argument);
            path.is_file()
                .then(|| url::Url::from_file_path(path).ok().map(Into::into))
                .flatten()
        }))
    }

    pub(crate) fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(|error| error.into_inner()))
    }
}
