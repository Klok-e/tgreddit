Status: ready-for-agent

# Telegram E2E: Repost Flow

## Problem Statement

The operator wants a way to gate which Reddit posts land in the channel. The
current flow sends posts directly to the channel, so every top-list hit
appears in the channel without operator review. The current live Telegram
E2E suite also posts directly to the channel and tests only delivery, not
the operator's actual workflow.

Three of the five fixture post ids in the live E2E suite have drifted in
type, so the suite fails before exercising delivery on every run.

## Solution

Change the operator's flow so the bot delivers posts to the operator's
private chat (the Channel Download Bot), and the operator copies a post to
the Repost Channel by tapping an inline button. Make the live E2E suite
exercise that full flow: deliver to the operator's chat, then simulate the
button tap, then assert the channel received the copy.

## User Stories

1. As the operator, I want new posts to land in my private chat with the
   bot, so that the channel stays clean until I approve them.
2. As the operator, I want a button on each delivered post to copy that
   post to the Repost Channel, so that I can move a post to the channel
   with a single tap.
3. As the operator, I want a second button on each delivered post to copy
   the post without the title, so that I can post clean media to the
   channel.
4. As the operator, I want the bot to remember the post it delivered to
   my chat, so that the button copy lands in the right channel without
   re-asking me.
5. As the operator, I want the live E2E suite to cover the real flow, so
   that regressions in delivery or in the repost path are caught.
6. As the operator, I want the live E2E suite to use stable Reddit
   fixtures, so that the suite is reliable and does not fail on type
   drift.
7. As the operator, I want the live E2E suite to test every supported
   post type, so that image, video, link, self-text, and gallery paths
   are all covered.
8. As the operator, I want both button variants tested for every post
   type, so that the with-caption and no-caption repost paths are
   equally covered.
9. As a maintainer, I want the live E2E suite to remain ignored by
   default, so that normal `cargo test` stays deterministic.
10. As a maintainer, I want the live E2E suite to read the user id from
    existing config rather than introduce a parallel config file, so
    that there is one source of truth for the operator id.
11. As a maintainer, I want `handle_new_post` to return the id(s) of the
    Telegram message(s) it produced, so that downstream flows (such as
    the simulated button click in tests) can refer to them.
12. As a maintainer, I want the live E2E tests to fail loudly if a
    fixture drifts in type, so that we know to hand-pick a replacement.

## Implementation Decisions

- The Channel Download Bot is the operator's existing private chat with
  the bot. The bot's behavior changes: it now delivers posts there
  instead of directly to the Repost Channel.
- The Repost Channel is registered for the operator via the existing
  `/register_channel` command (no change to the command surface).
- The test configuration is read from the existing `tgreddit.toml` and
  `telegram-e2e.toml` files. The operator id is taken from
  `authorized_user_ids[0]`, and the Repost Channel id is taken from
  `telegram-e2e.toml`'s `chat_id` (reinterpreted as channel id).
- `handle_new_post`'s public return type changes so callers (and tests)
  can reference the Telegram message id(s) the delivery produced. The
  shape is a two-variant enum: one variant for single-message
  deliveries, one variant for gallery media groups.
- The production code paths that call `handle_new_post` continue to
  ignore the new return value. They do not change behavior.
- The inline-button handlers that power the operator's tap-to-repost
  flow are exposed so the live E2E suite can invoke them directly with
  a constructed callback, simulating a button tap.
- The live E2E suite hand-picks five stable Reddit post ids, one per
  supported `PostType`. When a fixture drifts in type, the test fails
  loudly and the maintainer replaces the id.
- The live E2E suite uses a fresh temp SQLite database per test, so
  tests are independent.
- The live E2E suite registers the Repost Channel for the operator
  inside each test by writing directly to the database, so the test is
  self-contained.
- The live E2E suite asserts only that delivery and both button-tap
  simulations return `Ok`. It does not assert on the Telegram message
  body, on the inline-button markup shape, or on database state.

## Testing Decisions

- What makes a good test here: an ignored live test that drives the
  real Reddit API, the real Telegram Bot API, and a real download step
  for media posts. It proves the operator's full flow end-to-end and
  fails if any external dependency is broken.
- Modules tested:
  - The post delivery path (image, video, link, self-text, gallery).
  - The repost path (with caption and without).
  - The Channel Download Bot -> Repost Channel wiring.
- Prior art: the existing ignored `tests/telegram_e2e.rs` and
  `tests/reddit_live.rs` files. The new flow extends the existing
  `tests/telegram_e2e.rs` file rather than introducing a new one.
- Fixture discipline: each test hard-asserts the fixture's `PostType`
  and fails on drift. Replacement is a manual hand-pick.

## Out of Scope

- A real Telegram button click. The test invokes the handler directly.
- Asserting on the delivered message body, the inline-button markup, or
  the database state after a delivery or repost.
- A dry-run mode or a Telegram transport mock. The live suite contacts
  real services.
- Adding new `PostType` cases or a new gallery delivery path.
- Changing the operator-facing commands (`/sub`, `/unsub`, `/get`,
  `/register_channel`).
- Refactoring the repost logic itself; this PRD is about the test
  architecture, not the production behavior.

## Further Notes

- This PRD deliberately changes the contract of `handle_new_post`. The
  production-side callers stay the same and ignore the new return
  value, so the runtime behavior is unchanged for the operator.
- The test-only path that simulates a button tap is justified because
  Telegram does not allow programmatic button taps and because the
  real shape of the test (delivery + tap + channel receives) is
  already covered end-to-end by the no-error assertion on the
  copy/send-media-group call.
