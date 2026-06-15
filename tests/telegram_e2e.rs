use anyhow::{Context, Result, bail};
use secrecy::ExposeSecret;
use serde::Deserialize;
use std::fs;
use teloxide::Bot;
use tgreddit::{config::Config, db::Database, handle_post, reddit};

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
#[ignore = "sends a live link post to the configured Telegram test channel"]
async fn telegram_e2e_sends_link_post() -> Result<()> {
    send_case(TestCase {
        post_id: "1lt84xt",
        expected_type: reddit::PostType::Link,
    })
    .await
}

#[tokio::test]
#[ignore = "sends a live self-text post to the configured Telegram test channel"]
async fn telegram_e2e_sends_self_text_post() -> Result<()> {
    send_case(TestCase {
        post_id: "9168hd",
        expected_type: reddit::PostType::SelfText,
    })
    .await
}

#[tokio::test]
#[ignore = "sends a live image post to the configured Telegram test channel"]
async fn telegram_e2e_sends_image_post() -> Result<()> {
    send_case(TestCase {
        post_id: "d16jkk",
        expected_type: reddit::PostType::Image,
    })
    .await
}

#[tokio::test]
#[ignore = "sends a live gallery post to the configured Telegram test channel"]
async fn telegram_e2e_sends_gallery_post() -> Result<()> {
    send_case(TestCase {
        post_id: "1hr5vjo",
        expected_type: reddit::PostType::Gallery,
    })
    .await
}

#[tokio::test]
#[ignore = "sends a live video post to the configured Telegram test channel"]
async fn telegram_e2e_sends_video_post() -> Result<()> {
    send_case(TestCase {
        post_id: "1eokk28",
        expected_type: reddit::PostType::Video,
    })
    .await
}

async fn send_case(test_case: TestCase) -> Result<()> {
    let mut app_config = read_app_config()?;
    let e2e_config = read_e2e_config()?;
    let temp_dir = tempfile::tempdir()?;
    app_config.db_path = temp_dir.path().join("telegram-e2e.db3");

    let mut db = Database::open(&app_config)?;
    db.migrate()?;
    drop(db);

    let post = reddit::get_link(test_case.post_id).await?;
    if post.post_type != test_case.expected_type {
        bail!(
            "fixture {} expected {:?}, got {:?}",
            test_case.post_id,
            test_case.expected_type,
            post.post_type
        );
    }

    let tg = Bot::new(app_config.telegram_bot_token.expose_secret());
    handle_post::handle_new_post(&app_config, &tg, e2e_config.chat_id, &post).await?;
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
