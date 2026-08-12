use crate::{
    config, db,
    handle_post::{DeliveredMessages, handle_new_post, handle_video_link, process_post},
    messages, reddit,
    reddit::{PostType, TopPostsTimePeriod},
    types::{RepostAction, SubscriptionArgs, decode_repost_callback},
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
        CallbackQuery, ChatId, FileId, ForceReply, InputFile, InputMedia, InputMediaPhoto, Message,
        MessageId, Update,
    },
    utils::command::{BotCommands, ParseError},
};
use url::Url;

const TELEGRAM_BOT_API_URL_ENV: &str = "TELEGRAM_BOT_API_URL";
const MAX_REPOST_CAPTION_CHARS: usize = 1024;

type CaptionEditStore = Arc<Mutex<HashMap<i64, CaptionEditState>>>;

#[derive(Debug, Clone, PartialEq, Eq)]
enum CaptionEditState {
    AwaitingInput {
        post_id: String,
        is_gallery: bool,
        source_message_id: MessageId,
        prompt_message_id: MessageId,
    },
    AwaitingConfirmation {
        post_id: String,
        is_gallery: bool,
        source_message_id: MessageId,
        preview_message_id: MessageId,
        caption: String,
    },
}

impl CaptionEditState {
    fn post_id(&self) -> &str {
        match self {
            Self::AwaitingInput { post_id, .. } | Self::AwaitingConfirmation { post_id, .. } => {
                post_id
            }
        }
    }

