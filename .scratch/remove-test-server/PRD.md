Status: ready-for-agent

# Remove Fake Reddit Test Server PRD

## Problem Statement

The project has an in-tree local HTTP test server that pretends to be Reddit for OAuth transport tests. This violates the active testing policy: normal tests must not use local HTTP servers that simulate Reddit API behavior. The violation makes the test suite inconsistent with the repository rules and encourages future agents to add more fake-Reddit network tests instead of pure unit tests or ignored live Reddit integration tests.

The project also has historical scratch planning docs that still describe mocked HTTP responses as an acceptable testing approach for Reddit OAuth work. Those docs conflict with the current testing policy and should be corrected so future work does not repeat the same mistake.

## Solution

Remove the fake Reddit test server and the normal test that depends on it. Keep normal test coverage at pure, deterministic seams such as token response parsing, URL construction, query construction, and JSON fixture deserialization. Add an ignored live Reddit integration test for the OAuth transport’s real request path, so behavior that requires Reddit network semantics is validated only when explicitly requested.

Update the scratch PRD/issues for the Reddit OAuth work so they align with the current testing policy: no local HTTP servers that pretend to be Reddit; Reddit API behavior tests are either pure unit tests without network I/O or ignored live integration tests against real Reddit.

## User Stories

1. As a maintainer, I want the normal test suite to follow the documented testing policy, so that `cargo test` remains deterministic and policy-compliant.
2. As a maintainer, I want fake Reddit HTTP servers removed, so that tests do not encode assumptions about Reddit network behavior in local socket fixtures.
3. As a maintainer, I want OAuth token parsing covered by pure unit tests, so that token response validation stays fast and deterministic.
4. As a maintainer, I want OAuth URL construction covered without network I/O, so that endpoint construction regressions are caught by normal tests.
5. As a maintainer, I want Reddit listing deserialization covered by JSON fixtures, so that post classification remains protected without calling Reddit.
6. As a maintainer, I want real OAuth request behavior covered by an ignored live test, so that network-dependent behavior is exercised only when explicitly requested.
7. As a maintainer, I want the ignored live Reddit test to be minimal and behavior-level, so that it is resilient to normal Reddit content changes.
8. As a maintainer, I want the live Reddit test documented through the existing command convention, so that agents know how to run it intentionally.
9. As an AFK agent, I want the scratch planning docs to match the active testing policy, so that I do not implement future work using a forbidden fake Reddit server.
10. As an AFK agent, I want completed historical issue notes corrected without erasing history, so that I can see both what happened and why it must be changed.
11. As a reviewer, I want the test-only custom base URL seam removed if it only exists for the fake server, so that production code has fewer unused hooks.
12. As a reviewer, I want the cleanup to avoid broad OAuth refactoring, so that the behavioral risk of the policy fix stays low.
13. As a project owner, I want normal tests to stay independent of live Reddit, Telegram, and remote media hosts, so that local validation remains reliable.
14. As a project owner, I want live Reddit validation to remain opt-in, so that normal CI and local development are not affected by network availability or Reddit rate limits.
15. As a future contributor, I want clear testing language in planning docs, so that I understand which Reddit behaviors belong in pure tests and which belong in ignored live tests.

## Implementation Decisions

- Delete the local fake Reddit HTTP server module.
- Remove the normal OAuth test that starts a local socket and points the transport at it.
- Remove the test-only custom Reddit base URL constructor if no production or supported test use remains.
- Keep the OAuth transport’s default constructor pointed at the production Reddit auth and OAuth hosts.
- Preserve existing pure OAuth tests for token response parsing and default OAuth URL construction.
- Add an ignored live Reddit integration test that constructs the OAuth transport and fetches a small top listing from a stable public subreddit.
- The live test should assert that the returned listing has at least one child, not exact headers, exact token payload fields, or specific post IDs.
- Keep public behavior unchanged for feed fetching, direct post lookup, subreddit validation, and Telegram delivery.
- Update historical scratch docs by adding correction notes rather than rewriting completed work as if it never happened.
- Do not add new dependencies for mocking or HTTP interception.

## Testing Decisions

- Good normal tests for Reddit behavior are pure and deterministic. They test stable seams such as token parsing, URL construction, request/query construction helpers, Reddit JSON fixture deserialization, post classification, gallery metadata handling, and error mapping.
- Normal tests must not use local HTTP servers that pretend to be Reddit.
- Normal tests must not depend on live Reddit, Telegram, or remote media hosts.
- OAuth request behavior that requires real network semantics should live in an ignored integration test against real Reddit.
- The ignored live Reddit test should be run explicitly with `cargo test --test reddit_live -- --ignored --nocapture`.
- The standard verification path for the cleanup is `cargo fmt`, `cargo clippy`, and `cargo test`.
- The ignored live Reddit test should not be run as part of normal verification unless the operator explicitly wants live Reddit validation.

## Out of Scope

- Reworking the Reddit OAuth transport beyond removing the fake-server test seam.
- Adding retries, rate-limit handling, or token refresh behavior beyond what already exists.
- Changing Telegram delivery behavior.
- Changing subreddit subscription behavior.
- Replacing Reddit JSON parsing or post classification logic.
- Introducing mocked HTTP frameworks, proxy-based tests, or new local network test harnesses.
- Running live Telegram E2E tests.

## Further Notes

The active testing policy is the source of truth: local HTTP servers that pretend to be Reddit are forbidden. Reddit API behavior tests must either be pure unit tests without network I/O or ignored live integration tests against real Reddit.

This cleanup should not require a domain `CONTEXT.md` update because it changes testing policy compliance and test architecture, not project domain terminology.
