use super::*;
use anyhow::{Context, Result};
use log::info;
use thiserror::Error;
use url::Url;

static REDDIT_BASE_URL: &str = "https://www.reddit.com";
const USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

fn get_base_url() -> Url {
    Url::parse(REDDIT_BASE_URL).unwrap()
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
    info!("getting top posts for /r/{subreddit} limit={limit} time={time:?}");
    let url = get_base_url()
        .join(&format!("/r/{subreddit}/top.json"))
        .unwrap();
    let client = create_client().build()?;
    let res = client
        .get(url)
        .query(&[
            ("limit", &limit.to_string()),
            ("t", &format!("{time:?}").to_lowercase()),
        ])
        .send()
        .await?
        .error_for_status()?
        .json::<ListingResponse>()
        .await?;
    let posts = res.data.children.into_iter().map(|e| e.data).collect();
    Ok(posts)
}

fn create_client() -> reqwest::ClientBuilder {
    reqwest::Client::builder().user_agent(USER_AGENT)
}

pub async fn get_link(link_id: &str) -> Result<Post> {
    info!("getting link id {link_id}");
    let url = get_base_url().join("/api/info.json")?;
    let client = create_client().build()?;
    let res = client
        .get(url)
        .query(&[("id", &format!("t3_{link_id}"))])
        .send()
        .await?
        .json::<ListingResponse>()
        .await?;

    res.data
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
