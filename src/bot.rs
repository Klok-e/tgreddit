use crate::{
    config, db,
    handle_post::{
        DeliveredMessages, handle_new_post, handle_video_link, handle_x_tweet_link, process_post,
    },
    messages, reddit,
    reddit::{PostType, TopPostsTimePeriod},
    types::{
        MediaKind, PublishVariant, RepostAction, ReviewContentKind, ReviewPost, RichText,
        SubscriptionArgs, TelegramMediaFile, decode_repost_callback,
    },
    x_tweet,
};
use anyhow::{Context, Result};
use lazy_static::lazy_static;
use log::{debug, error, info, warn};
use regex::Regex;
use secrecy::ExposeSecret;
use std::{collections::HashMap, env, sync::Arc, sync::Mutex, time::Duration};
use teloxide::sugar::request::RequestReplyExt;
use teloxide::{
    dispatching::DefaultKey,
    dptree,
    prelude::*,
    types::{
        CallbackQuery, ChatId, ForceReply, InlineKeyboardMarkup, InputFile, InputMedia,
        InputMediaPhoto, Message, MessageEntity, MessageEntityKind, MessageEntityRef, MessageId,
        Update,
    },
    utils::command::{BotCommands, ParseError},
};
use url::Url;

const TELEGRAM_BOT_API_URL_ENV: &str = "TELEGRAM_BOT_API_URL";
type CaptionEditStore = Arc<Mutex<HashMap<i64, CaptionEditState>>>;

const GENERIC_USER_ERROR: &str = "The requested operation could not be completed.";
const UNRETRIEVABLE_X_TWEET: &str = "The X Tweet could not be retrieved.";

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
struct UserFacingError {
    message: String,
    #[source]
    source: Option<anyhow::Error>,
}

fn user_error(message: impl Into<String>, source: impl Into<anyhow::Error>) -> anyhow::Error {
    UserFacingError {
        message: message.into(),
        source: Some(source.into()),
    }
    .into()
}

fn user_message(message: impl Into<String>) -> anyhow::Error {
    UserFacingError {
        message: message.into(),
        source: None,
    }
    .into()
}

fn direct_media_user_error(is_x_tweet: bool, error: anyhow::Error) -> anyhow::Error {
    let message = if is_x_tweet && x_tweet::is_retrieval_error(&error) {
        UNRETRIEVABLE_X_TWEET
    } else if is_x_tweet {
        "The X Tweet could not be processed."
    } else {
        "The video link could not be processed."
    };
    user_error(message, error)
}

fn user_facing_message_or<'a>(err: &'a anyhow::Error, fallback: &'a str) -> &'a str {
    err.chain()
        .find_map(|cause| cause.downcast_ref::<UserFacingError>())
        .map(|err| err.message.as_str())
        .unwrap_or(fallback)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CaptionEditState {
    post_id: String,
    prompt_message_id: MessageId,
}

impl CaptionEditState {
    fn post_id(&self) -> &str {
        &self.post_id
    }
}

#[derive(BotCommands, Clone)]
#[command(
    rename_rule = "lowercase",
    description = "These commands are supported:"
)]
pub enum Command {
    #[command(description = "display this text")]
    Help,
    #[command(
        description = "subscribe to subreddit's top posts",
        parse_with = parse_subscribe_message
    )]
    Sub(SubscriptionArgs),
    #[command(description = "unsubscribe from subreddit's top posts")]
    Unsub(String),
    #[command(description = "list subreddit subscriptions")]
    ListSubs,
    #[command(description = "get top posts", parse_with = parse_subscribe_message)]
    Get(SubscriptionArgs),
    #[command(description = "register channel to which the bot is supposed to post")]
    RegisterChannel(i64),
    #[command(description = "cancel the active caption edit")]
    Cancel,
}

pub struct MyBot {
    pub dispatcher: Dispatcher<Arc<Bot>, anyhow::Error, DefaultKey>,
    pub tg: Arc<Bot>,
}

impl MyBot {
    pub async fn new(config: Arc<config::Config>) -> Result<Self> {
        let client = teloxide::net::default_reqwest_settings()
            .timeout(Duration::from_secs(600))
            .build()
            .expect("Client creation failed");
        let mut tg = Bot::with_client(config.telegram_bot_token.expose_secret(), client);
        if let Some(url) = env::var_os(TELEGRAM_BOT_API_URL_ENV) {
            tg = tg.set_api_url(
                Url::parse(url.to_str().expect("Unicode string expected"))
                    .expect("Bot api must be a url"),
            );
        }

        tg.set_my_commands(Command::bot_commands()).await?;

        let tg = Arc::new(tg);
        let caption_edits: CaptionEditStore = Arc::new(Mutex::new(HashMap::new()));

        let handler = dptree::entry()
            .branch(
                Update::filter_message().branch(
                    dptree::filter(|msg: Message, config: Arc<config::Config>| {
                        msg.from
                            .map(|user| config.authorized_user_ids.contains(&user.id.0))
                            .unwrap_or_default()
                    })
                    .branch(
                        dptree::entry()
                            .filter_command::<Command>()
                            .endpoint(handle_command),
                    )
                    .branch(dptree::entry().endpoint(handle_no_command)),
                ),
            )
            .branch(
                Update::filter_callback_query().branch(
                    dptree::filter(|msg: CallbackQuery, config: Arc<config::Config>| {
                        config.authorized_user_ids.contains(&msg.from.id.0)
                    })
                    .endpoint(callback_handler),
                ),
            );

        let dispatcher = Dispatcher::builder(tg.clone(), handler)
            .dependencies(dptree::deps![config.clone(), caption_edits])
            .default_handler(|upd| async move {
                warn!("unhandled update: {upd:?}");
            })
            .error_handler(LoggingErrorHandler::with_custom_text(
                "an error has occurred in the dispatcher",
            ))
            .build();

        let my_bot = MyBot { dispatcher, tg };
        Ok(my_bot)
    }

    pub fn spawn(
        mut self,
    ) -> (
        tokio::task::JoinHandle<()>,
        teloxide::dispatching::ShutdownToken,
    ) {
        let shutdown_token = self.dispatcher.shutdown_token();
        (
            tokio::spawn(async move { self.dispatcher.dispatch().await }),
            shutdown_token,
        )
    }
}

