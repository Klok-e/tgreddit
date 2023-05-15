use serde_derive::{Deserialize, Serialize};

use crate::reddit::{PostType, TopPostsTimePeriod};
use std::path::PathBuf;

#[derive(Debug)]
pub struct Video {
    pub path: PathBuf,
    pub title: String,
    pub width: u16,
    pub height: u16,
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
