//! Live Reddit OAuth integration tests.
//!
//! These tests exercise the real Reddit OAuth transport against `reddit.com`.
//! They are ignored by default so that normal `cargo test` stays deterministic
//! and does not call live Reddit. Run them explicitly with:
//!
//! ```bash
//! cargo test --test reddit_live -- --ignored --nocapture
//! ```
//!
//! See `docs/agents/testing.md` for the project's testing policy.

use anyhow::Result;
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
