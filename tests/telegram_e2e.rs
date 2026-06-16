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

#[tokio::test]
#[ignore = "sends a live link post and re-posts it to the configured Telegram test channel"]
async fn telegram_e2e_delivers_and_reposts_link_post() -> Result<()> {
    run_case(TestCase {
        post_id: "1a4w7p",
        expected_type: reddit::PostType::Link,
    })
    .await
}

#[tokio::test]
#[ignore = "sends a live self-text post and re-posts it to the configured Telegram test channel"]
async fn telegram_e2e_delivers_and_reposts_self_text_post() -> Result<()> {
    run_case(TestCase {
        post_id: "1a6h2c",
        expected_type: reddit::PostType::SelfText,
    })
    .await
}

#[tokio::test]
#[ignore = "sends a live image post and re-posts it to the configured Telegram test channel"]
async fn telegram_e2e_delivers_and_reposts_image_post() -> Result<()> {
    run_case(TestCase {
        post_id: "1bjmswl",
        expected_type: reddit::PostType::Image,
    })
    .await
}

#[tokio::test]
#[ignore = "sends a live gallery post and re-posts it to the configured Telegram test channel"]
async fn telegram_e2e_delivers_and_reposts_gallery_post() -> Result<()> {
    run_case(TestCase {
        post_id: "1a0j1c",
        expected_type: reddit::PostType::Gallery,
    })
    .await
}

#[tokio::test]
#[ignore = "sends a live video post and re-posts it to the configured Telegram test channel"]
async fn telegram_e2e_delivers_and_reposts_video_post() -> Result<()> {
    run_case(TestCase {
        post_id: "1eqpp2",
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

    // Use a per-test transport bound to this test's runtime, not the
    // process-global static. A `reqwest::Client` is bound to the
    // runtime that created it, so sharing the static across
    // `#[tokio::test]` runtimes (which are per-test) fails with
    // "dispatch task is gone".
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

    // Record the post so the gallery delivery's foreign-key insert and the
    // seam's `get_post_title` lookup both succeed. Production callers go
    // through `process_post`, which does this same record step first.
    let db = Database::open(&app_config)?;
    db.record_post_seen_with_current_time(operator_id, &post)?;
    drop(db);

    let tg = Bot::new(app_config.telegram_bot_token.expose_secret());
    let delivered = handle_post::handle_new_post(&app_config, &tg, operator_id, &post).await?;

    // Repost with caption.
    let db = Database::open(&app_config)?;
    bot::handle_repost_from_callback(db, ChatId(operator_id), &tg, &post, &delivered, true).await?;

    // Repost without caption.
    let db = Database::open(&app_config)?;
    bot::handle_repost_from_callback(db, ChatId(operator_id), &tg, &post, &delivered, false)
        .await?;

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
