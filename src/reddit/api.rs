use super::oauth::RedditOAuthTransport;
use super::*;
use anyhow::{Context, Result};
use log::info;
use std::sync::OnceLock;
use thiserror::Error;
use url::Url;

static REDDIT_BASE_URL: &str = "https://www.reddit.com";

static TRANSPORT: OnceLock<RedditOAuthTransport> = OnceLock::new();

fn get_transport() -> Result<&'static RedditOAuthTransport> {
    // The transport caches its bearer token, so build it once and reuse it.
    // If two threads race, only the first `set` wins; the loser discards its
    // (uncached) transport and uses the winner's via the final `get`.
    if let Some(transport) = TRANSPORT.get() {
        return Ok(transport);
    }
    let transport = RedditOAuthTransport::new()?;
    let _ = TRANSPORT.set(transport);
    Ok(TRANSPORT.get().expect("transport just set above"))
}

pub fn format_url_from_path(path: &str, base_url: Option<&str>) -> String {
    let base_url = match base_url {
        Some(u) => u,
        None => REDDIT_BASE_URL,
    };
    format!("{base_url}{path}")
}

pub fn to_old_reddit_url(url: &str) -> String {
    // If this fails it's bug
    let mut url = Url::parse(url).unwrap();
    url.set_host(Some("old.reddit.com")).unwrap();
    url.to_string()
}

pub fn format_subreddit_url(subreddit: &str, base_url: Option<&str>) -> String {
    format_url_from_path(&format!("/r/{subreddit}"), base_url)
}

pub async fn get_subreddit_top_posts(
    subreddit: &str,
    limit: u32,
    time: &TopPostsTimePeriod,
) -> Result<Vec<Post>> {
    info!("getting top posts for /r/{subreddit} limit={limit} time={time}");
    let transport = get_transport()?;
    let path = format!("/r/{subreddit}/top.json");
    let query = [("limit", limit.to_string()), ("t", time.to_string())];
    let listing = transport.get_json::<ListingResponse>(&path, &query).await?;
    Ok(listing.data.children.into_iter().map(|e| e.data).collect())
}

pub async fn get_link(link_id: &str) -> Result<Post> {
    get_link_via(get_transport()?, link_id).await
}

pub(crate) async fn get_link_via(transport: &RedditOAuthTransport, link_id: &str) -> Result<Post> {
    info!("getting link id {link_id}");
    let path = "/api/info.json";
    let query = [("id", format!("t3_{link_id}"))];
    let listing = transport.get_json::<ListingResponse>(path, &query).await?;

    listing
        .data
        .children
        .into_iter()
        .map(|e| e.data)
        .next()
        .context("no post in response")
}

#[allow(clippy::large_enum_variant)]
#[derive(Error, Debug)]
pub enum SubredditAboutError {
    #[error("no such subreddit")]
    NoSuchSubreddit,
    #[error("subreddit is inaccessible: {reason}")]
    Inaccessible { reason: String },
    #[error("failed to parse subreddit about response")]
    Parse(#[from] serde_json::Error),
    #[error(transparent)]
    Transport(anyhow::Error),
}

pub async fn get_subreddit_about(subreddit: &str) -> Result<SubredditAbout, SubredditAboutError> {
    info!("getting subreddit about for /r/{subreddit}");
    let transport = get_transport().map_err(SubredditAboutError::Transport)?;
    let path = format!("/r/{subreddit}/about.json");
    let query: [(&str, String); 0] = [];
    let response = transport
        .send_authenticated(&path, &query)
        .await
        .map_err(|e| SubredditAboutError::Transport(anyhow::Error::from(e)))?;
    parse_subreddit_about_response(response.status, &response.body)
}

/// Parse a Reddit subreddit about response into a `SubredditAbout` or a typed
/// validation error. This is a pure function so it can be tested with fixture
/// JSON without making any network requests.
///
/// Reddit returns:
/// - `200 OK` with a `SubredditAboutResponse` body for valid subreddits
/// - `302 FOUND` (legacy) for nonexistent subreddits that redirect to search
/// - `404 Not Found` with a structured JSON error for nonexistent subreddits
/// - `403 Forbidden` with a `reason` field (`private`, `banned`, `gated`,
///   `quarantined`) for inaccessible subreddits
/// - Other non-success statuses are treated as inaccessible with the parsed
///   reason (or `"unknown"` if the body has no `reason` field)
fn parse_subreddit_about_response(
    status: reqwest::StatusCode,
    body: &str,
) -> Result<SubredditAbout, SubredditAboutError> {
    match status {
        reqwest::StatusCode::OK => {
            let response: SubredditAboutResponse = serde_json::from_str(body)?;
            Ok(response.data)
        }
        reqwest::StatusCode::FOUND | reqwest::StatusCode::NOT_FOUND => {
            Err(SubredditAboutError::NoSuchSubreddit)
        }
        _ => {
            let reason = parse_reddit_error_reason(body).unwrap_or_else(|| "unknown".to_string());
            Err(SubredditAboutError::Inaccessible { reason })
        }
    }
}

/// Extract the `reason` field from a Reddit JSON error body, if present.
/// Reddit error bodies look like `{"error": 403, "reason": "banned", "message": "..."}`.
fn parse_reddit_error_reason(body: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("reason").and_then(|r| r.as_str().map(str::to_string)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image_post() -> &'static str {
        r#"{
            "id": "img1",
            "subreddit": "pics",
            "title": "An image",
            "is_video": false,
            "permalink": "/r/pics/comments/img1/an_image/",
            "url": "https://i.redd.it/example.jpg",
            "post_hint": "image",
            "is_self": false,
            "is_gallery": false
        }"#
    }

