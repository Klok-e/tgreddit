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

**X Tweet Body**:
The text TGReddit derives from an X Tweet for its default Repost Caption, excluding all `t.co` URLs while retaining other URLs. When that Tweet directly quotes another Tweet, the quoted Tweet's cleaned text follows the source text under a `Quoted @author:` label. It remains the caption source when the Tweet has media.
_Avoid_: Tweet description, X metadata

**Direct Media Submission**:
A media URL supplied directly to the Channel Download Bot. It produces one Review Post and is not automatically replayed.
_Avoid_: Direct download, media playlist

**X Tweet Submission**:
A Direct Media Submission whose URL identifies an X Tweet. A repost resolves to its original Tweet. It produces a text Review Post when that Tweet has no attached media, otherwise one media Review Post containing every photo and video attached to that Tweet in X order. A directly quoted Tweet contributes text to the X Tweet Body but no media.
_Avoid_: X status, Tweet downloader

**Unretrievable X Tweet Submission**:
An X Tweet Submission whose Tweet data cannot be retrieved or whose complete attached media cannot be downloaded. It produces no Review Post and reports the retrieval failure to the operator.
_Avoid_: Download error, failed link

**Review Post**:
The message in the Channel Download Bot that shows the current Repost Caption, one visible link to the Reddit post, and the controls for selecting an output. Direct media submissions show their submitted URL. It remains the authoritative preview while the operator edits or confirms publication.

**Source URL**:
The one visible URL appended by the `Post (with link)` Publish Variant. For downloaded media it is the exact input used to download the media; for Reddit galleries and self-text posts it is the Reddit submission permalink.

**Publish Variant**:
The selected Repost Channel output: the current Repost Caption, media without a caption, or the current Repost Caption followed by the Source URL.

**Selected-output review**:
The confirmation state entered after choosing a Publish Variant. The Review Post keeps the proposed content visible and offers variant-specific confirmation and cancellation before publishing.
