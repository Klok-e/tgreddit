Status: ready-for-agent

# Live E2E Exercises Full Deliver-Then-Repost Flow

## Parent

.scratch/telegram-e2e-repost/PRD.md

## What to build

Rewrite the live Telegram E2E suite in `tests/telegram_e2e.rs` so each per-`PostType` test exercises the operator's full flow end-to-end: the bot delivers a Reddit post to the operator's Channel Download Bot chat, then a constructed inline-button callback triggers the repost path that copies the delivered post to the registered Repost Channel. The test does not assert on Telegram message bodies, inline-button markup, or database state after a delivery or repost; it asserts only that the delivery and both repost variants (with caption and without caption) return `Ok`.

To make the test callable, expose the inline-button repost handlers (`handle_repost` and `handle_repost_gallery` in `src/bot.rs`) as a direct-invocation seam that takes a constructed callback payload (the message id(s), the post, and the with-caption flag) rather than going through `teloxide`'s `CallbackQuery` dispatcher. The exposed seam is used by the test only; the dispatcher continues to call into the same logic unchanged for the production tap path.

Read the operator id from `tgreddit.toml`'s `authorized_user_ids[0]` (no parallel config file). Read the Repost Channel id from `telegram-e2e.toml`'s `chat_id` (reinterpreted as the channel id). Each test opens a fresh temp SQLite database and writes the Repost Channel registration directly to the database for the operator, so the test is self-contained.

The agent selects five fresh Reddit post ids, one per `PostType` (Image, Video, Link, SelfText, Gallery), targeting stable, long-lived posts unlikely to drift. Each test hard-asserts the fixture's `PostType` and fails loudly on drift, surfacing the actual id and type so the maintainer can hand-pick a replacement. The selection is the agent's responsibility, not a pre-pick from a human.

## Acceptance criteria

- [ ] The inline-button repost handlers are exposed so a test can invoke them directly with a constructed callback payload, without going through `teloxide`'s dispatcher.
- [ ] The production callback dispatcher still routes to the same repost logic unchanged.
- [ ] `tests/telegram_e2e.rs` reads the operator id from `tgreddit.toml`'s `authorized_user_ids[0]`.
- [ ] `tests/telegram_e2e.rs` reads the Repost Channel id from `telegram-e2e.toml`'s `chat_id` (reinterpreted as the channel id).
- [ ] Each per-`PostType` test opens a fresh temp SQLite database and registers the Repost Channel for the operator by writing directly to the database.
- [ ] Each per-`PostType` test delivers a Reddit post to the operator's chat via `handle_new_post`, captures the message id(s) from the new return value, and then invokes the exposed repost handler twice (with caption, without caption).
- [ ] Each per-`PostType` test asserts the fixture's `PostType` matches expectation and fails loudly on drift.
- [ ] Each per-`PostType` test asserts only that delivery and both repost variants return `Ok`. No assertion on Telegram message body, inline-button markup, or database state.
- [ ] The agent has selected and committed five Reddit fixture ids, one per supported `PostType` (Image, Video, Link, SelfText, Gallery). Selection is the agent's own work; no maintainer pre-pick is required.
- [ ] The tests remain `#[ignore]` and do not run as part of normal `cargo test`.
- [ ] `cargo fmt` and `cargo clippy` pass.

## Blocked by

- .scratch/telegram-e2e-repost/issues/01-handle-new-post-returns-message-ids.md
