use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use reqwest::{
    Url,
    header::{HeaderMap, HeaderName, HeaderValue},
};
use serde::Deserialize;
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
/// status and the response body as text. Callers use this to map
/// non-success statuses to typed errors (for example, subreddit about
/// validation) without going through `get_json`'s `error_for_status` path.
#[derive(Debug)]
pub struct RedditRawResponse {
    pub status: reqwest::StatusCode,
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
        {
            let guard = self.token.lock().await;
            if let Some(token) = guard.as_ref().filter(|token| token.is_fresh_at(now)) {
                return Ok(token.clone());
            }
        }

        let token = self.request_bearer_token().await?;
        *self.token.lock().await = Some(token.clone());
        Ok(token)
    }

    pub async fn get_json<T>(&self, path: &str, query: &[(&str, String)]) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        let response = self.send_authenticated(path, query).await?;
        if !response.status.is_success() {
            anyhow::bail!(
                "Reddit API returned non-success status {} for path {}",
                response.status,
                path
            );
        }
        serde_json::from_str(&response.body).with_context(|| {
            format!(
                "failed to parse Reddit API response for path {path}: {}",
                response.body
            )
        })
    }

    /// Send an authenticated GET to the OAuth API and return the raw response
    /// (status + body text) without interpreting it. Callers that need to map
    /// non-success statuses to typed errors (for example, subreddit about
    /// validation) use this instead of `get_json`.
    pub async fn send_authenticated(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<RedditRawResponse> {
        let token = self.bearer_token().await?;
        let url = self.build_authenticated_url(path, query)?;
        let mut request = self.client.get(url).headers(self.app_headers()?);
        request = request.bearer_auth(&token.access_token);
        for (key, value) in &token.extra_headers {
            request = request.header(key, value);
        }

        let response = request.send().await?;
        let status = response.status();
        let body = response.text().await?;
        Ok(RedditRawResponse { status, body })
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
