Status: complete

# Direct Post Lookup Via OAuth

## Parent

.scratch/reddit-html-scraping/PRD.md

## What to build

Switch direct Reddit post lookup to authenticated Reddit JSON through the OAuth transport. Debug-post and repost lookup behavior should keep returning the same usable post shape as before, including galleries where Reddit JSON provides gallery metadata.

## Acceptance criteria

- [ ] Direct post lookup uses authenticated OAuth JSON with `raw_json=1`.
- [ ] Direct lookup returns the existing post shape for image, hosted video, external link, self, and gallery posts.
- [ ] Debug-post behavior continues to work with the authenticated lookup path.
- [ ] Direct lookup behavior is covered by deterministic JSON fixture tests without live Reddit.
- [ ] Existing tests still pass.

## Blocked by

- .scratch/reddit-html-scraping/issues/01-oauth-transport.md

## Comments

### AFK completed

Direct post lookup now uses the OAuth JSON transport with raw_json=1; deterministic local tests cover image, hosted video, external link, self, gallery, request construction, and missing-post behavior. Validation passed.