async fn delete_edit_prompt(tg: &Bot, chat_id: ChatId, state: &CaptionEditState) {
    if let Err(err) = tg.delete_message(chat_id, state.prompt_message_id).await {
        warn!("failed to delete caption edit prompt: {err}");
    }
}

async fn edit_review_content(tg: &Bot, review: &ReviewPost) -> Result<()> {
    let content = messages::compose_review_text(&review.caption, &review.metadata);
    let chat_id = ChatId(review.chat_id);
    let confirmation = review
        .pending_publish_variant
        .map(messages::format_publish_confirmation_buttons);
    if review.content_kind == ReviewContentKind::Text
        || (review.content_kind == ReviewContentKind::Gallery
            && review.review_message_id == review.control_message_id)
    {
        let request = tg
            .edit_message_text(chat_id, review.review_message_id, content.text)
            .entities(content.entities);
        if review.review_message_id == review.control_message_id {
            request
                .reply_markup(confirmation.unwrap_or_default())
                .await?;
        } else {
            request.await?;
        }
    } else {
        let request = tg
            .edit_message_caption(chat_id, review.review_message_id)
            .caption(content.text)
            .caption_entities(content.entities);
        if review.review_message_id == review.control_message_id {
            request
                .reply_markup(confirmation.unwrap_or_default())
                .await?;
        } else {
            request.await?;
        }
    }
    Ok(())
}

async fn restore_review_keyboard(tg: &Bot, review: &ReviewPost) -> Result<()> {
    let keyboard = review
        .previous_keyboard
        .clone()
        .unwrap_or_else(|| match review.content_kind {
            ReviewContentKind::Media | ReviewContentKind::Gallery => {
                messages::format_media_repost_buttons_for_id(
                    &review.post_id,
                    review.content_kind == ReviewContentKind::Gallery,
                )
            }
            ReviewContentKind::Text => messages::format_text_repost_buttons_for_id(&review.post_id),
        });
    tg.edit_message_reply_markup(ChatId(review.chat_id), review.control_message_id)
        .reply_markup(keyboard)
        .await?;
    Ok(())
}

fn take_caption_edit(
    caption_edits: &CaptionEditStore,
    chat_id: ChatId,
) -> Option<CaptionEditState> {
    caption_edits
        .lock()
        .expect("caption edit store poisoned")
        .remove(&chat_id.0)
}

fn take_caption_edit_for_post(
    caption_edits: &CaptionEditStore,
    chat_id: ChatId,
    post_id: &str,
) -> Option<CaptionEditState> {
    let mut edits = caption_edits.lock().expect("caption edit store poisoned");
    if edits
        .get(&chat_id.0)
        .is_some_and(|edit| edit.post_id() == post_id)
    {
        edits.remove(&chat_id.0)
    } else {
        None
    }
}

async fn cancel_caption_edit(
    caption_edits: &CaptionEditStore,
    tg: &Bot,
    db: &db::Database,
    chat_id: ChatId,
) -> bool {
    let Some(state) = take_caption_edit(caption_edits, chat_id) else {
        return false;
    };
    delete_edit_prompt(tg, chat_id, &state).await;
    if let Ok(Some(review)) = db.get_review_post(chat_id.0, state.post_id()) {
        if let Err(err) = restore_review_keyboard(tg, &review).await {
            warn!("failed to restore review keyboard: {err:#}");
        }
        if let Err(err) = db.clear_review_publish(chat_id.0, state.post_id()) {
            warn!("failed to clear review publish state: {err:#}");
        }
    }
    true
}

#[derive(Debug, PartialEq, Eq)]
enum CaptionInput<'a> {
    Valid(&'a str),
    Blank,
    TooLong { maximum: usize },
}

fn validate_caption_input(text: &str, maximum: usize) -> CaptionInput<'_> {
    if text.trim().is_empty() {
        CaptionInput::Blank
    } else if messages::utf16_len(text) > maximum {
        CaptionInput::TooLong { maximum }
    } else {
        CaptionInput::Valid(text)
    }
}

async fn handle_caption_input(
    message: &Message,
    tg: &Bot,
    config: &config::Config,
    caption_edits: &CaptionEditStore,
) -> Result<bool> {
    let chat_id = message.chat.id;
    let reply_to = message.reply_to_message().map(|message| message.id);
    let state = caption_edits
        .lock()
        .expect("caption edit store poisoned")
        .get(&chat_id.0)
        .cloned();
    let Some(state) = state else {
        return Ok(false);
    };
    let post_id = state.post_id.clone();
    let prompt_message_id = state.prompt_message_id;
    if reply_to != Some(prompt_message_id) {
        return Ok(false);
    }

    let Some(text) = message.text() else {
        tg.send_message(chat_id, "Send the Repost Caption as a text reply.")
            .reply_to(prompt_message_id)
            .await?;
        return Ok(true);
    };
    let db = db::Database::open(config)?;
    let review = db
        .get_review_post(chat_id.0, &post_id)?
        .context("review post is unavailable")?;
    let maximum =
        messages::max_repost_caption_len(review.content_kind, &review.metadata, &review.source_url)
            .context("review metadata or source URL exceeds Telegram's message limit")?;
    let caption = match validate_caption_input(text, maximum) {
        CaptionInput::Valid(caption) => caption,
        CaptionInput::Blank => {
            tg.send_message(
                chat_id,
                "The Repost Caption cannot be blank. Use Post (no caption) instead.",
            )
            .reply_to(prompt_message_id)
            .await?;
            return Ok(true);
        }
        CaptionInput::TooLong { maximum } => {
            tg.send_message(
                chat_id,
                format!("The Repost Caption must be at most {maximum} UTF-16 units."),
            )
            .reply_to(prompt_message_id)
            .await?;
            return Ok(true);
        }
    };

    let rich_caption = RichText {
        text: caption.to_owned(),
        entities: message.entities().unwrap_or_default().to_vec(),
    };
    messages::validate_entities(&rich_caption.text, &rich_caption.entities)
        .context("invalid Telegram formatting in Repost Caption")?;

    let transitioned = {
        let edits = caption_edits.lock().expect("caption edit store poisoned");
        matches!(
            edits.get(&chat_id.0),
            Some(active) if active.prompt_message_id == prompt_message_id
        )
    };
    if !transitioned {
        return Ok(true);
    }

    db.update_review_caption(chat_id.0, &post_id, &rich_caption)?;
    let mut review = review;
    review.caption = rich_caption;
    edit_review_content(tg, &review).await?;
    take_caption_edit_for_post(caption_edits, chat_id, &post_id);
    if let Err(err) = tg.delete_message(chat_id, prompt_message_id).await {
        warn!("failed to delete completed caption prompt: {err}");
    }
    if let Err(err) = tg.delete_message(chat_id, message.id).await {
        warn!("failed to delete completed caption reply: {err}");
    }
    Ok(true)
}

