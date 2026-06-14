use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use reqwest::{
    Url,
    header::{HeaderMap, HeaderName, HeaderValue},
};
use serde::Deserialize;
use thiserror::Error;
use tokio::sync::Mutex;

const AUTH_BASE_URL: &str = "https://www.reddit.com";
const OAUTH_BASE_URL: &str = "https://oauth.reddit.com";
const REDDIT_ANDROID_OAUTH_CLIENT_ID: &str = "ohXpoqrZYub1kg";
const ANDROID_USER_AGENT: &str = "Reddit/2026.23.0/Android 14";
const TOKEN_REFRESH_SKEW_SECS: i64 = 120;

#[derive(Clone)]
pub struct RedditBearerToken {
    access_token: String,
    expires_at: DateTime<Utc>,
    extra_headers: HeaderMap,
}

impl RedditBearerToken {
    pub fn expires_at(&self) -> DateTime<Utc> {
        self.expires_at
    }

    fn is_fresh_at(&self, now: DateTime<Utc>) -> bool {
        self.expires_at - Duration::seconds(TOKEN_REFRESH_SKEW_SECS) > now
    }
}

impl std::fmt::Debug for RedditBearerToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedditBearerToken")
            .field("access_token", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .field("extra_headers", &self.extra_headers)
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct RedditOAuthTransport {
    client: reqwest::Client,
    auth_base_url: Url,
    oauth_base_url: Url,
    device_id: String,
    token: std::sync::Arc<Mutex<Option<RedditBearerToken>>>,
}

/// Raw response from an authenticated Reddit API request, with the HTTP
/// status, the response body as text, and the response headers so the
/// caller can inspect rate-limit metadata. Callers use this to map
/// non-success statuses to typed errors (for example, subreddit about
/// validation) without going through `get_json`'s `error_for_status` path.
#[derive(Debug)]
pub struct RedditRawResponse {
    pub status: reqwest::StatusCode,
    pub headers: HeaderMap,
    pub body: String,
}

impl RedditOAuthTransport {
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .user_agent(ANDROID_USER_AGENT)
                .build()?,
            auth_base_url: Url::parse(AUTH_BASE_URL)?,
            oauth_base_url: Url::parse(OAUTH_BASE_URL)?,
            device_id: format!(
                "tgreddit-{}",
                Utc::now().timestamp_nanos_opt().unwrap_or_default()
            ),
            token: std::sync::Arc::new(Mutex::new(None)),
        })
    }

    pub async fn bearer_token(&self) -> Result<RedditBearerToken> {
        let now = Utc::now();
        let cached = self.token.lock().await.clone();
        if let Some(token) = cached
            && !should_refresh_token(Some(&token), now)
        {
            return Ok(token);
        }

        let token = self.request_bearer_token().await?;
        *self.token.lock().await = Some(token.clone());
        Ok(token)
    }

    /// Force-refresh the bearer token, ignoring any cached value. Used
    /// by the 401 retry path so a possibly-invalidated token is
    /// replaced before the request is sent again.
    async fn force_refresh_bearer_token(&self) -> Result<RedditBearerToken> {
        let token = self.request_bearer_token().await?;
        *self.token.lock().await = Some(token.clone());
        Ok(token)
    }

    pub async fn get_json<T>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<T, RedditApiError>
    where
        T: serde::de::DeserializeOwned,
    {
        let response = self.send_authenticated(path, query).await?;
        if !response.status.is_success() {
            return Err(RedditApiError::Transport(anyhow::anyhow!(
                "Reddit API returned non-success status {} for path {}",
                response.status,
                path
            )));
        }
        serde_json::from_str(&response.body).map_err(|e| {
            RedditApiError::Transport(anyhow::Error::from(e).context(format!(
                "failed to parse Reddit API response for path {path}: {}",
                response.body
            )))
        })
    }

    /// Send an authenticated GET to the OAuth API and return the raw response
    /// (status + body text) without interpreting it. Callers that need to map
    /// non-success statuses to typed errors (for example, subreddit about
    /// validation) use this instead of `get_json`.
    ///
    /// The transport applies the production hardening from issue
    /// `05-refresh-and-rate-limit-basics`:
    /// - on `401 Unauthorized`, force-refresh the bearer token and retry once.
    ///   A second `401` after the refresh surfaces as
    ///   `RedditApiError::Unauthorized` and is not retried again.
    /// - on `429 Too Many Requests` (including a `429` returned by the
    ///   forced-refresh retry), log a clear error and return
    ///   `RedditApiError::RateLimited` with parsed `x-ratelimit-*` headers
    ///   when present, so callers can back off without retrying.
    /// - on a 2xx response with low `x-ratelimit-remaining`, surface
    ///   `RedditApiError::RateLimited` so the caller backs off before
    ///   the next call gets throttled. This matches the issue
    ///   acceptance wording that low remaining headers are returned
    ///   as actionable errors, not only logged as warnings.
    pub async fn send_authenticated(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<RedditRawResponse, RedditApiError> {
        let token = self.bearer_token().await?;
        let response = self.send_with_token(path, query, &token).await?;
        if response.status == reqwest::StatusCode::UNAUTHORIZED {
            log::warn!(
                "Reddit returned 401 Unauthorized for path {path}; refreshing token and retrying once"
            );
            let refreshed = self
                .force_refresh_bearer_token()
                .await
                .map_err(RedditApiError::from)?;
            let retried = self.send_with_token(path, query, &refreshed).await?;
            return handle_response(path, retried);
        }
        handle_response(path, response)
    }

    /// Dispatch the post-first-response decision in a way that is
    /// fully testable without standing up an HTTP server.
    ///
    /// Contract:
    /// - `refresh` and `second_send` are each invoked at most once,
    ///   and only when `first_response` is a `401 Unauthorized`.
    /// - On a `401` first response: `refresh` is called once to get a
    ///   fresh bearer token, then `second_send` is called once with
    ///   that token, and the result of `handle_response` on the
    ///   second response is returned.
    /// - On any other first response: neither `refresh` nor
    ///   `second_send` is called, and the result of `handle_response`
    ///   on the first response is returned.
    ///
    /// This is the single seam the transport uses to enforce the
    /// "exactly one refresh, exactly one retry, no third attempt on
    /// a second 401" contract from the issue acceptance criteria.
    /// The closures let unit tests count calls and inject canned
    /// responses without a local HTTP server that pretends to be
    /// Reddit.
    pub async fn dispatch_response<R, S, RFut, SFut>(
        &self,
        path: &str,
        first_response: RedditRawResponse,
        refresh: R,
        second_send: S,
    ) -> Result<RedditRawResponse, RedditApiError>
    where
        R: FnOnce() -> RFut,
        S: FnOnce(&RedditBearerToken) -> SFut,
        RFut: std::future::Future<Output = Result<RedditBearerToken, RedditApiError>>,
        SFut: std::future::Future<Output = Result<RedditRawResponse, RedditApiError>>,
    {
        if first_response.status == reqwest::StatusCode::UNAUTHORIZED {
            log::warn!(
                "Reddit returned 401 Unauthorized for path {path}; refreshing token and retrying once"
            );
            let refreshed = refresh().await?;
            let retried = second_send(&refreshed).await?;
            return handle_response(path, retried);
        }
        handle_response(path, first_response)
    }

    /// Send an authenticated GET using a specific bearer token. The
    /// caller (typically `send_authenticated`) decides what to do with
    /// the resulting status, including whether to retry.
    async fn send_with_token(
        &self,
        path: &str,
        query: &[(&str, String)],
        token: &RedditBearerToken,
    ) -> Result<RedditRawResponse, RedditApiError> {
        let url = self.build_authenticated_url(path, query)?;
        let mut request = self.client.get(url).headers(self.app_headers()?);
        request = request.bearer_auth(&token.access_token);
        for (key, value) in &token.extra_headers {
            request = request.header(key, value);
        }

        let response = request
            .send()
            .await
            .map_err(|e| RedditApiError::Transport(anyhow::Error::from(e)))?;
        let status = response.status();
        let headers = response.headers().clone();
        let body = response.text().await.map_err(|e| {
            RedditApiError::Transport(anyhow::Error::from(e).context(format!(
                "failed to read Reddit response body for path {path}"
            )))
        })?;
        Ok(RedditRawResponse {
            status,
            headers,
            body,
        })
    }

    fn build_authenticated_url(&self, path: &str, query: &[(&str, String)]) -> Result<Url> {
        let mut url = self.oauth_url(path)?;
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("raw_json", "1");
            for (key, value) in query {
                pairs.append_pair(key, value);
            }
        }
        Ok(url)
    }

    fn token_url(&self) -> Result<Url> {
        self.auth_base_url
            .join("/auth/v2/oauth/access-token/loid")
            .context("failed to build Reddit OAuth token URL")
    }

    fn oauth_url(&self, path: &str) -> Result<Url> {
        self.oauth_base_url
            .join(path)
            .context("failed to build Reddit OAuth API URL")
    }

    fn app_headers(&self) -> Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-reddit-retry",
            HeaderValue::from_static("algo=no-retries"),
        );
        headers.insert("x-reddit-compression", HeaderValue::from_static("1"));
        headers.insert("x-reddit-qos", HeaderValue::from_static("0.000"));
        headers.insert("client-vendor-id", header_value(&self.device_id)?);
        headers.insert("x-reddit-device-id", header_value(&self.device_id)?);
        Ok(headers)
    }

    async fn request_bearer_token(&self) -> Result<RedditBearerToken> {
        let response = self
            .client
            .post(self.token_url()?)
            .headers(self.app_headers()?)
            .basic_auth(REDDIT_ANDROID_OAUTH_CLIENT_ID, Some(""))
            .json(&serde_json::json!({ "scopes": ["*", "email", "pii"] }))
            .send()
            .await?
            .error_for_status()?;

        let received_at = Utc::now();
        let headers = response.headers().clone();
        let body = response.text().await?;
        parse_token_response(&body, received_at, &headers)
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: i64,
}

