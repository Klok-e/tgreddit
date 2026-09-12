use log::error;
use secrecy::SecretString;
use serde::Deserialize;
use std::{env, path::PathBuf};

use crate::{
    PKG_NAME,
    reddit::{PostType, TopPostsTimePeriod},
};

const CONFIG_PATH_ENV: &str = "CONFIG_PATH";
pub const DEFAULT_LIMIT: u32 = 1;
pub const DEFAULT_TIME_PERIOD: TopPostsTimePeriod = TopPostsTimePeriod::Day;
pub const DEFAULT_X_TWEET_API_BASE_URL: &str = "https://api.fxtwitter.com";

#[derive(Deserialize, Debug, Default)]
pub struct Config {
    pub authorized_user_ids: Vec<u64>,
    #[serde(default = "default_db_path")]
    pub db_path: PathBuf,
    pub telegram_bot_token: SecretString,
    pub check_interval_secs: u64,
    #[serde(default = "default_skip_initial_send")]
    pub skip_initial_send: bool,
    pub links_base_url: Option<String>,
    pub default_limit: Option<u32>,
    pub default_time: Option<TopPostsTimePeriod>,
    pub default_filter: Option<PostType>,
    #[serde(default = "default_x_tweet_api_base_url")]
    pub x_tweet_api_base_url: String,
}

pub fn read_config() -> Config {
    env::var(CONFIG_PATH_ENV)
        .map_err(|_| format!("{CONFIG_PATH_ENV} environment variable not set"))
        .and_then(|config_path| std::fs::read_to_string(config_path).map_err(|e| e.to_string()))
        .and_then(|str| toml::from_str(&str).map_err(|e| e.to_string()))
        .unwrap_or_else(|err| {
            error!("failed to read config: {err}");
            std::process::exit(1);
        })
}

fn default_db_path() -> PathBuf {
    let xdg_dirs = xdg::BaseDirectories::with_prefix(PKG_NAME);
    xdg_dirs.place_state_file("data.db3").unwrap()
}

fn default_skip_initial_send() -> bool {
    true
}

fn default_x_tweet_api_base_url() -> String {
    DEFAULT_X_TWEET_API_BASE_URL.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn x_tweet_api_defaults_to_fxtwitter() {
        let config: Config = toml::from_str(
            r#"
                authorized_user_ids = []
                telegram_bot_token = "token"
                check_interval_secs = 60
            "#,
        )
        .expect("minimal configuration is valid");

        assert_eq!(config.x_tweet_api_base_url, DEFAULT_X_TWEET_API_BASE_URL);
    }
}
