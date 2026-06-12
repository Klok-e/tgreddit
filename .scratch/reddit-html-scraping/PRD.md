Status: ready-for-agent

# Reddit HTML Scraping PRD

## Problem Statement

The bot currently fails before Telegram media handling because Reddit returns `403 text/html` for JSON Data API reads used to fetch subreddit top posts, direct post details, and subreddit metadata. Browser-like headers and cookie prefetch do not restore JSON/API access.

Public old Reddit HTML pages still return successful listing pages for the same subreddit top feeds. The user needs subscribed feed delivery restored without adding browser automation, OAuth, or a large rewrite of downstream Telegram/media behavior.

## Solution

Replace Reddit JSON reads with old Reddit HTML scraping for public subreddit top feeds and direct post lookup. Parse old Reddit listing/post HTML into the existing internal post shape, classify scraped posts conservatively, and keep the existing Telegram delivery paths for images, videos, links, and self posts.

Every scraped post should carry enough post classification metadata to avoid the current direct-link enrichment path that calls blocked Reddit API endpoints. When metadata is incomplete, the bot should degrade to link delivery and log a clear reason instead of panicking.

## User Stories

1. As a bot operator, I want subscribed subreddit top feeds to work again, so that Reddit API `403` responses do not stop the bot from delivering posts.
2. As a Telegram chat member, I want image posts from subscribed subreddits to appear as Telegram photos, so that normal feed behavior is restored.
3. As a Telegram chat member, I want downloadable video posts to continue using the existing video delivery path, so that media posts remain convenient to view.
4. As a Telegram chat member, I want external link posts to arrive as link messages, so that non-media Reddit posts are still visible.
5. As a Telegram chat member, I want self posts to arrive as text/link messages, so that discussion posts are not silently skipped.
6. As a Telegram chat member, I want unsupported gallery posts to arrive as links, so that galleries do not crash or disappear while full gallery extraction is deferred.
7. As a bot operator, I want direct Reddit post lookup to work through old Reddit HTML, so that debug and repost workflows can still fetch a post by id.
8. As a bot operator, I want subreddit subscription validation to avoid blocked JSON endpoints, so that `/sub` does not fail only because `/about.json` is blocked.
9. As a bot operator, I want malformed old Reddit pages to produce clear errors, so that access gates and selector breakage are diagnosable from logs.
10. As a bot operator, I want promoted posts skipped, so that ad-like listing entries are not delivered as normal subreddit content.
11. As a bot operator, I want empty or malformed entries skipped, so that partial HTML nodes do not crash polling.
12. As a bot operator, I want request volume to stay low, so that scraping only replaces existing polling reads and does not add aggressive retries.
13. As a maintainer, I want scraping isolated behind existing Reddit fetch functions, so that bot, database, and Telegram code stay mostly unchanged.
14. As a maintainer, I want a pure parser for old Reddit listing HTML, so that selector behavior can be tested without network access.
15. As a maintainer, I want a pure parser for old Reddit direct-post HTML, so that direct lookup can be tested with deterministic fixtures.
16. As a maintainer, I want a pure classifier for scraped post attributes, so that image/video/link/self/gallery rules are explicit and covered by tests.
17. As a maintainer, I want gallery classification to be conservative, so that missing gallery metadata cannot trigger existing gallery panics.
18. As a maintainer, I want failed enrichment to be non-fatal, so that a single blocked or malformed direct post page does not stop post handling.
19. As a maintainer, I want debug logs for fetched URL, status, content type, parsed count, and classification, so that future Reddit changes can be diagnosed quickly.
20. As a maintainer, I want no normal tests to hit live Reddit, so that CI and local test runs stay deterministic.
21. As a maintainer, I want existing internal post formatting and repost buttons preserved, so that user-visible Telegram messages remain consistent.
22. As a maintainer, I want old Reddit HTML checks to detect login/interstitial/no-subreddit pages, so that failures are explicit rather than misclassified as empty feeds.
23. As a deployer, I want runtime verification commands to confirm old Reddit HTML works on the target host, so that local success does not hide deployment network differences.
24. As a deployer, I want logs to prove no blocked JSON endpoints are used in normal polling, so that the fix can be verified after restart.

