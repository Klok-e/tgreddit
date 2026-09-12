use serde_derive::{Deserialize, Serialize};
use tempfile::TempDir;

use crate::{
    db::Recordable,
    reddit::{PostType, TopPostsTimePeriod},
};
use std::path::PathBuf;
use teloxide::types::{FileId, InlineKeyboardMarkup, MessageEntity, MessageId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Photo,
    Video,
}

impl MediaKind {
    pub(crate) const fn as_db_value(self) -> &'static str {
        match self {
            Self::Photo => "photo",
            Self::Video => "video",
        }
    }

    pub(crate) fn from_db_value(value: &str) -> Option<Self> {
        match value {
            "photo" => Some(Self::Photo),
            "video" => Some(Self::Video),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelegramMediaFile {
    pub file_id: FileId,
    pub kind: MediaKind,
}

#[derive(Debug)]
pub struct Video {
    pub path: PathBuf,
    pub url: String,
    pub id: String,
    pub title: String,
    pub width: u16,
    pub height: u16,
    pub _video_tempdir: TempDir,
}

impl Recordable for Video {
    fn id(&self) -> &str {
        &self.id
    }

    fn title(&self) -> &str {
        &self.title
    }

    fn subreddit(&self) -> &str {
        "video download"
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Subscription {
    pub chat_id: i64,
    pub subreddit: String,
    pub limit: Option<u32>,
    pub time: Option<TopPostsTimePeriod>,
    pub filter: Option<PostType>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionArgs {
    pub subreddit: String,
    pub limit: Option<u32>,
    pub time: Option<TopPostsTimePeriod>,
    pub filter: Option<PostType>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepostAction {
    #[serde(rename = "p")]
    Post,
    #[serde(rename = "n")]
    PostWithoutCaption,
    #[serde(rename = "l")]
    PostWithLink,
    #[serde(rename = "f")]
    ConfirmPublish,
    #[serde(rename = "c")]
    CancelPublish,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PublishVariant {
    #[serde(rename = "p")]
    Caption,
    #[serde(rename = "n")]
    WithoutCaption,
    #[serde(rename = "l")]
    WithLink,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewContentKind {
    #[serde(rename = "m")]
    Media,
    #[serde(rename = "g")]
    Gallery,
    #[serde(rename = "t")]
    Text,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RichText {
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entities: Vec<MessageEntity>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewPost {
    pub chat_id: i64,
    pub post_id: String,
    pub source_url: String,
    pub caption: RichText,
    pub content_kind: ReviewContentKind,
    pub review_message_id: MessageId,
    pub control_message_id: MessageId,
    pub metadata: RichText,
    pub pending_publish_variant: Option<PublishVariant>,
    pub previous_keyboard: Option<InlineKeyboardMarkup>,
    pub published_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepostCallbackData {
    #[serde(rename = "a")]
    pub action: RepostAction,
    #[serde(rename = "p", skip_serializing_if = "Option::is_none")]
    pub post_id: Option<String>,
    #[serde(rename = "g", default, skip_serializing_if = "is_false")]
    pub is_gallery: bool,
}

fn is_false(value: &bool) -> bool {
    !value
}

pub fn decode_repost_callback(data: &str) -> serde_json::Result<RepostCallbackData> {
    serde_json::from_str(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_new_repost_callback() {
        let encoded = serde_json::to_string(&RepostCallbackData {
            action: RepostAction::Post,
            post_id: Some("abc123".to_owned()),
            is_gallery: true,
        })
        .unwrap();

        assert_eq!(encoded, r#"{"a":"p","p":"abc123","g":true}"#);
        assert!(encoded.len() <= 64);
        assert_eq!(
            decode_repost_callback(&encoded).unwrap(),
            RepostCallbackData {
                action: RepostAction::Post,
                post_id: Some("abc123".to_owned()),
                is_gallery: true,
            }
        );
    }

    #[test]
    fn rejects_legacy_repost_callbacks() {
        let with_caption = r#"{"n":"abc123","c":true,"d":true}"#;
        let without_caption = r#"{"n":"abc123","c":false,"d":false}"#;
        let legacy_edit_caption = r#"{"a":"e","p":"abc123"}"#;
        let legacy_publish_caption = r#"{"a":"y"}"#;

        assert!(decode_repost_callback(with_caption).is_err());
        assert!(decode_repost_callback(without_caption).is_err());
        assert!(decode_repost_callback(legacy_edit_caption).is_err());
        assert!(decode_repost_callback(legacy_publish_caption).is_err());
    }

    #[test]
    fn all_current_callback_actions_fit_telegram_limit() {
        let actions = [
            RepostAction::Post,
            RepostAction::PostWithoutCaption,
            RepostAction::PostWithLink,
            RepostAction::ConfirmPublish,
            RepostAction::CancelPublish,
        ];

        for action in actions {
            let encoded = serde_json::to_string(&RepostCallbackData {
                action,
                post_id: Some("a2345678901234567890123456789012".to_owned()),
                is_gallery: true,
            })
            .unwrap();
            assert!(
                encoded.len() <= 64,
                "{action:?} callback is {} bytes: {encoded}",
                encoded.len()
            );
            assert_eq!(decode_repost_callback(&encoded).unwrap().action, action);
        }
    }

    #[test]
    fn publish_confirmation_callbacks_need_no_post_data() {
        for (action, expected) in [
            (RepostAction::ConfirmPublish, r#"{"a":"f"}"#),
            (RepostAction::CancelPublish, r#"{"a":"c"}"#),
        ] {
            let encoded = serde_json::to_string(&RepostCallbackData {
                action,
                post_id: None,
                is_gallery: false,
            })
            .unwrap();
            assert_eq!(encoded, expected);
            assert_eq!(decode_repost_callback(&encoded).unwrap().action, action);
        }
    }
}
