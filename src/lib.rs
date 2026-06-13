pub mod args;
pub mod bot;
pub mod config;
pub mod db;
pub mod download;
pub mod handle_post;
pub mod messages;
pub mod reddit;
pub mod types;
pub mod ytdlp;

pub const PKG_NAME: &str = env!("CARGO_PKG_NAME");