async fn handle_no_command(
    message: Message,
    tg: Arc<Bot>,
    config: Arc<config::Config>,
    caption_edits: CaptionEditStore,
) -> Result<()> {
    async fn handle(message: &Message, tg: &Arc<Bot>, config: &Arc<config::Config>) -> Result<()> {
        lazy_static! {
            static ref RE_REDDIT: Regex = Regex::new(r"comments/(\w+)").unwrap();
        }

        let Some(text) = message.text() else {
            tg.send_message(message.chat.id, "This message does not contain text.")
                .await?;
            return Ok(());
        };

        if let Some(reply) = malformed_command_reply(text) {
            tg.send_message(message.chat.id, reply).await?;
            return Ok(());
        }

        let db = db::Database::open(config)
            .map_err(|err| user_error("The local database could not be accessed.", err))?;
        if let Some(link) = parse_x_tweet_url(text) {
            handle_x_tweet_link(
                &db,
                tg,
                message.chat.id.0,
                &link,
                &config.x_tweet_api_base_url,
            )
            .await
            .map_err(|err| direct_media_user_error(true, err))?;
        } else if is_youtube_url(text) {
            let link =
                Url::parse(text).map_err(|err| user_error("The video link is invalid.", err))?;
            handle_video_link(&db, tg, message.chat.id.0, &link)
                .await
                .map_err(|err| direct_media_user_error(false, err))?;
        } else {
            let Some(id) = RE_REDDIT
                .captures(text)
                .and_then(|captures| captures.get(1))
                .map(|id| id.as_str())
            else {
                tg.send_message(
                    message.chat.id,
                    "Send a Reddit post URL, X/Twitter Tweet URL, YouTube URL, or a bot command.",
                )
                .await?;
                return Ok(());
            };
            let post = reddit::get_link(id)
                .await
                .map_err(|err| user_error("The Reddit post could not be retrieved.", err))?;
            let chat_id = message.chat.id.0;
            db.record_post_seen_with_current_time(chat_id, &post)
                .map_err(|err| user_error("The Reddit post could not be recorded.", err))?;
            handle_new_post(config, tg, chat_id, &post)
                .await
                .map_err(|err| user_error("The Reddit post could not be delivered.", err))?;
        }

        Ok(())
    }
    match handle_caption_input(&message, &tg, &config, &caption_edits).await {
        Ok(true) => return Ok(()),
        Ok(false) => {}
        Err(err) => {
            error!("failed to handle repost caption: {err:?}");
            tg.send_message(
                message.chat.id,
                user_facing_message_or(&err, "The repost caption could not be updated."),
            )
            .await?;
            return Ok(());
        }
    }
    if let Err(err) = handle(&message, &tg, &config).await {
        error!("failed to handle message: {err:?}");
        tg.send_message(
            message.chat.id,
            user_facing_message_or(&err, "The message could not be processed."),
        )
        .await?;
    }

    Ok(())
}

async fn handle_command(
    message: Message,
    tg: Arc<Bot>,
    command: Command,
    config: Arc<config::Config>,
    caption_edits: CaptionEditStore,
) -> Result<()> {
    async fn handle(
        message: &Message,
        tg: &Bot,
        command: Command,
        config: Arc<config::Config>,
        caption_edits: &CaptionEditStore,
    ) -> Result<()> {
        let db = db::Database::open(&config)
            .map_err(|err| user_error("The local database could not be accessed.", err))?;
        match command {
            Command::Help => {
                tg.send_message(message.chat.id, Command::descriptions().to_string())
                    .await?;
            }
            Command::Sub(mut args) => {
                let chat_id = message.chat.id.0;
                let subreddit_about = reddit::get_subreddit_about(&args.subreddit).await;
                match subreddit_about {
                    Ok(data) => {
                        args.subreddit = data.display_name;
                        db.subscribe(chat_id, &args)?;
                        info!("subscribed in chat id {chat_id} with {args:#?};");
                        tg.send_message(
                            ChatId(chat_id),
                            format!("Subscribed to r/{}", args.subreddit),
                        )
                        .await?;
                    }
                    Err(reddit::SubredditAboutError::NoSuchSubreddit) => {
                        tg.send_message(ChatId(chat_id), "No such subreddit")
                            .await?;
                    }
                    Err(reddit::SubredditAboutError::Inaccessible { reason }) => {
                        warn!(
                            "refusing to subscribe to r/{}: inaccessible ({reason})",
                            args.subreddit
                        );
                        tg.send_message(
                            ChatId(chat_id),
                            format!("This subreddit is not accessible ({reason})"),
                        )
                        .await?;
                    }
                    Err(err) => {
                        return Err(user_error(
                            "Subreddit information could not be retrieved.",
                            err,
                        ));
                    }
                }
            }
            Command::Unsub(subreddit) => {
                let chat_id = message.chat.id.0;
                let subreddit = subreddit.replace("r/", "");
                let reply = match db.unsubscribe(chat_id, &subreddit) {
                    Ok(sub) => format!("Unsubscribed from r/{sub}"),
                    Err(_) => format!("Error: Not subscribed to r/{subreddit}"),
                };
                tg.send_message(ChatId(chat_id), reply).await?;
            }
            Command::ListSubs => {
                let subs = db.get_subscriptions_for_chat(message.chat.id.0)?;
                let reply = messages::format_subscription_list(&subs);
                tg.send_message(message.chat.id, reply).await?;
            }
            Command::Get(args) => {
                handle_get_command(db, args, config, message, tg).await?;
            }
            Command::RegisterChannel(channel_id) => {
                db.set_repost_channel(message.chat.id.0, channel_id)?;
                tg.send_message(
                    message.chat.id,
                    format!("Repost channel {channel_id} added successfully"),
                )
                .await?;
            }
            Command::Cancel => {
                if !cancel_caption_edit(caption_edits, tg, &db, message.chat.id).await {
                    tg.send_message(message.chat.id, "No caption edit is active.")
                        .await?;
                }
            }
        };

        Ok(())
    }

    if let Err(err) = handle(&message, &tg, command, config, &caption_edits).await {
        error!("failed to handle message: {err:?}");
        tg.send_message(
            message.chat.id,
            user_facing_message_or(&err, "The command could not be completed."),
        )
        .await?;
    }

    Ok(())
}