    fn interactive_message_id(&self) -> MessageId {
        match self {
            Self::AwaitingInput {
                prompt_message_id, ..
            } => *prompt_message_id,
            Self::AwaitingConfirmation {
                preview_message_id, ..
            } => *preview_message_id,
        }
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
    #[command(description = "repost to the registered channel", parse_with = "split")]
    RepostToChannel {
        message_id: i32,
        description: String,
    },
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

async fn mark_edit_cancelled(tg: &Bot, chat_id: ChatId, state: CaptionEditState) {
    if let Err(err) = tg
        .edit_message_text(chat_id, state.interactive_message_id(), "Cancelled")
        .await
    {
        warn!("failed to mark caption edit as cancelled: {err}");
    }
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

fn take_caption_confirmation(
    caption_edits: &CaptionEditStore,
    chat_id: ChatId,
    preview_message_id: MessageId,
) -> Option<CaptionEditState> {
    let mut edits = caption_edits.lock().expect("caption edit store poisoned");
    if matches!(
        edits.get(&chat_id.0),
        Some(CaptionEditState::AwaitingConfirmation {
            preview_message_id: active_preview,
            ..
        }) if *active_preview == preview_message_id
    ) {
        edits.remove(&chat_id.0)
    } else {
        None
    }
}

async fn cancel_caption_edit(caption_edits: &CaptionEditStore, tg: &Bot, chat_id: ChatId) -> bool {
    let Some(state) = take_caption_edit(caption_edits, chat_id) else {
        return false;
    };
    mark_edit_cancelled(tg, chat_id, state).await;
    true
}

#[derive(Debug, PartialEq, Eq)]
enum CaptionInput<'a> {
    Valid(&'a str),
    Blank,
    TooLong,
}

fn validate_caption_input(text: &str) -> CaptionInput<'_> {
    if text.trim().is_empty() {
        CaptionInput::Blank
    } else if text.encode_utf16().count() > MAX_REPOST_CAPTION_CHARS {
        CaptionInput::TooLong
    } else {
        CaptionInput::Valid(text)
    }
}

async fn handle_caption_input(
    message: &Message,
    tg: &Bot,
    caption_edits: &CaptionEditStore,
) -> Result<bool> {
    let chat_id = message.chat.id;
    let reply_to = message.reply_to_message().map(|message| message.id);
    let state = caption_edits
        .lock()
        .expect("caption edit store poisoned")
        .get(&chat_id.0)
        .cloned();
    let Some(CaptionEditState::AwaitingInput {
        post_id,
        is_gallery,
        source_message_id,
        prompt_message_id,
    }) = state
    else {
        return Ok(false);
    };
    if reply_to != Some(prompt_message_id) {
        return Ok(false);
    }

    let Some(text) = message.text() else {
        tg.send_message(chat_id, "Send the Repost Caption as plain text.")
            .reply_to(prompt_message_id)
            .await?;
        return Ok(true);
    };
    let caption = match validate_caption_input(text) {
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
        CaptionInput::TooLong => {
            tg.send_message(
                chat_id,
                format!(
                    "The Repost Caption must be at most {MAX_REPOST_CAPTION_CHARS} characters."
                ),
            )
            .reply_to(prompt_message_id)
            .await?;
            return Ok(true);
        }
    };

    let preview = tg
        .send_message(chat_id, format!("Repost Caption preview:\n\n{caption}"))
        .reply_markup(messages::format_caption_confirmation_buttons())
        .await?;
    let transitioned = {
        let mut edits = caption_edits.lock().expect("caption edit store poisoned");
        if matches!(
            edits.get(&chat_id.0),
            Some(CaptionEditState::AwaitingInput {
                prompt_message_id: active_prompt,
                ..
            }) if *active_prompt == prompt_message_id
        ) {
            edits.insert(
                chat_id.0,
                CaptionEditState::AwaitingConfirmation {
                    post_id,
                    is_gallery,
                    source_message_id,
                    preview_message_id: preview.id,
                    caption: caption.to_owned(),
                },
            );
            true
        } else {
            false
        }
    };
    if !transitioned {
        tg.edit_message_text(chat_id, preview.id, "Cancelled")
            .await?;
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

        let text = message.text().context("No text in message")?;

        let db = db::Database::open(config)?;
        if let Some(link) = parse_twitter_status_url(text) {
            handle_video_link(&db, tg, message.chat.id.0, &link).await?;
        } else if is_youtube_url(text) {
            let link = Url::parse(text)?;
            handle_video_link(&db, tg, message.chat.id.0, &link).await?;
        } else {
            let id = RE_REDDIT
                .captures(text)
                .context("Couldn't match reddit post url")?
                .get(1)
                .context("Couldn't find reddit post id")?
                .as_str();
            let post = reddit::get_link(id).await?;
            let chat_id = message.chat.id.0;
            db.record_post_seen_with_current_time(chat_id, &post)?;
            handle_new_post(config, tg, chat_id, &post).await?;
        }

        Ok(())
    }
    if handle_caption_input(&message, &tg, &caption_edits).await? {
        return Ok(());
    }
    if let Err(err) = handle(&message, &tg, &config).await {
        error!("failed to handle message: {err:?}");
        tg.send_message(message.chat.id, format!("Something went wrong: {err}"))
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
        let db = db::Database::open(&config)?;
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
                        Err(err).context("Couldn't download about.json for subreddit")?;
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
            Command::RepostToChannel {
                description,
                message_id,
            } => {
                let button_data = match description.as_str() {
                    "" => None,
                    _ => Some(description),
                };
                handle_repost(db, message.chat.id, tg, message_id, button_data).await?;
            }
            Command::Cancel => {
                if !cancel_caption_edit(caption_edits, tg, message.chat.id).await {
                    tg.send_message(message.chat.id, "No caption edit is active.")
                        .await?;
                }
            }
        };

        Ok(())
    }

    if let Err(err) = handle(&message, &tg, command, config, &caption_edits).await {
        error!("failed to handle message: {err:?}");
        tg.send_message(message.chat.id, "Something went wrong")
            .await?;
    }

    Ok(())
}

async fn handle_repost(
    db: db::Database,
    chat_id: ChatId,
    tg: &Bot,
    message_id: i32,
    caption: Option<String>,
) -> Result<()> {
    let Some(repost_channel_id) = db.get_repost_channel(chat_id.0)? else {
        tg.send_message(chat_id, "Repost channel not registered".to_string())
            .await?;
        return Ok(());
    };
    let caption = if let Some(caption) = &caption {
        caption
    } else {
        ""
    };
    tg.copy_message(ChatId(repost_channel_id), chat_id, MessageId(message_id))
        .caption(caption)
        .send()
        .await?;
    Ok(())
}

async fn handle_repost_gallery(
    db: db::Database,
    chat_id: ChatId,
    tg: &Bot,
    gallery_file_ids: Vec<FileId>,
    post_caption: Option<String>,
) -> Result<()> {
    let mut media_group = vec![];
    let mut first = true;

    for file_id in gallery_file_ids {
        let mut input_media_photo = InputMediaPhoto::new(InputFile::file_id(file_id));
        // The first media item carries the caption for the whole gallery.
        if first {
            if let Some(caption) = &post_caption {
                input_media_photo = input_media_photo.caption(caption);
            }
            first = false;
        }

        media_group.push(InputMedia::Photo(input_media_photo))
    }

    let Some(repost_channel_id) = db.get_repost_channel(chat_id.0)? else {
        tg.send_message(chat_id, "Repost channel not registered".to_string())
            .await?;
        return Ok(());
    };

    tg.send_media_group(ChatId(repost_channel_id), media_group)
        .await?;
    Ok(())
}

/// Direct-invocation seam for the inline-button repost flow. Accepts a
/// constructed callback payload (the message id(s), the post, and the
/// with-caption flag) and drives the same flow as `callback_handler` would
/// for an inline-button callback, without going through teloxide's
/// `CallbackQuery` dispatcher.
///
/// This is used by the live E2E tests to simulate a button tap. The
/// production dispatcher continues to call the same underlying repost
/// logic unchanged.
#[doc(hidden)]
pub async fn handle_repost_from_callback(
    db: db::Database,
    chat_id: ChatId,
    tg: &Bot,
    post: &reddit::Post,
    delivered: &DeliveredMessages,
    with_caption: bool,
) -> Result<()> {
    let caption = if with_caption {
        Some(db.get_post_title(chat_id.0, &post.id)?)
    } else {
        None
    };
    handle_repost_with_caption(db, chat_id, tg, post, delivered, caption).await
}

/// Direct-invocation seam for reposting with an explicit plain-text caption.
#[doc(hidden)]
pub async fn handle_repost_with_caption(
    db: db::Database,
    chat_id: ChatId,
    tg: &Bot,
    post: &reddit::Post,
    delivered: &DeliveredMessages,
    caption: Option<String>,
) -> Result<()> {
    match delivered {
        DeliveredMessages::Single(message_id) => {
            handle_repost(db, chat_id, tg, message_id.0, caption).await
        }
        DeliveredMessages::Gallery(_) => {
            let tg_file_ids = db.get_telegram_files_for_post(&post.id, chat_id.0)?;
            handle_repost_gallery(db, chat_id, tg, tg_file_ids, caption).await
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
        .context("failed to get posts")?
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
            process_post(&db, chat_id, &post, &config, tg).await?;
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

async fn callback_handler(
    q: CallbackQuery,
    config: Arc<config::Config>,
    tg: Arc<Bot>,
    caption_edits: CaptionEditStore,
) -> Result<()> {
    let db = db::Database::open(&config)?;
    let msg = q.message.context("callback message is unavailable")?;
    let data = decode_repost_callback(q.data.as_deref().context("callback data is missing")?)?;
    let chat_id = msg.chat().id;
    let msg_id = if let Some(reply_id) = msg
        .regular_message()
        .and_then(|x| x.reply_to_message())
        .map(|x| x.id)
    {
        reply_id
    } else {
        msg.id()
    };

    match data.action {
        RepostAction::Post | RepostAction::PostWithoutCaption => {
            tg.answer_callback_query(q.id).await?;
            let post_id = data.post_id.context("repost callback has no post id")?;
            let previous = take_caption_edit_for_post(&caption_edits, chat_id, &post_id);
            if let Some(previous) = previous {
                mark_edit_cancelled(&tg, chat_id, previous).await;
            }

            let caption = if data.action == RepostAction::Post {
                Some(db.get_post_title(chat_id.0, &post_id)?)
            } else {
                None
            };
            if data.is_gallery {
                let file_ids = db.get_telegram_files_for_post(&post_id, chat_id.0)?;
                handle_repost_gallery(db, chat_id, &tg, file_ids, caption)
                    .await
                    .context("Failed handling gallery repost")?;
            } else {
                handle_repost(db, chat_id, &tg, msg_id.0, caption)
                    .await
                    .context("Failed handling repost")?;
            }
        }
        RepostAction::EditCaption => {
            tg.answer_callback_query(q.id).await?;
            let post_id = data.post_id.context("edit callback has no post id")?;
            if let Some(previous) = take_caption_edit(&caption_edits, chat_id) {
                mark_edit_cancelled(&tg, chat_id, previous).await;
            }
            let current_caption = db.get_post_title(chat_id.0, &post_id)?;
            let prompt = tg
                .send_message(
                    chat_id,
                    format!(
                        "Reply with the complete replacement Repost Caption.\n\nCurrent caption:\n{current_caption}"
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
                    CaptionEditState::AwaitingInput {
                        post_id,
                        is_gallery: data.is_gallery,
                        source_message_id: msg_id,
                        prompt_message_id: prompt.id,
                    },
                );
        }
        RepostAction::PublishCaption => {
            let state = take_caption_confirmation(&caption_edits, chat_id, msg.id());
            let Some(CaptionEditState::AwaitingConfirmation {
                post_id,
                is_gallery,
                source_message_id,
                preview_message_id,
                caption,
            }) = state
            else {
                tg.answer_callback_query(q.id)
                    .text("This caption edit is no longer active.")
                    .await?;
                return Ok(());
            };
            tg.answer_callback_query(q.id).await?;

            let publish_result: Result<()> = async {
                if is_gallery {
                    let file_ids = db.get_telegram_files_for_post(&post_id, chat_id.0)?;
                    handle_repost_gallery(db, chat_id, &tg, file_ids, Some(caption)).await
                } else {
                    handle_repost(db, chat_id, &tg, source_message_id.0, Some(caption)).await
                }
            }
            .await;
            let status = if publish_result.is_ok() {
                "Published"
            } else {
                "Failed to publish"
            };
            tg.edit_message_text(chat_id, preview_message_id, status)
                .await?;
            publish_result?;
        }
        RepostAction::CancelCaption => {
            let state = take_caption_confirmation(&caption_edits, chat_id, msg.id());
            if let Some(state) = state {
                tg.answer_callback_query(q.id).await?;
                mark_edit_cancelled(&tg, chat_id, state).await;
            } else {
                tg.answer_callback_query(q.id)
                    .text("This caption edit is no longer active.")
                    .await?;
            }
        }
    }

    Ok(())
}

/// Return the first http(s) URL in `text` if it points at a Twitter/X
/// status page (i.e. `/{user}/status/{id}` on `twitter.com`,
/// `mobile.twitter.com`, or `x.com`).
fn parse_twitter_status_url(text: &str) -> Option<Url> {
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
    fn validates_plain_text_caption_input() {
        assert_eq!(
            validate_caption_input("hello\nworld 👋"),
            CaptionInput::Valid("hello\nworld 👋")
        );
        assert_eq!(validate_caption_input(""), CaptionInput::Blank);
        assert_eq!(validate_caption_input(" \n\t"), CaptionInput::Blank);
        let maximum = "a".repeat(MAX_REPOST_CAPTION_CHARS);
        assert_eq!(
            validate_caption_input(&maximum),
            CaptionInput::Valid(&maximum)
        );
        let too_long = "a".repeat(MAX_REPOST_CAPTION_CHARS + 1);
        assert_eq!(validate_caption_input(&too_long), CaptionInput::TooLong);
        let emoji_limit = "👋".repeat(MAX_REPOST_CAPTION_CHARS / 2);
        assert_eq!(
            validate_caption_input(&emoji_limit),
            CaptionInput::Valid(&emoji_limit)
        );
        assert_eq!(
            validate_caption_input(&format!("{emoji_limit}👋")),
            CaptionInput::TooLong
        );
    }

    #[test]
    fn caption_edit_state_identifies_post_and_interactive_message() {
        let state = CaptionEditState::AwaitingConfirmation {
            post_id: "post-1".to_owned(),
            is_gallery: false,
            source_message_id: MessageId(10),
            preview_message_id: MessageId(11),
            caption: "replacement".to_owned(),
        };

        assert_eq!(state.post_id(), "post-1");
        assert_eq!(state.interactive_message_id(), MessageId(11));
    }

    #[test]
    fn confirmation_can_only_be_consumed_once_by_its_preview() {
        let edits: CaptionEditStore = Arc::new(Mutex::new(HashMap::from([(
            1,
            CaptionEditState::AwaitingConfirmation {
                post_id: "post-1".to_owned(),
                is_gallery: false,
                source_message_id: MessageId(10),
                preview_message_id: MessageId(11),
                caption: "replacement".to_owned(),
            },
        )])));

        assert!(take_caption_confirmation(&edits, ChatId(1), MessageId(12)).is_none());
        assert!(take_caption_confirmation(&edits, ChatId(1), MessageId(11)).is_some());
        assert!(take_caption_confirmation(&edits, ChatId(1), MessageId(11)).is_none());
    }

    #[test]
    fn direct_repost_only_consumes_an_edit_for_the_same_post() {
        let edits: CaptionEditStore = Arc::new(Mutex::new(HashMap::from([(
            1,
            CaptionEditState::AwaitingInput {
                post_id: "post-1".to_owned(),
                is_gallery: false,
                source_message_id: MessageId(10),
                prompt_message_id: MessageId(11),
            },
        )])));

        assert!(take_caption_edit_for_post(&edits, ChatId(1), "post-2").is_none());
        assert!(take_caption_edit_for_post(&edits, ChatId(1), "post-1").is_some());
    }

    #[test]
    fn test_parse_twitter_status_url_accepts_twitter_status() {
        let url = parse_twitter_status_url("https://twitter.com/someuser/status/1234567890")
            .expect("twitter.com status URL should be accepted");
        assert_eq!(url.host_str(), Some("twitter.com"));
        assert_eq!(url.path(), "/someuser/status/1234567890");
    }

    #[test]
    fn test_parse_twitter_status_url_accepts_mobile_twitter_status() {
        let url = parse_twitter_status_url("https://mobile.twitter.com/someuser/status/1234567890")
            .expect("mobile.twitter.com status URL should be accepted");
        assert_eq!(url.host_str(), Some("mobile.twitter.com"));
    }

    #[test]
    fn test_parse_twitter_status_url_accepts_x_status() {
        let url = parse_twitter_status_url("https://x.com/someuser/status/1234567890")
            .expect("x.com status URL should be accepted");
        assert_eq!(url.host_str(), Some("x.com"));
    }

    #[test]
    fn test_parse_twitter_status_url_rejects_twitter_profile() {
        assert!(parse_twitter_status_url("https://twitter.com/someuser").is_none());
    }

    #[test]
    fn test_parse_twitter_status_url_rejects_twitter_search() {
        assert!(parse_twitter_status_url("https://twitter.com/search?q=hello").is_none());
    }

    #[test]
    fn test_parse_twitter_status_url_rejects_twitter_home() {
        assert!(parse_twitter_status_url("https://twitter.com/").is_none());
        assert!(parse_twitter_status_url("https://twitter.com").is_none());
    }

    #[test]
    fn test_parse_twitter_status_url_rejects_unrelated_hosts() {
        assert!(parse_twitter_status_url("https://example.com/foo/status/1").is_none());
        assert!(parse_twitter_status_url("https://x.com.evil.example/foo").is_none());
    }

    #[test]
    fn test_parse_twitter_status_url_rejects_status_with_extra_path() {
        assert!(parse_twitter_status_url("https://twitter.com/user/status/123/photo/1").is_none());
    }

    #[test]
    fn test_parse_twitter_status_url_rejects_status_with_empty_id() {
        assert!(parse_twitter_status_url("https://twitter.com/user/status/").is_none());
    }

    #[test]
    fn test_parse_twitter_status_url_ignores_non_url_text() {
        assert!(parse_twitter_status_url("just some text").is_none());
        assert!(parse_twitter_status_url("twitter.com/user/status/1").is_none());
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
