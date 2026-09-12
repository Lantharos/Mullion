use std::{
    fs::OpenOptions,
    io::{Read, Write},
    path::Path,
    time::{Duration, Instant},
};

use crate::RuntimeError;

const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const MAX_ATTEMPTS: u32 = 4;

#[derive(Clone, Copy, Debug)]
pub struct DownloadProgress {
    pub downloaded: u64,
    pub total: Option<u64>,
    pub attempt: u32,
}

/// Downloads a bounded file. Known-size partial files resume across calls;
/// response sizes also allow continuation within this call. Callers must
/// serialize writes to the destination and verify the completed file's digest.
pub fn download_file(
    url: &str,
    destination: &Path,
    expected_size: Option<u64>,
    max_size: u64,
    progress: &mut impl FnMut(DownloadProgress),
) -> Result<(), RuntimeError> {
    if max_size == 0 || expected_size.is_some_and(|size| size == 0 || size > max_size) {
        return Err(failed(
            "download size exceeds the permitted limit or is empty",
        ));
    }
    let deadline = Instant::now() + DOWNLOAD_TIMEOUT;
    let mut failures = 0;
    let mut total = expected_size;
    loop {
        let offset = match std::fs::metadata(destination) {
            Ok(metadata) if total.is_some_and(|size| metadata.len() <= size) => metadata.len(),
            Ok(_) => 0,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => return Err(error.into()),
        };
        progress(DownloadProgress {
            downloaded: offset,
            total,
            attempt: failures + 1,
        });
        if total == Some(offset) {
            return Ok(());
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(failed("download exceeded 30 minutes; retry to resume"));
        }
        let request = Transfer {
            url,
            destination,
            offset,
            max_size,
            timeout: remaining,
            attempt: failures + 1,
        };
        match request.run(&mut total, progress) {
            Ok(true) => return Ok(()),
            Ok(false) => failures = 0,
            Err(TransferError::Fatal(error)) => return Err(error),
            Err(TransferError::Retry(error)) => {
                failures += 1;
                if failures >= MAX_ATTEMPTS || Instant::now() >= deadline {
                    return Err(failed(format!(
                        "{error}; downloaded bytes were retained for retry"
                    )));
                }
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

struct Transfer<'a> {
    url: &'a str,
    destination: &'a Path,
    offset: u64,
    max_size: u64,
    timeout: Duration,
    attempt: u32,
}

impl Transfer<'_> {
    fn run(
        &self,
        total: &mut Option<u64>,
        progress: &mut impl FnMut(DownloadProgress),
    ) -> Result<bool, TransferError> {
        let mut request = ureq::get(self.url)
            .config()
            .timeout_global(Some(self.timeout))
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_recv_response(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .build()
            .header("Accept-Encoding", "identity");
        if self.offset > 0 {
            request = request.header("Range", format!("bytes={}-", self.offset));
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
        let length = response
            .headers()
            .get("content-length")
            .map(|value| {
                value
                    .to_str()
                    .ok()
                    .and_then(|value| value.parse::<u64>().ok())
                    .ok_or_else(invalid_response)
            })
            .transpose()?;
        let (start, end) = match code {
            200 => {
                if total.is_some() && length.is_some() && *total != length {
                    return Err(invalid_response());
                }
                if total.is_none() {
                    *total = length;
                }
                (0, *total)
            }
            206 => {
                let range = response
                    .headers()
                    .get("content-range")
                    .and_then(|value| value.to_str().ok())
                    .ok_or_else(invalid_response)?;
                let (range, size) = range
                    .strip_prefix("bytes ")
                    .and_then(|value| value.split_once('/'))
                    .ok_or_else(invalid_response)?;
                let (start, end) = range.split_once('-').ok_or_else(invalid_response)?;
                let start = start.parse::<u64>().map_err(|_| invalid_response())?;
                let end = end.parse::<u64>().map_err(|_| invalid_response())?;
                let size = size.parse::<u64>().map_err(|_| invalid_response())?;
                if start != self.offset
                    || end < start
                    || end >= size
                    || total.is_some_and(|total| total != size)
                {
                    return Err(invalid_response());
                }
                *total = Some(size);
                (start, Some(end + 1))
            }
            _ => {
                return Err(TransferError::Fatal(failed(format!(
                    "download server returned HTTP {code}"
                ))));
            }
        };
        if total.is_some_and(|size| size > self.max_size)
            || length.is_some_and(|length| end.is_some_and(|end| length != end - start))
        {
            return Err(invalid_response());
        }
        let mut output = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(start == 0)
            .append(start > 0)
            .open(self.destination)?;
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
            if read as u64 > end.unwrap_or(self.max_size) - downloaded {
                return Err(invalid_response());
            }
            output.write_all(&buffer[..read])?;
            downloaded += read as u64;
            if last_report.elapsed() >= Duration::from_millis(100) {
                progress(DownloadProgress {
                    downloaded,
                    total: *total,
                    attempt: self.attempt,
                });
                last_report = Instant::now();
            }
        }
        output.sync_all()?;
        if end.is_some_and(|end| downloaded != end) {
            return Err(TransferError::Retry(
                "download ended before the file was complete".to_string(),
            ));
        }
        progress(DownloadProgress {
            downloaded,
            total: *total,
            attempt: self.attempt,
        });
        Ok(total.is_none_or(|total| downloaded == total))
    }
}

fn invalid_response() -> TransferError {
    TransferError::Fatal(failed(
        "download response exceeds its size limit or does not match the expected file size",
    ))
}

fn failed(message: impl Into<String>) -> RuntimeError {
    RuntimeError::InstallationFailed(message.into())
}
