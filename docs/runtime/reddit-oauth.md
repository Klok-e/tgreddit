# Reddit OAuth Transport

This document describes the Reddit transport the bot uses at runtime, its
fragility, its rate-limit behavior, and a short manual smoke checklist for
proving parity with the previous post paths after a deployment.

## Overview

The bot talks to Reddit over its authenticated JSON API. The transport
(`src/reddit/oauth.rs`) follows the same anonymous OAuth flow used by the
[Redlib][redlib] Reddit frontend: it acquires a Reddit bearer token with the
Android client id and then issues JSON requests against
`https://oauth.reddit.com` with the bearer token plus app-like request
headers.

[redlib]: https://github.com/redlib-org/redlib

The transport is the only Reddit data source. There is no HTML scraping, no
public-Redlib-instance dependency, no headless browser, and no fallback
chain. The Reddit JSON shape (`Post`, `ListingResponse`, gallery metadata)
is the same one the bot has always consumed, so Telegram delivery behavior
(image, hosted video, external link, self post, gallery) is unchanged.

### Endpoints

| Purpose              | URL                                                                 |
| -------------------- | ------------------------------------------------------------------- |
| Bearer token request | `https://www.reddit.com/auth/v2/oauth/access-token/loid`            |
| Authenticated JSON   | `https://oauth.reddit.com/<path>.json?raw_json=1`                    |

`raw_json=1` is added by the transport on every authenticated request so
media URLs come back in their usable form (for example, direct `i.redd.it`
image URLs and `v.redd.it` DASH manifests for hosted video).

### App-like request headers

The transport sends an Android-style `User-Agent` and the same
`x-reddit-*` headers the official Android client uses. The exact set
lives in `RedditOAuthTransport::app_headers` in `src/reddit/oauth.rs`.

### Token state in memory

The transport caches a single in-memory `RedditBearerToken` per process.
The cached token holds the bearer token, its absolute expiry, and the
`x-reddit-loid` / `x-reddit-session` response headers returned alongside
the access token. The token is reused across requests until it is close
to expiring or until a `401` forces a refresh.

## Token Lifecycle

The transport keeps token handling minimal and explicit:

- **Acquisition**: on the first authenticated request, the transport POSTs
  to the token endpoint and parses the response. The token URL, the
  Android client id, the request body, and the `x-reddit-*` headers are
  baked in.
- **Refresh before expiry**: a cached token is considered fresh while its
  expiry is more than `TOKEN_REFRESH_SKEW_SECS` (120 seconds) in the
  future. Once it is within that window, the next authenticated request
  triggers a refresh.
- **Forced refresh on `401`**: a `401 Unauthorized` response from a
  Reddit endpoint triggers exactly one token refresh and exactly one
  retry of the same request. A second `401` from the retry is a hard
  `RedditApiError::Unauthorized` and is not retried again; the bot stops
  polling and the error is surfaced to the operator.
- **No retry on `429`**: rate-limit responses are surfaced as typed
  errors (see below) and the transport does not retry. The operator
  decides whether to back off.

Bearer tokens are not printed in logs or errors. `RedditBearerToken`'s
`Debug` impl redacts the access token, and the rate-limit and
authorization log lines reference only the request path, the
`x-ratelimit-*` counters, and the HTTP status.

## Rate Limits

Reddit signals rate-limit pressure through the
`x-ratelimit-used` / `x-ratelimit-remaining` / `x-ratelimit-reset`
response headers and through `429 Too Many Requests`. The transport
treats both signals as actionable conditions:

- `429` and 2xx responses with a `remaining` value strictly below 10.0
  are both surfaced as `RedditApiError::RateLimited` and logged with the
  parsed counters. The transport does not retry.
- Missing or unparseable `x-ratelimit-*` headers fall back to a
  status-only message so the operator can still see which path was
  throttled.
- Request volume is aligned with the current polling behavior
  (`check_interval_secs` in the config). The transport does not add
  aggressive retries, so a `429` is unlikely under normal operation but
  can still occur if the bot is restarted frequently or if multiple
  processes share an IP.

The exact low-budget threshold lives in
`RATE_LIMIT_LOW_THRESHOLD` in `src/reddit/oauth.rs`.

## Runtime Risks And Fragility

This transport depends on Reddit continuing to accept the
Android-client-style anonymous OAuth flow that Redlib uses. Operators
should be aware of the following failure modes:

- **Auth endpoint drift**: if Reddit changes the token endpoint, the
  client id, or the headers it accepts, the transport will start
  receiving `401 Unauthorized` or `403 Forbidden` responses. The OAuth
  flow will fail and the bot will surface `RedditApiError::Unauthorized`
  or `Inaccessible` subreddit errors. The bot does not log bearer tokens
  or session cookies, so there is no static credential to rotate; the
  fix is to update the transport's constants in `src/reddit/oauth.rs`
  and the app-like headers.
