Status: complete

# Add Live Reddit OAuth Test

## Parent

.scratch/remove-test-server/PRD.md

## What to build

Add an ignored live Reddit integration test that validates the OAuth transport against real Reddit instead of a local fake server. The test should exercise a small authenticated listing request and assert only stable behavior.

## Acceptance criteria

- [ ] An ignored Rust integration test exists for live Reddit OAuth behavior.
- [ ] The test constructs the default OAuth transport and fetches a small top listing from a stable public subreddit.
- [ ] The test asserts that the returned listing contains at least one child.
- [ ] The test does not assert exact request headers, exact token payload details, or specific post IDs.
- [ ] The test is ignored by default and can be run explicitly with `cargo test --test reddit_live -- --ignored --nocapture`.
- [ ] Normal `cargo test` does not call live Reddit.
- [ ] `cargo fmt`, `cargo clippy`, and `cargo test` pass.

## Blocked by

- .scratch/remove-test-server/issues/01-remove-fake-reddit-server.md

## AFK completed

Added an ignored live Reddit OAuth integration test that fetches a small top listing from r/announcements through the default OAuth transport and asserts the listing has children. Verified fmt, clippy, normal tests, and the explicit live Reddit test.