    fn hosted_video_post() -> &'static str {
        r#"{
            "id": "vid1",
            "subreddit": "videos",
            "title": "A hosted video",
            "is_video": true,
            "permalink": "/r/videos/comments/vid1/a_hosted_video/",
            "url": "https://v.redd.it/example/DASH_720.mp4",
            "post_hint": "hosted:video",
            "is_self": false,
            "is_gallery": false
        }"#
    }

    fn external_link_post() -> &'static str {
        r#"{
            "id": "lnk1",
            "subreddit": "rust",
            "title": "An external link",
            "is_video": false,
            "permalink": "/r/rust/comments/lnk1/an_external_link/",
            "url": "https://example.com/article",
            "post_hint": "link",
            "is_self": false,
            "is_gallery": false
        }"#
    }

    fn self_text_post() -> &'static str {
        r#"{
            "id": "sft1",
            "subreddit": "rust",
            "title": "A self post",
            "is_video": false,
            "permalink": "/r/rust/comments/sft1/a_self_post/",
            "url": "https://www.reddit.com/r/rust/comments/sft1/a_self_post/",
            "post_hint": null,
            "is_self": true,
            "is_gallery": false
        }"#
    }

    fn gallery_post() -> &'static str {
        r#"{
            "id": "gal1",
            "subreddit": "pics",
            "title": "A gallery",
            "is_video": false,
            "permalink": "/r/pics/comments/gal1/a_gallery/",
            "url": "https://www.reddit.com/r/pics/comments/gal1/a_gallery/",
            "post_hint": null,
            "is_self": false,
            "is_gallery": true,
            "gallery_data": {
                "items": [
                    { "media_id": "img1" },
                    { "media_id": "img2" }
                ]
            },
            "media_metadata": {
                "img1": { "s": { "x": 100, "y": 100, "u": "https://i.redd.it/img1.jpg" } },
                "img2": { "s": { "x": 200, "y": 200, "u": "https://i.redd.it/img2.jpg" } }
            }
        }"#
    }

    fn listing(children: &[&str]) -> String {
        // Each child in a Reddit `Listing` is `{"kind": "t3", "data": <post>}`.
        // We only deserialize `data` (see `ListingItem`), so the wrapper around
        // each child is what makes the fixture round-trip into `ListingItem`.
        let wrapped = children
            .iter()
            .map(|c| format!(r#"{{"data": {c}}}"#))
            .collect::<Vec<_>>()
            .join(", ");
        format!(r#"{{ "data": {{ "children": [{wrapped}] }} }}"#)
    }

    fn posts_from(json: &str) -> Vec<Post> {
        let listing: ListingResponse = serde_json::from_str(json)
            .expect("listing fixture should deserialize as ListingResponse");
        listing.data.children.into_iter().map(|c| c.data).collect()
    }

    #[test]
    fn oauth_listing_classifies_image_post() {
        let posts = posts_from(&listing(&[image_post()]));
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].id, "img1");
        assert_eq!(posts[0].post_type, PostType::Image);
        assert_eq!(posts[0].post_hint.as_deref(), Some("image"));
    }

    #[test]
    fn oauth_listing_classifies_hosted_video_post() {
        let posts = posts_from(&listing(&[hosted_video_post()]));
        assert_eq!(posts[0].id, "vid1");
        assert_eq!(posts[0].post_type, PostType::Video);
        assert_eq!(posts[0].post_hint.as_deref(), Some("hosted:video"));
    }

    #[test]
    fn oauth_listing_classifies_external_link_post() {
        let posts = posts_from(&listing(&[external_link_post()]));
        assert_eq!(posts[0].id, "lnk1");
        assert_eq!(posts[0].post_type, PostType::Link);
        assert_eq!(posts[0].post_hint.as_deref(), Some("link"));
    }

    #[test]
    fn oauth_listing_classifies_self_text_post() {
        let posts = posts_from(&listing(&[self_text_post()]));
        assert_eq!(posts[0].id, "sft1");
        assert_eq!(posts[0].post_type, PostType::SelfText);
    }

    #[test]
    fn oauth_listing_classifies_gallery_post_with_metadata() {
        let posts = posts_from(&listing(&[gallery_post()]));
        let post = &posts[0];
        assert_eq!(post.id, "gal1");
        assert_eq!(post.post_type, PostType::Gallery);

        let gallery_data = post
            .gallery_data
            .as_ref()
            .expect("gallery_data must be present for gallery post");
        assert_eq!(gallery_data.items.len(), 2);
        assert_eq!(gallery_data.items[0].media_id, "img1");
        assert_eq!(gallery_data.items[1].media_id, "img2");

        let media_metadata = post
            .media_metadata
            .as_ref()
            .expect("media_metadata must be present for gallery post");
        assert_eq!(media_metadata.len(), 2);
        let img1 = &media_metadata["img1"];
        let s = img1.s.as_ref().expect("media metadata must have s");
        assert_eq!(s.x, 100);
        assert_eq!(s.y, 100);
        assert_eq!(s.url, "https://i.redd.it/img1.jpg");
        let img2 = &media_metadata["img2"];
        let s = img2.s.as_ref().expect("media metadata must have s");
        assert_eq!(s.url, "https://i.redd.it/img2.jpg");
    }

    #[test]
    fn oauth_listing_classifies_all_post_types_in_one_response() {
        let posts = posts_from(&listing(&[
            image_post(),
            hosted_video_post(),
            external_link_post(),
            self_text_post(),
            gallery_post(),
        ]));
        assert_eq!(posts.len(), 5);
        assert_eq!(posts[0].post_type, PostType::Image);
        assert_eq!(posts[1].post_type, PostType::Video);
        assert_eq!(posts[2].post_type, PostType::Link);
        assert_eq!(posts[3].post_type, PostType::SelfText);
        assert_eq!(posts[4].post_type, PostType::Gallery);
    }

    #[test]
    fn oauth_listing_round_trips_post_fields() {
        let posts = posts_from(&listing(&[image_post()]));
        let post = &posts[0];
        assert_eq!(post.id, "img1");
        assert_eq!(post.subreddit, "pics");
        assert_eq!(post.title, "An image");
        assert_eq!(post.permalink, "/r/pics/comments/img1/an_image/");
        assert_eq!(post.url, "https://i.redd.it/example.jpg");
    }

    fn valid_about_body() -> &'static str {
        r#"{
            "kind": "t5",
            "data": {
                "display_name": "rust"
            }
        }"#
    }

    #[test]
    fn parse_about_response_200_returns_subreddit_about() {
        let result =
            parse_subreddit_about_response(reqwest::StatusCode::OK, valid_about_body()).unwrap();
        assert_eq!(result.display_name, "rust");
    }

    #[test]
    fn parse_about_response_302_maps_to_no_such_subreddit() {
        let err = parse_subreddit_about_response(
            reqwest::StatusCode::FOUND,
            r#"{"error": 302, "message": "Found"}"#,
        )
        .unwrap_err();
        assert!(matches!(err, SubredditAboutError::NoSuchSubreddit));
    }

    #[test]
    fn parse_about_response_404_maps_to_no_such_subreddit() {
        let err = parse_subreddit_about_response(
            reqwest::StatusCode::NOT_FOUND,
            r#"{"error": 404, "message": "Not Found"}"#,
        )
        .unwrap_err();
        assert!(matches!(err, SubredditAboutError::NoSuchSubreddit));
    }

    #[test]
    fn parse_about_response_403_maps_known_reasons_to_inaccessible() {
        let cases = [
            (
                "private",
                r#"{"error": 403, "reason": "private", "message": "Forbidden"}"#,
            ),
            (
                "banned",
                r#"{"error": 403, "reason": "banned", "message": "This community is banned"}"#,
            ),
            (
                "gated",
                r#"{"error": 403, "reason": "gated", "message": "This community is gated"}"#,
            ),
            (
                "quarantined",
                r#"{"error": 403, "reason": "quarantined", "message": "This community is quarantined"}"#,
            ),
        ];
        for (expected_reason, body) in cases {
            let err =
                parse_subreddit_about_response(reqwest::StatusCode::FORBIDDEN, body).unwrap_err();
            match err {
                SubredditAboutError::Inaccessible { reason } => {
                    assert_eq!(
                        reason, expected_reason,
                        "failed for reason={expected_reason}"
                    );
                }
                other => {
                    panic!("expected Inaccessible for reason={expected_reason}, got {other:?}")
                }
            }
        }
    }

    #[test]
    fn parse_about_response_403_without_reason_maps_to_inaccessible_unknown() {
        let err = parse_subreddit_about_response(
            reqwest::StatusCode::FORBIDDEN,
            r#"{"error": 403, "message": "Forbidden"}"#,
        )
        .unwrap_err();
        match err {
            SubredditAboutError::Inaccessible { reason } => {
                assert_eq!(reason, "unknown");
            }
            other => panic!("expected Inaccessible, got {other:?}"),
        }
    }

    #[test]
    fn parse_about_response_403_with_non_json_body_maps_to_inaccessible_unknown() {
        let err = parse_subreddit_about_response(reqwest::StatusCode::FORBIDDEN, "not json at all")
            .unwrap_err();
        match err {
            SubredditAboutError::Inaccessible { reason } => {
                assert_eq!(reason, "unknown");
            }
            other => panic!("expected Inaccessible, got {other:?}"),
        }
    }

    #[test]
    fn parse_about_response_500_with_reason_maps_to_inaccessible() {
        let err = parse_subreddit_about_response(
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            r#"{"error": 500, "reason": "server_error", "message": "Internal Server Error"}"#,
        )
        .unwrap_err();
        match err {
            SubredditAboutError::Inaccessible { reason } => {
                assert_eq!(reason, "server_error");
            }
            other => panic!("expected Inaccessible, got {other:?}"),
        }
    }

    #[test]
    fn parse_about_response_200_with_invalid_json_returns_parse_error() {
        let err = parse_subreddit_about_response(reqwest::StatusCode::OK, "this is not valid json")
            .unwrap_err();
        assert!(
            matches!(err, SubredditAboutError::Parse(_)),
            "expected Parse error, got {err:?}"
        );
    }

    #[test]
    fn parse_about_response_200_with_missing_data_field_returns_parse_error() {
        // Valid JSON but missing the required `data` field.
        let err = parse_subreddit_about_response(reqwest::StatusCode::OK, r#"{"kind": "t5"}"#)
            .unwrap_err();
        assert!(
            matches!(err, SubredditAboutError::Parse(_)),
            "expected Parse error, got {err:?}"
        );
    }

    #[test]
    fn reddit_error_reason_extracts_known_reasons() {
        for reason in ["private", "banned", "gated", "quarantined"] {
            let body = format!(r#"{{"error": 403, "reason": "{reason}", "message": "x"}}"#);
            assert_eq!(
                parse_reddit_error_reason(&body),
                Some(reason.to_string()),
                "failed for reason={reason}",
            );
        }
    }

    #[test]
    fn reddit_error_reason_returns_none_when_missing() {
        assert_eq!(
            parse_reddit_error_reason(r#"{"error": 403, "message": "Forbidden"}"#),
            None
        );
        assert_eq!(parse_reddit_error_reason("not json"), None);
        assert_eq!(parse_reddit_error_reason(""), None);
    }

    #[test]
    fn subreddit_about_error_displays_reason() {
        let err = SubredditAboutError::Inaccessible {
            reason: "private".to_string(),
        };
        assert_eq!(format!("{err}"), "subreddit is inaccessible: private");
    }
}