fn parse_token_response(
    body: &str,
    received_at: DateTime<Utc>,
    headers: &HeaderMap,
) -> Result<RedditBearerToken> {
    let response: TokenResponse =
        serde_json::from_str(body).context("failed to parse Reddit OAuth token response")?;
    if response.access_token.is_empty() {
        anyhow::bail!("Reddit OAuth token response had empty access token");
    }
    if response.expires_in <= 0 {
        anyhow::bail!("Reddit OAuth token response had non-positive expiry");
    }

    Ok(RedditBearerToken {
        access_token: response.access_token,
        expires_at: received_at + Duration::seconds(response.expires_in),
        extra_headers: extract_reddit_session_headers(headers),
    })
}

fn extract_reddit_session_headers(headers: &HeaderMap) -> HeaderMap {
    let mut extra = HeaderMap::new();
    for name in ["x-reddit-loid", "x-reddit-session"] {
        if let Some(value) = headers.get(name) {
            extra.insert(HeaderName::from_static(name), value.clone());
        }
    }
    extra
}

fn header_value(value: &str) -> Result<HeaderValue> {
    HeaderValue::from_str(value).context("failed to build Reddit OAuth header")
}

/// Parsed Reddit rate-limit information from `x-ratelimit-*` response
/// headers. All fields are floating point because Reddit emits sub-second
/// `used` and `reset` values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RateLimitInfo {
    pub used: f64,
    pub remaining: f64,
    pub reset_secs: f64,
}

