use serde_derive::{Deserialize, Serialize};
use tempdir::TempDir;

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
    pub video_tempdir: TempDir,
}

impl Recordable for Video {
    fn id(&self) -> &str {
        &self.id
    }

    fn title(&self) -> &str {
        &self.title
    }

    fn subreddit(&self) -> &str {
        "youtube download"
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
