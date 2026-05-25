use super::*;
use super::response::ResponseError;

#[derive(Clone)]
pub(super) struct ErrorBodyDrainLimiter {
    semaphore: Arc<tokio::sync::Semaphore>,
    #[cfg(test)]
    dropped: Arc<AtomicUsize>,
}

impl ErrorBodyDrainLimiter {
    pub(super) fn new(limit: usize) -> Self {
        Self {
            semaphore: Arc::new(tokio::sync::Semaphore::new(limit)),
            #[cfg(test)]
            dropped: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub(super) fn try_spawn_error_response_body_drain(&self, response: reqwest::Response) {
        let Ok(permit) = self.semaphore.clone().try_acquire_owned() else {
            #[cfg(test)]
            self.dropped.fetch_add(1, Ordering::SeqCst);
            return;
        };
        tokio::spawn(async move {
            let _permit = permit;
            drain_error_response_body(response).await;
        });
    }

    #[cfg(test)]
    pub(super) fn hold_all_permits_for_test(&self) -> Vec<OwnedSemaphorePermit> {
        (0..MAX_BACKGROUND_ERROR_BODY_DRAINS)
            .map(|_| {
                self.semaphore
                    .clone()
                    .try_acquire_owned()
                    .expect("test should be able to hold all drain permits")
            })
            .collect()
    }

    #[cfg(test)]
    pub(super) fn dropped_for_test(&self) -> usize {
        self.dropped.load(Ordering::SeqCst)
    }
}


impl DepositWalletRelayerClient {
    pub(super) async fn send(&self, method: Method, url: Url, body: Option<String>) -> Result<Vec<u8>> {
        self.send_with_success_limit(method, url, body, MAX_SUCCESS_BODY_BYTES)
            .await
    }

    pub(super) async fn send_with_success_limit(
        &self,
        method: Method,
        url: Url,
        body: Option<String>,
        success_body_limit: usize,
    ) -> Result<Vec<u8>> {
        self.send_with_success_limit_and_retry_after(method, url, body, success_body_limit)
            .await
            .map_err(|error| error.error)
    }

    pub(super) async fn send_with_success_limit_and_retry_after(
        &self,
        method: Method,
        url: Url,
        body: Option<String>,
        success_body_limit: usize,
    ) -> std::result::Result<Vec<u8>, ResponseError> {
        let mut headers = self
            .auth
            .headers()
            .map_err(|error| ResponseError::new(error, None))?;
        if body.is_some() {
            headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        }

        let mut request = self
            .http
            .request(method.clone(), url)
            .headers(headers);
        if let Some(body) = body {
            request = request.body(body);
        }

        let response = request
            .send()
            .await
            .map_err(|error| ResponseError::new(RelayerError::Http(error.without_url()), None))?;
        if !response.status().is_success() {
            let status = response.status();
            let retry_after = retry_after_duration(response.headers());
            let retry_after_message = retry_after_summary_from_duration(retry_after);
            self.error_body_drain_limiter
                .try_spawn_error_response_body_drain(response);
            if status == StatusCode::TOO_MANY_REQUESTS {
                return Err(ResponseError::new(RelayerError::QuotaExhausted, retry_after));
            }
            return Err(ResponseError::new(
                RelayerError::Api {
                    status: status.as_u16(),
                    message: format!(
                        "deposit-wallet relayer request failed with HTTP {status}{retry_after_message}"
                    ),
                },
                retry_after,
            ));
        }

        read_limited_response_body(response, success_body_limit).await
            .map_err(|error| ResponseError::new(error, None))
    }

    #[cfg(test)]
    pub(super) fn hold_error_body_drain_permits_for_test(&self) -> Vec<OwnedSemaphorePermit> {
        self.error_body_drain_limiter.hold_all_permits_for_test()
    }

    #[cfg(test)]
    pub(super) fn dropped_error_body_drains_for_test(&self) -> usize {
        self.error_body_drain_limiter.dropped_for_test()
    }
}

pub(super) async fn read_limited_response_body(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(RelayerError::Other(RESPONSE_BODY_TOO_LARGE_MESSAGE.to_string()));
    }

    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| RelayerError::Http(error.without_url()))?
    {
        if body.len().saturating_add(chunk.len()) > limit {
            return Err(RelayerError::Other(RESPONSE_BODY_TOO_LARGE_MESSAGE.to_string()));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

pub(super) async fn drain_error_response_body(response: reqwest::Response) {
    let drain = read_limited_response_body(response, MAX_ERROR_BODY_DRAIN_BYTES);
    let _ = tokio::time::timeout(ERROR_BODY_DRAIN_TIMEOUT, drain).await;
}

pub(super) fn retry_after_duration(headers: &HeaderMap) -> Option<Duration> {
    retry_after_duration_at(headers, SystemTime::now())
}

pub(super) fn retry_after_duration_at(headers: &HeaderMap, now: SystemTime) -> Option<Duration> {
    let value = headers
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)?;
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let retry_at = httpdate::parse_http_date(value).ok()?;
    Some(retry_at.duration_since(now).unwrap_or(Duration::ZERO))
}

pub(super) fn retry_after_summary_from_duration(retry_after: Option<Duration>) -> String {
    retry_after
        .map(|duration| format!("; retry after {}s", duration.as_secs()))
        .unwrap_or_default()
}