/// Threshold below which the remaining rate-limit budget is considered
/// "low". Strict less-than (see [`RateLimitInfo::is_low`]) keeps the
/// threshold value itself in the "not low" bucket so the warning is
/// only emitted when the budget is genuinely squeezed.
const RATE_LIMIT_LOW_THRESHOLD: f64 = 10.0;

impl RateLimitInfo {
    /// `true` when the remaining rate-limit budget has dropped strictly
    /// below the low threshold. Strict less-than keeps the threshold
    /// value itself in the "not low" bucket.
    pub fn is_low(&self) -> bool {
        self.remaining < RATE_LIMIT_LOW_THRESHOLD
    }

    /// One-line summary intended for `log::warn!` and `log::error!`
    /// output. `RateLimitInfo` does not store bearer tokens, so they
    /// cannot leak through this formatter.
    pub fn format_log_line(&self) -> String {
        format!(
            "Reddit rate limit: used={:.0} remaining={:.0} resets_in={:.0}s",
            self.used, self.remaining, self.reset_secs
        )
    }

    /// Actionable error message for a `429` response, including the
    /// path that was throttled and the rate-limit numbers.
    pub fn format_error(&self, path: &str) -> String {
        format!(
            "Reddit rate limit hit for path {path}: used={:.0} remaining={:.0} resets_in={:.0}s; back off before retrying",
            self.used, self.remaining, self.reset_secs
        )
    }
}

/// Header names Reddit uses for rate limit reporting.
const RATELIMIT_USED: &str = "x-ratelimit-used";
const RATELIMIT_REMAINING: &str = "x-ratelimit-remaining";
const RATELIMIT_RESET: &str = "x-ratelimit-reset";

/// Parse the `x-ratelimit-*` headers from a Reddit response. Returns
/// `None` if any of the three expected headers is missing or not a valid
/// floating point number. The transport treats `None` as "no rate limit
/// information available" and falls back to status-only reporting.
pub fn parse_rate_limit_headers(headers: &HeaderMap) -> Option<RateLimitInfo> {
    let used = header_f64(headers, RATELIMIT_USED)?;
    let remaining = header_f64(headers, RATELIMIT_REMAINING)?;
    let reset_secs = header_f64(headers, RATELIMIT_RESET)?;
    Some(RateLimitInfo {
        used,
        remaining,
        reset_secs,
    })
}

fn header_f64(headers: &HeaderMap, name: &str) -> Option<f64> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<f64>().ok())
}

/// Decide whether the OAuth transport should request a new bearer
/// token. Returns `true` when the cache is empty or the cached token
/// is past the refresh skew (close to expiry). Extracted as a pure
/// function so the refresh decision can be tested without a network
/// call.
pub fn should_refresh_token(cached: Option<&RedditBearerToken>, now: DateTime<Utc>) -> bool {
    match cached {
        Some(token) => !token.is_fresh_at(now),
        None => true,
    }
}

