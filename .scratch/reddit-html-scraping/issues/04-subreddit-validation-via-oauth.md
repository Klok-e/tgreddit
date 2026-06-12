Status: ready-for-agent

# Subreddit Validation Via OAuth

## Parent

.scratch/reddit-html-scraping/PRD.md

## What to build

Switch subreddit metadata and subscription validation to authenticated Reddit JSON through the OAuth transport. User-facing `/sub` validation should still accept valid public subreddits and reject nonexistent or inaccessible subreddits with clear behavior.

## Acceptance criteria

- [ ] Subreddit about fetching uses authenticated OAuth JSON with `raw_json=1`.
- [ ] Valid public subreddits return display metadata compatible with current subscription behavior.
- [ ] Nonexistent subreddit responses are mapped to the existing no-such-subreddit behavior.
- [ ] Forbidden/private/banned/gated/quarantined responses are mapped to clear validation errors.
- [ ] Error mapping is covered by deterministic JSON fixture tests without live Reddit.
- [ ] Existing tests still pass.

## Blocked by

- .scratch/reddit-html-scraping/issues/01-oauth-transport.md
