use crate::{
    db::Recordable,
    reddit::{self},
    types::{
        CaptionPlacement, PublishVariant, RepostAction, RepostCallbackData, ReviewContentKind,
        RichText, Subscription, Video,
    },
};
use itertools::Itertools;
use teloxide::types::{
    InlineKeyboardButton, InlineKeyboardMarkup, MessageEntity, MessageEntityKind,
};

pub const TELEGRAM_MEDIA_CAPTION_LIMIT: usize = 1_024;
pub const TELEGRAM_TEXT_LIMIT: usize = 4_096;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RichTextError {
    #[error("entity {index} has an empty range")]
    EmptyEntity { index: usize },
    #[error("entity {index} range is outside the text")]
    EntityOutOfBounds { index: usize },
    #[error("entity {index} range splits a UTF-16 surrogate pair")]
    EntityNotOnUtf16Boundary { index: usize },
}

pub fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

pub fn telegram_text_limit(content_kind: ReviewContentKind) -> usize {
    match content_kind {
        ReviewContentKind::Media | ReviewContentKind::Gallery => TELEGRAM_MEDIA_CAPTION_LIMIT,
        ReviewContentKind::Text => TELEGRAM_TEXT_LIMIT,
    }
}

pub fn max_repost_caption_len(
    content_kind: ReviewContentKind,
    metadata: &RichText,
    source_url: &str,
) -> Option<usize> {
    let metadata_suffix_len = if metadata.text.is_empty() {
        0
    } else {
        utf16_len("\n\n") + utf16_len(&metadata.text)
    };
    let source_suffix_len = utf16_len("\n\n") + utf16_len(source_url);
    telegram_text_limit(content_kind).checked_sub(metadata_suffix_len.max(source_suffix_len))
}

fn is_utf16_boundary(text: &str, offset: usize) -> bool {
    offset == 0
        || text
            .chars()
            .scan(0, |position, character| {
                *position += character.len_utf16();
                Some(*position)
            })
            .any(|position| position == offset)
}

pub fn validate_entities(text: &str, entities: &[MessageEntity]) -> Result<(), RichTextError> {
    let text_len = utf16_len(text);
    for (index, entity) in entities.iter().enumerate() {
        if entity.length == 0 {
            return Err(RichTextError::EmptyEntity { index });
        }
        let Some(end) = entity.offset.checked_add(entity.length) else {
            return Err(RichTextError::EntityOutOfBounds { index });
        };
        if end > text_len {
            return Err(RichTextError::EntityOutOfBounds { index });
        }
        if !is_utf16_boundary(text, entity.offset) || !is_utf16_boundary(text, end) {
            return Err(RichTextError::EntityNotOnUtf16Boundary { index });
        }
    }
    Ok(())
}

fn append_rich_text(target: &mut RichText, separator: &str, addition: &RichText) {
    let entity_offset = utf16_len(&target.text) + utf16_len(separator);
    target.text.push_str(separator);
    target.text.push_str(&addition.text);
    target
        .entities
        .extend(addition.entities.iter().cloned().map(|mut entity| {
            entity.offset += entity_offset;
            entity
        }));
}

pub fn compose_review_text(caption: &RichText, metadata: &RichText) -> RichText {
    if caption.text.is_empty() {
        return metadata.clone();
    }
    if metadata.text.is_empty() {
        return caption.clone();
    }

    let mut result = caption.clone();
    append_rich_text(&mut result, "\n\n", metadata);
    result
}

pub fn append_source_url(caption: &RichText, source_url: &str) -> RichText {
    let source = RichText {
        text: source_url.to_owned(),
        entities: vec![MessageEntity::new(
            MessageEntityKind::Url,
            0,
            utf16_len(source_url),
        )],
    };
    let mut result = caption.clone();
    append_rich_text(&mut result, "\n\n", &source);
    result
}

pub fn format_review_url(url: &str) -> RichText {
    RichText {
        text: url.to_owned(),
        entities: vec![MessageEntity::new(
            MessageEntityKind::Url,
            0,
            utf16_len(url),
        )],
    }
}

pub fn format_reddit_review_metadata(post_url: &str) -> RichText {
    format_review_url(post_url)
}

pub fn format_video_review_metadata(video: &Video) -> RichText {
    format_review_url(&video.url)
}

pub fn format_reddit_review(post: &reddit::Post, post_url: &str) -> RichText {
    compose_review_text(
        &RichText {
            text: post.title.clone(),
            entities: Vec::new(),
        },
        &format_reddit_review_metadata(post_url),
    )
}

pub fn format_video_review(video: &Video) -> RichText {
    compose_review_text(
        &RichText {
            text: video.title.clone(),
            entities: Vec::new(),
        },
        &format_video_review_metadata(video),
    )
}

fn callback_data_for_id(post_id: &str, action: RepostAction, is_gallery: bool) -> String {
    let data = serde_json::to_string(&RepostCallbackData {
        action,
        post_id: Some(post_id.to_owned()),
        is_gallery,
    })
    .expect("repost callback data should serialize");
    assert!(data.len() <= 64, "Telegram callback data exceeds 64 bytes");
    data
}

