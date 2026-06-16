# Testing Policy

## Normal Tests

Normal `cargo test` tests must be deterministic and must not depend on live external services.

Unit tests should use pure fixtures, mocked data, and in-process helpers. For external service-adjacent code, unit tests should cover local behavior.

Do not add local HTTP servers that pretend to be external services. This is absolute: external service behavior tests must either be pure unit tests without network I/O or ignored live integration tests against the real service.

## Live External Service Tests

External service behavior includes any behavior whose correctness depends on a server or service outside this process, including Reddit, Telegram, OAuth endpoints, media hosts, and download-tool interactions with those hosts.

Tests that exercise real external service behavior must live under `tests/` as integration tests and must be ignored by default.

Live external service integration tests should use real service endpoints.

When a change depends on behavior owned by an external service, agents should add or update an ignored live integration test when they can run it locally. If they do not add one, they must explain why in the final response.

If required credentials or config are missing, agents should ask for them when live validation is necessary to prove the change. If the missing live test is not blocking the local implementation, they may continue with deterministic tests but must report the unrun live-validation gap in the final response.

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

These tests send real messages to the configured testing Telegram channel.

## Verification Expectations

After code changes, run:

```bash
cargo fmt
cargo clippy
cargo test
```

Run integration tests when changes touch code these tests cover.
