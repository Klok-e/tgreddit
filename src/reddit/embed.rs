use anyhow::{Context, Result};
use scraper::{Html, Selector};
use std::{sync::LazyLock, time::Duration};
use url::Url;

use super::{Post, PostType};

static TITLE_LINK: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("a#embed-title[href]").unwrap());
static VIDEO_PLAYER: LazyLock<Selector> =
    LazyLock::new(|| Selector::parse("shreddit-player[post-id][src]").unwrap());

pub(super) async fn fetch_video(subreddit: &str, link_id: &str) -> Result<(Post, Url)> {
    let mut embed_url = Url::parse("https://embed.reddit.com/")?;
    embed_url
        .path_segments_mut()
        .expect("Reddit embed base URL supports path segments")
        .extend(["r", subreddit, "comments", link_id, ""]);
    let html = reqwest::Client::builder()
        .user_agent("Mozilla/5.0")
        .timeout(Duration::from_secs(15))
        .build()?
        .get(embed_url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    parse_video(&html, link_id)
}

fn parse_video(html: &str, link_id: &str) -> Result<(Post, Url)> {
    let document = Html::parse_document(html);
    let fullname = format!("t3_{link_id}");
    let playlist = document
        .select(&VIDEO_PLAYER)
        .filter(|player| player.attr("post-id") == Some(fullname.as_str()))
        .filter_map(|player| Url::parse(player.attr("src")?).ok())
        .find(|url| {
            url.scheme() == "https"
                && url.host_str() == Some("v.redd.it")
                && url.port().is_none()
                && url.username().is_empty()
                && url.password().is_none()
                && url.path().ends_with("/HLSPlaylist.m3u8")
        })
        .context("Embed has no Reddit-hosted video playlist for the requested post")?;

    let title_link = document
        .select(&TITLE_LINK)
        .next()
        .context("Embed has no post title link")?;
    let mut post_url = Url::parse(title_link.attr("href").unwrap())?;
    anyhow::ensure!(
        post_url.scheme() == "https" && post_url.host_str() == Some("www.reddit.com"),
        "Embed title does not link to Reddit"
    );
    let mut path = post_url
        .path_segments()
        .context("Embed post URL has no path")?;
    let (Some("r"), Some(subreddit), Some("comments"), Some(id)) =
        (path.next(), path.next(), path.next(), path.next())
    else {
        anyhow::bail!("Embed title does not link to a subreddit post");
    };
    anyhow::ensure!(id == link_id, "Embed title links to a different post");
    let subreddit = subreddit.to_owned();
    post_url.set_query(None);
    post_url.set_fragment(None);
    let title = title_link.text().collect::<String>();
    let title = title.trim();
    anyhow::ensure!(!title.is_empty(), "Embed has an empty post title");

    let post = Post {
        id: link_id.to_owned(),
        subreddit,
        title: title.to_owned(),
        permalink: post_url.path().to_owned(),
        url: post_url.to_string(),
        post_hint: Some("hosted:video".to_owned()),
        post_type: PostType::Video,
        gallery_data: None,
        media_metadata: None,
    };
    Ok((post, playlist))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embed_without_its_own_video_does_not_return_an_unrelated_video() {
        let embed = r#"
            <a id="embed-title" href="https://www.reddit.com/r/example/comments/abc123/text_post/">
                Text post
            </a>
            <shreddit-player post-id="t3_xyz678"
                src="https://v.redd.it/example/HLSPlaylist.m3u8"></shreddit-player>
        "#;

        assert!(parse_video(embed, "abc123").is_err());
    }
}