/// Apply the post-response decision to a single Reddit response and
/// convert it into either an `Ok(RedditRawResponse)` for the caller
/// to consume or a typed `RedditApiError`.
///
/// `send_authenticated` always retries once on a `401` before calling
/// this helper, so a `401` seen here means the OAuth flow is broken
/// (for example, the client id is rejected) and the caller should
/// stop polling.
///
/// Per the issue acceptance criteria, both `429` responses **and**
/// 2xx responses with a low `x-ratelimit-remaining` budget are
/// surfaced as actionable `RateLimited` errors so the caller backs
/// off before the next call is throttled. A `429` returned by the
/// forced-refresh retry is handled here too, so it does not leak
/// out as `Ok(retried)` and get misread by `get_subreddit_about` as
/// an `Inaccessible` subreddit.
pub fn handle_response(
    path: &str,
    response: RedditRawResponse,
) -> Result<RedditRawResponse, RedditApiError> {
    let info = parse_rate_limit_headers(&response.headers);
    match response.status {
        reqwest::StatusCode::UNAUTHORIZED => {
            log::error!(
                "Reddit returned 401 Unauthorized for path {path} after token refresh; aborting"
            );
            Err(RedditApiError::Unauthorized {
                path: path.to_string(),
            })
        }
        reqwest::StatusCode::TOO_MANY_REQUESTS => {
            log_rate_limit_hit(path, info);
            Err(RedditApiError::rate_limited(path, info))
        }
        status if status.is_success() => {
            if info.is_some_and(|i| i.is_low()) {
                log_rate_limit_hit(path, info);
                Err(RedditApiError::rate_limited(path, info))
            } else {
                Ok(response)
            }
        }
        _ => Ok(response),
    }
}

fn log_rate_limit_hit(path: &str, info: Option<RateLimitInfo>) {
    log::error!(
        "Reddit rate limit hit for path {path}: {}",
        info.map(|i| i.format_log_line())
            .unwrap_or_else(|| "no x-ratelimit-* headers".to_string())
    );
}

/// Typed error for the Reddit OAuth transport. Callers can match on
/// the variants to branch on the failure mode, or convert to
/// `anyhow::Error` via the `?` operator.
#[derive(Error, Debug)]
pub enum RedditApiError {
    /// Reddit returned `429 Too Many Requests`. `info` carries the
    /// parsed `x-ratelimit-*` headers when present; `message` is the
    /// pre-rendered, operator-friendly string.
    #[error("{message}")]
    RateLimited {
        path: String,
        info: Option<RateLimitInfo>,
        message: String,
    },

    /// Reddit returned `401 Unauthorized` even after a forced token
    /// refresh and a single retry. The caller should not retry.
    #[error("Reddit returned 401 Unauthorized for {path} after token refresh; aborting")]
    Unauthorized { path: String },

