Status: complete

# OAuth Transport

## Parent

.scratch/reddit-html-scraping/PRD.md

## What to build

Add a minimal Reddit OAuth transport that can obtain an anonymous bearer token using the Redlib-style Android OAuth flow and use that token for authenticated Reddit JSON requests.

This slice should prove the bot can make an authenticated JSON request to Reddit without changing feed, direct-post, or subscription behavior yet.

## Acceptance criteria

- [ ] The app can request and store an in-memory Reddit bearer token with its expiry.
- [ ] Authenticated JSON requests can be made against `oauth.reddit.com` with the bearer token and app-like headers.
- [ ] Token response parsing and authenticated request construction are covered by deterministic tests that do not call live Reddit.
- [ ] Bearer tokens are not printed in logs or errors.
- [ ] Existing tests still pass.

## Blocked by

None - can start immediately

## Comments

### AFK completed

OAuth transport slice meets issue acceptance criteria; validation passed; scope limited to src/reddit/oauth.rs test-harness change in current worktree.

### Correction note

The `test-harness change` referenced in the original completion note introduced a local HTTP server that pretended to be Reddit for the OAuth transport test. That approach violated the active testing policy in `docs/agents/testing.md`, which forbids local HTTP servers that simulate Reddit in normal tests. The fake Reddit test server has since been removed (see `.scratch/remove-test-server/issues/01-remove-fake-reddit-server.md`), and OAuth transport coverage is now split between pure deterministic seams (token response parsing, default URL construction) and an ignored live Reddit integration test (see `.scratch/remove-test-server/issues/02-add-live-reddit-oauth-test.md`). The historical completion note above is preserved as-is so the change history is not rewritten as if the violation never happened.