fn repost_channel_id(db: &db::Database, chat_id: ChatId) -> Result<ChatId> {
    let Some(repost_channel_id) = db.get_repost_channel(chat_id.0)? else {
        return Err(user_message("No repost channel is registered."));
    };
    Ok(ChatId(repost_channel_id))
}

async fn handle_repost_media(
    db: db::Database,
    chat_id: ChatId,
    tg: &Bot,
    message_id: MessageId,
    caption: Option<&RichText>,
) -> Result<()> {
    let repost_channel_id = repost_channel_id(&db, chat_id)?;
    let caption = caption.cloned().unwrap_or_default();
    tg.copy_message(repost_channel_id, chat_id, message_id)
        .caption(caption.text)
        .caption_entities(caption.entities)
        .send()
        .await?;
    Ok(())
}

async fn handle_repost_gallery(
    db: db::Database,
    chat_id: ChatId,
    tg: &Bot,
    gallery_files: Vec<TelegramMediaFile>,
    post_caption: Option<&RichText>,
) -> Result<()> {
    let mut media_group = vec![];
    let mut first = true;

    for media in gallery_files {
        match media.kind {
            MediaKind::Photo => {
                let mut item = InputMediaPhoto::new(InputFile::file_id(media.file_id));
                if first {
                    if let Some(caption) = post_caption {
                        item = item
                            .caption(&caption.text)
                            .caption_entities(caption.entities.clone());
                    }
                    first = false;
                }
                media_group.push(InputMedia::Photo(item));
            }
            MediaKind::Video => {
                let mut item =
                    teloxide::types::InputMediaVideo::new(InputFile::file_id(media.file_id));
                if first {
                    if let Some(caption) = post_caption {
                        item = item
                            .caption(&caption.text)
                            .caption_entities(caption.entities.clone());
                    }
                    first = false;
                }
                media_group.push(InputMedia::Video(item));
            }
        }
    }

    let repost_channel_id = repost_channel_id(&db, chat_id)?;

    tg.send_media_group(repost_channel_id, media_group).await?;
    Ok(())
}

async fn handle_repost_text(
    db: db::Database,
    chat_id: ChatId,
    tg: &Bot,
    content: &RichText,
) -> Result<()> {
    let repost_channel_id = repost_channel_id(&db, chat_id)?;
    tg.send_message(repost_channel_id, &content.text)
        .entities(content.entities.clone())
        .await?;
    Ok(())
}

/// Direct-invocation seam for reposting with an explicit rich-text caption.
#[doc(hidden)]
pub async fn handle_repost_with_rich_caption(
    db: db::Database,
    chat_id: ChatId,
    tg: &Bot,
    post: &reddit::Post,
    delivered: &DeliveredMessages,
    caption: Option<RichText>,
) -> Result<()> {
    match delivered {
        DeliveredMessages::Single(message_id) => {
            if matches!(
                post.post_type,
                PostType::Link | PostType::SelfText | PostType::Unknown
            ) {
                let content = caption.unwrap_or_else(|| RichText {
                    text: post.title.clone(),
                    entities: Vec::new(),
                });
                handle_repost_text(db, chat_id, tg, &content).await
            } else {
                handle_repost_media(db, chat_id, tg, *message_id, caption.as_ref()).await
            }
        }
        DeliveredMessages::Gallery(_) => {
            let tg_file_ids = db.get_telegram_files_for_post(&post.id, chat_id.0)?;
            handle_repost_gallery(db, chat_id, tg, tg_file_ids, caption.as_ref()).await
        }
    }
}

async fn handle_get_command(
    db: db::Database,
    args: SubscriptionArgs,
    config: Arc<config::Config>,
    message: &Message,
    tg: &Bot,
) -> Result<(), anyhow::Error> {
    let subreddit = &args.subreddit;
    let limit = args
        .limit
        .or(config.default_limit)
        .unwrap_or(config::DEFAULT_LIMIT);
    let time = args
        .time
        .or(config.default_time)
        .unwrap_or(config::DEFAULT_TIME_PERIOD);
    let filter = args.filter.or(config.default_filter);
    let chat_id = message.chat.id.0;
    let posts = reddit::get_subreddit_top_posts(subreddit, limit, &time)
        .await
        .map_err(|err| {
            user_error(
                format!("Posts from r/{subreddit} could not be retrieved."),
                err.context("failed to get posts"),
            )
        })?
        .into_iter()
        .filter(|p| {
            if filter.is_some() {
                filter.as_ref() == Some(&p.post_type)
            } else {
                true
            }
        })
        .collect::<Vec<_>>();
    debug!("got {} post(s) for subreddit /r/{}", posts.len(), subreddit);
    if !posts.is_empty() {
        for post in posts {
            process_post(&db, chat_id, &post, &config, tg)
                .await
                .map_err(|err| {
                    user_error(
                        format!("Posts from r/{subreddit} could not be delivered."),
                        err,
                    )
                })?;
        }
    } else {
        tg.send_message(message.chat.id, "No posts found").await?;
    };
    Ok(())
}

