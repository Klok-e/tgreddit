# tgreddit

A telegram bot that gives you a feed of top posts from your favorite subreddits.

This repository is a fork of [raine/tgreddit](https://github.com/raine/tgreddit), maintained for this deployment and its additional behavior.

The killer feature: No need to visit Reddit, as all media is embedded thanks to
[yt-dlp][yt-dlp] and Telegram's excellent media support.

Intended to be self-hosted, as Reddit's API has rate-limiting and downloading
videos with `yt-dlp` can be resource intensive.

<img align=left src="https://user-images.githubusercontent.com/11027/178097057-83b27933-9876-405a-b151-a148960819df.jpeg" width=20% height=20%>
<img align=left src="https://user-images.githubusercontent.com/11027/178096986-5f651336-8208-4c40-9c41-58c95173b24d.jpeg" width=20% height=20%>
<img src="https://user-images.githubusercontent.com/11027/178099572-e55c7f3c-986b-4804-8540-1004b36950df.jpeg" width=20% height=20%>

## install

```sh
$ cargo install tgreddit
```

### requirements

Depends on [yt-dlp][yt-dlp] and ffmpeg. Reliable YouTube downloads also require [Deno](https://deno.com/) on `PATH` so yt-dlp can solve YouTube's JavaScript challenges.

### Raspberry Pi service

[`deploy/systemd/tgreddit.service`](deploy/systemd/tgreddit.service) is the production service definition for this fork's Raspberry Pi deployment. It expects TGReddit, pipx-managed yt-dlp, Deno, and the configuration file at the paths declared in the unit. The best-effort startup maintenance commands update yt-dlp and its injected curl-cffi dependency before launching the bot.

## testing

Normal validation:

```sh
cargo fmt --check
cargo clippy
cargo test
```

The X Tweet provider contract test is ignored because it calls FxTwitter. Run it from the local workstation when validating X Tweet support:

```sh
cargo test --test x_tweet_live -- --ignored --nocapture
```

Live Telegram E2E tests are ignored because they use real network, Reddit
fixtures, and the configured Telegram test channel. They read app settings from
`tgreddit.toml` and the target chat from `telegram-e2e.toml`.

```sh
cp telegram-e2e.example.toml telegram-e2e.toml
CONFIG_PATH=tgreddit.toml cargo test --test telegram_e2e -- --ignored --nocapture
```

The E2E tests intentionally leave sent messages in the test channel.

## bot commands

### `/sub <subreddit> [limit=<limit>] [time=<time>] [filter=<filter>]`

Add a subscription to subreddit's top posts with optional options. Subscriptions
are conversation specific, and may be added in channels where the bot is
participating or in private chats with the bot.

If the options are not given, when checking for new posts, the program will
default to configuration in config.toml, if any.

Example: `/sub AnimalsBeingJerks limit=5 time=week filter=video`

Explanation: Subscribe to top posts in r/AnimalsBeingJerks so that the top 5
posts of the weekly top list are considered. Whenever a new post appears among
those top 5 posts, they will be posted in the conversation.

See the
[example configuration](#example-toml-configuration-with-the-options-explained)
below for further explanation on `limit`, `time`, and `filter`.

### `/unsub <subreddit>`

Remove a subscription from the current conversation.

### `/listsubs`

List all subreddit subscriptions for the current conversation.

### `/get <subreddit> [limit=<limit>] [time=<time>] [filter=<filter>]`

Get the current top posts similarly to how subscribing to a subreddit would
return new posts.

### Reviewing and reposting

Every private Reddit review message shows one visible, clickable link to the Reddit post. Directly submitted media shows its submitted URL. Media and galleries include **Post**, **Post (no caption)**, and **Post (with link)** buttons plus a second-row arrow: **⬆️** moves the caption above media and becomes **⬇️**, which moves it back below. Link and self-text posts include **Post** and **Post (with link)**.

For an X/Twitter Tweet, TGReddit fetches text and attached media through the configured FxTwitter-compatible API. It posts every source Tweet photo and video in order, while a directly quoted Tweet contributes labeled text only. The Repost Caption keeps line breaks and non-`t.co` links but removes all `t.co` URLs. If it cannot fit Telegram's media-caption limit with the visible source link, TGReddit shortens it at a Unicode boundary and adds an ellipsis. A Tweet without source media produces a text Review Post.

**Post** selects the current Repost Caption. **Post (no caption)** selects media without a caption. **Post (with link)** appends a blank line and the exact Source URL; this is the submitted download URL for directly downloaded media and the Reddit submission permalink for galleries and self-text posts. Caption placement is stored per media Review Post, persists across restart, and moves the whole `Post (with link)` caption with its Source URL.

Choosing a variant replaces the review keyboard with a variant-specific **Confirm** button and **Cancel**. Captioned variants also open a ForceReply editor. Telegram cannot prefill a reply, so the prompt shows the current caption for reference. A reply replaces the Repost Caption on the original review message, preserves Telegram-native formatting such as bold, italic, spoilers, code, quotes, and embedded links, and removes both the prompt and reply after a successful edit.

Only one edit can be active per private chat. Starting another edit cancels the previous prompt. Completed captions, rich-text formatting, Source URLs, review targets, and pending confirmation choices are stored in SQLite so review posts remain usable after a restart; unfinished ForceReply prompts may be cancelled by a restart.

Telegram limits media captions to 1,024 UTF-16 units and text messages to 4,096. TGReddit reserves room for the visible post link and the Source URL before accepting an edit, reports the available limit when a reply is too long, and never truncates an edited caption or URL. The initial automatic caption for an over-limit X/Twitter Tweet is the exception: it is shortened with an ellipsis as described above.

After successful publication, the bot removes the review buttons to prevent duplicate posts. Cancellation restores the previous keyboard, and a Telegram publication failure preserves the edited caption and restores the keyboard for retrying.

## configuration

### env vars

- `CONFIG_PATH`: Path to TOML configuration file. **required**

### example toml configuration with the options explained

Example config without comments:
[config.example.toml](https://raw.githubusercontent.com/raine/tgreddit/master/config.example.toml)

```toml
# Path to a SQLite database used to track seen posts.
# Optional. Defaults to $HOME/.local/state/tgreddit/data.db3.
db_path = "/path/to/data.db3"

# List of Telegram user ids that can use the commands provided by the bot.
authorized_users = [
  123123123
]

# Token of your Telegram bot - you get this from @botfather.
telegram_bot_token = "..."

# How often to query each configured subreddit for new posts. Applies only if
# keep_running is enabled.
check_interval_secs = 600

# Whether posts seen on the first check of a new subreddit are considered new
# or not. Generally having this enabled is better unless you want multiple new
# messages when a new subreddit is added.
# Optional. Defaults to true.
skip_initial_send = true

# FxTwitter-compatible API used to retrieve X Tweet text, photos, and videos.
# Optional. Defaults to https://api.fxtwitter.com. Set this to a self-hosted
# FxTwitter-compatible instance to keep X Tweet lookups on infrastructure you control.
x_tweet_api_base_url = "https://api.fxtwitter.com"

# Set the post comments links to use an alternative frontend. Useful as the
# official Reddit web app is increasingly user hostile on mobile. Possible
# alternative frontends include teddit.net and libredd.it, but you can use any.
# Optional. Defaults to official Reddit.
links_base_url = "https://teddit.net"

# Set default limit of posts to fetch for each subreddit. Used when not
# specified for a subreddit in the /sub command.
#
# Explanation in more detail: Whenever the bot gets the list of top posts for a
# subreddit, it will only consider the first <limit> posts. For example, if
# your limit is 5, the first time around bot will see 5 new posts and mark those
# as seen and not post anything because it's the first check. Next time around, if
# there's an unseen post among those 5 top posts, it will be posted in Telegram.
#
# So essentially larger the number used as limit, the more posts you can
# expect to see. For example, with time=month and limit=1 you would see a new post
# only when the montly top post changes, which is not that often.
#
# Optional. The default is 1.
default_limit = 1

# Set default time period of top list fetched. Used when not specified for a
# subreddit. String and one of: hour, day, week, month, year, all.
# Optional. The default is `day`.
default_time = "day"

# Set default filter for post type. When fetching for new posts, only posts
# matching the filter are considered.
# String and one of: image, video, link, self_text, gallery
# Optional and unset by default, meaning all post types are considered.
default_filter = "video"
```

Perhaps the simplest way to determine a Telegram channel's ID is to open the
channel in [Telegram Web client][telegram-web] and observing the numeric value
in page URL.

## reddit transport

The bot fetches Reddit data through a Redlib-style anonymous OAuth
transport: it acquires a Reddit bearer token and then calls
`https://oauth.reddit.com` JSON endpoints with app-like request
headers. There is no HTML scraping, no public-Redlib-instance
dependency, and no fallback chain.

See [`docs/runtime/reddit-oauth.md`](docs/runtime/reddit-oauth.md) for
the transport overview, its fragility and rate-limit risks, and the
manual smoke checklist for parity with the existing image, hosted
video, external link, self post, and gallery delivery paths.

## have an idea, question or a bug report?

Feel free to open an issue or start a new discussion.

[yt-dlp]: https://github.com/yt-dlp/yt-dlp
[telegram-web]: https://web.telegram.org/