fn action_callback_data(action: RepostAction) -> String {
    let data = serde_json::to_string(&RepostCallbackData {
        action,
        post_id: None,
        is_gallery: false,
    })
    .expect("repost action callback should serialize");
    assert!(data.len() <= 64, "Telegram callback data exceeds 64 bytes");
    data
}

pub fn format_media_repost_buttons<T: Recordable>(
    post: &T,
    is_gallery: bool,
) -> InlineKeyboardMarkup {
    format_media_repost_buttons_for_id(post.id(), is_gallery, CaptionPlacement::BelowMedia)
}

pub fn format_media_repost_buttons_for_id(
    post_id: &str,
    is_gallery: bool,
    caption_placement: CaptionPlacement,
) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::default()
        .append_row([
            InlineKeyboardButton::callback(
                "Post",
                callback_data_for_id(post_id, RepostAction::Post, is_gallery),
            ),
            InlineKeyboardButton::callback(
                "Post (no caption)",
                callback_data_for_id(post_id, RepostAction::PostWithoutCaption, is_gallery),
            ),
            InlineKeyboardButton::callback(
                "Post (with link)",
                callback_data_for_id(post_id, RepostAction::PostWithLink, is_gallery),
            ),
        ])
        .append_row([InlineKeyboardButton::callback(
            caption_placement.toggle_symbol(),
            callback_data_for_id(post_id, RepostAction::ToggleCaptionPlacement, is_gallery),
        )])
}

pub fn format_text_repost_buttons<T: Recordable>(post: &T) -> InlineKeyboardMarkup {
    format_text_repost_buttons_for_id(post.id())
}

pub fn format_text_repost_buttons_for_id(post_id: &str) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::default().append_row([
        InlineKeyboardButton::callback(
            "Post",
            callback_data_for_id(post_id, RepostAction::Post, false),
        ),
        InlineKeyboardButton::callback(
            "Post (with link)",
            callback_data_for_id(post_id, RepostAction::PostWithLink, false),
        ),
    ])
}

pub fn format_publish_confirmation_buttons(variant: PublishVariant) -> InlineKeyboardMarkup {
    let confirm_label = match variant {
        PublishVariant::Caption => "Confirm post",
        PublishVariant::WithoutCaption => "Confirm without caption",
        PublishVariant::WithLink => "Confirm with link",
    };
    InlineKeyboardMarkup::default().append_row([
        InlineKeyboardButton::callback(
            confirm_label,
            action_callback_data(RepostAction::ConfirmPublish),
        ),
        InlineKeyboardButton::callback("Cancel", action_callback_data(RepostAction::CancelPublish)),
    ])
}

