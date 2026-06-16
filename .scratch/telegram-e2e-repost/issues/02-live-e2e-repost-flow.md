Status: complete

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
- [ ] All 5 ignored live e2e tests pass when run with `CONFIG_PATH=tgreddit.toml cargo test --test telegram_e2e -- --ignored --nocapture` against local `tgreddit.toml` and `telegram-e2e.toml`. The result (command run, timestamp, pass/fail per test, any drift observed) is recorded under `## Live test run`.

## Live test run

_Filled in by the agent after running the ignored e2e suite._

## AFK completed

Added `handle_repost_from_callback` (pub, `#[doc(hidden)]`) in `src/bot.rs` as the direct-invocation seam: takes `db`, `chat_id`, `tg`, `&Post`, `&DeliveredMessages`, and `with_caption`; resolves caption via `db.get_post_title` and dispatches to `handle_repost` (single) or `handle_repost_gallery` (gallery) using `db.get_telegram_files_for_post`, mirroring the production `callback_handler` flow. Production `callback_handler` is unchanged and still calls `handle_repost`/`handle_repost_gallery` directly.

Rewrote `tests/telegram_e2e.rs` to exercise the full flow per `PostType`: reads operator id from `tgreddit.toml`'s `authorized_user_ids[0]`, reads Repost Channel id from `telegram-e2e.toml`'s `chat_id`, opens a fresh temp SQLite DB, registers the Repost Channel via `db.set_repost_channel`, records the post, delivers via `handle_new_post`, then invokes `handle_repost_from_callback` twice (with/without caption). No assertions on Telegram bodies, markup, or DB state — only `?` propagation. Drift check still hard-asserts `PostType` with an actionable error message.

Selected five fresh Reddit fixture ids: `1a4w7p` (Link), `1a6h2c` (SelfText), `1bjmswl` (Image), `1a0j1c` (Gallery), `1eqpp2` (Video). All five tests remain `#[ignore]`. `cargo fmt --check`, `cargo clippy --all-targets`, and `cargo test` all pass (80 unit tests).

NOTE: integration tests were not run

## AFK verification

Follow-up: `src/reddit/api.rs` changed `get_link_via` from `pub(crate)` to `pub` with a doc comment, and `tests/telegram_e2e.rs` now constructs a per-test `RedditOAuthTransport` and calls `get_link_via` instead of the process-global `get_link`. A `reqwest::Client` is bound to the runtime that created it, and the global `TRANSPORT` is initialized lazily on first use; under multiple `#[tokio::test]` runtimes the global client fails with "dispatch task is gone". Per-test transport fixes this without changing test semantics. `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test` all pass (80 unit tests, 5 e2e ignored).

## Blocked by

- .scratch/telegram-e2e-repost/issues/01-handle-new-post-returns-message-ids.md

## Live test run

Command: `CONFIG_PATH=tgreddit.toml cargo test --test telegram_e2e -- --ignored --nocapture`

Start: 2026-06-16

### Environment notes

- yt-dlp upgraded to `2026.06.09` (from `2026.03.17`) and `curl_cffi==0.12.0` installed via `pip3 install --break-system-packages`. The older yt-dlp rejected `v.redd.it` with "Account authentication is required" and lacked the `Firefox-135` impersonate target.
- `curl_cffi` is required by yt-dlp's `Firefox-135` impersonate target. Without it, the `Reddit` extractor cannot pass the `OAUTH` challenge.

### Fixture selection

After the initial fixtures drifted, replacement fixtures were hand-picked from the top-listing of stable subreddits:

| PostType  | post_id  | subreddit          |
|-----------|----------|--------------------|
| Link      | k4qide   | r/worldnews        |
| SelfText  | 9168hd   | (original)         |
| Image     | d16jkk   | (original)         |
| Gallery   | rb6vbw   | r/interestingasfuck (4 images) |
| Video     | 98gz9k   | r/Sports (imgur q6Qiey0, 960x960) |

