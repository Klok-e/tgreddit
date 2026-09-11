//! Live yt-dlp integration test against the YouTube video from the deployment handoff.
//!
//! This test is ignored by default so normal test runs remain deterministic. Run it explicitly:
//!
//! ```bash
//! cargo test --test ytdlp_live -- --ignored --nocapture
//! ```

use anyhow::Result;

const HANDOFF_VIDEO_URL: &str = "https://youtu.be/IZcKbD0yGzg";
const HANDOFF_VIDEO_ID: &str = "IZcKbD0yGzg";
const X_VIDEO_URL: &str = "https://x.com/unsafe_call/status/2098416568459776104";
const X_TWEET_TEXT: &str = "day one of giving the fly unlimited white monster and cigs until its neurons are trained to win me bug bounties https://t.co/W5VSyNJ8ne";

#[test]
#[ignore = "downloads a live YouTube video with the locally installed yt-dlp"]
fn downloads_handoff_youtube_video() -> Result<()> {
    let _ = env_logger::try_init();
    let video = tgreddit::ytdlp::download(HANDOFF_VIDEO_URL)?;

    assert_eq!(video.id, HANDOFF_VIDEO_ID);
    assert!(video.width > 0, "expected a nonzero video width");
    assert!(video.height > 0, "expected a nonzero video height");
    assert!(video.path.is_file(), "downloaded video should exist");

    Ok(())
}

#[test]
#[ignore = "downloads a live X video with the locally installed yt-dlp"]
fn downloads_x_video_with_tco_tweet_description() -> Result<()> {
    let _ = env_logger::try_init();
    let video = tgreddit::ytdlp::download(X_VIDEO_URL)?;

    assert_eq!(video.description.as_deref(), Some(X_TWEET_TEXT));

    Ok(())
}
