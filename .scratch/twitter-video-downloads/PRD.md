# Twitter/X Video Downloads

Status: ready-for-agent

## Problem Statement

The bot can currently download direct YouTube links pasted into an authorized Telegram chat, but it does not recognize Twitter/X status links as downloadable video links. When a user wants to repost a video from a tweet, they need the bot to fetch the tweet media and send it to Telegram through the same delivery and repost flow used for existing direct video downloads.

The user does not need Reddit posts that link to Twitter/X to be handled as downloadable videos, because those posts are rare for this workflow.

## Solution

Authorized users can paste a public Twitter/X status URL into the bot chat. The bot recognizes supported tweet URL shapes, downloads the video through the existing `yt-dlp` based video downloader, records the downloaded video in the existing seen-post tracking, and sends it to Telegram with the same caption and repost buttons used for existing direct video downloads.

Supported URL shapes are:

- `https://twitter.com/{user}/status/{id}`
- `https://mobile.twitter.com/{user}/status/{id}`
- `https://x.com/{user}/status/{id}`

When `yt-dlp` writes multiple output files for a tweet, the bot sends the file with the oldest modified timestamp. If timestamps cannot distinguish files, path ordering is used as a deterministic fallback.

## User Stories

1. As an authorized Telegram user, I want to paste a Twitter status URL, so that the bot downloads the tweet video into the chat.
2. As an authorized Telegram user, I want to paste an X status URL, so that the bot works with the current Twitter domain.
3. As an authorized Telegram user, I want to paste a mobile Twitter status URL, so that copied mobile links work without manual editing.
4. As an authorized Telegram user, I want Twitter/X downloads to use the existing video delivery flow, so that captions and repost buttons behave consistently with YouTube downloads.
5. As an authorized Telegram user, I want a downloaded Twitter/X video to be recorded in duplicate tracking, so that repeated links do not create inconsistent history behavior.
6. As an authorized Telegram user, I want unsupported Twitter/X pages such as profiles and searches to be ignored by the video-link detector, so that the bot does not try to download arbitrary pages.
7. As an authorized Telegram user, I want Reddit links to keep working as they do today, so that adding Twitter/X does not break normal Reddit post handling.
8. As an authorized Telegram user, I want existing YouTube link support to keep working, so that the new Twitter/X support is additive.
9. As an authorized Telegram user, I want public tweet videos to download without additional setup, so that the first version is simple to use.
10. As an authorized Telegram user, I want a clear error message when `yt-dlp` cannot download a tweet, so that failures are understandable in the existing bot error style.
11. As a maintainer, I want the video-link matching code to use generic naming rather than YouTube-only naming, so that the code reflects its broader behavior.
12. As a maintainer, I want the internal record label for direct downloads to stop saying `youtube download`, so that Twitter/X downloads are not mislabeled.
13. As a maintainer, I want multi-output `yt-dlp` behavior to be deterministic, so that tweet downloads do not depend on filesystem iteration order.
14. As a maintainer, I want unit coverage for supported and unsupported Twitter/X URLs, so that URL recognition does not regress.
15. As a maintainer, I want unit coverage for selecting the oldest modified output file, so that multi-output tweet behavior remains stable.
16. As a maintainer, I want no new Twitter API integration, so that the feature avoids extra credentials, rate limits, and API maintenance burden.
17. As a maintainer, I want no cookie/auth configuration in the first version, so that secret handling and operational complexity stay unchanged.
18. As a maintainer, I want the implementation to reuse the existing downloader seam, so that the feature stays small and does not duplicate media download logic.

## Implementation Decisions

- Direct Telegram message handling will recognize Twitter/X status URLs in addition to the existing YouTube URL recognition.
- Twitter/X support is limited to direct pasted links. Reddit post classification will not be changed to treat Twitter/X links as downloadable video posts.
- The feature will keep using the existing `yt-dlp` based downloader rather than adding a Twitter-specific downloader or API integration.
- Supported Twitter/X hosts are `twitter.com`, `mobile.twitter.com`, and `x.com`.
- Supported Twitter/X paths must identify a status URL with a user segment and `/status/{id}` segment.
- Profile URLs, search URLs, home URLs, and other non-status Twitter/X pages are out of scope for URL matching.
- Existing YouTube direct-link support must remain supported.
- Video-link naming in command/message handling should become generic, or the code should clearly separate YouTube and Twitter/X matchers without retaining misleading YouTube-only comments for shared behavior.
- When `yt-dlp` outputs more than one file, the downloader will choose the file with the oldest modified timestamp.
- If file modification timestamps are unavailable or equal, deterministic path ordering will be used as a fallback.
- The direct-download duplicate/history label currently tied to YouTube should be renamed to a generic label such as `video download`.
- No database schema changes are expected.
- No configuration changes are expected.
- No new runtime dependency is expected beyond the existing `yt-dlp` requirement.
- Existing Telegram delivery behavior for a direct downloaded video remains unchanged: send the downloaded file as a video, include HTML caption, include repost buttons, and log upload success.

## Testing Decisions

- Tests should cover externally observable decisions at stable seams rather than testing regex internals directly where avoidable.
- URL detection should be tested with accepted Twitter/X status URL shapes and rejected non-status Twitter/X pages.
- Existing YouTube URL detection should be covered or preserved by tests so the feature is additive.
- Reddit URL handling should remain covered by existing behavior; this PRD does not require new Reddit post classification tests.
- Downloader output selection should be tested without invoking real `yt-dlp` by creating multiple files in a temporary directory with controlled modification timestamps.
- The multi-output selection test should verify that the oldest modified file is selected.
- The multi-output selection test should verify deterministic fallback behavior when timestamps tie or cannot distinguish files.
- Live Telegram integration tests are not required for this PRD unless implementation changes Telegram delivery behavior beyond recognizing Twitter/X URLs.
- Normal verification should include formatting, linting, and unit tests according to the repository guidelines.

## Out of Scope

- Downloading Twitter/X media from Reddit posts that link to Twitter/X.
- Twitter/X profile pages, search pages, home pages, moments, spaces, communities, or other non-status pages.
- Private, deleted, age-restricted, login-gated, or otherwise auth-required Twitter/X content.
- Cookie configuration for `yt-dlp`.
- Twitter API integration.
- Sending all media from tweets with multiple video/media outputs.
- Telegram media groups or album handling for multi-output tweets.
- New bot commands for Twitter/X downloads.
- New configuration keys.
- Database schema changes.

## Further Notes

- This feature is intentionally a minimal extension of the existing direct video download flow.
- `yt-dlp` support for Twitter/X can be brittle due to upstream site changes; failures should surface through the existing bot error handling path.
- If public Twitter/X downloads prove unreliable without cookies, cookie support should be considered as a separate PRD or issue.