pub fn format_subscription_list(post: &[Subscription]) -> String {
    fn format_subscription(sub: &Subscription) -> String {
        let mut args = vec![];
        if let Some(time) = sub.time {
            args.push(format!("time={time}"));
        }
        if let Some(limit) = sub.limit {
            args.push(format!("limit={limit}"));
        }
        if let Some(filter) = sub.filter {
            args.push(format!("filter={filter}"));
        }

        let args_str = if !args.is_empty() {
            format!("({})", args.join(", "))
        } else {
            "".to_string()
        };

        [sub.subreddit.to_owned(), args_str]
            .join(" ")
            .trim_end()
            .to_string()
    }

    if post.is_empty() {
        "No subscriptions".to_owned()
    } else {
        post.iter().map(format_subscription).join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{db::Recordable, reddit::TopPostsTimePeriod};
    use std::collections::HashMap;
    use teloxide::types::InlineKeyboardButtonKind;

    struct TestPost;

    impl Recordable for TestPost {
        fn id(&self) -> &str {
            "abc123"
        }

        fn title(&self) -> &str {
            "title"
        }

        fn subreddit(&self) -> &str {
            "test"
        }
    }

    fn reddit_post(permalink: &str, url: &str) -> reddit::Post {
        reddit::Post {
            id: "abc123".to_owned(),
            subreddit: "test".to_owned(),
            title: "A title".to_owned(),
            permalink: permalink.to_owned(),
            url: url.to_owned(),
            post_hint: None,
            post_type: reddit::PostType::Link,
            gallery_data: None,
            media_metadata: Some(HashMap::new()),
        }
    }

    fn button_action(button: &InlineKeyboardButton) -> RepostAction {
        let InlineKeyboardButtonKind::CallbackData(data) = &button.kind else {
            panic!("expected callback button")
        };
        crate::types::decode_repost_callback(data).unwrap().action
    }

    #[test]
    fn media_repost_buttons_offer_all_publish_variants() {
        let keyboard = format_media_repost_buttons(&TestPost, true);
        let buttons = &keyboard.inline_keyboard[0];

        assert_eq!(
            buttons
                .iter()
                .map(|button| button.text.as_str())
                .collect_vec(),
            ["Post", "Post (no caption)", "Post (with link)"]
        );
        assert_eq!(
            buttons.iter().map(button_action).collect_vec(),
            [
                RepostAction::Post,
                RepostAction::PostWithoutCaption,
                RepostAction::PostWithLink,
            ]
        );
        let placement = &keyboard.inline_keyboard[1];
        assert_eq!(placement[0].text, "⬆️");
        assert_eq!(
            button_action(&placement[0]),
            RepostAction::ToggleCaptionPlacement
        );
    }

    #[test]
    fn media_repost_buttons_describe_the_next_caption_placement() {
        let keyboard =
            format_media_repost_buttons_for_id("abc123", false, CaptionPlacement::AboveMedia);

        assert_eq!(keyboard.inline_keyboard[1][0].text, "⬇️");
        assert_eq!(
            button_action(&keyboard.inline_keyboard[1][0]),
            RepostAction::ToggleCaptionPlacement
        );
    }

    #[test]
    fn text_repost_buttons_offer_caption_and_link_variants() {
        let keyboard = format_text_repost_buttons(&TestPost);
        let buttons = &keyboard.inline_keyboard[0];

        assert_eq!(
            buttons
                .iter()
                .map(|button| button.text.as_str())
                .collect_vec(),
            ["Post", "Post (with link)"]
        );
        assert_eq!(
            buttons.iter().map(button_action).collect_vec(),
            [RepostAction::Post, RepostAction::PostWithLink]
        );
    }

    #[test]
    fn confirmation_buttons_describe_the_selected_variant() {
        for (variant, label) in [
            (PublishVariant::Caption, "Confirm post"),
            (PublishVariant::WithoutCaption, "Confirm without caption"),
            (PublishVariant::WithLink, "Confirm with link"),
        ] {
            let keyboard = format_publish_confirmation_buttons(variant);
            let buttons = &keyboard.inline_keyboard[0];
            assert_eq!(buttons[0].text, label);
            assert_eq!(buttons[1].text, "Cancel");
            assert_eq!(button_action(&buttons[0]), RepostAction::ConfirmPublish);
            assert_eq!(button_action(&buttons[1]), RepostAction::CancelPublish);
        }
    }

    #[test]
    fn reddit_metadata_is_one_raw_source_url() {
        let post = reddit_post(
            "/r/test/comments/abc123/a_title/",
            "https://example.com/article",
        );
        let comments = post.format_permalink_url(None);
        let metadata = format_reddit_review_metadata(&comments);

        assert_eq!(metadata.text, comments);
        assert_eq!(metadata.entities.len(), 1);
        validate_entities(&metadata.text, &metadata.entities).unwrap();
        assert_eq!(metadata.entities[0].kind, MessageEntityKind::Url);
        assert_eq!(metadata.entities[0].offset, 0);
        assert_eq!(metadata.entities[0].length, utf16_len(&metadata.text));
    }

    #[test]
    fn rich_text_composition_preserves_utf16_entity_offsets() {
        let caption = RichText {
            text: "😀 bold".to_owned(),
            entities: vec![MessageEntity::bold(3, 4)],
        };
        let published = append_source_url(&caption, "https://example.com/😀");

        assert_eq!(published.text, "😀 bold\n\nhttps://example.com/😀");
        assert_eq!(published.entities[0], MessageEntity::bold(3, 4));
        assert_eq!(published.entities[1].offset, 9);
        assert_eq!(published.entities[1].length, 22);
        assert_eq!(published.entities[1].kind, MessageEntityKind::Url);
        validate_entities(&published.text, &published.entities).unwrap();
    }

    #[test]
    fn caption_limit_reserves_the_larger_visible_suffix() {
        let metadata = RichText {
            text: "Source: https://example.com".to_owned(),
            entities: Vec::new(),
        };
        assert_eq!(
            max_repost_caption_len(ReviewContentKind::Media, &metadata, "https://short.example",),
            Some(1_024 - utf16_len("\n\nSource: https://example.com"))
        );
        assert_eq!(
            max_repost_caption_len(
                ReviewContentKind::Text,
                &RichText::default(),
                "https://example.com/😀",
            ),
            Some(4_096 - utf16_len("\n\nhttps://example.com/😀"))
        );
    }

    #[test]
    fn entity_validation_rejects_surrogate_splits_and_out_of_bounds_ranges() {
        assert_eq!(
            validate_entities("😀x", &[MessageEntity::bold(1, 1)]),
            Err(RichTextError::EntityNotOnUtf16Boundary { index: 0 })
        );
        assert_eq!(
            validate_entities("abc", &[MessageEntity::bold(1, 3)]),
            Err(RichTextError::EntityOutOfBounds { index: 0 })
        );
        assert_eq!(
            validate_entities("abc", &[MessageEntity::bold(1, 0)]),
            Err(RichTextError::EmptyEntity { index: 0 })
        );
    }

    #[test]
    fn test_format_subscription_list() {
        assert_eq!(
            format_subscription_list(&[
                Subscription {
                    chat_id: 1,
                    subreddit: "foo".to_owned(),
                    limit: None,
                    time: None,
                    filter: None,
                },
                Subscription {
                    chat_id: 1,
                    subreddit: "bar".to_owned(),
                    limit: Some(1),
                    time: Some(TopPostsTimePeriod::Week),
                    filter: None,
                },
            ]),
            "foo\nbar (time=week, limit=1)"
        )
    }
}
