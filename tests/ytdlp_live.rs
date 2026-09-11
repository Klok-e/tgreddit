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
