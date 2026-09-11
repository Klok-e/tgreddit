# TGReddit

TGReddit delivers posts to an operator for review and lets the operator publish
selected posts to a Telegram channel.

## Language

**Channel Download Bot**:
The operator's private chat with the bot, where posts are delivered for review
before publication.

**Repost Channel**:
The Telegram channel where the operator publishes selected posts.

**Repost Caption**:
The standalone rich text attached to a post in the Repost Channel. It does not include private review metadata unless the operator explicitly supplies that text.
_Avoid_: Title, description

**Review Post**:
The message in the Channel Download Bot that shows the current Repost Caption, one visible link to the Reddit post, and the controls for selecting an output. Direct media submissions show their submitted URL. It remains the authoritative preview while the operator edits or confirms publication.

**Source URL**:
The one visible URL appended by the `Post (with link)` Publish Variant. For downloaded media it is the exact input used to download the media; for Reddit galleries and self-text posts it is the Reddit submission permalink.

**Publish Variant**:
The selected Repost Channel output: the current Repost Caption, media without a caption, or the current Repost Caption followed by the Source URL.

**Selected-output review**:
The confirmation state entered after choosing a Publish Variant. The Review Post keeps the proposed content visible and offers variant-specific confirmation and cancellation before publishing.