## Implementation Decisions

- Keep the existing public Reddit fetch interface for subreddit top posts and direct post lookup. Change the internal transport from Reddit JSON/API endpoints to old Reddit HTML.
- Use a real HTML parser with CSS selectors instead of regex parsing. Parse listing nodes from old Reddit's site table and direct-post pages from the first post node.
- Use browser-like request headers and a bounded timeout for old Reddit HTML requests.
- Validate that successful responses are HTML and contain old Reddit listing/post markers before parsing.
- Extract post id, subreddit, permalink, outbound URL, title, gallery flag, and domain from old Reddit node attributes and title text.
- Skip promoted entries and malformed entries missing required id, title, permalink, or URL.
- Classify scraped posts from URL, domain, and gallery flag. Image, video, self, and link posts should receive `post_hint` values that prevent blocked direct-link enrichment in normal feed handling.
- Treat galleries as link posts in the first version unless complete gallery metadata is available. This avoids calling the current gallery path with missing media metadata.
- Replace direct-post lookup with an old Reddit comments-page scrape for the requested id.
- Make post enrichment failure non-fatal. If enrichment fails, log the failure and continue with listing data when available.
- Replace subreddit metadata validation with an old Reddit HTML existence check, preserving user-facing subscription validation without relying on blocked JSON.
- Keep downstream media handling, message formatting, repost buttons, seen-post recording, and filters intact unless a narrow change is required to avoid blocked API calls or panics.
- Add observability around old Reddit fetches, parse counts, classification decisions, fallback-to-link decisions, and blocked/malformed page detection.
- Keep OAuth, browser automation, full gallery extraction, self-text extraction, private/quarantined/age-gated subreddit support, and full Reddit JSON parity out of first scope.

## Testing Decisions

- Good tests should verify external behavior at stable seams: HTML input becomes expected internal post data, classification maps attributes to post types, and handler behavior does not panic when enrichment fails.
- Parser and classifier tests should use deterministic fixture HTML or compact inline HTML. Normal tests must not call live Reddit, Telegram, or remote media hosts.
- Listing parser tests should cover image posts, external link posts, promoted-post skipping, malformed/no-post pages, gallery fallback behavior, and requested limit truncation.
- Direct-post parser tests should cover a valid comments page and a page with no usable post node.
- Classifier tests should cover Reddit image hosts, image extensions, video hosts, gifv URLs, comments/self URLs, gallery flags, and unknown external links.
- Handler-level tests should cover missing `post_hint` behavior at the highest feasible seam, proving failed direct enrichment logs and continues rather than panicking.
- Existing tests for command parsing, message formatting, database behavior, and yt-dlp parsing should remain valid and unchanged unless behavior intentionally changes.
- Verification after implementation should run formatter, linter, and unit tests, then do a manual smoke run for direct-post lookup and target-host old Reddit reachability.

## Out of Scope

- OAuth or authenticated Reddit API integration.
- Browser automation or headless browser scraping.
- Full parity with Reddit JSON fields.
- Reliable gallery media extraction in first version.
- Reliable self-text extraction in first version.
- Private, quarantined, NSFW-login-gated, or otherwise access-gated subreddit support.
- Aggressive retrying, high-volume scraping, or bypassing Reddit access controls.
- Reworking Telegram delivery UX beyond fallback-to-link behavior for unsupported posts.

## Further Notes

Current evidence shows old Reddit HTML works for public top listing pages while JSON/API paths return blocked HTML. The first implementation should therefore restore simple feed delivery through old Reddit HTML and leave deeper media parity for later milestones.

Recommended milestone order:

1. Restore simple feed delivery with listing parsing, conservative classification, gallery-as-link fallback, and parser/classifier tests.
2. Restore direct post lookup through old Reddit comments-page scraping and make enrichment non-panicking.
3. Improve media parity for hosted video, gif handling, galleries, and crossposts.
4. Add richer observability so future Reddit-side breakage is diagnosable from logs.