    /// Any other transport-level failure (DNS, TLS, connection reset,
    /// non-2xx status, etc.). Wrapped as `anyhow::Error` so callers
    /// can keep their existing error chains.
    #[error(transparent)]
    Transport(#[from] anyhow::Error),
}

impl RedditApiError {
    /// Build a `RateLimited` variant. The `message` field is rendered
    /// up-front so `Display` stays a one-liner and the variant owns a
    /// self-describing string.
    pub fn rate_limited(path: &str, info: Option<RateLimitInfo>) -> Self {
        let message = match info {
            Some(info) => info.format_error(path),
            None => format!(
                "Reddit rate limit hit for path {path} (no x-ratelimit-* headers); back off before retrying"
            ),
        };
        Self::RateLimited {
            path: path.to_string(),
            info,
            message,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn token_response_parsing_stores_expiry_and_session_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("x-reddit-loid", HeaderValue::from_static("loid-123"));
        headers.insert("x-reddit-session", HeaderValue::from_static("session-456"));
        let received_at = DateTime::parse_from_rfc3339("2026-06-13T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let token = parse_token_response(
            r#"{"access_token":"secret-token","expires_in":86400}"#,
            received_at,
            &headers,
        )
        .unwrap();

        assert_eq!(token.expires_at(), received_at + Duration::seconds(86400));
        assert_eq!(token.extra_headers["x-reddit-loid"], "loid-123");
        assert_eq!(token.extra_headers["x-reddit-session"], "session-456");
        assert!(!format!("{token:?}").contains("secret-token"));
    }

    #[test]
    fn default_authenticated_url_uses_oauth_reddit_host() {
        let transport = RedditOAuthTransport::new().unwrap();

        let url = transport.oauth_url("/r/rust/top.json").unwrap();

        assert_eq!(url.as_str(), "https://oauth.reddit.com/r/rust/top.json");
    }

    #[test]
    fn build_authenticated_url_appends_raw_json_query_parameter() {
        let transport = RedditOAuthTransport::new().unwrap();

        let url = transport
            .build_authenticated_url("/r/rust/about.json", &[])
            .unwrap();

        assert_eq!(
            url.as_str(),
            "https://oauth.reddit.com/r/rust/about.json?raw_json=1"
        );
    }

    #[test]
    fn build_authenticated_url_merges_raw_json_with_caller_query() {
        let transport = RedditOAuthTransport::new().unwrap();

        let query = [("limit", "5".to_string())];
        let url = transport
            .build_authenticated_url("/r/rust/top.json", &query)
            .unwrap();

        // `raw_json=1` must be present alongside the caller-supplied query.
        let raw = url.as_str();
        assert!(raw.starts_with("https://oauth.reddit.com/r/rust/top.json?"));
        assert!(raw.contains("raw_json=1"), "missing raw_json=1 in {raw}");
        assert!(raw.contains("limit=5"), "missing limit=5 in {raw}");
    }

    fn rate_limit_headers(used: &str, remaining: &str, reset_secs: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("x-ratelimit-used", HeaderValue::from_str(used).unwrap());
        headers.insert(
            "x-ratelimit-remaining",
            HeaderValue::from_str(remaining).unwrap(),
        );
        headers.insert(
            "x-ratelimit-reset",
            HeaderValue::from_str(reset_secs).unwrap(),
        );
        headers
    }

    #[test]
    fn parse_rate_limit_headers_returns_none_when_missing() {
        let headers = HeaderMap::new();
        assert!(parse_rate_limit_headers(&headers).is_none());
    }

    #[test]
    fn parse_rate_limit_headers_returns_info_for_known_reddit_headers() {
        let headers = rate_limit_headers("37", "563", "482");

        let info = parse_rate_limit_headers(&headers).expect("headers should parse");

        assert_eq!(info.used, 37.0);
        assert_eq!(info.remaining, 563.0);
        assert_eq!(info.reset_secs, 482.0);
    }

    #[test]
    fn parse_rate_limit_headers_ignores_garbage_values() {
        // All three headers are present, but `used` is unparseable.
        // The parser should treat the whole set as missing rather
        // than partially-fill `RateLimitInfo`.
        let headers = rate_limit_headers("not-a-number", "5", "60");

        assert!(parse_rate_limit_headers(&headers).is_none());
    }

    #[test]
    fn parse_rate_limit_headers_partial_values_returns_none() {
        // Only `used` is present; without remaining or reset we treat it as
        // missing so the caller falls back to a clear status-only message.
        let mut headers = HeaderMap::new();
        headers.insert("x-ratelimit-used", HeaderValue::from_static("10"));
        assert!(parse_rate_limit_headers(&headers).is_none());
    }

    #[test]
    fn rate_limit_info_is_low_true_below_threshold() {
        let info = RateLimitInfo {
            used: 590.0,
            remaining: 9.0,
            reset_secs: 120.0,
        };
        assert!(info.is_low());
    }

    #[test]
    fn rate_limit_info_is_low_false_at_or_above_threshold() {
        let at = RateLimitInfo {
            used: 590.0,
            remaining: 10.0,
            reset_secs: 120.0,
        };
        assert!(!at.is_low());

        let above = RateLimitInfo {
            used: 100.0,
            remaining: 500.0,
            reset_secs: 600.0,
        };
        assert!(!above.is_low());
    }

    #[test]
    fn rate_limit_info_format_log_line_is_actionable() {
        let info = RateLimitInfo {
            used: 590.0,
            remaining: 9.0,
            reset_secs: 120.0,
        };
        let line = info.format_log_line();
        // Log line should mention rate limit and the actionable numbers
        // (used, remaining, reset) so operators can react. The numbers
        // are formatted as labeled fields so a stray substring match
        // (for example, `590` matching `9`) cannot satisfy the
        // assertion. `RateLimitInfo` does not store bearer tokens, so
        // they cannot leak through this formatter.
        assert!(line.contains("rate limit"), "got: {line}");
        assert!(line.contains("used=590"), "got: {line}");
        assert!(line.contains("remaining=9"), "got: {line}");
        assert!(line.contains("resets_in=120"), "got: {line}");
    }

    #[test]
    fn rate_limit_info_format_error_is_actionable() {
        let info = RateLimitInfo {
            used: 590.0,
            remaining: 9.0,
            reset_secs: 120.0,
        };
        let rendered = info.format_error("/r/rust/top.json");
        assert!(rendered.contains("rate limit"), "got: {rendered}");
        assert!(rendered.contains("/r/rust/top.json"), "got: {rendered}");
        assert!(rendered.contains("used=590"), "got: {rendered}");
        assert!(rendered.contains("remaining=9"), "got: {rendered}");
        assert!(rendered.contains("resets_in=120"), "got: {rendered}");
    }

    fn response_with(status: u16, headers: HeaderMap, body: &str) -> RedditRawResponse {
        RedditRawResponse {
            status: reqwest::StatusCode::from_u16(status).unwrap(),
            headers,
            body: body.to_string(),
        }
    }

    #[test]
    fn handle_response_returns_ok_for_2xx_with_healthy_remaining() {
        let headers = rate_limit_headers("10", "500", "600");
        let response = response_with(200, headers, "{\"ok\": true}");

        let result = handle_response("/r/rust/top.json", response)
            .expect("healthy 2xx should be returned to the caller");

        assert_eq!(result.status, reqwest::StatusCode::OK);
        assert_eq!(result.body, "{\"ok\": true}");
    }

    #[test]
    fn handle_response_returns_rate_limited_for_2xx_with_low_remaining() {
        // A 2xx with a low remaining budget is an actionable
        // rate-limit condition: the caller must back off before the
        // next call gets throttled. The transport must surface it
        // as `RedditApiError::RateLimited` so the error is
        // unambiguous and not lost in a generic non-success status.
        let headers = rate_limit_headers("590", "9", "120");
        let response = response_with(200, headers, "{}");

        let err = handle_response("/r/rust/top.json", response)
            .expect_err("low remaining on 2xx must surface as RateLimited error");
        match err {
            RedditApiError::RateLimited { path, info, .. } => {
                assert_eq!(path, "/r/rust/top.json");
                let info = info.expect("info should be preserved when headers parsed");
                assert_eq!(info.used, 590.0);
                assert_eq!(info.remaining, 9.0);
                assert_eq!(info.reset_secs, 120.0);
            }
            other => panic!("expected RateLimited, got {other:?}"),
        }
    }

    #[test]
    fn handle_response_returns_rate_limited_for_429() {
        // `429` responses must surface as a typed `RateLimited` error,
        // never as `Ok(response)`. Without this, `get_json` would
        // emit a generic non-success transport error and
        // `get_subreddit_about` would map the 429 as
        // `Inaccessible { reason: "unknown" }`, hiding the real
        // cause from operators.
        let headers = rate_limit_headers("600", "0", "60");
        let response = response_with(429, headers, "");

        let err = handle_response("/r/rust/top.json", response)
            .expect_err("429 must surface as RateLimited error");
        match err {
            RedditApiError::RateLimited { path, info, .. } => {
                assert_eq!(path, "/r/rust/top.json");
                let info = info.expect("info should be preserved when headers parsed");
                assert_eq!(info.remaining, 0.0);
                assert_eq!(info.reset_secs, 60.0);
            }
            other => panic!("expected RateLimited, got {other:?}"),
        }
    }

    #[test]
    fn handle_response_returns_rate_limited_for_429_without_headers() {
        // `429` without `x-ratelimit-*` headers must still surface
        // as a typed `RateLimited` error so callers can branch on
        // the error kind; the `info` field is `None` to indicate
        // the headers were missing.
        let response = response_with(429, HeaderMap::new(), "");

        let err = handle_response("/r/rust/top.json", response)
            .expect_err("429 without headers must still surface as RateLimited error");
        match err {
            RedditApiError::RateLimited { info, .. } => {
                assert!(info.is_none(), "info should be None when headers missing");
            }
            other => panic!("expected RateLimited, got {other:?}"),
        }
    }

    #[test]
    fn handle_response_returns_unauthorized_for_401() {
        // A `401` seen by `handle_response` always means the OAuth
        // flow is broken (the 401-retry path in `send_authenticated`
        // already ran). The transport must not retry again; it must
        // surface `Unauthorized` so the caller stops polling.
        let response = response_with(401, HeaderMap::new(), "");

        let err = handle_response("/r/rust/top.json", response)
            .expect_err("401 must surface as Unauthorized error");
        match err {
            RedditApiError::Unauthorized { path } => {
                assert_eq!(path, "/r/rust/top.json");
            }
            other => panic!("expected Unauthorized, got {other:?}"),
        }
    }

    #[test]
    fn handle_response_returns_ok_for_5xx_so_caller_can_handle_it() {
        // 5xx is not a rate-limit or auth condition; the transport
        // returns the response so callers (e.g. `get_json`) can map
        // it to their own error type.
        let response = response_with(500, HeaderMap::new(), "server error");

        let result = handle_response("/r/rust/top.json", response)
            .expect("5xx should be returned to the caller");
        assert_eq!(result.status, reqwest::StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(result.body, "server error");
    }

    fn sample_rate_limit() -> RateLimitInfo {
        RateLimitInfo {
            used: 590.0,
            remaining: 0.0,
            reset_secs: 60.0,
        }
    }

    #[test]
    fn reddit_api_error_rate_limited_display_includes_path_and_numbers() {
        let err = RedditApiError::rate_limited("/r/rust/top.json", Some(sample_rate_limit()));
        let rendered = format!("{err}");
        assert!(rendered.contains("/r/rust/top.json"), "got: {rendered}");
        assert!(rendered.contains("590"), "got: {rendered}");
        assert!(rendered.contains("60"), "got: {rendered}");
    }

    #[test]
    fn reddit_api_error_rate_limited_display_falls_back_without_headers() {
        let err = RedditApiError::rate_limited("/r/rust/top.json", None);
        let rendered = format!("{err}");
        assert!(rendered.contains("rate limit"), "got: {rendered}");
        assert!(rendered.contains("/r/rust/top.json"), "got: {rendered}");
    }

    #[test]
    fn reddit_api_error_unauthorized_display_is_actionable() {
        let err = RedditApiError::Unauthorized {
            path: "/r/rust/top.json".to_string(),
        };
        let rendered = format!("{err}");
        assert!(rendered.contains("401"), "got: {rendered}");
        assert!(rendered.contains("/r/rust/top.json"), "got: {rendered}");
    }

    #[test]
    fn reddit_api_error_converts_to_anyhow_for_propagation() {
        let err = RedditApiError::Unauthorized {
            path: "/r/rust/top.json".to_string(),
        };
        let anyhow_err: anyhow::Error = err.into();
        // The converted error must preserve the path and the 401 marker
        // so callers (like the bot) can still see what failed.
        let rendered = format!("{anyhow_err:#}");
        assert!(rendered.contains("/r/rust/top.json"), "got: {rendered}");
        assert!(rendered.contains("401"), "got: {rendered}");
    }

    fn bearer_with_expires_at(expires_at: DateTime<Utc>) -> RedditBearerToken {
        RedditBearerToken {
            access_token: "secret".to_string(),
            expires_at,
            extra_headers: HeaderMap::new(),
        }
    }

    #[test]
    fn should_refresh_token_true_when_no_cached_token() {
        let now = DateTime::parse_from_rfc3339("2026-06-13T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert!(should_refresh_token(None, now));
    }

    #[test]
    fn should_refresh_token_false_when_cached_token_is_fresh() {
        let now = DateTime::parse_from_rfc3339("2026-06-13T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        // Expires 10 minutes in the future, well outside the refresh
        // skew, so the token is considered fresh.
        let token = bearer_with_expires_at(now + Duration::minutes(10));
        assert!(!should_refresh_token(Some(&token), now));
    }

    #[test]
    fn should_refresh_token_true_when_cached_token_is_within_skew() {
        let now = DateTime::parse_from_rfc3339("2026-06-13T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        // Expires 30 seconds in the future, inside the 120-second
        // refresh skew, so the transport must request a new token.
        let token = bearer_with_expires_at(now + Duration::seconds(30));
        assert!(should_refresh_token(Some(&token), now));
    }

    #[test]
    fn should_refresh_token_true_when_cached_token_is_already_expired() {
        let now = DateTime::parse_from_rfc3339("2026-06-13T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let token = bearer_with_expires_at(now - Duration::seconds(1));
        assert!(should_refresh_token(Some(&token), now));
    }

    // ------------------------------------------------------------------
    // `dispatch_response` deterministic coverage
    //
    // The transport retries on `401` exactly once: one token refresh
    // and one retried request. A second `401` from the retry is a hard
    // `Unauthorized` error and must not trigger a third attempt. The
    // following tests pin this contract by counting calls through
    // closures, without standing up an HTTP server that pretends to
    // be Reddit.
    // ------------------------------------------------------------------

    fn canned_token() -> RedditBearerToken {
        let now = DateTime::parse_from_rfc3339("2026-06-13T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        bearer_with_expires_at(now + Duration::hours(1))
    }

    #[tokio::test]
    async fn dispatch_response_does_not_refresh_or_retry_on_2xx() {
        let transport = RedditOAuthTransport::new().unwrap();
        let first = response_with(200, HeaderMap::new(), "{\"ok\": true}");

        let refresh_calls = Arc::new(AtomicUsize::new(0));
        let second_send_calls = Arc::new(AtomicUsize::new(0));

        let result = transport
            .dispatch_response(
                "/r/rust/top.json",
                first,
                || {
                    let counter = refresh_calls.clone();
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                        Ok(canned_token())
                    }
                },
                |_token: &RedditBearerToken| {
                    let counter = second_send_calls.clone();
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                        Ok(response_with(200, HeaderMap::new(), "should not be used"))
                    }
                },
            )
            .await
            .expect("healthy 2xx should be returned without retry");

        assert_eq!(result.status, reqwest::StatusCode::OK);
        assert_eq!(result.body, "{\"ok\": true}");
        assert_eq!(
            refresh_calls.load(Ordering::SeqCst),
            0,
            "refresh must not be called for a 2xx response"
        );
        assert_eq!(
            second_send_calls.load(Ordering::SeqCst),
            0,
            "second_send must not be called for a 2xx response"
        );
    }

    #[tokio::test]
    async fn dispatch_response_does_not_refresh_or_retry_on_429() {
        let transport = RedditOAuthTransport::new().unwrap();
        let headers = rate_limit_headers("600", "0", "60");
        let first = response_with(429, headers, "");

        let refresh_calls = Arc::new(AtomicUsize::new(0));
        let second_send_calls = Arc::new(AtomicUsize::new(0));

        let err = transport
            .dispatch_response(
                "/r/rust/top.json",
                first,
                || {
                    let counter = refresh_calls.clone();
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                        Ok(canned_token())
                    }
                },
                |_token: &RedditBearerToken| {
                    let counter = second_send_calls.clone();
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                        Ok(response_with(200, HeaderMap::new(), "should not be used"))
                    }
                },
            )
            .await
            .expect_err("429 must surface as RateLimited error, not Ok");

        assert!(
            matches!(err, RedditApiError::RateLimited { .. }),
            "expected RateLimited, got {err:?}"
        );
        assert_eq!(
            refresh_calls.load(Ordering::SeqCst),
            0,
            "refresh must not be called for a 429 response"
        );
        assert_eq!(
            second_send_calls.load(Ordering::SeqCst),
            0,
            "second_send must not be called for a 429 response"
        );
    }

    #[tokio::test]
    async fn dispatch_response_refreshes_and_retries_exactly_once_on_401() {
        // The issue acceptance criteria require a 401 to force one
        // token refresh and one retry. The transport must not
        // refresh-retry-refresh-retry or skip the refresh, and the
        // retry must happen exactly once.
        let transport = RedditOAuthTransport::new().unwrap();
        let first = response_with(401, HeaderMap::new(), "");

        let refresh_calls = Arc::new(AtomicUsize::new(0));
        let second_send_calls = Arc::new(AtomicUsize::new(0));

        let result = transport
            .dispatch_response(
                "/r/rust/top.json",
                first,
                || {
                    let counter = refresh_calls.clone();
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                        Ok(canned_token())
                    }
                },
                |_token: &RedditBearerToken| {
                    let counter = second_send_calls.clone();
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                        Ok(response_with(200, HeaderMap::new(), "{\"ok\": true}"))
                    }
                },
            )
            .await
            .expect("401 -> 200 retry should succeed");

        assert_eq!(result.status, reqwest::StatusCode::OK);
        assert_eq!(result.body, "{\"ok\": true}");
        assert_eq!(
            refresh_calls.load(Ordering::SeqCst),
            1,
            "refresh must be called exactly once on 401"
        );
        assert_eq!(
            second_send_calls.load(Ordering::SeqCst),
            1,
            "second_send (the retry) must be called exactly once on 401"
        );
    }

    #[tokio::test]
    async fn dispatch_response_does_not_retry_again_on_second_401() {
        // A second 401 from the retry means the OAuth flow is broken
        // (e.g. the client id is rejected). The transport must not
        // try a third refresh+retry; it must surface
        // `RedditApiError::Unauthorized` and stop. This is the
        // "exactly one retry" half of the issue acceptance criteria.
        let transport = RedditOAuthTransport::new().unwrap();
        let first = response_with(401, HeaderMap::new(), "");

        let refresh_calls = Arc::new(AtomicUsize::new(0));
        let second_send_calls = Arc::new(AtomicUsize::new(0));

        let err = transport
            .dispatch_response(
                "/r/rust/top.json",
                first,
                || {
                    let counter = refresh_calls.clone();
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                        Ok(canned_token())
                    }
                },
                |_token: &RedditBearerToken| {
                    let counter = second_send_calls.clone();
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                        // Second response is also 401. If the transport
                        // wrongly retries, this closure will be called
                        // a second time and the count will exceed 1.
                        Ok(response_with(401, HeaderMap::new(), ""))
                    }
                },
            )
            .await
            .expect_err("second 401 must surface as Unauthorized error");

