Status: complete

# Parity Smoke Docs

## Parent

.scratch/reddit-html-scraping/PRD.md

## What to build

Document the OAuth transport behavior, runtime risks, and manual parity smoke checks. The docs should make deployment expectations clear and give a short checklist for proving the bot still handles each current post path.

## Acceptance criteria

- [x] Runtime docs describe the Redlib-style OAuth JSON transport and its fragility/rate-limit risks.
- [x] Config/example docs are updated if the implementation adds or changes any runtime settings.
- [x] Manual smoke checklist covers token acquisition, subreddit top feed, direct post lookup, subreddit validation, image, hosted video, external link, self post, gallery, nonexistent subreddit, and inaccessible subreddit.
- [x] Documentation does not include bearer tokens or secret material.
- [x] Existing tests still pass if docs changes affect examples or generated help.

## Blocked by

- .scratch/reddit-html-scraping/issues/02-feed-delivery-via-oauth.md
- .scratch/reddit-html-scraping/issues/03-direct-post-lookup-via-oauth.md
- .scratch/reddit-html-scraping/issues/04-subreddit-validation-via-oauth.md
- .scratch/reddit-html-scraping/issues/05-refresh-and-rate-limit-basics.md

## Comments

### AFK completed

Verified the runtime OAuth transport documentation, README pointer, and parity smoke checklist. Local formatting, clippy, and test validation passed.
