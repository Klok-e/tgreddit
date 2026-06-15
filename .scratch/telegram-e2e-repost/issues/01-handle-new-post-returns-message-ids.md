Status: ready-for-agent

# `handle_new_post` Returns Delivered Message Id(s)

## Parent

.scratch/telegram-e2e-repost/PRD.md

## What to build

Change `handle_new_post` in `src/handle_post.rs` so its public return type carries the Telegram message id(s) that the delivery actually produced. Shape the return value as a two-variant enum: one variant carrying a single `MessageId` for image, video, link, self-text, and unknown post deliveries, and one variant carrying a `Vec<MessageId>` for gallery media groups.

Every per-`PostType` helper (`handle_new_image_post`, `handle_new_video_post`, `handle_new_link_post`, `handle_new_self_post`, `handle_new_gallery_post`) must thread the new return value through, and the public `handle_new_post` must propagate it. The production callers in `src/main.rs` (the subscription polling loop and the `--debug-post --chat-id` path) and in `src/bot.rs` (the `/get` command) continue to call `handle_new_post` exactly as today and ignore the new return value, so runtime behavior is unchanged. `process_post` in `src/handle_post.rs` continues to swallow any error from `handle_new_post` and otherwise ignore its value.

## Acceptance criteria

- [ ] A two-variant enum is added that carries a single Telegram `MessageId` for non-gallery deliveries and a `Vec<MessageId>` for gallery deliveries.
- [ ] `handle_new_post` returns the enum.
- [ ] Every per-`PostType` helper in `src/handle_post.rs` returns the enum and the value reflects the message id(s) actually sent to Telegram.
- [ ] The production callers in `src/main.rs` and `src/bot.rs` ignore the new return value; their existing call sites compile and behave identically.
- [ ] `process_post` continues to log the delivery error path unchanged and otherwise ignores the value.
- [ ] Deterministic unit coverage exists for the enum's variant selection logic where it does not require live Telegram.
- [ ] `cargo fmt`, `cargo clippy`, and `cargo test` pass.

## Blocked by

None - can start immediately