fn parse_subscribe_message(input: String) -> Result<(SubscriptionArgs,), ParseError> {
    lazy_static! {
        static ref SUBREDDIT_RE: Regex = Regex::new(r"^[^\s]+").unwrap();
        static ref LIMIT_RE: Regex = Regex::new(r"\blimit=(\d+)\b").unwrap();
        static ref TIME_RE: Regex = Regex::new(r"\btime=(\w+)\b").unwrap();
        static ref FILTER_RE: Regex = Regex::new(r"\bfilter=(\w+)\b").unwrap();
    }

    let subreddit_match = SUBREDDIT_RE
        .find(&input)
        .ok_or_else(|| ParseError::Custom("No subreddit given".into()))?;
    let subreddit = subreddit_match
        .as_str()
        .to_string()
        .replace("/r/", "")
        .replace("r/", "");
    let rest = &input[(subreddit_match.end())..];

    let limit: Option<u32> = LIMIT_RE
        .captures(rest)
        .and_then(|caps| caps.get(1))
        .and_then(|m| m.as_str().parse().ok());

    let time = Ok(TIME_RE.captures(rest))
        .map(|o| o.and_then(|caps| caps.get(1)))
        .and_then(|o| match o {
            Some(m) => m
                .as_str()
                .parse::<TopPostsTimePeriod>()
                .map(Some)
                .map_err(|e| ParseError::IncorrectFormat(e.into())),
            None => Ok(None),
        })?;

    let filter = Ok(FILTER_RE.captures(rest))
        .map(|o| o.and_then(|caps| caps.get(1)))
        .and_then(|o| match o {
            Some(m) => m
                .as_str()
                .parse::<PostType>()
                .map(Some)
                .map_err(|e| ParseError::IncorrectFormat(e.into())),
            None => Ok(None),
        })?;

    let args = SubscriptionArgs {
        subreddit,
        limit,
        time,
        filter,
    };

    Ok((args,))
}

fn malformed_command_reply(input: &str) -> Option<&'static str> {
    let command = input.split_whitespace().next()?.strip_prefix('/')?;
    let command = command.split('@').next()?.to_ascii_lowercase();

    match command.as_str() {
        "registerchannel" => {
            Some("Usage: /registerchannel <channel_id>\n\nExample: /registerchannel -1001234567890")
        }
        "sub" => Some(
            "Usage: /sub <subreddit> [limit=<number>] [time=<period>] [filter=<type>]\n\nExample: /sub rust limit=5 time=week",
        ),
        "get" => Some(
            "Usage: /get <subreddit> [limit=<number>] [time=<period>] [filter=<type>]\n\nExample: /get rust limit=5 time=week",
        ),
        "unsub" => Some("Usage: /unsub <subreddit>\n\nExample: /unsub rust"),
        "help" | "listsubs" | "cancel" => Some("This command does not accept arguments."),
        _ => Some("Unknown command. Send /help to see the available commands."),
    }
}

fn legacy_source_url(message: &Message) -> String {
    let Some(text) = message.text().or_else(|| message.caption()) else {
        return String::new();
    };
    let entities = message
        .entities()
        .or_else(|| message.caption_entities())
        .unwrap_or_default();
    source_url_from_rich_text(text, entities)
}

fn source_url_from_rich_text(text: &str, entities: &[MessageEntity]) -> String {
    MessageEntityRef::parse(text, entities)
        .iter()
        .find_map(|entity| match entity.kind() {
            MessageEntityKind::TextLink { url } => Some(url.to_string()),
            MessageEntityKind::Url => Some(entity.text().to_owned()),
            _ => None,
        })
        .unwrap_or_default()
}

fn legacy_review_metadata(message: &Message, caption: &str) -> RichText {
    let Some(text) = message.text().or_else(|| message.caption()) else {
        return RichText::default();
    };
    let Some(suffix) = text.strip_prefix(caption) else {
        return RichText::default();
    };
    let (separator, metadata_text) = if let Some(metadata) = suffix.strip_prefix("\n\n") {
        ("\n\n", metadata)
    } else if let Some(metadata) = suffix.strip_prefix('\n') {
        ("\n", metadata)
    } else {
        return RichText::default();
    };
    let offset = messages::utf16_len(caption) + messages::utf16_len(separator);
    let entities = message
        .entities()
        .or_else(|| message.caption_entities())
        .unwrap_or_default()
        .iter()
        .filter(|entity| entity.offset >= offset)
        .cloned()
        .map(|mut entity| {
            entity.offset -= offset;
            entity
        })
        .collect();
    RichText {
        text: metadata_text.to_owned(),
        entities,
    }
}

fn legacy_review_caption(message: &Message) -> RichText {
    let text = message
        .text()
        .or_else(|| message.caption())
        .unwrap_or_default();
    let caption_text = text
        .split_once("\n\n")
        .or_else(|| text.split_once('\n'))
        .map_or(text, |(caption, _)| caption);
    let caption_len = messages::utf16_len(caption_text);
    let entities = message
        .entities()
        .or_else(|| message.caption_entities())
        .unwrap_or_default()
        .iter()
        .filter(|entity| entity.offset + entity.length <= caption_len)
        .cloned()
        .collect();
    RichText {
        text: caption_text.to_owned(),
        entities,
    }
}

fn bootstrap_review_post(
    db: &db::Database,
    message: &Message,
    post_id: &str,
) -> Result<ReviewPost> {
    let chat_id = message.chat.id.0;
    let caption = legacy_review_caption(message);
    let metadata = legacy_review_metadata(message, &caption.text);
    let content_kind = if message.text().is_some() {
        ReviewContentKind::Text
    } else {
        ReviewContentKind::Media
    };
    let review = ReviewPost {
        chat_id,
        post_id: post_id.to_owned(),
        source_url: legacy_source_url(message),
        caption,
        content_kind,
        review_message_id: message.id,
        control_message_id: message.id,
        metadata,
        pending_publish_variant: None,
        previous_keyboard: None,
        published_at: None,
    };
    db.upsert_review_post(&review)?;
    Ok(review)
}

