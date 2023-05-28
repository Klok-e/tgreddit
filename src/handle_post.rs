use crate::reddit::{self};
use crate::{config, db, download::*, messages, ytdlp};
use anyhow::{Context, Result};
use log::*;
use url::Url;

use std::collections::HashMap;
use std::string::ToString;
use std::{borrow::Cow, path::PathBuf};
use teloxide::types::InputFile;
use teloxide::{
    payloads::{SendMessageSetters, SendPhotoSetters, SendVideoSetters},
    types::InputMediaPhoto,
};
use teloxide::{prelude::*, types::InputMedia};
use tempdir::TempDir;

pub async fn handle_video_link(
    db: &db::Database,
    tg: &Bot,
    chat_id: i64,
    link: &Url,
) -> Result<()> {
    let video = tokio::task::block_in_place(|| ytdlp::download(link.as_str()))
        .context("Failed to download video from link")?;

    db.record_post_seen_with_current_time(chat_id, &video)?;

    info!("got a video: {video:?}");
    let caption = messages::format_link_video_caption_html(&video);
    tg.send_video(ChatId(chat_id), InputFile::file(&video.path))
        .parse_mode(teloxide::types::ParseMode::Html)
        .caption(&caption)
        .height(video.height.into())
        .width(video.width.into())
        .reply_markup(messages::format_repost_buttons(&video))
        .await?;
    info!(
        "video uploaded post_id={} chat_id={chat_id} video={video:?}",
        video.id
    );
    Ok(())
}

async fn handle_new_video_post(
    config: &config::Config,
    tg: &Bot,
    chat_id: i64,
    post: &reddit::Post,
) -> Result<()> {
    let video = tokio::task::block_in_place(|| ytdlp::download(&post.url))
        .context("Failed to download video from post")?;

    info!("got a video: {video:?}");
    let caption = messages::format_media_caption_html(post, config.links_base_url.as_deref());
    tg.send_video(ChatId(chat_id), InputFile::file(&video.path))
        .parse_mode(teloxide::types::ParseMode::Html)
        .caption(&caption)
        .height(video.height.into())
        .width(video.width.into())
        .reply_markup(messages::format_repost_buttons(post))
        .await?;
    info!(
        "video uploaded post_id={} chat_id={chat_id} video={video:?}",
        post.id
    );
    Ok(())
}

async fn handle_new_image_post(
    config: &config::Config,
    tg: &Bot,
    chat_id: i64,
    post: &reddit::Post,
) -> Result<()> {
    match download_url_to_tmp(&post.url).await {
        Ok((path, _tmp_dir)) => {
            // path will be deleted when _tmp_dir when goes out of scope
            let caption =
                messages::format_media_caption_html(post, config.links_base_url.as_deref());
            tg.send_photo(ChatId(chat_id), InputFile::file(path))
                .parse_mode(teloxide::types::ParseMode::Html)
                .caption(&caption)
                .reply_markup(messages::format_repost_buttons(post))
                .await?;
            info!("image uploaded post_id={} chat_id={chat_id}", post.id);
            Ok(())
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
) -> Result<()> {
    let message_html = messages::format_link_message_html(post, config.links_base_url.as_deref());
    tg.send_message(ChatId(chat_id), message_html)
        .parse_mode(teloxide::types::ParseMode::Html)
        .disable_web_page_preview(false)
        .reply_markup(messages::format_repost_buttons(post))
        .await?;
    info!("message sent post_id={} chat_id={chat_id}", post.id);
    Ok(())
}

async fn handle_new_self_post(
    config: &config::Config,
    tg: &Bot,
    chat_id: i64,
    post: &reddit::Post,
) -> Result<()> {
    let message_html = messages::format_media_caption_html(post, config.links_base_url.as_deref());
    tg.send_message(ChatId(chat_id), message_html)
        .parse_mode(teloxide::types::ParseMode::Html)
        .disable_web_page_preview(true)
        .reply_markup(messages::format_repost_buttons(post))
        .await?;
    info!("message sent post_id={} chat_id={chat_id}", post.id);
    Ok(())
}

async fn download_gallery(post: &reddit::Post) -> Result<HashMap<String, (PathBuf, TempDir)>> {
    let media_metadata_map = post
        .media_metadata
        .as_ref()
        .expect("expected media_metadata to exist in gallery post");

    let mut map: HashMap<String, (PathBuf, TempDir)> = HashMap::new();
    for (id, media_metadata) in media_metadata_map {
        let s = &media_metadata.s;
        let url = &s.url.replace("&amp;", "&");
        info!("got media id={id} x={} y={} url={}", &s.x, &s.y, url);
        map.insert(id.to_string(), download_url_to_tmp(url).await?);
    }

    Ok(map)
}

async fn handle_new_gallery_post(
    config: &config::Config,
    tg: &Bot,
    chat_id: i64,
    post: &reddit::Post,
) -> Result<()> {
    // post.gallery_data is an array that describes the order of photos in the gallery, while
    // post.media_metadata is a map that contains the URL for each photo
    let gallery_data_items = &post
        .gallery_data
        .as_ref()
        .expect("expected media_metadata to exist in gallery post")
        .items;
    let gallery_files_map = download_gallery(post).await?;
    let mut media_group = vec![];
    let mut first = true;

    for item in gallery_data_items {
        let file = gallery_files_map.get(&item.media_id);
        match file {
            Some((image_path, _tempdir)) => {
                let mut input_media_photo = InputMediaPhoto::new(InputFile::file(image_path));
                // The first InputMediaPhoto in the vector needs to contain the caption and parse_mode;
                if first {
                    let caption =
                        messages::format_media_caption_html(post, config.links_base_url.as_deref());
                    input_media_photo = input_media_photo
                        .caption(&caption)
                        .parse_mode(teloxide::types::ParseMode::Html);
                    first = false;
                }

                media_group.push(InputMedia::Photo(input_media_photo))
            }
            None => {
                error!("could not find downloaded image for gallery data item: {item:?}");
            }
        }
    }

    let gallery_msg = tg.send_media_group(ChatId(chat_id), media_group).await?;
    let db = db::Database::open(config)?;
    for msg in gallery_msg {
        let file_meta = &msg
            .photo()
            .expect("Photo expected")
            .iter()
            .max_by_key(|x| x.file.size)
            .expect("There must be at least one element")
            .file;
        db.add_telegram_file(&post.id, chat_id, &file_meta.id, &file_meta.unique_id)?;
    }

    tg.send_message(ChatId(chat_id), "To repost:")
        .reply_markup(messages::format_repost_buttons_gallery(post, true))
        .send()
        .await?;

    info!("gallery uploaded post_id={} chat_id={chat_id}", post.id);

    Ok(())
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
) -> Result<()> {
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
            handle_new_link_post(config, tg, chat_id, &post).await
        }
    }
}