- **Login-required endpoints**: the bot does not have a logged-in Reddit
  account. Endpoints that require a user session are out of scope and
  will fail with the same auth error variants.
- **Rate-limit pressure**: a high-traffic deployment, a shared egress
  IP, or frequent restarts can exhaust the OAuth client's
  `x-ratelimit-remaining` budget. The transport surfaces this as a
  typed error and does not retry, so the operator is expected to reduce
  poll frequency, reduce the number of subscriptions, or wait for the
  reset window.
- **Response-shape changes**: the transport preserves the existing JSON
  deserialization (`Post`, `ListingResponse`, gallery fields) rather
  than copying Redlib's response handling. If Reddit renames a field
  the bot relies on, the relevant fixture-based unit tests in
  `src/reddit/api.rs` will catch it without contacting Reddit.
- **No fallback chain**: the transport is the only data source. If
  Reddit becomes unreachable, the bot will log transport errors and
  stop delivering posts until the next successful poll. There is no
  HTML scraping fallback and no public-Redlib-instance fallback.

## Manual Smoke Checklist

Run the bot with a real `CONFIG_PATH` and a Telegram chat you can read
messages in, then walk through these checks. Each item should pass
end-to-end; the existing fixture tests in `src/reddit/api.rs` already
cover the JSON-shape side of the same scenarios, so the manual run is
about wiring the transport to a live Reddit.

The checklist assumes the bot binary is built with
`cargo build --release` and run with `CONFIG_PATH=/path/to/tgreddit.toml`.

1. **Token acquisition** — start the bot and confirm the first
   authenticated poll completes without a `401 Unauthorized` or
   `RedditApiError::Unauthorized` in the logs. The first poll
   implicitly exercises the token endpoint
   (`/auth/v2/oauth/access-token/loid`) and the `x-ratelimit-*`
   header parsing.
2. **Subreddit top feed** — `/get <subreddit> limit=5 time=week` (or
   `/sub <subreddit>` and wait one `check_interval_secs` cycle) and
   confirm the bot returns the current top listings from
   `https://oauth.reddit.com/r/<subreddit>/top.json?raw_json=1`.
3. **Direct post lookup** — `tgreddit --debug-post <linkid>` with a
   real post id and confirm the post JSON deserializes through
   `get_link` and prints without errors. This exercises
   `https://oauth.reddit.com/api/info.json?id=t3_<linkid>&raw_json=1`.
4. **Subreddit validation** — `/sub <subreddit>` against a valid
   public subreddit and confirm the bot replies with
   `Subscribed to r/<subreddit>` after the OAuth-backed
   `get_subreddit_about` call succeeds.
5. **Image delivery** — subscribe to (or `/get`) a subreddit that
   currently has an image top post and confirm the post arrives as a
   Telegram photo (or video, for an animated GIF).
6. **Hosted video delivery** — subscribe to a subreddit that currently
   has a hosted video top post and confirm the existing
   `yt-dlp`-backed video path delivers the post as a Telegram video.
7. **External link delivery** — confirm an external-link top post
   (a `post_hint: link` post whose `url` is not `reddit.com` or
   `i.redd.it`) arrives as a link message.
8. **Self post delivery** — confirm a self post (an `is_self: true`
   post with no external media) arrives as a text/link message.
9. **Gallery delivery** — confirm a gallery post (a post with
   `is_gallery: true`, populated `gallery_data.items`, and matching
   `media_metadata`) arrives through the existing gallery media
   delivery path.
10. **Nonexistent subreddit** — `/sub thissubredditdoesnotexist12345`
    and confirm the bot replies with `No such subreddit` (the
    `302/404` response from `/r/<subreddit>/about.json` is mapped to
    `SubredditAboutError::NoSuchSubreddit`).
11. **Inaccessible subreddit** — `/sub` against a subreddit that
    returns `403` with a `reason` of `private`, `banned`, `gated`, or
    `quarantined` and confirm the bot replies with
    `This subreddit is not accessible (<reason>)`.

If any item fails, capture the relevant log line and the failing
subreddit or post id; the transport logs the request path and the
`x-ratelimit-*` counters, but never the bearer token.

## Related Files

- `src/reddit/oauth.rs` — transport, token parsing, rate-limit parsing.
- `src/reddit/api.rs` — top feed, direct post lookup, subreddit
  validation, error mapping.
- `tests/reddit_live.rs` — ignored live Reddit integration test
  (`cargo test --test reddit_live -- --ignored --nocapture`).
- `docs/agents/testing.md` — the project's testing policy, including
  when to run the ignored live Reddit test.