async fn select_publish_variant(
    db: &db::Database,
    tg: &Bot,
    caption_edits: &CaptionEditStore,
    message: &Message,
    post_id: &str,
    is_gallery: bool,
    variant: PublishVariant,
) -> Result<bool> {
    let chat_id = message.chat.id;
    if let Some(previous) = take_caption_edit(caption_edits, chat_id) {
        delete_edit_prompt(tg, chat_id, &previous).await;
        if let Some(previous_review) = db.get_review_post(chat_id.0, previous.post_id())? {
            restore_review_keyboard(tg, &previous_review).await?;
            db.clear_review_publish(chat_id.0, previous.post_id())?;
        }
    }

    let review = match db.get_review_post(chat_id.0, post_id)? {
        Some(review) => review,
        None if is_gallery => return Ok(false),
        None => bootstrap_review_post(db, message, post_id)?,
    };
    let previous_keyboard = message.reply_markup();
    db.begin_review_publish(chat_id.0, post_id, variant, previous_keyboard)?;
    tg.edit_message_reply_markup(chat_id, review.control_message_id)
        .reply_markup(messages::format_publish_confirmation_buttons(variant))
        .await?;

    if variant != PublishVariant::WithoutCaption {
        let prompt = tg
            .send_message(
                chat_id,
                format!(
                    "Reply with a replacement Repost Caption, or confirm the current caption unchanged.\n\nCurrent caption:\n{}",
                    review.caption.text
                ),
            )
            .reply_markup(
                ForceReply::new()
                    .input_field_placeholder(Some("Enter the Repost Caption".to_owned())),
            )
            .await?;
        caption_edits
            .lock()
            .expect("caption edit store poisoned")
            .insert(
                chat_id.0,
                CaptionEditState {
                    post_id: post_id.to_owned(),
                    prompt_message_id: prompt.id,
                },
            );
    }
    Ok(true)
}

async fn publish_review(config: &config::Config, tg: &Bot, review: &ReviewPost) -> Result<()> {
    let variant = review
        .pending_publish_variant
        .context("no publish variant is selected")?;
    let content = match variant {
        PublishVariant::Caption => Some(review.caption.clone()),
        PublishVariant::WithoutCaption => None,
        PublishVariant::WithLink => Some(messages::append_source_url(
            &review.caption,
            &review.source_url,
        )),
    };
    let db = db::Database::open(config)?;
    let chat_id = ChatId(review.chat_id);
    match review.content_kind {
        ReviewContentKind::Media => {
            handle_repost_media(db, chat_id, tg, review.review_message_id, content.as_ref()).await
        }
        ReviewContentKind::Gallery => {
            let file_ids = db.get_telegram_files_for_post(&review.post_id, review.chat_id)?;
            handle_repost_gallery(db, chat_id, tg, file_ids, content.as_ref()).await
        }
        ReviewContentKind::Text => {
            let content = content.context("text review cannot be published without content")?;
            handle_repost_text(db, chat_id, tg, &content).await
        }
    }
}

async fn callback_handler(
    q: CallbackQuery,
    config: Arc<config::Config>,
    tg: Arc<Bot>,
    caption_edits: CaptionEditStore,
) -> Result<()> {
    let callback_id = q.id.clone();
    let chat_id = q
        .message
        .as_ref()
        .and_then(|message| message.regular_message())
        .map(|message| message.chat.id);

    if let Err(err) = callback_handler_inner(q, config, tg.clone(), caption_edits).await {
        error!("failed to handle repost callback: {err:?}");
        let message = user_facing_message_or(&err, "The repost action could not be completed.");
        tg.answer_callback_query(callback_id).text(message).await?;
        if let Some(chat_id) = chat_id {
            tg.send_message(chat_id, message).await?;
        }
    }

    Ok(())
}

async fn callback_handler_inner(
    q: CallbackQuery,
    config: Arc<config::Config>,
    tg: Arc<Bot>,
    caption_edits: CaptionEditStore,
) -> Result<()> {
    let db = db::Database::open(&config)?;
    let callback_message = q.message.context("callback message is unavailable")?;
    let message = callback_message
        .regular_message()
        .context("callback message is inaccessible")?;
    let data = q
        .data
        .as_deref()
        .ok_or_else(|| user_message("This review control is no longer supported."))?;
    let data = decode_repost_callback(data)
        .map_err(|err| user_error("This review control is no longer supported.", err))?;
    let chat_id = message.chat.id;

    match data.action {
        RepostAction::Post | RepostAction::PostWithoutCaption | RepostAction::PostWithLink => {
            let post_id = data.post_id.context("repost callback has no post id")?;
            let variant = match data.action {
                RepostAction::Post => PublishVariant::Caption,
                RepostAction::PostWithoutCaption => PublishVariant::WithoutCaption,
                RepostAction::PostWithLink => PublishVariant::WithLink,
                _ => unreachable!(),
            };
            let selected = select_publish_variant(
                &db,
                &tg,
                &caption_edits,
                message,
                &post_id,
                data.is_gallery,
                variant,
            )
            .await?;
            let answer = tg.answer_callback_query(q.id);
            if selected {
                answer.await?;
            } else {
                answer
                    .text("This older gallery is unavailable in the local database.")
                    .await?;
            }
        }
        RepostAction::ConfirmPublish => {
            let review = db
                .get_review_post_by_control_message(chat_id.0, message.id)?
                .context("review confirmation is no longer active")?;
            if !db.claim_review_publish(chat_id.0, &review.post_id)? {
                if review.published_at.is_some() {
                    tg.edit_message_reply_markup(chat_id, review.control_message_id)
                        .reply_markup(InlineKeyboardMarkup::default())
                        .await?;
                    tg.answer_callback_query(q.id).text("Published").await?;
                } else {
                    tg.answer_callback_query(q.id)
                        .text("This publication is already being handled.")
                        .await?;
                }
                return Ok(());
            }
            if let Some(edit) = take_caption_edit_for_post(&caption_edits, chat_id, &review.post_id)
            {
                delete_edit_prompt(&tg, chat_id, &edit).await;
            }
            match publish_review(&config, &tg, &review).await {
                Ok(()) => {
                    db.mark_review_published(chat_id.0, &review.post_id)?;
                    tg.edit_message_reply_markup(chat_id, review.control_message_id)
                        .reply_markup(InlineKeyboardMarkup::default())
                        .await?;
                    tg.answer_callback_query(q.id).text("Published").await?;
                }
                Err(err) => {
                    error!("failed to publish repost: {err:?}");
                    restore_review_keyboard(&tg, &review).await?;
                    db.clear_review_publish(chat_id.0, &review.post_id)?;
                    tg.answer_callback_query(q.id)
                        .text("Failed to publish")
                        .await?;
                    tg.send_message(chat_id, user_facing_message_or(&err, GENERIC_USER_ERROR))
                        .await?;
                }
            }
        }
        RepostAction::CancelPublish => {
            let review = db
                .get_review_post_by_control_message(chat_id.0, message.id)?
                .context("review confirmation is no longer active")?;
            if review.published_at.is_some() {
                tg.edit_message_reply_markup(chat_id, review.control_message_id)
                    .reply_markup(InlineKeyboardMarkup::default())
                    .await?;
                tg.answer_callback_query(q.id).text("Published").await?;
                return Ok(());
            }
            tg.answer_callback_query(q.id).await?;
            if let Some(edit) = take_caption_edit_for_post(&caption_edits, chat_id, &review.post_id)
            {
                delete_edit_prompt(&tg, chat_id, &edit).await;
            }
            restore_review_keyboard(&tg, &review).await?;
            db.clear_review_publish(chat_id.0, &review.post_id)?;
        }
    }

    Ok(())
}

