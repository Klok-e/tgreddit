use crate::reddit::{self};
use crate::{
    config, db,
    download::*,
    messages,
    types::{ReviewContentKind, ReviewPost, RichText},
    ytdlp,
};
use anyhow::{Context, Result};
use log::*;
use regex::Regex;
use url::Url;

use std::string::ToString;
use std::sync::LazyLock;
use std::{borrow::Cow, path::PathBuf};
use std::{collections::HashMap, path::Path};
use teloxide::types::{InputFile, InputMediaVideo, MessageId};
use teloxide::{
    payloads::{SendMessageSetters, SendPhotoSetters, SendVideoSetters},
    types::InputMediaPhoto,
};
use teloxide::{prelude::*, types::InputMedia};
use tempfile::TempDir;

fn source_url_for_post(post: &reddit::Post) -> String {
    match post.post_type {
        reddit::PostType::Gallery | reddit::PostType::SelfText => post.format_permalink_url(None),
        reddit::PostType::Image
        | reddit::PostType::Video
        | reddit::PostType::Link
        | reddit::PostType::Unknown => post.url.clone(),
    }
}

fn save_review_post(db: &db::Database, review: ReviewPost) -> Result<()> {
    db.upsert_review_post(&review)
}

/// The Telegram message id(s) produced by a single `handle_new_post` delivery.
///
/// One delivery produces exactly one Telegram message, except for gallery
/// media groups, which produce one Telegram message per media item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveredMessages {
    /// A single Telegram message was delivered (image, video, link, self-text, unknown).
    Single(MessageId),
    /// A gallery media group was delivered; one id per media item.
    Gallery(Vec<MessageId>),
}

pub async fn handle_video_link(
    db: &db::Database,
    tg: &Bot,
    chat_id: i64,
    link: &Url,
    is_twitter_status: bool,
) -> Result<()> {
    let video = tokio::task::block_in_place(|| ytdlp::download_direct(link.as_str()))
        .context("Failed to download video from link")?;

    db.record_post_seen_with_current_time(chat_id, &video)?;

    info!("got a video: {video:?}");
    let metadata = messages::format_video_review_metadata(&video);
    let caption = RichText {
        text: direct_video_caption(
            is_twitter_status,
            &video.title,
            video.source_description.as_deref(),
            &metadata,
        ),
        entities: Vec::new(),
    };
    let review = messages::compose_review_text(&caption, &metadata);
    let sent = tg
        .send_video(ChatId(chat_id), InputFile::file(&video.path))
        .caption(&review.text)
        .caption_entities(review.entities.clone())
        .height(video.height.into())
        .width(video.width.into())
        .reply_markup(messages::format_media_repost_buttons(&video, false))
        .await?;
    save_review_post(
        db,
        ReviewPost {
            chat_id,
            post_id: video.id.clone(),
            source_url: video.url.clone(),
            caption,
            content_kind: ReviewContentKind::Media,
            review_message_id: sent.id,
            control_message_id: sent.id,
            metadata,
            pending_publish_variant: None,
            previous_keyboard: None,
            published_at: None,
        },
    )?;
    info!(
        "video uploaded post_id={} chat_id={chat_id} video={video:?}",
        video.id
    );
    Ok(())
}

fn direct_video_caption(
    is_twitter_status: bool,
    title: &str,
    source_description: Option<&str>,
    metadata: &RichText,
) -> String {
    if is_twitter_status
        && let Some(source_description) =
            source_description.filter(|description| !description.trim().is_empty())
    {
        return truncate_caption_for_metadata(&clean_x_tweet_body(source_description), metadata);
    }
    title.to_owned()
}

fn clean_x_tweet_body(description: &str) -> String {
    static TCO_URL: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"[hH][tT][tT][pP][sS]?://[tT]\.[cC][oO]/[A-Za-z0-9]+")
            .expect("the t.co URL expression is valid")
    });

    description
        .split_inclusive('\n')
        .filter_map(|line_with_ending| {
            let (line, ending) = line_with_ending
                .strip_suffix('\n')
                .map_or((line_with_ending, ""), |line| (line, "\n"));
            let (cleaned_line, removed_tco_url) = remove_tco_urls_from_line(line, &TCO_URL);

            // A line made entirely of t.co URLs has no Tweet Body content, so
            // discard it together with its separator. Empty lines that were
            // already present stay intact as authored paragraph boundaries.
            if removed_tco_url && cleaned_line.trim().is_empty() {
                None
            } else {
                Some(format!("{cleaned_line}{ending}"))
            }
        })
        .collect::<Vec<_>>()
        .concat()
}

