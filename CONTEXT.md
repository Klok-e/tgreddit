# Glossary

This repo uses the following domain terms. Names in code, issues, and PRs
should match these exactly.

- **Channel Download Bot** — the operator's private chat with the bot. The
  bot delivers new Reddit posts to this chat, attached with an inline
  repost button. The operator can then move a post to the Repost Channel
  by tapping the button.

- **Repost Channel** — the Telegram channel the bot copies a delivered
  post into when the operator taps the inline repost button. The channel
  is registered for the operator via the `/register_channel` command.
