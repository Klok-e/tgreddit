# Repository Guidelines

## Project Structure

- `src/main.rs`: application startup, database migrations, signal handling, and subscription polling.
- `src/lib.rs`: public library surface used by the binary and integration tests.
- `src/bot.rs`: Telegram command handling, callbacks, authorization, and repost commands.
- `src/reddit/`: Reddit API client plus response and domain-adjacent data types.
- `src/db.rs`: SQLite schema migrations and persistence helpers. Append new migrations instead of editing old ones.
- `src/handle_post.rs`: dispatches Reddit posts to the correct Telegram delivery path.
- `src/messages.rs`: Telegram message, caption, and button formatting.
- `src/download.rs` and `src/ytdlp.rs`: media download helpers and `yt-dlp` integration.
- `tests/telegram_e2e.rs`: ignored live Telegram integration tests.
- `config.example.toml`: documented runtime configuration template.
- `telegram-e2e.example.toml`: documented local template for live Telegram integration tests.

## Commands

- `cargo check`: quick compile and type check.
- `CONFIG_PATH=/path/to/config.toml cargo run`: run the bot locally with a config file.
- `cargo fmt`: format Rust code after changes.
- `cargo clippy`: lint Rust code after changes.
- `cargo test`: run unit tests after changes.
- `CONFIG_PATH=tgreddit.toml cargo test --test telegram_e2e -- --ignored --nocapture`: run ignored live Telegram integration tests when the change needs real delivery validation.

Runtime video support requires `yt-dlp`; `ffmpeg` should be available for reliable media handling.

## Change Verification

- Review the diff for correctness, scope, and accidental unrelated edits.
- Run `cargo fmt`.
- Run `cargo clippy`.
- Run `cargo test`.
- Run ignored integration tests only when the issue explicitly needs live Reddit/Telegram delivery validation or the implementation changes Telegram delivery behavior.

## Coding Style

This is a Rust 2024 crate. Use standard `rustfmt` formatting, `snake_case` for files, modules, functions, and fields, `PascalCase` for types and enum variants, and `SCREAMING_SNAKE_CASE` for constants.

Prefer `anyhow::Result` for fallible application flow. Use typed errors, such as `thiserror`, when callers need to branch on error kind. Use `log` macros for runtime diagnostics; avoid `println!` in application code.

## Testing

Tests use Rust's built-in test framework and are colocated with the modules they cover in `#[cfg(test)]` blocks. Add or update tests for command parsing, database behavior, message formatting, Reddit post classification, and `yt-dlp` output parsing.

See `docs/agents/testing.md` for the full testing policy.

Keep unit tests deterministic. Do not make normal tests depend on live Reddit or Telegram APIs unless they are explicitly introduced as ignored integration tests.

Live Telegram integration tests are ignored by default. They require local `tgreddit.toml` for bot configuration and local `telegram-e2e.toml` for the test chat id. These tests send real messages to the configured Telegram channel and intentionally do not delete them.

## Configuration & Secrets

Local runs require `CONFIG_PATH` to point at a TOML config. Use `config.example.toml` as the template for available keys and defaults.

Do not commit real Telegram bot tokens, production chat IDs, local Telegram E2E config, or local SQLite database files. `authorized_users` controls which Telegram users can invoke bot commands.

## Documentation

Update `README.md` and `config.example.toml` when commands, configuration keys, or user-visible behavior change.

## Agent skills

### Issue tracker

Issues and PRDs are tracked as local markdown under `.scratch/<feature-slug>/`. See `docs/agents/issue-tracker.md`.

### Triage labels

Use the default five-label triage vocabulary. See `docs/agents/triage-labels.md`.

### Domain docs

This repo uses a single-context domain documentation layout. See `docs/agents/domain.md`.
