# Testing Policy

## Normal Tests

Normal `cargo test` tests must be deterministic and must not depend on live Reddit or Telegram APIs.

Unit tests should use pure fixtures, mocked data, and in-process helpers. For Reddit behavior, unit tests should cover:

- JSON deserialization.
- Post classification.
- Gallery metadata handling.
- URL construction.
- Request/query construction helpers.

Do not add local HTTP servers that pretend to be Reddit. This is absolute: Reddit API behavior tests must either be pure unit tests without network I/O or ignored live integration tests against real Reddit.

## Live Reddit Tests

Tests that exercise real Reddit API behavior must live under `tests/` as integration tests and must be ignored by default.

Live Reddit integration tests should use real Reddit endpoints and may require network access. They are appropriate for changes to:

- OAuth transport behavior.
- Feed fetching.
- Direct post lookup.
- Subreddit validation.
- Reddit response compatibility.

Run them explicitly when present with:

```bash
cargo test --test reddit_live -- --ignored --nocapture
```

## Live Telegram Tests

Live Telegram integration tests are ignored by default. They require local `tgreddit.toml` and `telegram-e2e.toml`.

Run them explicitly with:

```bash
CONFIG_PATH=tgreddit.toml cargo test --test telegram_e2e -- --ignored --nocapture
```

These tests send real messages to the configured Telegram channel and intentionally do not delete them.

## Verification Expectations

After code changes, run:

```bash
cargo fmt
cargo clippy
cargo test
```

Run ignored live Reddit or Telegram integration tests only when the change affects that external service behavior.