/// Return the first http(s) URL in `text` if it points at a Twitter/X
/// Tweet page (i.e. `/{user}/status/{id}` on `twitter.com`,
/// `mobile.twitter.com`, or `x.com`).
fn parse_x_tweet_url(text: &str) -> Option<Url> {
    let token = text
        .split_whitespace()
        .find(|tok| tok.starts_with("http://") || tok.starts_with("https://"))?;
    let url = Url::parse(token).ok()?;

    let host = url.host_str()?;
    if !matches!(host, "twitter.com" | "mobile.twitter.com" | "x.com") {
        return None;
    }

    let mut segments = url.path_segments()?.filter(|s| !s.is_empty());
    let _user = segments.next()?;
    let marker = segments.next()?;
    let _id = segments.next()?;
    if segments.next().is_some() || marker != "status" {
        return None;
    }
    Some(url)
}

/// Return true if `text` contains a YouTube watch or youtu.be share link.
fn is_youtube_url(text: &str) -> bool {
    lazy_static! {
        static ref RE_YOUTUBE: Regex =
            Regex::new(r"(?:youtube\.com/watch\?v=|youtu\.be/)([\w-]+)").unwrap();
    }
    RE_YOUTUBE.is_match(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_caption_input_with_utf16_limit() {
        const LIMIT: usize = 8;
        assert_eq!(
            validate_caption_input("hello", LIMIT),
            CaptionInput::Valid("hello")
        );
        assert_eq!(validate_caption_input("", LIMIT), CaptionInput::Blank);
        assert_eq!(validate_caption_input(" \n\t", LIMIT), CaptionInput::Blank);
        let maximum = "a".repeat(LIMIT);
        assert_eq!(
            validate_caption_input(&maximum, LIMIT),
            CaptionInput::Valid(&maximum)
        );
        let too_long = "a".repeat(LIMIT + 1);
        assert_eq!(
            validate_caption_input(&too_long, LIMIT),
            CaptionInput::TooLong { maximum: LIMIT }
        );
        let emoji_limit = "👋".repeat(LIMIT / 2);
        assert_eq!(
            validate_caption_input(&emoji_limit, LIMIT),
            CaptionInput::Valid(&emoji_limit)
        );
        assert_eq!(
            validate_caption_input(&format!("{emoji_limit}👋"), LIMIT),
            CaptionInput::TooLong { maximum: LIMIT }
        );
    }

    #[test]
    fn caption_edit_state_identifies_post() {
        let state = CaptionEditState {
            post_id: "post-1".to_owned(),
            prompt_message_id: MessageId(11),
        };

        assert_eq!(state.post_id(), "post-1");
        assert_eq!(state.prompt_message_id, MessageId(11));
    }

    #[test]
    fn direct_repost_only_consumes_an_edit_for_the_same_post() {
        let edits: CaptionEditStore = Arc::new(Mutex::new(HashMap::from([(
            1,
            CaptionEditState {
                post_id: "post-1".to_owned(),
                prompt_message_id: MessageId(11),
            },
        )])));

        assert!(take_caption_edit_for_post(&edits, ChatId(1), "post-2").is_none());
        assert!(take_caption_edit_for_post(&edits, ChatId(1), "post-1").is_some());
    }

    #[test]
    fn source_url_recovery_accepts_visible_and_legacy_links() {
        let visible = "Source: https://example.com/video";
        let visible_offset = messages::utf16_len("Source: ");
        assert_eq!(
            source_url_from_rich_text(
                visible,
                &[MessageEntity::new(
                    MessageEntityKind::Url,
                    visible_offset,
                    messages::utf16_len("https://example.com/video"),
                )],
            ),
            "https://example.com/video"
        );

        let hidden = "video link";
        assert_eq!(
            source_url_from_rich_text(
                hidden,
                &[MessageEntity::text_link(
                    reqwest::Url::parse("https://example.com/legacy").unwrap(),
                    0,
                    messages::utf16_len(hidden),
                )],
            ),
            "https://example.com/legacy"
        );
    }

    #[test]
    fn parse_x_tweet_url_accepts_twitter_tweet() {
        let url = parse_x_tweet_url("https://twitter.com/someuser/status/1234567890")
            .expect("twitter.com Tweet URL should be accepted");
        assert_eq!(url.host_str(), Some("twitter.com"));
        assert_eq!(url.path(), "/someuser/status/1234567890");
    }

    #[test]
    fn parse_x_tweet_url_accepts_mobile_twitter_tweet() {
        let url = parse_x_tweet_url("https://mobile.twitter.com/someuser/status/1234567890")
            .expect("mobile.twitter.com Tweet URL should be accepted");
        assert_eq!(url.host_str(), Some("mobile.twitter.com"));
    }

    #[test]
    fn parse_x_tweet_url_accepts_x_tweet() {
        let url = parse_x_tweet_url("https://x.com/someuser/status/1234567890")
            .expect("x.com Tweet URL should be accepted");
        assert_eq!(url.host_str(), Some("x.com"));
    }

    #[test]
    fn parse_x_tweet_url_rejects_twitter_profile() {
        assert!(parse_x_tweet_url("https://twitter.com/someuser").is_none());
    }

    #[test]
    fn parse_x_tweet_url_rejects_twitter_search() {
        assert!(parse_x_tweet_url("https://twitter.com/search?q=hello").is_none());
    }

    #[test]
    fn parse_x_tweet_url_rejects_twitter_home() {
        assert!(parse_x_tweet_url("https://twitter.com/").is_none());
        assert!(parse_x_tweet_url("https://twitter.com").is_none());
    }

    #[test]
    fn parse_x_tweet_url_rejects_unrelated_hosts() {
        assert!(parse_x_tweet_url("https://example.com/foo/status/1").is_none());
        assert!(parse_x_tweet_url("https://x.com.evil.example/foo").is_none());
    }

    #[test]
    fn parse_x_tweet_url_rejects_tweet_with_extra_path() {
        assert!(parse_x_tweet_url("https://twitter.com/user/status/123/photo/1").is_none());
    }

    #[test]
    fn parse_x_tweet_url_rejects_tweet_with_empty_id() {
        assert!(parse_x_tweet_url("https://twitter.com/user/status/").is_none());
    }

    #[test]
    fn parse_x_tweet_url_ignores_non_url_text() {
        assert!(parse_x_tweet_url("just some text").is_none());
        assert!(parse_x_tweet_url("twitter.com/user/status/1").is_none());
    }

    #[test]
    fn test_is_youtube_url_accepts_youtube_watch_url() {
        assert!(is_youtube_url(
            "https://www.youtube.com/watch?v=abc123def45"
        ));
        assert!(is_youtube_url("https://youtube.com/watch?v=abc123def45"));
        assert!(is_youtube_url("https://youtu.be/abc123def45"));
    }

    #[test]
    fn test_is_youtube_url_rejects_unrelated_text() {
        assert!(!is_youtube_url("https://twitter.com/user/status/1"));
        assert!(!is_youtube_url("https://example.com/watch?v=abc"));
        assert!(!is_youtube_url("just some text"));
    }

    #[test]
    fn test_parse_subscribe_message_only_subreddit() {
        let args = parse_subscribe_message("AnimalsBeingJerks".to_string()).unwrap();
        assert_eq!(
            args.0,
            SubscriptionArgs {
                subreddit: "AnimalsBeingJerks".to_string(),
                limit: None,
                time: None,
                filter: None,
            },
        )
    }

    #[test]
    fn malformed_commands_explain_required_arguments() {
        assert_eq!(
            malformed_command_reply("/registerchannel"),
            Some(
                "Usage: /registerchannel <channel_id>\n\nExample: /registerchannel -1001234567890"
            )
        );
    }

    #[test]
    fn unrecognized_slash_command_suggests_help() {
        assert_eq!(
            malformed_command_reply("/notacommand"),
            Some("Unknown command. Send /help to see the available commands.")
        );
        assert_eq!(
            malformed_command_reply("/reposttochannel@my_bot"),
            Some("Unknown command. Send /help to see the available commands.")
        );
        assert_eq!(malformed_command_reply("plain text"), None);
    }

    #[test]
    fn bot_commands_exclude_legacy_repost_command() {
        assert!(
            Command::bot_commands()
                .iter()
                .all(|command| command.command != "/reposttochannel")
        );
    }

    #[test]
    fn user_facing_error_keeps_technical_details_out_of_telegram_messages() {
        let classified = user_error(
            "The Reddit post could not be retrieved.",
            anyhow::anyhow!("OAuth transport returned HTTP 502"),
        );
        assert_eq!(
            user_facing_message_or(&classified, GENERIC_USER_ERROR),
            "The Reddit post could not be retrieved."
        );

        let expected = user_message("No repost channel is registered.");
        assert_eq!(
            user_facing_message_or(&expected, GENERIC_USER_ERROR),
            "No repost channel is registered."
        );

        let unclassified = anyhow::anyhow!("sqlite is locked");
        assert_eq!(
            user_facing_message_or(&unclassified, GENERIC_USER_ERROR),
            GENERIC_USER_ERROR
        );
    }

    #[test]
    fn unretrievable_x_tweet_uses_the_factual_submission_message() {
        let confirmed = direct_media_user_error(
            true,
            x_tweet::retrieval_error(anyhow::anyhow!("provider returned HTTP 502")),
        );
        assert_eq!(
            user_facing_message_or(&confirmed, GENERIC_USER_ERROR),
            UNRETRIEVABLE_X_TWEET
        );

        let non_x = direct_media_user_error(false, anyhow::anyhow!("media download failed"));
        assert_eq!(
            user_facing_message_or(&non_x, GENERIC_USER_ERROR),
            "The video link could not be processed."
        );

        let transient = direct_media_user_error(true, anyhow::anyhow!("network timeout"));
        assert_eq!(
            user_facing_message_or(&transient, GENERIC_USER_ERROR),
            "The X Tweet could not be processed."
        );
    }

    #[test]
    fn test_parse_subscribe_message_strips_prefix() {
        let args = parse_subscribe_message("r/AnimalsBeingJerks".to_string()).unwrap();
        assert_eq!(
            args.0,
            SubscriptionArgs {
                subreddit: "AnimalsBeingJerks".to_string(),
                limit: None,
                time: None,
                filter: None,
            },
        );

        let args = parse_subscribe_message("/r/AnimalsBeingJerks".to_string()).unwrap();
        assert_eq!(
            args.0,
            SubscriptionArgs {
                subreddit: "AnimalsBeingJerks".to_string(),
                limit: None,
                time: None,
                filter: None,
            },
        )
    }

    #[test]
    fn test_parse_subscribe_message() {
        let args =
            parse_subscribe_message("AnimalsBeingJerks limit=5 time=week filter=video".to_string())
                .unwrap();
        assert_eq!(
            args.0,
            SubscriptionArgs {
                subreddit: "AnimalsBeingJerks".to_string(),
                limit: Some(5),
                time: Some(TopPostsTimePeriod::Week),
                filter: Some(PostType::Video),
            },
        )
    }
}
