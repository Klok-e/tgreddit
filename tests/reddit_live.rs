//! Live Reddit integration tests.
//!
//! These tests exercise Reddit OAuth and public video embeds. The video test
//! requires yt-dlp, ffmpeg, and ffprobe. Tests are ignored by default so normal
//! `cargo test` stays deterministic
//! and does not call live Reddit. Run them explicitly with:
//!
//! ```bash
//! cargo test --test reddit_live -- --ignored --nocapture
//! ```
//!
//! See `docs/agents/testing.md` for the project's testing policy.

use anyhow::{Context, Result};
use std::process::Command;
use tgreddit::reddit::ListingResponse;
use tgreddit::reddit::oauth::RedditOAuthTransport;

/// A Reddit-official subreddit that is always populated, so the top listing is
/// guaranteed to contain at least one child regardless of when the test runs.
const STABLE_SUBREDDIT: &str = "announcements";

/// A small `limit` keeps the live request lightweight while still proving that
/// the OAuth transport returns real children.
const SMALL_LIMIT: u32 = 1;

/// `t=all` keeps the listing populated with stable, long-lived top posts
/// rather than depending on recent activity in the chosen subreddit.
const TIME_PERIOD: &str = "all";

#[tokio::test]
#[ignore = "calls live Reddit; run with cargo test --test reddit_live -- --ignored --nocapture"]
async fn oauth_transport_fetches_top_listing_from_live_reddit() -> Result<()> {
    let transport = RedditOAuthTransport::new()?;

    let path = format!("/r/{STABLE_SUBREDDIT}/top.json");
    let query = [
        ("limit", SMALL_LIMIT.to_string()),
        ("t", TIME_PERIOD.to_string()),
    ];

    let listing: ListingResponse = transport.get_json(&path, &query).await?;

    assert!(
        !listing.data.children.is_empty(),
        "expected at least one child in top listing for /r/{STABLE_SUBREDDIT}"
    );

    Ok(())
}

#[tokio::test]
#[ignore = "downloads the example Reddit video through the manual submission path"]
async fn manual_reddit_video_downloads_when_api_is_forbidden() -> Result<()> {
    let tgreddit::reddit::SubmittedPost {
        post,
        video_playlist,
    } = tgreddit::reddit::get_submitted_post("1y1qngh", Some("Eve")).await?;
    assert_eq!(post.id, "1y1qngh");
    assert_eq!(post.post_type, tgreddit::reddit::PostType::Video);
    assert_eq!(
        post.title,
        r#"Dear FCCP - May I make a counter-point to why we should keep the original "medallion""#
    );
    assert_eq!(
        post.url,
        "https://www.reddit.com/r/Eve/comments/1y1qngh/dear_fccp_may_i_make_a_counterpoint_to_why_we/"
    );
    let playlist =
        video_playlist.context("expected the public embed fallback after the API 403")?;
    let video = tgreddit::ytdlp::download(playlist.as_str())?;
    let probe = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_type",
            "-of",
            "csv=p=0",
        ])
        .arg(&video.path)
        .output()
        .context("ffprobe is required for the live Reddit video test")?;
    assert!(
        probe.status.success(),
        "ffprobe could not read the downloaded video"
    );
    let streams = std::str::from_utf8(&probe.stdout)?;
    assert!(streams.lines().any(|stream| stream == "video"));
    assert!(streams.lines().any(|stream| stream == "audio"));
    Ok(())
}
