//! Live E2E suite: each per-`PostType` test delivers a Reddit post to the
//! operator's chat via `handle_new_post` and then invokes the repost seam
//! twice (with and without caption) to copy the delivered post to the
//! Repost Channel. All tests run on a multi-threaded tokio runtime because
//! `handle_new_post`'s video path calls `tokio::task::block_in_place`,
//! which is only safe on a multi-threaded runtime.

use anyhow::{Context, Result, bail};
use secrecy::ExposeSecret;
use serde::Deserialize;
use std::fs;
use teloxide::{Bot, types::ChatId};
use tgreddit::{bot, config::Config, db::Database, handle_post, reddit};

const APP_CONFIG_PATH: &str = "tgreddit.toml";
const E2E_CONFIG_PATH: &str = "telegram-e2e.toml";

#[derive(Debug, Deserialize)]
struct TelegramE2eConfig {
    chat_id: i64,
}

struct TestCase {
    post_id: &'static str,
    expected_type: reddit::PostType,
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "sends a live link post and re-posts it to the configured Telegram test channel"]
async fn telegram_e2e_delivers_and_reposts_link_post() -> Result<()> {
    run_case(TestCase {
        post_id: "k4qide",
        expected_type: reddit::PostType::Link,
    })
    .await
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "sends a live self-text post and re-posts it to the configured Telegram test channel"]
async fn telegram_e2e_delivers_and_reposts_self_text_post() -> Result<()> {
    run_case(TestCase {
        post_id: "9168hd",
        expected_type: reddit::PostType::SelfText,
    })
    .await
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "sends a live image post and re-posts it to the configured Telegram test channel"]
async fn telegram_e2e_delivers_and_reposts_image_post() -> Result<()> {
    run_case(TestCase {
        post_id: "d16jkk",
        expected_type: reddit::PostType::Image,
    })
    .await
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "sends a live gallery post and re-posts it to the configured Telegram test channel"]
async fn telegram_e2e_delivers_and_reposts_gallery_post() -> Result<()> {
    run_case(TestCase {
        post_id: "rb6vbw",
        expected_type: reddit::PostType::Gallery,
    })
    .await
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "sends a live video post and re-posts it to the configured Telegram test channel"]
async fn telegram_e2e_delivers_and_reposts_video_post() -> Result<()> {
    run_case(TestCase {
        post_id: "98gz9k",
        expected_type: reddit::PostType::Video,
    })
    .await
}

async fn run_case(test_case: TestCase) -> Result<()> {
    let mut app_config = read_app_config()?;
    let e2e_config = read_e2e_config()?;

    let operator_id = i64::try_from(
        *app_config
            .authorized_user_ids
            .first()
            .context("tgreddit.toml must have at least one entry in authorized_user_ids")?,
    )
    .context("operator id from tgreddit.toml does not fit in i64")?;
    let repost_channel_id = e2e_config.chat_id;

    let temp_dir = tempfile::tempdir()?;
    app_config.db_path = temp_dir.path().join("telegram-e2e.db3");

    let mut db = Database::open(&app_config)?;
    db.migrate()?;
    db.set_repost_channel(operator_id, repost_channel_id)?;
    drop(db);

    // Use a per-test transport instead of the process-global one: each
    // `#[tokio::test]` runs on its own runtime, and the global transport's
    // `reqwest::Client` is bound to the runtime that first created it, so
    // sharing it across tests fails with "dispatch task is gone".
    let transport = reddit::oauth::RedditOAuthTransport::new()?;
    let post = reddit::get_link_via(&transport, test_case.post_id).await?;
    if post.post_type != test_case.expected_type {
        let expected = test_case.expected_type;
        bail!(
            "fixture {} drifted from {:?} to {:?}; pick a new {:?} fixture and update the test",
            test_case.post_id,
            expected,
            post.post_type,
            expected,
        );
    }

    // Record the post before delivery so the gallery insert's foreign-key
    // and the seam's `get_post_title` lookup both succeed; `process_post`
    // does this same step in production.
    let db = Database::open(&app_config)?;
    db.record_post_seen_with_current_time(operator_id, &post)?;
    drop(db);

    let tg = Bot::new(app_config.telegram_bot_token.expose_secret());
    let delivered = handle_post::handle_new_post(&app_config, &tg, operator_id, &post).await?;

    for with_caption in [true, false] {
        let db = Database::open(&app_config)?;
        bot::handle_repost_from_callback(
            db,
            ChatId(operator_id),
            &tg,
            &post,
            &delivered,
            with_caption,
        )
        .await
        .with_context(|| format!("repost with_caption={with_caption} failed"))?;
    }

    Ok(())
}

fn read_app_config() -> Result<Config> {
    let config = fs::read_to_string(APP_CONFIG_PATH)
        .with_context(|| format!("failed to read {APP_CONFIG_PATH}"))?;
    toml::from_str(&config).with_context(|| format!("failed to parse {APP_CONFIG_PATH}"))
}

fn read_e2e_config() -> Result<TelegramE2eConfig> {
    let config = fs::read_to_string(E2E_CONFIG_PATH)
        .with_context(|| format!("failed to read {E2E_CONFIG_PATH}"))?;
    toml::from_str(&config).with_context(|| format!("failed to parse {E2E_CONFIG_PATH}"))
}