fn remove_tco_urls_from_line(line: &str, tco_url: &Regex) -> (String, bool) {
    let mut cleaned = String::with_capacity(line.len());
    let mut previous_end = 0;
    let mut follows_removed_url = false;
    let mut removed_tco_url = false;

    for matched_url in tco_url.find_iter(line) {
        append_after_tco_removal(
            &mut cleaned,
            &line[previous_end..matched_url.start()],
            follows_removed_url,
        );
        previous_end = matched_url.end();
        follows_removed_url = true;
        removed_tco_url = true;
    }
    append_after_tco_removal(&mut cleaned, &line[previous_end..], follows_removed_url);

    (cleaned, removed_tco_url)
}

fn append_after_tco_removal(destination: &mut String, text: &str, follows_removed_url: bool) {
    if follows_removed_url
        && (text.is_empty()
            || text
                .chars()
                .next()
                .is_some_and(is_tco_url_boundary_punctuation))
    {
        let trimmed_len = destination.trim_end_matches([' ', '\t', '\r']).len();
        destination.truncate(trimmed_len);
    }
    let text = if follows_removed_url
        && (destination.is_empty() || destination.ends_with([' ', '\t', '\r']))
    {
        text.trim_start_matches([' ', '\t', '\r'])
    } else {
        text
    };

    destination.push_str(text);
}

fn is_tco_url_boundary_punctuation(character: char) -> bool {
    character.is_ascii_punctuation() || matches!(character, '…' | '—' | '–' | '“' | '”' | '‘' | '’')
}

fn truncate_caption_for_metadata(caption: &str, metadata: &RichText) -> String {
    let metadata_len = if metadata.text.is_empty() {
        0
    } else {
        messages::utf16_len("\n\n") + messages::utf16_len(&metadata.text)
    };
    let maximum = messages::TELEGRAM_MEDIA_CAPTION_LIMIT.saturating_sub(metadata_len);
    if messages::utf16_len(caption) <= maximum {
        return caption.to_owned();
    }
    if maximum == 0 {
        return String::new();
    }

    let mut truncated = String::new();
    let prefix_maximum = maximum - messages::utf16_len("…");
    for character in caption.chars() {
        if messages::utf16_len(&truncated) + character.len_utf16() > prefix_maximum {
            break;
        }
        truncated.push(character);
    }
    truncated.push('…');
    truncated
}

async fn handle_new_video_post(
    config: &config::Config,
    tg: &Bot,
    chat_id: i64,
    post: &reddit::Post,
) -> Result<DeliveredMessages> {
    let video = tokio::task::block_in_place(|| ytdlp::download(&post.url))
        .context("Failed to download video from post")?;

    info!("got a video: {video:?}");
    let source_url = source_url_for_post(post);
    let caption = RichText {
        text: post.title.clone(),
        entities: Vec::new(),
    };
    let metadata = messages::format_reddit_review_metadata(
        &post.format_permalink_url(config.links_base_url.as_deref()),
    );
    let review = messages::compose_review_text(&caption, &metadata);
    let sent = tg
        .send_video(ChatId(chat_id), InputFile::file(&video.path))
        .caption(&review.text)
        .caption_entities(review.entities.clone())
        .height(video.height.into())
        .width(video.width.into())
        .reply_markup(messages::format_media_repost_buttons(post, false))
        .await?;
    let db = db::Database::open(config)?;
    save_review_post(
        &db,
        ReviewPost {
            chat_id,
            post_id: post.id.clone(),
            source_url,
            caption,
            content_kind: ReviewContentKind::Media,
            review_message_id: sent.id,
            control_message_id: sent.id,
            metadata,
            pending_publish_variant: None,
            previous_keyboard: None,
            published_at: None,
        },
    )?;
    info!(
        "video uploaded post_id={} chat_id={chat_id} video={video:?}",
        post.id
    );
    Ok(DeliveredMessages::Single(sent.id))
}

