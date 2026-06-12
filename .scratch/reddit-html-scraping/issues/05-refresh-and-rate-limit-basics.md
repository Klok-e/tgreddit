Status: ready-for-agent

# Refresh And Rate Limit Basics

## Parent

.scratch/reddit-html-scraping/PRD.md

## What to build

Add minimal production hardening for the OAuth transport: refresh the token before expiry, retry once after `401 Unauthorized`, and report Reddit rate-limit responses clearly without aggressive retry loops.

## Acceptance criteria

- [ ] Token refresh is triggered before token expiry.
- [ ] A `401 Unauthorized` response forces one token refresh and one retry.
- [ ] `429` responses and low remaining rate-limit headers are logged clearly and returned as actionable errors.
- [ ] Refresh and rate-limit behavior is covered by deterministic tests without live Reddit.
- [ ] Bearer tokens are not printed in logs or errors.
- [ ] Existing tests still pass.

## Blocked by

- .scratch/reddit-html-scraping/issues/01-oauth-transport.md
- .scratch/reddit-html-scraping/issues/02-feed-delivery-via-oauth.md
- .scratch/reddit-html-scraping/issues/03-direct-post-lookup-via-oauth.md
- .scratch/reddit-html-scraping/issues/04-subreddit-validation-via-oauth.md
