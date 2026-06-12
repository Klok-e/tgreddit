Status: ready-for-agent

# Feed Delivery Via OAuth

## Parent

.scratch/reddit-html-scraping/PRD.md

## What to build

Switch subreddit top feed fetching to authenticated Reddit JSON through the OAuth transport. The visible bot behavior should stay the same: subscribed feeds still produce image, hosted video, external link, self, and gallery deliveries through the existing post model and Telegram handlers.

## Acceptance criteria

- [ ] Subreddit top feed fetching uses `oauth.reddit.com` with `raw_json=1`.
- [ ] Existing post deserialization continues to classify image, hosted video, external link, self, and gallery posts correctly from OAuth JSON.
- [ ] Gallery posts still include enough metadata for the current gallery delivery path.
- [ ] Feed behavior is covered by deterministic JSON fixture tests without live Reddit.
- [ ] Existing tests still pass.

## Blocked by

- .scratch/reddit-html-scraping/issues/01-oauth-transport.md
