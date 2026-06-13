Status: ready-for-agent

## Parent

.scratch/twitter-video-downloads/PRD.md

## What to build

Add end-to-end support for direct Twitter/X status video downloads. When an authorized Telegram user pastes a public Twitter/X status URL, the bot should recognize it as a direct video link, download it through the existing `yt-dlp` video-download flow, record it in the existing duplicate/history tracking, and send it to Telegram with the same caption and repost-button behavior used by existing direct video downloads.

The supported URL shapes are Twitter status URLs, mobile Twitter status URLs, and X status URLs. Non-status Twitter/X pages should not be treated as downloadable video links. Existing direct YouTube downloads and Reddit link handling should continue to behave as they do today.

If `yt-dlp` writes multiple output files, the bot should send the file with the oldest modified timestamp. If timestamps cannot distinguish files, selection should fall back to deterministic path ordering. The direct-download record label should use generic video-download wording rather than YouTube-specific wording.

## Acceptance criteria

- [ ] Pasting `https://twitter.com/{user}/status/{id}` into an authorized bot chat triggers the existing direct video download and Telegram send flow.
- [ ] Pasting `https://mobile.twitter.com/{user}/status/{id}` into an authorized bot chat triggers the existing direct video download and Telegram send flow.
- [ ] Pasting `https://x.com/{user}/status/{id}` into an authorized bot chat triggers the existing direct video download and Telegram send flow.
- [ ] Existing direct YouTube link downloads still work.
- [ ] Existing Reddit post URL handling still works and Twitter/X Reddit post classification is not added.
- [ ] Twitter/X profile, search, and other non-status pages are not treated as downloadable video links.
- [ ] Direct downloaded videos are recorded with generic video-download wording rather than YouTube-specific wording.
- [ ] If multiple files are emitted by `yt-dlp`, the oldest modified output file is selected, with deterministic path ordering as fallback.
- [ ] Unit tests cover supported Twitter/X URL recognition, unsupported Twitter/X URL rejection, preserved YouTube recognition, and multi-output file selection.
- [ ] Normal verification passes with formatting, linting, and unit tests.

## Blocked by

None - can start immediately
