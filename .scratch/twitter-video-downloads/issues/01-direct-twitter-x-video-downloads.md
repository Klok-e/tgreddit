Status: complete

## Parent

.scratch/twitter-video-downloads/PRD.md

## What to build

Add end-to-end support for direct Twitter/X status video downloads. When an authorized Telegram user pastes a public Twitter/X status URL, the bot should recognize it as a direct video link, download it through the existing `yt-dlp` video-download flow, record it in the existing duplicate/history tracking, and send it to Telegram with the same caption and repost-button behavior used by existing direct video downloads.

The supported URL shapes are Twitter status URLs, mobile Twitter status URLs, and X status URLs. Non-status Twitter/X pages should not be treated as downloadable video links. Existing direct YouTube downloads and Reddit link handling should continue to behave as they do today.

If `yt-dlp` writes multiple output files, the bot should send the file with the oldest modified timestamp. If timestamps cannot distinguish files, selection should fall back to deterministic path ordering. The direct-download record label should use generic video-download wording rather than YouTube-specific wording.

## Acceptance criteria

- [x] Pasting `https://twitter.com/{user}/status/{id}` into an authorized bot chat triggers the existing direct video download and Telegram send flow.
- [x] Pasting `https://mobile.twitter.com/{user}/status/{id}` into an authorized bot chat triggers the existing direct video download and Telegram send flow.
- [x] Pasting `https://x.com/{user}/status/{id}` into an authorized bot chat triggers the existing direct video download and Telegram send flow.
- [x] Existing direct YouTube link downloads still work.
- [x] Existing Reddit post URL handling still works and Twitter/X Reddit post classification is not added.
- [x] Twitter/X profile, search, and other non-status pages are not treated as downloadable video links.
- [x] Direct downloaded videos are recorded with generic video-download wording rather than YouTube-specific wording.
- [x] If multiple files are emitted by `yt-dlp`, the oldest modified output file is selected, with deterministic path ordering as fallback.
- [x] Unit tests cover supported Twitter/X URL recognition, unsupported Twitter/X URL rejection, preserved YouTube recognition, and multi-output file selection.
- [x] Normal verification passes with formatting, linting, and unit tests.

## AFK completed

Verified by AFK verifier. Implemented direct Twitter/X status URL handling in `src/bot.rs` via a new `parse_twitter_status_url` helper that recognizes `twitter.com`, `mobile.twitter.com`, and `x.com` status URLs and routes them through the existing `handle_video_link` flow. YouTube recognition was preserved via a new `is_youtube_url` helper with regex and Reddit handling remained the fallback. `Video::subreddit` was renamed from `"youtube download"` to `"video download"` in `src/types.rs`. `get_video_path` in `src/ytdlp.rs` now sorts files by `(modified_time, path)` so the oldest modified yt-dlp output is selected with deterministic path-order tiebreak. Added unit tests for accepted/rejected Twitter/X URLs, preserved YouTube detection, and multi-output selection. `cargo fmt --check`, `cargo clippy --all-targets`, and `cargo test` (76 passed) all clean.

## Blocked by

None - can start immediately
