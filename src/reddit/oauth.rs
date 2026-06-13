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

impl RedditOAuthTransport {
    pub fn new() -> Result<Self> {
        Self::with_base_urls(AUTH_BASE_URL, OAUTH_BASE_URL)
    }

    pub(crate) fn with_base_urls(auth_base_url: &str, oauth_base_url: &str) -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .user_agent(ANDROID_USER_AGENT)
                .build()?,
            auth_base_url: Url::parse(auth_base_url)?,
            oauth_base_url: Url::parse(oauth_base_url)?,
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
        let token = self.bearer_token().await?;
        let mut url = self.oauth_url(path)?;
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("raw_json", "1");
            for (key, value) in query {
                pairs.append_pair(key, value);
            }
        }

        let mut request = self.client.get(url).headers(self.app_headers()?);
        request = request.bearer_auth(&token.access_token);
        for (key, value) in &token.extra_headers {
            request = request.header(key, value);
        }

        Ok(request
            .send()
            .await?
            .error_for_status()?
            .json::<T>()
            .await?)
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
    use serde_json::Value;
    use std::{
        collections::HashMap,
        io::{BufRead, BufReader, Read, Write},
        net::{TcpListener, TcpStream},
        sync::mpsc,
        thread,
    };

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
    fn authenticated_json_request_uses_token_and_app_headers() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .unwrap();

        runtime.block_on(async {
            let server = TestServer::start();
            let transport =
                RedditOAuthTransport::with_base_urls(&server.base_url(), &server.base_url())
                    .unwrap();

            let json: Value = transport
                .get_json("/r/rust/top.json", &[("limit", "1".to_owned())])
                .await
                .unwrap();

            assert_eq!(json["ok"], true);
            let second_json: Value = transport
                .get_json("/r/rust/top.json", &[("limit", "2".to_owned())])
                .await
                .unwrap();

            assert_eq!(second_json["ok"], true);
            let requests = server.join();
            assert_eq!(requests.len(), 3);

            let token_request = &requests[0];
            assert_eq!(token_request.method, "POST");
            assert_eq!(token_request.path, "/auth/v2/oauth/access-token/loid");
            assert!(token_request.headers["authorization"].starts_with("Basic "));
            assert_eq!(token_request.headers["user-agent"], ANDROID_USER_AGENT);
            assert_eq!(token_request.headers["x-reddit-compression"], "1");
            assert!(token_request.body.contains("\"scopes\""));

            let api_request = &requests[1];
            assert_eq!(api_request.method, "GET");
            assert_eq!(api_request.path, "/r/rust/top.json");
            assert_eq!(api_request.query, "raw_json=1&limit=1");
            assert_eq!(api_request.headers["authorization"], "Bearer secret-token");
            assert_eq!(api_request.headers["user-agent"], ANDROID_USER_AGENT);
            assert_eq!(api_request.headers["x-reddit-loid"], "loid-123");
            assert_eq!(api_request.headers["x-reddit-session"], "session-456");

            let cached_api_request = &requests[2];
            assert_eq!(cached_api_request.method, "GET");
            assert_eq!(cached_api_request.query, "raw_json=1&limit=2");
            assert_eq!(
                cached_api_request.headers["authorization"],
                "Bearer secret-token"
            );
        });
    }

    #[test]
    fn default_authenticated_url_uses_oauth_reddit_host() {
        let transport = RedditOAuthTransport::new().unwrap();

        let url = transport.oauth_url("/r/rust/top.json").unwrap();

        assert_eq!(url.as_str(), "https://oauth.reddit.com/r/rust/top.json");
    }

    struct TestServer {
        base_url: String,
        handle: thread::JoinHandle<Vec<RecordedRequest>>,
    }

    impl TestServer {
        fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let base_url = format!("http://{}", listener.local_addr().unwrap());
            let (ready_tx, ready_rx) = mpsc::channel();
            let handle = thread::spawn(move || {
                ready_tx.send(()).unwrap();
                let mut requests = Vec::new();
                for _ in 0..3 {
                    let (stream, _) = listener.accept().unwrap();
                    let request = read_request(&stream);
                    if request.path == "/auth/v2/oauth/access-token/loid" {
                        write_response(
                            stream,
                            &[
                                ("content-type", "application/json"),
                                ("x-reddit-loid", "loid-123"),
                                ("x-reddit-session", "session-456"),
                            ],
                            r#"{"access_token":"secret-token","expires_in":86400}"#,
                        );
                    } else {
                        write_response(
                            stream,
                            &[("content-type", "application/json")],
                            r#"{"ok":true}"#,
                        );
                    }
                    requests.push(request);
                }
                requests
            });
            ready_rx.recv().unwrap();
            Self { base_url, handle }
        }

        fn base_url(&self) -> String {
            self.base_url.clone()
        }

        fn join(self) -> Vec<RecordedRequest> {
            self.handle.join().unwrap()
        }
    }

    #[derive(Debug)]
    struct RecordedRequest {
        method: String,
        path: String,
        query: String,
        headers: HashMap<String, String>,
        body: String,
    }

    fn read_request(stream: &TcpStream) -> RecordedRequest {
        let mut reader = BufReader::new(stream);
        let mut first_line = String::new();
        reader.read_line(&mut first_line).unwrap();
        let mut parts = first_line.split_whitespace();
        let method = parts.next().unwrap().to_owned();
        let target = parts.next().unwrap().to_owned();
        let (path, query) = target
            .split_once('?')
            .map(|(path, query)| (path.to_owned(), query.to_owned()))
            .unwrap_or((target, String::new()));

        let mut headers = HashMap::new();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
            let (name, value) = line.trim_end().split_once(": ").unwrap();
            headers.insert(name.to_ascii_lowercase(), value.to_owned());
        }

        let content_length = headers
            .get("content-length")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        let mut body = vec![0; content_length];
        reader.read_exact(&mut body).unwrap();

        RecordedRequest {
            method,
            path,
            query,
            headers,
            body: String::from_utf8(body).unwrap(),
        }
    }

    fn write_response(mut stream: TcpStream, headers: &[(&str, &str)], body: &str) {
        write!(
            stream,
            "HTTP/1.1 200 OK\r\ncontent-length: {}\r\n",
            body.len()
        )
        .unwrap();
        for (name, value) in headers {
            write!(stream, "{name}: {value}\r\n").unwrap();
        }
        write!(stream, "\r\n{body}").unwrap();
    }
}
