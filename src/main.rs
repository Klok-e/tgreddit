use crate::{handle_post::process_post, types::*};
use anyhow::{Context, Result};
use handle_post::handle_new_post;
use log::*;
use reddit::{PostType, TopPostsTimePeriod};
use signal_hook::{
    consts::signal::{SIGINT, SIGTERM},
    iterator::Signals,
};

use std::string::ToString;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use teloxide::types::InputMediaPhoto;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, InputFile};
use teloxide::{prelude::*, types::InputMedia};

use tokio::sync::broadcast;

mod args;
mod bot;
mod config;
mod db;
mod download;
mod handle_post;
mod messages;
mod reddit;
mod types;
mod ytdlp;

const PKG_NAME: &str = env!("CARGO_PKG_NAME");

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    let config = Arc::new(config::read_config());
    info!("starting with config: {config:#?}");
    let mut db = db::Database::open(&config)?;
    db.migrate()?;
    drop(db);

    let (shutdown_tx, mut shutdown_rx) = broadcast::channel::<()>(1);
    let shutdown = Arc::new(AtomicBool::new(false));
    let bot = bot::MyBot::new(config.clone()).await?;

    // Any arguments are for things that help with debugging and development
    // Not optimized for usability.
    //
    // Usage: tgreddit --debug-post <linkid>                    => Fetch post and print deserialized post
    //        tgreddit --debug-post <linkid> --chat-id <chatid> => Also send to telegram
    let opts = args::parse_args();
    if let Some(post_id) = opts.opt_str("debug-post") {
        let post = reddit::get_link(&post_id).await.unwrap();
        info!("{:#?}", post);
        if let Some(chat_id) = opts.opt_str("chat-id") {
            let db = db::Database::open(&config)?;
            let chat_id = chat_id.parse().unwrap();
            db.record_post(chat_id, &post, None)?;
            return handle_new_post(&config, &bot.tg, chat_id, &post).await;
        }
        return Ok(());
    }

    let sub_check_loop_handle = {
        let shutdown = shutdown.clone();
        let tg = bot.tg.clone();
        tokio::task::spawn(async move {
            while !shutdown.load(Ordering::Acquire) {
                check_new_posts(&config, &tg).await.unwrap_or_else(|err| {
                    error!("failed to check for new posts: {err}");
                });

                tokio::select! {
                   _ = tokio::time::sleep(Duration::from_secs(config.check_interval_secs)) => {}
                   _ = shutdown_rx.recv() => {
                       break
                   }
                }
            }
        })
    };
    let (bot_handle, bot_shutdown_token) = bot.spawn();

    {
        let shutdown = shutdown.clone();
        std::thread::spawn(move || {
            let mut forward_signals =
                Signals::new([SIGINT, SIGTERM]).expect("unable to watch for signals");

            for signal in forward_signals.forever() {
                info!("got signal {signal}, shutting down...");
                shutdown.swap(true, Ordering::Relaxed);
                let _res = bot_shutdown_token.shutdown();
                let _res = shutdown_tx.send(()).unwrap_or_else(|_| {
                    // Makes the second Ctrl-C exit instantly
                    std::process::exit(0);
                });
            }
        });
    }

    if let Err(err) = tokio::try_join!(bot_handle, sub_check_loop_handle) {
        panic!("{err}")
    }

    Ok(())
}

async fn check_post_newness(
    config: &config::Config,
    tg: &Bot,
    chat_id: i64,
    filter: Option<reddit::PostType>,
    post: &reddit::Post,
    only_mark_seen: bool,
) -> Result<()> {
    let db = db::Database::open(config)?;
    if filter.is_some() && filter.as_ref() != Some(&post.post_type) {
        debug!("filter set and post does not match filter, skipping");
        return Ok(());
    }

    if db
        .is_post_seen(chat_id, post)
        .expect("failed to query if post is seen")
    {
        debug!("post already seen, skipping...");
        return Ok(());
    }

    if !only_mark_seen {
        // Intentionally marking post as seen if handling it fails. It's preferable to not have it
        // fail continuously.
        process_post(&db, chat_id, post, config, tg).await?;
    }

    db.record_post_seen_with_current_time(chat_id, post)?;
    info!("marked post seen: {}", post.id);

    Ok(())
}

async fn check_new_posts(config: &config::Config, tg: &Bot) -> Result<()> {
    info!("checking subscriptions for new posts");
    let db = db::Database::open(config)?;
    let subs = db.get_all_subscriptions()?;
    for sub in subs {
        check_new_posts_for_subscription(config, tg, &sub)
            .await
            .unwrap_or_else(|err| {
                error!("failed to check subscription for new posts: {err:?}");
            });
    }

    Ok(())
}

async fn check_new_posts_for_subscription(
    config: &config::Config,
    tg: &Bot,
    sub: &Subscription,
) -> Result<()> {
    let db = db::Database::open(config)?;
    let subreddit = &sub.subreddit;
    let limit = sub
        .limit
        .or(config.default_limit)
        .unwrap_or(config::DEFAULT_LIMIT);
    let time = sub
        .time
        .or(config.default_time)
        .unwrap_or(config::DEFAULT_TIME_PERIOD);
    let filter = sub.filter.or(config.default_filter);
    let chat_id = sub.chat_id;

    match reddit::get_subreddit_top_posts(subreddit, limit, &time).await {
        Ok(posts) => {
            debug!("got {} post(s) for subreddit /r/{}", posts.len(), subreddit);

            // First run should not send anything to telegram but the post should be marked
            // as seen, unless skip_initial_send is enabled
            let is_new_subreddit = !db
                .existing_posts_for_subreddit(chat_id, subreddit)
                .context("failed to query if subreddit has existing posts")?;
            let only_mark_seen = is_new_subreddit && config.skip_initial_send;

            for post in posts {
                debug!("got {post:?}");
                check_post_newness(config, tg, chat_id, filter, &post, only_mark_seen)
                    .await
                    .unwrap_or_else(|err| {
                        error!("failed to check post newness: {err:?}");
                    });
            }
        }
        Err(e) => {
            error!("failed to get posts for {}: {e:?}", subreddit)
        }
    };

    Ok(())
}