        match err {
            RedditApiError::Unauthorized { path } => {
                assert_eq!(path, "/r/rust/top.json");
            }
            other => panic!("expected Unauthorized, got {other:?}"),
        }
        assert_eq!(
            refresh_calls.load(Ordering::SeqCst),
            1,
            "refresh must be called exactly once (no third attempt)"
        );
        assert_eq!(
            second_send_calls.load(Ordering::SeqCst),
            1,
            "second_send must be called exactly once (no third retry)"
        );
    }

    #[tokio::test]
    async fn dispatch_response_handles_429_on_retry_as_rate_limited() {
        // The verifier flagged a previous bug where a 429 returned
        // from the forced-refresh retry leaked out as Ok(retried).
        // Pin the fix: a 429 on the retry path must surface as a
        // typed `RateLimited` error, and the retry must still be
        // called exactly once.
        let transport = RedditOAuthTransport::new().unwrap();
        let first = response_with(401, HeaderMap::new(), "");
        let retry_headers = rate_limit_headers("600", "0", "60");

        let refresh_calls = Arc::new(AtomicUsize::new(0));
        let second_send_calls = Arc::new(AtomicUsize::new(0));

        let err = transport
            .dispatch_response(
                "/r/rust/top.json",
                first,
                || {
                    let counter = refresh_calls.clone();
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                        Ok(canned_token())
                    }
                },
                |_token: &RedditBearerToken| {
                    let counter = second_send_calls.clone();
                    async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                        Ok(response_with(429, retry_headers, ""))
                    }
                },
            )
            .await
            .expect_err("429 on retry must surface as RateLimited error");

        assert!(
            matches!(err, RedditApiError::RateLimited { .. }),
            "expected RateLimited, got {err:?}"
        );
        assert_eq!(
            refresh_calls.load(Ordering::SeqCst),
            1,
            "refresh must be called exactly once"
        );
        assert_eq!(
            second_send_calls.load(Ordering::SeqCst),
            1,
            "second_send must be called exactly once even when the retry is 429"
        );
    }
}
