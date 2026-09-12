use anyhow::{Context, Result, bail};
use log::warn;
use serde::Deserialize;
use std::future::Future;
use url::Url;

use crate::{db::Recordable, types::MediaKind};

const FXTWITTER_USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, thiserror::Error)]
#[error("could not retrieve X Tweet")]
pub struct RetrievalError {
    #[source]
    source: anyhow::Error,
}

pub fn retrieval_error(source: anyhow::Error) -> anyhow::Error {
    RetrievalError { source }.into()
}

pub fn is_retrieval_error(error: &anyhow::Error) -> bool {
    error
        .chain()
        .any(|cause| cause.downcast_ref::<RetrievalError>().is_some())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tweet {
    pub id: String,
    pub text: String,
    pub quote: Option<QuotedTweet>,
    pub media: Vec<TweetMedia>,
}

impl Recordable for Tweet {
    fn id(&self) -> &str {
        &self.id
    }

    fn title(&self) -> &str {
        &self.text
    }

    fn subreddit(&self) -> &str {
        "X Tweet"
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuotedTweet {
    pub author_handle: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TweetMedia {
    pub kind: MediaKind,
    pub url: String,
    pub width: u16,
    pub height: u16,
}

#[derive(Deserialize)]
struct FxTwitterResponse {
    status: Option<FxTweet>,
}

#[derive(Deserialize)]
struct FxTweet {
    id: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    raw_text: Option<FxRawText>,
    #[serde(default)]
    quote: Option<FxQuotedTweet>,
    #[serde(default)]
    media: FxMedia,
}

#[derive(Deserialize)]
struct FxRawText {
    text: String,
}

#[derive(Deserialize)]
struct FxAuthor {
    screen_name: String,
}

#[derive(Deserialize)]
struct FxQuotedTweet {
    #[serde(default)]
    text: String,
    #[serde(default)]
    raw_text: Option<FxRawText>,
    author: FxAuthor,
}

#[derive(Default, Deserialize)]
struct FxMedia {
    #[serde(default)]
    all: Vec<FxMediaItem>,
}

#[derive(Deserialize)]
struct FxMediaItem {
    #[serde(rename = "type")]
    kind: String,
    url: String,
    width: u16,
    height: u16,
}

/// Fetches and normalizes an X Tweet through the configured FxTwitter-compatible API.
pub async fn fetch(url: &Url, api_base_url: &str) -> Result<Tweet> {
    let id = tweet_id(url)?;
    let endpoint = status_endpoint(api_base_url, id)?;
    let client = reqwest::Client::builder()
        .user_agent(FXTWITTER_USER_AGENT)
        .build()
        .context("could not construct X Tweet HTTP client")?;
    let response = retry_once_if(
        || client.get(endpoint.clone()).send(),
        |error: &reqwest::Error| {
            if error.is_connect() {
                warn!("X Tweet source connection failed; retrying once: {error}");
                true
            } else {
                false
            }
        },
    )
    .await
    .context("could not request X Tweet data")?
    .error_for_status()
    .context("X Tweet source returned an error")?;
    let body = response
        .text()
        .await
        .context("could not read X Tweet source response")?;
    parse_response(&body)
}

async fn retry_once_if<T, E, Request, RequestFuture, Retryable>(
    request: Request,
    retryable: Retryable,
) -> std::result::Result<T, E>
where
    Request: Fn() -> RequestFuture,
    RequestFuture: Future<Output = std::result::Result<T, E>>,
    Retryable: Fn(&E) -> bool,
{
    match request().await {
        Ok(value) => Ok(value),
        Err(error) if retryable(&error) => request().await,
        Err(error) => Err(error),
    }
}

fn tweet_id(url: &Url) -> Result<&str> {
    let mut segments = url
        .path_segments()
        .context("X Tweet URL has no path segments")?
        .filter(|segment| !segment.is_empty());
    let _author = segments.next().context("X Tweet URL has no author")?;
    let marker = segments
        .next()
        .context("X Tweet URL has no status marker")?;
    let id = segments.next().context("X Tweet URL has no Tweet id")?;
    if marker != "status"
        || segments.next().is_some()
        || !id.chars().all(|character| character.is_ascii_digit())
    {
        bail!("X Tweet URL is not a canonical Tweet URL");
    }
    Ok(id)
}

fn status_endpoint(api_base_url: &str, id: &str) -> Result<Url> {
    let mut base = Url::parse(api_base_url).context("invalid X Tweet API base URL")?;
    if !base.path().ends_with('/') {
        let path = format!("{}/", base.path());
        base.set_path(&path);
    }
    base.join(&format!("2/status/{id}"))
        .context("could not construct X Tweet API URL")
}

fn parse_response(body: &str) -> Result<Tweet> {
    let response: FxTwitterResponse =
        serde_json::from_str(body).context("could not parse X Tweet source response")?;
    let tweet = response
        .status
        .context("X Tweet source returned no Tweet")?;
    normalize_tweet(tweet)
}

fn normalize_tweet(tweet: FxTweet) -> Result<Tweet> {
    let text = tweet
        .raw_text
        .map(|raw_text| raw_text.text)
        .unwrap_or(tweet.text);
    let quote = tweet.quote.map(|quote| QuotedTweet {
        author_handle: quote.author.screen_name,
        text: quote
            .raw_text
            .map(|raw_text| raw_text.text)
            .unwrap_or(quote.text),
    });
    let media = tweet
        .media
        .all
        .into_iter()
        .map(normalize_media)
        .collect::<Result<Vec<_>>>()?;

    Ok(Tweet {
        id: tweet.id,
        text,
        quote,
        media,
    })
}

fn normalize_media(media: FxMediaItem) -> Result<TweetMedia> {
    let kind = match media.kind.as_str() {
        "photo" => MediaKind::Photo,
        "video" | "gif" => MediaKind::Video,
        unsupported => bail!("X Tweet source returned unsupported media type {unsupported:?}"),
    };
    Ok(TweetMedia {
        kind,
        url: media.url,
        width: media.width,
        height: media.height,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::VecDeque,
        sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };

    #[derive(Debug, PartialEq, Eq)]
    enum TestRequestError {
        Connect,
        Other,
    }

    #[tokio::test]
    async fn retries_a_connect_failure_once() {
        let attempts = AtomicUsize::new(0);
        let outcomes = Mutex::new(VecDeque::from([
            Err(TestRequestError::Connect),
            Ok("response"),
        ]));

        let result = retry_once_if(
            || {
                attempts.fetch_add(1, Ordering::Relaxed);
                std::future::ready(
                    outcomes
                        .lock()
                        .expect("outcomes lock is available")
                        .pop_front()
                        .expect("each attempt has an outcome"),
                )
            },
            |error| *error == TestRequestError::Connect,
        )
        .await;

        assert_eq!(result, Ok("response"));
        assert_eq!(attempts.load(Ordering::Relaxed), 2);
    }

    #[tokio::test]
    async fn does_not_retry_a_second_connect_failure() {
        let attempts = AtomicUsize::new(0);
        let outcomes = Mutex::new(VecDeque::<std::result::Result<(), TestRequestError>>::from(
            [
                Err(TestRequestError::Connect),
                Err(TestRequestError::Connect),
            ],
        ));

        let result = retry_once_if(
            || {
                attempts.fetch_add(1, Ordering::Relaxed);
                std::future::ready(
                    outcomes
                        .lock()
                        .expect("outcomes lock is available")
                        .pop_front()
                        .expect("each attempt has an outcome"),
                )
            },
            |error| *error == TestRequestError::Connect,
        )
        .await;

        assert_eq!(result, Err(TestRequestError::Connect));
        assert_eq!(attempts.load(Ordering::Relaxed), 2);
    }

    #[tokio::test]
    async fn returns_non_connect_failure_without_retrying() {
        let attempts = AtomicUsize::new(0);
        let outcomes = Mutex::new(VecDeque::<std::result::Result<(), TestRequestError>>::from(
            [Err(TestRequestError::Other)],
        ));

        let result = retry_once_if(
            || {
                attempts.fetch_add(1, Ordering::Relaxed);
                std::future::ready(
                    outcomes
                        .lock()
                        .expect("outcomes lock is available")
                        .pop_front()
                        .expect("each attempt has an outcome"),
                )
            },
            |error| *error == TestRequestError::Connect,
        )
        .await;

        assert_eq!(result, Err(TestRequestError::Other));
        assert_eq!(attempts.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn normalizes_source_media_and_a_single_quote() {
        let tweet = parse_response(
            r#"{
                "status": {
                    "id": "123",
                    "text": "display source text",
                    "raw_text": {"text": "source https://t.co/media"},
                    "author": {"screen_name": "source"},
                    "media": {"all": [
                        {"type": "photo", "url": "https://media.example/one.jpg", "width": 100, "height": 200},
                        {"type": "video", "url": "https://media.example/two.mp4", "width": 300, "height": 400}
                    ]},
                    "quote": {
                        "id": "456",
                        "text": "display quote text",
                        "raw_text": {"text": "quoted https://t.co/quote"},
                        "author": {"screen_name": "quoted"},
                        "media": {"all": [{"type": "photo", "url": "https://media.example/ignored.jpg", "width": 5, "height": 6}]},
                        "quote": {
                            "id": "789",
                            "text": "nested quote",
                            "author": {"screen_name": "nested"},
                            "media": {"all": []}
                        }
                    }
                }
            }"#,
        )
        .expect("fixture is a valid FxTwitter response");

        assert_eq!(tweet.id, "123");
        assert_eq!(tweet.text, "source https://t.co/media");
        assert_eq!(
            tweet.quote,
            Some(QuotedTweet {
                author_handle: "quoted".to_owned(),
                text: "quoted https://t.co/quote".to_owned(),
            })
        );
        assert_eq!(
            tweet.media,
            vec![
                TweetMedia {
                    kind: MediaKind::Photo,
                    url: "https://media.example/one.jpg".to_owned(),
                    width: 100,
                    height: 200,
                },
                TweetMedia {
                    kind: MediaKind::Video,
                    url: "https://media.example/two.mp4".to_owned(),
                    width: 300,
                    height: 400,
                },
            ]
        );
    }

    #[test]
    fn rejects_unknown_media_without_dropping_it() {
        let error = parse_response(
            r#"{
                "status": {
                    "id": "123",
                    "text": "text",
                    "author": {"screen_name": "source"},
                    "media": {"all": [{"type": "mosaic_photo", "url": "https://media.example/mosaic.jpg", "width": 1, "height": 1}]}
                }
            }"#,
        )
        .expect_err("unsupported media must reject the full Tweet");

        assert!(error.to_string().contains("unsupported media type"));
    }

    #[test]
    fn builds_a_status_endpoint_from_a_configured_base_url() {
        assert_eq!(
            status_endpoint("https://x-source.example/api", "123")
                .unwrap()
                .as_str(),
            "https://x-source.example/api/2/status/123"
        );
    }
}