async fn handle_new_image_post(
    config: &config::Config,
    tg: &Bot,
    chat_id: i64,
    post: &reddit::Post,
) -> Result<DeliveredMessages> {
    match download_url_to_tmp(&post.url).await {
        Ok((path, _tmp_dir)) => {
            // path will be deleted when _tmp_dir when goes out of scope
            let source_url = source_url_for_post(post);
            let caption = RichText {
                text: post.title.clone(),
                entities: Vec::new(),
            };
            let metadata = messages::format_reddit_review_metadata(
                &post.format_permalink_url(config.links_base_url.as_deref()),
            );
            let review = messages::compose_review_text(&caption, &metadata);
            if is_gif(&path) {
                let sent = tg
                    .send_video(ChatId(chat_id), InputFile::file(path))
                    .caption(&review.text)
                    .caption_entities(review.entities.clone())
                    .reply_markup(messages::format_media_repost_buttons(post, false))
                    .await?;

                let db = db::Database::open(config)?;
                save_review_post(
                    &db,
                    ReviewPost {
                        chat_id,
                        post_id: post.id.clone(),
                        source_url,
                        caption,
                        content_kind: ReviewContentKind::Media,
                        review_message_id: sent.id,
                        control_message_id: sent.id,
                        metadata,
                        pending_publish_variant: None,
                        previous_keyboard: None,
                        published_at: None,
                    },
                )?;

                info!("gif uploaded post_id={} chat_id={chat_id}", post.id);
                Ok(DeliveredMessages::Single(sent.id))
            } else {
                let sent = tg
                    .send_photo(ChatId(chat_id), InputFile::file(path))
                    .caption(&review.text)
                    .caption_entities(review.entities.clone())
                    .reply_markup(messages::format_media_repost_buttons(post, false))
                    .await?;

                let db = db::Database::open(config)?;
                save_review_post(
                    &db,
                    ReviewPost {
                        chat_id,
                        post_id: post.id.clone(),
                        source_url,
                        caption,
                        content_kind: ReviewContentKind::Media,
                        review_message_id: sent.id,
                        control_message_id: sent.id,
                        metadata,
                        pending_publish_variant: None,
                        previous_keyboard: None,
                        published_at: None,
                    },
                )?;

                info!("image uploaded post_id={} chat_id={chat_id}", post.id);
                Ok(DeliveredMessages::Single(sent.id))
            }
        }
        Err(e) => {
            error!("failed to download image: {e:?}");
            Err(e)
        }
    }
}

async fn handle_new_link_post(
    config: &config::Config,
    tg: &Bot,
    chat_id: i64,
    post: &reddit::Post,
) -> Result<DeliveredMessages> {
    let source_url = source_url_for_post(post);
    let caption = RichText {
        text: post.title.clone(),
        entities: Vec::new(),
    };
    let metadata = messages::format_reddit_review_metadata(
        &post.format_permalink_url(config.links_base_url.as_deref()),
    );
    let review = messages::compose_review_text(&caption, &metadata);
    let sent = tg
        .send_message(ChatId(chat_id), &review.text)
        .entities(review.entities.clone())
        .reply_markup(messages::format_text_repost_buttons(post))
        .await?;
    let db = db::Database::open(config)?;
    save_review_post(
        &db,
        ReviewPost {
            chat_id,
            post_id: post.id.clone(),
            source_url,
            caption,
            content_kind: ReviewContentKind::Text,
            review_message_id: sent.id,
            control_message_id: sent.id,
            metadata,
            pending_publish_variant: None,
            previous_keyboard: None,
            published_at: None,
        },
    )?;
    info!("message sent post_id={} chat_id={chat_id}", post.id);
    Ok(DeliveredMessages::Single(sent.id))
}

async fn handle_new_self_post(
    config: &config::Config,
    tg: &Bot,
    chat_id: i64,
    post: &reddit::Post,
) -> Result<DeliveredMessages> {
    let source_url = source_url_for_post(post);
    let caption = RichText {
        text: post.title.clone(),
        entities: Vec::new(),
    };
    let metadata = messages::format_reddit_review_metadata(
        &post.format_permalink_url(config.links_base_url.as_deref()),
    );
    let review = messages::compose_review_text(&caption, &metadata);
    let sent = tg
        .send_message(ChatId(chat_id), &review.text)
        .entities(review.entities.clone())
        .reply_markup(messages::format_text_repost_buttons(post))
        .await?;
    let db = db::Database::open(config)?;
    save_review_post(
        &db,
        ReviewPost {
            chat_id,
            post_id: post.id.clone(),
            source_url,
            caption,
            content_kind: ReviewContentKind::Text,
            review_message_id: sent.id,
            control_message_id: sent.id,
            metadata,
            pending_publish_variant: None,
            previous_keyboard: None,
            published_at: None,
        },
    )?;
    info!("message sent post_id={} chat_id={chat_id}", post.id);
    Ok(DeliveredMessages::Single(sent.id))
}