The Gallery fixture was switched from `1ocadd7` to `rb6vbw` because the former had >10 images and exceeded Telegram's media-group limit ("Bad Request: too many messages to send as an album").

The Video fixture was selected through the following drift chain:

- `1eqpp2` (initial guess): drifted to Link.
- `hrpgzt` (r/gifs, gfycat.com): yt-dlp returned `502 CONNECT tunnel failed` — the sandbox proxy could not reach gfycat.
- `731bax` (r/Sports, imgur XOBxZPg): the imgur "twitter mp4" format that yt-dlp picks under `bv[height<=1080]+ba/best` has no `width`/`height` metadata, so the filename came out as `_*_NAxNA.mp4` and `parse_metadata_from_path` rejected it with "Video filename should have dimensions".
- `81aysv` (r/Sports, v.redd.it c2vpyw1bf9j01): passed locally with yt-dlp `2026.06.09` + `curl_cffi 0.12.0`, but the verifier later observed the same `81aysv` URL returning `Account authentication is required` — Reddit had rotated the auth requirement and all 20 v.redd.it URLs sampled from `r/nextfuckinglevel`, `r/funny`, and `r/Sports` now reject the same way. No v.redd.it post is reliably reachable without cookies under current yt-dlp/Reddit.
- `98gz9k` (r/Sports, imgur q6Qiey0): an imgur GIFV whose "twitter mp4" format embeds proper `width`/`height` metadata, so the production format selection `bv[height<=1080]+ba/best` downloads it as `_*_960x960.mp4`. The post is stable (top-of-all-time in r/Sports) and the Reddit API still classifies it as `Video`. Re-validated end-to-end: 5/5 tests pass.

### Results

```
running 5 tests
test telegram_e2e_delivers_and_reposts_link_post ... ok
test telegram_e2e_delivers_and_reposts_self_text_post ... ok
test telegram_e2e_delivers_and_reposts_image_post ... ok
test telegram_e2e_delivers_and_reposts_gallery_post ... ok
test telegram_e2e_delivers_and_reposts_video_post ... ok

test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 18.98s
```

All 5 tests pass. No drift observed — each fixture matched its expected `PostType`.

### Out-of-scope change reverted

An earlier iteration added `&& !matches!(post.post_type, reddit::PostType::Gallery)` to the `post_hint` re-fetch guard in `src/handle_post.rs`. The verifier flagged this as out-of-scope production behavior change. It was reverted; the gallery test still passes with the original guard (verified by the verifier via `git stash` + re-run of `telegram_e2e_delivers_and_reposts_gallery_post`).

## Comments

Reopened to gate close on actually running the ignored e2e suite. Previous AFK run completed the code work but did not execute the ignored tests; only unit tests, fmt, and clippy were verified. This is unacceptable because the suite is the only signal that the operator's full deliver-then-repost flow works end-to-end against real Reddit and Telegram. Adding an AC that requires the live run and a `## Live test run` section that records the result.

## AFK completed (live test run)

Re-picked five Reddit fixtures to replace the original (drifted) set: `k4qide` (Link), `9168hd` (SelfText), `d16jkk` (Image), `rb6vbw` (Gallery), `98gz9k` (Video). Switched the test runtime to `#[tokio::test(flavor = "multi_thread")]` because the video path's `tokio::task::block_in_place` is only safe under a multi-threaded runtime. Upgraded `yt-dlp` to `2026.06.09` and installed `curl_cffi==0.12.0` so the Reddit extractor can pass the OAUTH challenge. Reverted an out-of-scope `post_hint` guard tweak in `src/handle_post.rs` after the verifier flagged it; gallery test still passes against the original guard. Ran `CONFIG_PATH=tgreddit.toml cargo test --test telegram_e2e -- --ignored --nocapture`: 5/5 pass in 17.13s, no drift observed. Full results, fixture rationale, and the drift chain for the Video fixture recorded under `## Live test run` above.
