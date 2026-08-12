use serde_derive::{Deserialize, Serialize};
use tempfile::TempDir;

use crate::{
    db::Recordable,
    reddit::{PostType, TopPostsTimePeriod},
};
use std::path::PathBuf;

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

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename = "BtnDt")]
pub struct ButtonCallbackData {
    #[serde(rename = "n")]
    pub post_id: String,
    #[serde(rename = "c")]
    pub copy_caption: bool,
    #[serde(rename = "d")]
    pub is_gallery: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepostAction {
    #[serde(rename = "p")]
    Post,
    #[serde(rename = "n")]
    PostWithoutCaption,
    #[serde(rename = "e")]
    EditCaption,
    #[serde(rename = "y")]
    PublishCaption,
    #[serde(rename = "x")]
    CancelCaption,
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

#[derive(Debug, PartialEq, Eq)]
pub struct DecodedRepostCallback {
    pub action: RepostAction,
    pub post_id: Option<String>,
    pub is_gallery: bool,
}

pub fn decode_repost_callback(data: &str) -> serde_json::Result<DecodedRepostCallback> {
    if let Ok(data) = serde_json::from_str::<RepostCallbackData>(data) {
        return Ok(DecodedRepostCallback {
            action: data.action,
            post_id: data.post_id,
            is_gallery: data.is_gallery,
        });
    }

    let legacy = serde_json::from_str::<ButtonCallbackData>(data)?;
    Ok(DecodedRepostCallback {
        action: if legacy.copy_caption {
            RepostAction::Post
        } else {
            RepostAction::PostWithoutCaption
        },
        post_id: Some(legacy.post_id),
        is_gallery: legacy.is_gallery,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_new_repost_callback() {
        let encoded = serde_json::to_string(&RepostCallbackData {
            action: RepostAction::EditCaption,
            post_id: Some("abc123".to_owned()),
            is_gallery: true,
        })
        .unwrap();

        assert!(encoded.len() <= 64);
        assert_eq!(
            decode_repost_callback(&encoded).unwrap(),
            DecodedRepostCallback {
                action: RepostAction::EditCaption,
                post_id: Some("abc123".to_owned()),
                is_gallery: true,
            }
        );
    }

    #[test]
    fn decodes_legacy_repost_callbacks() {
        let with_caption = r#"{"n":"abc123","c":true,"d":true}"#;
        let without_caption = r#"{"n":"abc123","c":false,"d":false}"#;

        assert_eq!(
            decode_repost_callback(with_caption).unwrap().action,
            RepostAction::Post
        );
        assert_eq!(
            decode_repost_callback(without_caption).unwrap().action,
            RepostAction::PostWithoutCaption
        );
    }

    #[test]
    fn confirmation_callback_is_compact() {
        let encoded = serde_json::to_string(&RepostCallbackData {
            action: RepostAction::PublishCaption,
            post_id: None,
            is_gallery: false,
        })
        .unwrap();

        assert_eq!(encoded, r#"{"a":"y"}"#);
        assert!(encoded.len() <= 64);
    }
}