async fn download_gallery(post: &reddit::Post) -> Result<HashMap<String, (PathBuf, TempDir)>> {
    let media_metadata_map = post
        .media_metadata
        .as_ref()
        .expect("expected media_metadata to exist in gallery post");

    let mut map: HashMap<String, (PathBuf, TempDir)> = HashMap::new();
    for (id, media_metadata) in media_metadata_map {
        let s = media_metadata
            .s
            .as_ref()
            .context("Media metadata not available")?;
        let url = &s.url.replace("&amp;", "&");
        info!("got media id={id} x={} y={} url={}", s.x, s.y, url);
        map.insert(id.to_string(), download_url_to_tmp(url).await?);
    }

    Ok(map)
}

async fn handle_new_gallery_post(
    config: &config::Config,
    tg: &Bot,
    chat_id: i64,
    post: &reddit::Post,
) -> Result<DeliveredMessages> {
    // post.gallery_data is an array that describes the order of photos in the gallery, while
    // post.media_metadata is a map that contains the URL for each photo
    let gallery_data_items = &post
        .gallery_data
        .as_ref()
        .expect("expected media_metadata to exist in gallery post")
        .items;
    let gallery_files_map = download_gallery(post).await?;
    let source_url = source_url_for_post(post);
    let caption = RichText {
        text: post.title.clone(),
        entities: Vec::new(),
    };
    let metadata = messages::format_reddit_review_metadata(
        &post.format_permalink_url(config.links_base_url.as_deref()),
    );
    let review = messages::compose_review_text(&caption, &metadata);
    let mut media_group = vec![];
    let mut first = true;

    for item in gallery_data_items {
        let file = gallery_files_map.get(&item.media_id);
        match file {
            Some((image_path, _tempdir)) => {
                if is_gif(image_path) {
                    let mut input_media_video = InputMediaVideo::new(InputFile::file(image_path));
                    if first {
                        input_media_video = input_media_video
                            .caption(&review.text)
                            .caption_entities(review.entities.clone());
                        first = false;
                    }
                    media_group.push(InputMedia::Video(input_media_video));
                } else {
                    let mut input_media_photo = InputMediaPhoto::new(InputFile::file(image_path));
                    if first {
                        input_media_photo = input_media_photo
                            .caption(&review.text)
                            .caption_entities(review.entities.clone());
                        first = false;
                    }
                    media_group.push(InputMedia::Photo(input_media_photo));
                }
            }
            None => {
                error!("could not find downloaded image for gallery data item: {item:?}");
            }
        }
    }

    let gallery_msg = tg.send_media_group(ChatId(chat_id), media_group).await?;
    let delivered_ids: Vec<MessageId> = gallery_msg.iter().map(|m| m.id).collect();
    let db = db::Database::open(config)?;
    for msg in gallery_msg {
        let file_meta = if let Some(video) = msg.video() {
            &video.file
        } else if let Some(photo) = msg.photo() {
            &photo
                .iter()
                .max_by_key(|x| x.file.size)
                .expect("There must be at least one element")
                .file
        } else {
            panic!("Neither photo nor video found in message");
        };
        db.add_telegram_file(&post.id, chat_id, &file_meta.id, &file_meta.unique_id)?;
    }

    let control = tg
        .send_message(ChatId(chat_id), "To repost:")
        .reply_markup(messages::format_media_repost_buttons(post, true))
        .send()
        .await?;

    let review_message_id = *delivered_ids
        .first()
        .context("gallery delivery returned no messages")?;
    save_review_post(
        &db,
        ReviewPost {
            chat_id,
            post_id: post.id.clone(),
            source_url,
            caption,
            content_kind: ReviewContentKind::Gallery,
            review_message_id,
            control_message_id: control.id,
            metadata,
            pending_publish_variant: None,
            previous_keyboard: None,
            published_at: None,
        },
    )?;

    info!("gallery uploaded post_id={} chat_id={chat_id}", post.id);

    Ok(DeliveredMessages::Gallery(delivered_ids))
}

pub async fn process_post(
    db: &db::Database,
    chat_id: i64,
    post: &reddit::Post,
    config: &config::Config,
    tg: &Bot,
) -> Result<()> {
    db.record_post_seen_with_current_time(chat_id, post)?;
    if let Err(e) = handle_new_post(config, tg, chat_id, post).await {
        error!("failed to handle new post: {e:?}");
    };
    Ok(())
}

