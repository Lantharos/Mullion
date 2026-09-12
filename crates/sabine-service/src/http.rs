use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;

use crate::{ServiceError, ServiceResult};

pub(crate) fn fetch_manifest<T: DeserializeOwned>(url: &str) -> ServiceResult<T> {
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut failure = String::from("release manifest request timed out");
    for attempt in 0..3 {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        let response = ureq::get(url)
            .config()
            .timeout_global(Some(remaining))
            .timeout_connect(Some(Duration::from_secs(15)))
            .http_status_as_error(false)
            .build()
            .call();
        match response {
            Ok(mut response) => {
                let status = response.status().as_u16();
                if status == 200 {
                    match response
                        .body_mut()
                        .with_config()
                        .limit(1024 * 1024)
                        .read_to_vec()
                    {
                        Ok(body) => {
                            return serde_json::from_slice(&body).map_err(|error| {
                                ServiceError::Update(format!("invalid release manifest: {error}"))
                            });
                        }
                        Err(error @ ureq::Error::BodyExceedsLimit(_)) => {
                            return Err(ServiceError::Update(format!(
                                "release manifest is too large: {error}"
                            )));
                        }
                        Err(error) => failure = format!("release manifest read failed: {error}"),
                    }
                } else if status == 429 || status >= 500 {
                    failure = format!("release manifest server returned HTTP {status}");
                } else {
                    return Err(ServiceError::Update(format!(
                        "release manifest server returned HTTP {status}"
                    )));
                }
            }
            Err(error) => failure = format!("release manifest request failed: {error}"),
        }
        if attempt < 2 {
            std::thread::sleep(
                Duration::from_secs(1 << attempt)
                    .min(deadline.saturating_duration_since(Instant::now())),
            );
        }
    }
    Err(ServiceError::Update(failure))
}
