Status: ready-for-agent

# Redlib-Style Reddit OAuth JSON PRD

## Problem Statement

The bot currently fails before Telegram media handling because Reddit returns `403 text/html` for unauthenticated JSON Data API reads used to fetch subreddit top posts, direct post details, and subreddit metadata.

HTML scraping, headless browser rendering, and public Reddit frontends were explored as alternatives, but they either fail from this network, do not provide reliable parity, or add too much operational fragility. The user wants one implementation path with parity to current behavior and no fallback stack.

## Solution

Replace unauthenticated Reddit JSON requests with a clean-room Redlib-style OAuth JSON transport. The bot should obtain an anonymous Reddit bearer token using the same Android-client-style OAuth flow Redlib uses, then call `https://oauth.reddit.com` JSON endpoints with app-like headers.

Keep the existing Reddit JSON data model and Telegram delivery behavior. The source remains Reddit JSON, not HTML scraping and not a public Redlib instance.

## User Stories

1. As a bot operator, I want subscribed subreddit top feeds to work again, so that unauthenticated Reddit `403` responses do not stop post delivery.
2. As a Telegram chat member, I want image posts to continue arriving as Telegram photos, so that current feed behavior is preserved.
3. As a Telegram chat member, I want hosted video posts to continue using the existing video delivery path, so that current media behavior is preserved.
4. As a Telegram chat member, I want external link posts to continue arriving as link messages, so that non-media Reddit posts remain visible.
5. As a Telegram chat member, I want self posts to continue arriving as text/link messages, so that discussion posts are not skipped.
6. As a Telegram chat member, I want gallery posts to continue arriving through the existing gallery media path, so that parity with current JSON behavior is preserved.
7. As a bot operator, I want direct Reddit post lookup to keep working, so that debug and repost workflows can fetch a post by id.
8. As a bot operator, I want subreddit subscription validation to keep working, so that `/sub` still rejects nonexistent or inaccessible subreddits.
9. As a bot operator, I want Reddit auth/token failures to produce clear logs, so that runtime failures are diagnosable from service logs.
10. As a bot operator, I want token refresh to happen automatically, so that the bot can run longer than one token lifetime without manual restart.
11. As a bot operator, I want rate-limit responses handled explicitly, so that the bot does not loop aggressively when Reddit throttles requests.
12. As a maintainer, I want the existing `Post` deserializer and media handlers preserved where possible, so that this change stays focused on Reddit transport.
13. As a maintainer, I want no public Redlib instance dependency, so that the bot does not depend on third-party uptime, trust, or instance policy.
14. As a maintainer, I want a clean-room implementation, so that AGPL Redlib source is used only as behavioral reference and not copied into this MIT project.
15. As a maintainer, I want deterministic tests around token/client behavior, endpoint construction, and error mapping, so that normal tests do not need live Reddit.
16. As a deployer, I want configuration documented for this transport, so that runtime requirements and risks are clear before deployment.
17. As a deployer, I want a manual smoke checklist covering image, video, link, self, gallery, direct post, and subreddit validation, so that parity can be verified after rollout.

## Implementation Decisions

- Implement a single Reddit source: Redlib-style OAuth JSON. Do not add old Reddit HTML, new Reddit HTML, headless browser, RSS, public Redlib instance, or fallback chains.
- Use `https://www.reddit.com/auth/v2/oauth/access-token/loid` to obtain an anonymous bearer token using Android-client-style headers and the Android client id observed in Redlib.
- Use the bearer token against `https://oauth.reddit.com` JSON endpoints.
- Preserve existing public Reddit API functions for top posts, direct post lookup, and subreddit metadata, changing only their internal transport/auth behavior.
- Preserve existing `Post` JSON deserialization and downstream Telegram/media handling wherever possible.
- Add `raw_json=1` to Reddit JSON requests so media URLs remain directly usable.
- Store token state in memory, including bearer token, expiry, loid/session headers returned by Reddit, and rate-limit counters from response headers.
- Refresh the token before expiry and force refresh on `401 Unauthorized`.
- Map `403`, `404`, and Reddit JSON error reasons such as `private`, `banned`, `gated`, and `quarantined` into existing user-facing subreddit validation behavior where possible.
- Treat `429` and low remaining rate-limit headers as backoff conditions, not parse failures.
- Log auth creation, token refresh, endpoint status, rate-limit headers, and mapped Reddit errors without logging bearer tokens.
- Do not copy Redlib source code verbatim because Redlib is AGPL-3.0-only. Reimplement the minimal behavior cleanly in this repository style.
- Prefer the smallest dependency surface that works. If plain `reqwest` proves reliable, use it. If Reddit rejects plain `reqwest`, evaluate `wreq` or equivalent TLS/header emulation as a deliberate implementation decision.
- Keep request volume aligned with current bot polling behavior. Do not add aggressive retries.

## Testing Decisions

- Good tests verify behavior at stable seams: token response parsing, token refresh decision logic, endpoint construction, auth header injection, rate-limit parsing, and Reddit error mapping.
- Normal tests must be pure and deterministic. They must not depend on live Reddit, Telegram, or remote media hosts, and they must not use local HTTP servers that pretend to be Reddit.
- Normal Reddit tests must use pure fixtures or in-process helpers without network I/O (for example, token response parsing, URL construction, request/query construction helpers, Reddit JSON fixture deserialization, post classification, gallery metadata handling, and error mapping).
- Reddit API behavior that requires real network semantics belongs in ignored live Reddit integration tests under `tests/`, not in the normal test suite.
- Run ignored live Reddit integration tests explicitly with `cargo test --test reddit_live -- --ignored --nocapture`.
- Existing tests for command parsing, database behavior, message formatting, Reddit post classification, and yt-dlp parsing should remain valid.
- Add fixture coverage proving existing `Post` deserialization still supports image, hosted video, external link, self, and gallery JSON returned through OAuth.
- Add tests that bearer tokens are never included in formatted logs/errors if logging helpers are introduced.
- Manual smoke verification should run against live Reddit only after implementation: token acquisition, subreddit top feed, direct post lookup, subreddit about, image delivery, hosted video delivery, link delivery, self post delivery, gallery delivery, nonexistent sub, and private/gated sub.

## Out of Scope

- HTML scraping from old Reddit or new Reddit.
- Headless browser rendering.
- RSS-based ingestion.
- Depending on a public or self-hosted Redlib frontend as the bot's data source.
- Official Reddit app-registration OAuth flow, unless this Redlib-style approach is rejected before implementation begins.
- Logged-in user Reddit features, commenting, voting, mod actions, or private user data.
- Reworking Telegram delivery UX beyond what is necessary to preserve current behavior.
- Copying AGPL Redlib code into this repository.

## Further Notes

Live probe on June 12, 2026 confirmed Redlib-style OAuth works from this machine:

- token request returned HTTP `200`, bearer token, and roughly 24-hour expiry
- OAuth listing endpoint returned current Reddit `Listing` JSON
- direct post lookup returned usable post JSON
- subreddit about returned metadata JSON
- gallery posts included `gallery_data.items[].media_id` and matching `media_metadata`
- hosted video posts included `post_hint = hosted:video`, `is_video = true`, and Reddit video metadata
- external link and self-post fields matched current deserializer needs
- nonexistent and private subreddit responses returned structured JSON errors

The implementation should proceed as a transport/auth replacement, not a parser rewrite.