pub async fn handle_new_post(
    config: &config::Config,
    tg: &Bot,
    chat_id: i64,
    post: &reddit::Post,
) -> Result<DeliveredMessages> {
    info!("got new {post:#?}");
    let mut post = Cow::Borrowed(post);

    // Sometimes post_hint is not in top list response but exists when getting the link directly,
    // but not always
    // TODO: It appears that post with is_gallery=true will never have post_hint set
    if post.post_hint.is_none() {
        info!("post missing post_hint, getting like directly");
        post = Cow::Owned(reddit::get_link(&post.id).await.unwrap());
    }

    match post.post_type {
        reddit::PostType::Image => handle_new_image_post(config, tg, chat_id, &post)
            .await
            .context("Failed handling new image"),
        reddit::PostType::Video => handle_new_video_post(config, tg, chat_id, &post)
            .await
            .context("Failed handling new video"),
        reddit::PostType::Link => handle_new_link_post(config, tg, chat_id, &post)
            .await
            .context("Failed handling new link post"),
        reddit::PostType::SelfText => handle_new_self_post(config, tg, chat_id, &post)
            .await
            .context("Failed handling new self"),
        reddit::PostType::Gallery => handle_new_gallery_post(config, tg, chat_id, &post)
            .await
            .context("Failed handling new gallery"),
        // /r/bestof posts have no characteristics like post_hint that could be used to
        // determine them as a type of Link; as a workaround, post Unknown post types the same way
        // as a link
        reddit::PostType::Unknown => {
            warn!("unknown post type, post={post:?}");
            handle_new_link_post(config, tg, chat_id, &post)
                .await
                .context("Failed handling unknown post")
        }
    }
}

