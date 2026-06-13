use super::oauth::RedditOAuthTransport;
use super::*;
use anyhow::{Context, Result};
use log::info;
use std::sync::OnceLock;
use thiserror::Error;
use url::Url;

static REDDIT_BASE_URL: &str = "https://www.reddit.com";
const USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

fn get_base_url() -> Url {
    Url::parse(REDDIT_BASE_URL).unwrap()
}

fn create_client() -> reqwest::ClientBuilder {
    reqwest::Client::builder().user_agent(USER_AGENT)
}

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
    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),
    #[error(transparent)]
    UrlParseError(#[from] url::ParseError),
    #[error(transparent)]
    IO(#[from] std::io::Error),
}

pub async fn get_subreddit_about(subreddit: &str) -> Result<SubredditAbout, SubredditAboutError> {
    info!("getting subreddit about for /r/{subreddit}");
    let client = create_client()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let url = get_base_url().join(&format!("/r/{subreddit}/about.json"))?;
    let res = client.get(url).send().await?.error_for_status()?;

    match res.status() {
        reqwest::StatusCode::FOUND => Err(SubredditAboutError::NoSuchSubreddit),
        _ => {
            let data = res.json::<SubredditAboutResponse>().await?.data;
            Ok(data)
        }
    }
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

    fn empty_listing() -> String {
        r#"{"data": {"children": []}}"#.to_owned()
    }

    fn start_info_json_server(
        listing_json: &str,
    ) -> (crate::reddit::test_server::TestServer, RedditOAuthTransport) {
        use crate::reddit::test_server::{TestResponse, TestServer, TestServerConfig};
        use std::collections::HashMap;

        let mut responses_by_path = HashMap::new();
        responses_by_path.insert(
            "/api/info.json".to_owned(),
            TestResponse::json(listing_json),
        );
        let server = TestServer::start(TestServerConfig {
            token_response: Some(TestResponse::json_with_session(
                r#"{"access_token":"secret-token","expires_in":86400}"#,
                "loid-123",
                "session-456",
            )),
            responses_by_path,
            default_response: None,
            stop_after: 1,
        });
        let transport =
            RedditOAuthTransport::with_base_urls(&server.base_url(), &server.base_url()).unwrap();
        (server, transport)
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .unwrap()
    }

    #[test]
    fn get_link_uses_oauth_info_json_with_bearer_and_raw_json() {
        let rt = runtime();
        rt.block_on(async {
            let (server, transport) = start_info_json_server(&listing(&[image_post()]));
            let post = get_link_via(&transport, "img1").await.unwrap();
            assert_eq!(post.id, "img1");

            let requests = server.join();
            assert_eq!(requests.len(), 2);

            let token_request = &requests[0];
            assert_eq!(token_request.method, "POST");
            assert_eq!(token_request.path, "/auth/v2/oauth/access-token/loid");

            let api_request = &requests[1];
            assert_eq!(api_request.method, "GET");
            assert_eq!(api_request.path, "/api/info.json");
            assert!(
                api_request.query.contains("id=t3_img1"),
                "expected id=t3_img1 in query, got {}",
                api_request.query
            );
            assert!(
                api_request.query.contains("raw_json=1"),
                "expected raw_json=1 in query, got {}",
                api_request.query
            );
            assert_eq!(api_request.headers["authorization"], "Bearer secret-token");
        });
    }

    #[test]
    fn get_link_via_oauth_returns_image_post() {
        let rt = runtime();
        rt.block_on(async {
            let (_server, transport) = start_info_json_server(&listing(&[image_post()]));
            let post = get_link_via(&transport, "img1").await.unwrap();
            assert_eq!(post.id, "img1");
            assert_eq!(post.post_type, PostType::Image);
            assert_eq!(post.post_hint.as_deref(), Some("image"));
            assert_eq!(post.subreddit, "pics");
            assert_eq!(post.title, "An image");
            assert_eq!(post.url, "https://i.redd.it/example.jpg");
        });
    }

    #[test]
    fn get_link_via_oauth_returns_hosted_video_post() {
        let rt = runtime();
        rt.block_on(async {
            let (_server, transport) = start_info_json_server(&listing(&[hosted_video_post()]));
            let post = get_link_via(&transport, "vid1").await.unwrap();
            assert_eq!(post.id, "vid1");
            assert_eq!(post.post_type, PostType::Video);
            assert_eq!(post.post_hint.as_deref(), Some("hosted:video"));
            assert!(post.url.contains("DASH_720.mp4"));
        });
    }

    #[test]
    fn get_link_via_oauth_returns_external_link_post() {
        let rt = runtime();
        rt.block_on(async {
            let (_server, transport) = start_info_json_server(&listing(&[external_link_post()]));
            let post = get_link_via(&transport, "lnk1").await.unwrap();
            assert_eq!(post.id, "lnk1");
            assert_eq!(post.post_type, PostType::Link);
            assert_eq!(post.post_hint.as_deref(), Some("link"));
            assert_eq!(post.url, "https://example.com/article");
        });
    }

    #[test]
    fn get_link_via_oauth_returns_self_text_post() {
        let rt = runtime();
        rt.block_on(async {
            let (_server, transport) = start_info_json_server(&listing(&[self_text_post()]));
            let post = get_link_via(&transport, "sft1").await.unwrap();
            assert_eq!(post.id, "sft1");
            assert_eq!(post.post_type, PostType::SelfText);
        });
    }

    #[test]
    fn get_link_via_oauth_returns_gallery_post_with_metadata() {
        let rt = runtime();
        rt.block_on(async {
            let (_server, transport) = start_info_json_server(&listing(&[gallery_post()]));
            let post = get_link_via(&transport, "gal1").await.unwrap();
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
            assert_eq!(
                media_metadata["img1"].s.as_ref().unwrap().url,
                "https://i.redd.it/img1.jpg"
            );
            assert_eq!(
                media_metadata["img2"].s.as_ref().unwrap().url,
                "https://i.redd.it/img2.jpg"
            );
        });
    }

    #[test]
    fn get_link_via_oauth_errors_when_no_post_in_response() {
        let rt = runtime();
        rt.block_on(async {
            let (_server, transport) = start_info_json_server(&empty_listing());
            let err = get_link_via(&transport, "missing").await.unwrap_err();
            assert!(
                format!("{err}").contains("no post in response"),
                "expected missing-post error, got {err:?}"
            );
        });
    }
}
