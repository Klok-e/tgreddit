use crate::*;
use anyhow::Result;
use lazy_static::lazy_static;
use regex::Regex;
use std::sync::Arc;
use teloxide::{
    dispatching::DefaultKey,
    types::MessageId,
    utils::command::{BotCommands, ParseError},
};

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
}

pub struct MyBot {
    pub dispatcher: Dispatcher<Arc<Bot>, anyhow::Error, DefaultKey>,
    pub tg: Arc<Bot>,
}

impl MyBot {
    pub async fn new(config: Arc<config::Config>) -> Result<Self> {
        let client = teloxide::net::default_reqwest_settings()
            .timeout(Duration::from_secs(120))
            .build()
            .expect("Client creation failed");
        let tg = Arc::new(Bot::with_client(
            config.telegram_bot_token.expose_secret(),
            client,
        ));
        tg.set_my_commands(Command::bot_commands()).await?;

        let handler = dptree::entry()
            .branch(
                Update::filter_message().branch(
                    dptree::filter(|msg: Message, config: Arc<config::Config>| {
                        msg.from()
                            .map(|user| config.authorized_user_ids.contains(&user.id.0))
                            .unwrap_or_default()
                    })
                    .filter_command::<Command>()
                    .endpoint(handle_command),
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
            .dependencies(dptree::deps![config.clone()])
            .default_handler(|upd| async move {
                warn!("unhandled update: {:?}", upd);
            })
            .error_handler(LoggingErrorHandler::with_custom_text(
                "an error has occurred in the dispatcher",
            ))
            .build();

        let my_bot = MyBot {
            dispatcher,
            tg: tg.clone(),
        };
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

pub async fn handle_command(
    message: Message,
    tg: Arc<Bot>,
    command: Command,
    config: Arc<config::Config>,
) -> Result<()> {
    async fn handle(
        message: &Message,
        tg: &Bot,
        command: Command,
        config: Arc<config::Config>,
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
        };

        Ok(())
    }

    if let Err(err) = handle(&message, &tg, command, config).await {
        error!("failed to handle message: {:?}", err);
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
        tg.send_message(
            chat_id,
            "Repost channel not registered".to_string(),
        )
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
    gallery_file_ids: Vec<String>,
    post_caption: Option<String>,
) -> Result<()> {
    let mut media_group = vec![];
    let mut first = true;

    for file_id in gallery_file_ids {
        let mut input_media_photo = InputMediaPhoto::new(InputFile::file_id(file_id));
        // The first InputMediaPhoto in the vector needs to contain the caption and parse_mode;
        if first {
            if let Some(caption) = &post_caption {
                input_media_photo = input_media_photo.caption(caption);
            }
            input_media_photo = input_media_photo.parse_mode(teloxide::types::ParseMode::Html);
            first = false;
        }

        media_group.push(InputMedia::Photo(input_media_photo))
    }

    let Some(repost_channel_id) = db.get_repost_channel(chat_id.0)? else {
        tg.send_message(
            chat_id,
            "Repost channel not registered".to_string(),
        )
        .await?;
        return Ok(());
    };

    tg.send_media_group(ChatId(repost_channel_id), media_group)
        .await?;
    Ok(())
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
            db.record_post(chat_id, &post, None)?;
            if let Err(e) = handle_new_post(&config, tg, chat_id, &post).await {
                error!("failed to handle new post: {e:?}");
            }
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
) -> Result<()> {
    let db = db::Database::open(&config)?;

    let msg = q.message.expect("Message must exist");
    let data = q.data.expect("Data expected");
    let data: ButtonCallbackData = serde_json::from_str(&data)?;
    let caption = if data.copy_caption {
        Some(db.get_post_title(msg.chat.id.0, &data.post_id)?)
    } else {
        None
    };
    let msg_id = if let Some(reply_id) = msg.reply_to_message() {
        reply_id.id
    } else {
        msg.id
    };
    if data.is_gallery {
        let tg_file_ids = db.get_telegram_files_for_post(&data.post_id, msg.chat.id.0)?;
        handle_repost_gallery(db, msg.chat.id, &tg, tg_file_ids, caption).await?;
    } else {
        handle_repost(db, msg.chat.id, &tg, msg_id.0, caption).await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