fn is_gif(path: &Path) -> bool {
    path.extension()
        .and_then(|x| x.to_str().map(|x| x == "gif"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn post(post_type: reddit::PostType) -> reddit::Post {
        reddit::Post {
            id: "post-1".to_owned(),
            subreddit: "test".to_owned(),
            title: "title".to_owned(),
            permalink: "/r/test/comments/post-1/title/".to_owned(),
            url: "https://media.example/file".to_owned(),
            post_hint: None,
            post_type,
            gallery_data: None,
            media_metadata: Some(HashMap::new()),
        }
    }

    #[test]
    fn source_url_mapping_uses_exact_media_url_and_reddit_permalink() {
        for post_type in [
            reddit::PostType::Image,
            reddit::PostType::Video,
            reddit::PostType::Link,
            reddit::PostType::Unknown,
        ] {
            assert_eq!(
                source_url_for_post(&post(post_type)),
                "https://media.example/file"
            );
        }
        for post_type in [reddit::PostType::Gallery, reddit::PostType::SelfText] {
            assert_eq!(
                source_url_for_post(&post(post_type)),
                "https://www.reddit.com/r/test/comments/post-1/title/"
            );
        }
    }

    #[test]
    fn x_status_uses_the_full_structured_description_as_its_caption() {
        let link = Url::parse("https://x.com/example/status/123").unwrap();
        let description = "First line 👋\nhttps://t.co/example\nA link https://example.com stays";

        assert_eq!(
            direct_video_caption(
                true,
                "uploader - shortened title",
                Some(description),
                &RichText {
                    text: link.to_string(),
                    entities: Vec::new(),
                },
            ),
            "First line 👋\nA link https://example.com stays"
        );
    }

    #[test]
    fn x_status_removes_all_tco_urls_and_cleans_the_local_gaps() {
        let description =
            "Read https://t.co/abc123, then https://t.co/Def456!\n\nhttps://example.com/kept";

        assert_eq!(
            direct_video_caption(true, "title", Some(description), &RichText::default()),
            "Read, then!\n\nhttps://example.com/kept"
        );
    }

    #[test]
    fn x_status_preserves_authored_whitespace_and_punctuation_outside_tco_urls() {
        let description = "Two  spaces : https://t.co/abc123 next\nNo t.co  here\n";

        assert_eq!(
            direct_video_caption(true, "title", Some(description), &RichText::default()),
            "Two  spaces : next\nNo t.co  here\n"
        );
    }

    #[test]
    fn x_status_keeps_quotes_unicode_punctuation_and_emoji_adjacent_to_tco_urls() {
        let description =
            "See \"https://t.co/abc123\" and https://t.co/Def456…now https://t.co/Ghi789🔥";

        assert_eq!(
            direct_video_caption(true, "title", Some(description), &RichText::default()),
            "See \"\" and…now 🔥"
        );
    }

    #[test]
    fn x_review_keeps_the_source_url_on_a_blank_separate_line() {
        let metadata = RichText {
            text: "https://x.com/example/status/123".to_owned(),
            entities: Vec::new(),
        };
        let caption = direct_video_caption(
            true,
            "title",
            Some("Tweet body https://t.co/abc123"),
            &metadata,
        );

        assert_eq!(
            messages::compose_review_text(
                &RichText {
                    text: caption,
                    entities: Vec::new(),
                },
                &metadata,
            )
            .text,
            "Tweet body\n\nhttps://x.com/example/status/123"
        );
    }

    #[test]
    fn x_status_with_only_tco_urls_has_no_repost_caption() {
        assert_eq!(
            direct_video_caption(
                true,
                "uploader - title",
                Some("https://t.co/abc123\nhttps://t.co/Def456"),
                &RichText::default(),
            ),
            ""
        );
    }

    #[test]
    fn x_status_without_a_description_falls_back_to_its_title() {
        assert_eq!(
            direct_video_caption(
                true,
                "uploader - shortened title",
                Some(" \n\t"),
                &RichText::default(),
            ),
            "uploader - shortened title"
        );
    }

    #[test]
    fn x_status_with_an_overlong_utf16_description_is_truncated_with_room_for_metadata() {
        let metadata = RichText {
            text: "https://x.com/example/status/123".to_owned(),
            entities: Vec::new(),
        };
        let description = format!(
            "https://t.co/abc123\n{}",
            "👋".repeat(messages::TELEGRAM_MEDIA_CAPTION_LIMIT / 2)
        );
        let caption = direct_video_caption(
            true,
            "uploader - short title",
            Some(&description),
            &metadata,
        );

        assert!(caption.ends_with('…'));
        assert!(!caption.contains("t.co"));
        assert!(
            caption
                .chars()
                .all(|character| character == '👋' || character == '…')
        );
        assert!(
            messages::utf16_len(
                &messages::compose_review_text(
                    &RichText {
                        text: caption,
                        entities: Vec::new(),
                    },
                    &metadata,
                )
                .text
            ) <= messages::TELEGRAM_MEDIA_CAPTION_LIMIT
        );
    }

    #[test]
    fn non_x_direct_video_keeps_its_title_caption() {
        assert_eq!(
            direct_video_caption(
                false,
                "video title",
                Some("extractor description"),
                &RichText::default(),
            ),
            "video title"
        );
    }

    /// Which variant of `DeliveredMessages` a given `PostType` produces.
    /// This mirrors the dispatch in `handle_new_post` and is the unit-testable
    /// piece of the variant selection logic.
    fn expected_variant_kind(post_type: reddit::PostType) -> VariantKind {
        match post_type {
            reddit::PostType::Gallery => VariantKind::Gallery,
            reddit::PostType::Image
            | reddit::PostType::Video
            | reddit::PostType::Link
            | reddit::PostType::SelfText
            | reddit::PostType::Unknown => VariantKind::Single,
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    enum VariantKind {
        Single,
        Gallery,
    }

    #[test]
    fn delivered_messages_single_carries_message_id() {
        let id = MessageId(42);
        match DeliveredMessages::Single(id) {
            DeliveredMessages::Single(carried) => assert_eq!(carried, id),
            other => panic!("expected Single variant, got {other:?}"),
        }
    }

    #[test]
    fn delivered_messages_gallery_carries_message_ids() {
        let ids = vec![MessageId(1), MessageId(2), MessageId(3)];
        match DeliveredMessages::Gallery(ids.clone()) {
            DeliveredMessages::Gallery(carried) => assert_eq!(carried, ids),
            other => panic!("expected Gallery variant, got {other:?}"),
        }
    }

    #[test]
    fn variant_selection_picks_single_for_non_gallery_posts() {
        assert_eq!(
            expected_variant_kind(reddit::PostType::Image),
            VariantKind::Single
        );
        assert_eq!(
            expected_variant_kind(reddit::PostType::Video),
            VariantKind::Single
        );
        assert_eq!(
            expected_variant_kind(reddit::PostType::Link),
            VariantKind::Single
        );
        assert_eq!(
            expected_variant_kind(reddit::PostType::SelfText),
            VariantKind::Single
        );
        assert_eq!(
            expected_variant_kind(reddit::PostType::Unknown),
            VariantKind::Single
        );
    }

    #[test]
    fn variant_selection_picks_gallery_for_gallery_posts() {
        assert_eq!(
            expected_variant_kind(reddit::PostType::Gallery),
            VariantKind::Gallery
        );
    }
}
