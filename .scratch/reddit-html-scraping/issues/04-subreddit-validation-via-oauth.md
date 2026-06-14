Status: complete

# Subreddit Validation Via OAuth

## Parent

.scratch/reddit-html-scraping/PRD.md

## What to build

Switch subreddit metadata and subscription validation to authenticated Reddit JSON through the OAuth transport. User-facing `/sub` validation should still accept valid public subreddits and reject nonexistent or inaccessible subreddits with clear behavior.

## Acceptance criteria

- [x] Subreddit about fetching uses authenticated OAuth JSON with `raw_json=1`.
- [x] Valid public subreddits return display metadata compatible with current subscription behavior.
- [x] Nonexistent subreddit responses are mapped to the existing no-such-subreddit behavior.
- [x] Forbidden/private/banned/gated/quarantined responses are mapped to clear validation errors.
- [x] Error mapping is covered by deterministic JSON fixture tests without live Reddit.
- [x] Existing tests still pass.

## Blocked by

- .scratch/reddit-html-scraping/issues/01-oauth-transport.md

## Comments

### AFK completed

`get_subreddit_about` now uses the OAuth JSON transport with `raw_json=1` via a new `send_authenticated` method on `RedditOAuthTransport` that returns the raw status + body. A pure `parse_subreddit_about_response` function maps Reddit responses to `SubredditAbout` or typed `SubredditAboutError` variants: `NoSuchSubreddit` for 302/404, `Inaccessible { reason }` for 403/private/banned/gated/quarantined, and `Parse` for invalid JSON. `bot.rs` now sends a user-facing message for the `Inaccessible` case. Deterministic JSON fixture tests cover 200, 302, 404, 403 with each reason, 403 without a reason, 403 with non-JSON body, 500 with a reason, invalid JSON, and missing `data` field. `cargo fmt && cargo clippy && cargo test` passes.

### AFK completed

Verified the OAuth-backed subreddit about implementation, deterministic error-mapping tests, and live OAuth transport smoke test. Validation passed with `cargo fmt`, `cargo clippy`, `cargo test`, and `cargo test --test reddit_live -- --ignored --nocapture`.
