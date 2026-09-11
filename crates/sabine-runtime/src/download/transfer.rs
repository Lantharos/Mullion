use std::{
    fs::OpenOptions,
    io::{Read, Write},
    path::Path,
    time::{Duration, Instant},
};

use crate::{RuntimeError, RuntimeInstallProgress, RuntimeInstallStep};

const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const MAX_ATTEMPTS: u32 = 4;

pub(crate) fn download_file(
    url: &str,
    destination: &Path,
    size: u64,
    progress: &mut impl FnMut(RuntimeInstallProgress),
) -> Result<(), RuntimeError> {
    let deadline = Instant::now() + DOWNLOAD_TIMEOUT;
    let mut failures = 0;
    loop {
        let offset = match std::fs::metadata(destination) {
            Ok(metadata) if metadata.len() <= size => metadata.len(),
            Ok(_) => {
                std::fs::remove_file(destination)?;
                0
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => return Err(error.into()),
        };
        if offset == size {
            return Ok(());
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(failed(
                "runtime download exceeded 30 minutes; retry to resume",
            ));
        }
        progress(RuntimeInstallProgress::new(
            RuntimeInstallStep::Downloading,
            Some(0.05 + 0.65 * offset as f32 / size as f32),
            if offset == 0 {
                "Downloading runtime"
            } else {
                "Resuming runtime download"
            },
        ));
        match transfer(url, destination, offset, size, remaining, progress) {
            Ok(()) => failures = 0,
            Err(TransferError::Fatal(error)) => return Err(error),
            Err(TransferError::Retry(error)) => {
                failures += 1;
                if failures >= MAX_ATTEMPTS || Instant::now() >= deadline {
                    return Err(failed(format!(
                        "{error}; downloaded bytes were retained for retry"
                    )));
                }
                progress(RuntimeInstallProgress::new(
                    RuntimeInstallStep::Downloading,
                    None,
                    format!("Download interrupted; reconnecting ({failures}/{MAX_ATTEMPTS})"),
                ));
                std::thread::sleep(Duration::from_secs(1 << (failures - 1)));
            }
        }
    }
}

enum TransferError {
    Retry(String),
    Fatal(RuntimeError),
}

impl From<std::io::Error> for TransferError {
    fn from(error: std::io::Error) -> Self {
        Self::Fatal(error.into())
    }
}

fn transfer(
    url: &str,
    destination: &Path,
    offset: u64,
    size: u64,
    timeout: Duration,
    progress: &mut impl FnMut(RuntimeInstallProgress),
) -> Result<(), TransferError> {
    let mut request = ureq::get(url)
        .config()
        .timeout_global(Some(timeout))
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .http_status_as_error(false)
        .build()
        .header("Accept-Encoding", "identity");
    if offset > 0 {
        request = request.header("Range", format!("bytes={offset}-"));
    }
    let response = request
        .call()
        .map_err(|error| TransferError::Retry(error.to_string()))?;
    let code = response.status().as_u16();
    if code == 429 || code >= 500 {
        return Err(TransferError::Retry(format!(
            "download server returned HTTP {code}"
        )));
    }
    let invalid = || {
        TransferError::Fatal(failed(
            "download response does not match the CEF archive metadata",
        ))
    };
    let (start, end) = match code {
        200 => (0, size),
        206 => {
            let range = response
                .headers()
                .get("content-range")
                .and_then(|value| value.to_str().ok())
                .ok_or_else(invalid)?;
            let (range, total) = range
                .strip_prefix("bytes ")
                .and_then(|value| value.split_once('/'))
                .ok_or_else(invalid)?;
            let (start, end) = range.split_once('-').ok_or_else(invalid)?;
            let start = start.parse::<u64>().map_err(|_| invalid())?;
            let end = end.parse::<u64>().map_err(|_| invalid())?;
            if start != offset
                || end < start
                || end >= size
                || total.parse::<u64>().ok() != Some(size)
            {
                return Err(invalid());
            }
            (start, end + 1)
        }
        _ => {
            return Err(TransferError::Fatal(failed(format!(
                "download server returned HTTP {code}"
            ))));
        }
    };
    if let Some(length) = response.headers().get("content-length")
        && length
            .to_str()
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            != Some(end - start)
    {
        return Err(invalid());
    }
    let mut output = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(start == 0)
        .append(start > 0)
        .open(destination)?;
    let mut reader = response.into_body().into_reader();
    let mut downloaded = start;
    let mut buffer = [0_u8; 64 * 1024];
    let mut last_report = Instant::now();
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|error| TransferError::Retry(error.to_string()))?;
        if read == 0 {
            break;
        }
        if read as u64 > end - downloaded {
            return Err(invalid());
        }
        output.write_all(&buffer[..read])?;
        downloaded += read as u64;
        if last_report.elapsed() >= Duration::from_millis(100) {
            let portion = downloaded as f32 / size as f32;
            progress(RuntimeInstallProgress::new(
                RuntimeInstallStep::Downloading,
                Some(0.05 + portion * 0.65),
                format!("Downloading runtime ({:.0}%)", portion * 100.0),
            ));
            last_report = Instant::now();
        }
    }
    output.sync_all()?;
    if downloaded != end {
        return Err(TransferError::Retry(
            "download ended before the archive was complete".to_string(),
        ));
    }
    Ok(())
}

fn failed(message: impl Into<String>) -> RuntimeError {
    RuntimeError::InstallationFailed(message.into())
}
