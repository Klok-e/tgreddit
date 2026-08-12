use crate::{
    db::Recordable,
    reddit::{self},
    types::{RepostAction, RepostCallbackData, Subscription, Video},
};
use itertools::Itertools;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};

fn escape(html: &str) -> String {
    html.replace('<', "&lt;").replace('>', "&gt;")
}

fn format_html_anchor(href: &str, text: &str) -> String {
    format!(r#"<a href="{href}">{}</a>"#, escape(text))
}

fn format_subreddit_link(subreddit: &str, base_url: Option<&str>) -> String {
    format_html_anchor(
        &reddit::format_subreddit_url(subreddit, base_url),
        &format!("/r/{subreddit}"),
    )
}

fn format_meta_html(post: &reddit::Post, links_base_url: Option<&str>) -> String {
    let subreddit_link = format_subreddit_link(&post.subreddit, links_base_url);
    let comments_link = format_html_anchor(&post.format_permalink_url(links_base_url), "comments");

    // If using custom links base url, the old reddit link doesn't make sense.
    match links_base_url {
        Some(_) => format!("{subreddit_link} [{comments_link}]"),
        None => {
            let old_comments_link = format_html_anchor(&post.format_old_permalink_url(), "old");
            format!("{subreddit_link} [{comments_link}, {old_comments_link}]")
        }
    }
}

pub fn format_media_caption_html(post: &reddit::Post, links_base_url: Option<&str>) -> String {
    let title = &post.title;
    let meta = format_meta_html(post, links_base_url);
    format!("{title}\n{meta}")
}

pub fn format_link_video_caption_html(video: &Video) -> String {
    let title = &video.title;
    let meta = format_html_anchor(&video.url, "video link");
    format!("{title}\n{meta}")
}

fn callback_data<T: Recordable>(post: &T, action: RepostAction, is_gallery: bool) -> String {
    let data = serde_json::to_string(&RepostCallbackData {
        action,
        post_id: Some(post.id().to_owned()),
        is_gallery,
    })
    .expect("repost callback data should serialize");
    assert!(data.len() <= 64, "Telegram callback data exceeds 64 bytes");
    data
}

pub fn format_media_repost_buttons<T: Recordable>(
    post: &T,
    is_gallery: bool,
) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::default().append_row([
        InlineKeyboardButton::callback("Post", callback_data(post, RepostAction::Post, is_gallery)),
        InlineKeyboardButton::callback(
            "Post (no caption)",
            callback_data(post, RepostAction::PostWithoutCaption, is_gallery),
        ),
        InlineKeyboardButton::callback(
            "Edit caption",
            callback_data(post, RepostAction::EditCaption, is_gallery),
        ),
    ])
}

pub fn format_post_button<T: Recordable>(post: &T) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::default().append_row([InlineKeyboardButton::callback(
        "Post",
        callback_data(post, RepostAction::Post, false),
    )])
}

pub fn format_caption_confirmation_buttons() -> InlineKeyboardMarkup {
    let serialize = |action| {
        serde_json::to_string(&RepostCallbackData {
            action,
            post_id: None,
            is_gallery: false,
        })
        .expect("caption confirmation callback should serialize")
    };
    InlineKeyboardMarkup::default().append_row([
        InlineKeyboardButton::callback("Publish", serialize(RepostAction::PublishCaption)),
        InlineKeyboardButton::callback("Cancel", serialize(RepostAction::CancelCaption)),
    ])
}

pub fn format_link_message_html(post: &reddit::Post, links_base_url: Option<&str>) -> String {
    let title = format_html_anchor(&post.url, &post.title);
    let meta = format_meta_html(post, links_base_url);
    format!("{title}\n{meta}")
}

pub fn format_subscription_list(post: &[Subscription]) -> String {
    fn format_subscription(sub: &Subscription) -> String {
        let mut args = vec![];
        if let Some(time) = sub.time {
            args.push(format!("time={time}"));
        }
        if let Some(limit) = sub.limit {
            args.push(format!("limit={limit}"));
        }
        if let Some(filter) = sub.filter {
            args.push(format!("filter={filter}"));
        }

        let args_str = if !args.is_empty() {
            format!("({})", args.join(", "))
        } else {
            "".to_string()
        };

        [sub.subreddit.to_owned(), args_str]
            .join(" ")
            .trim_end()
            .to_string()
    }

    if post.is_empty() {
        "No subscriptions".to_owned()
    } else {
        post.iter().map(format_subscription).join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Recordable;
    use crate::reddit::TopPostsTimePeriod;
    use teloxide::types::InlineKeyboardButtonKind;

    struct TestPost;

    impl Recordable for TestPost {
        fn id(&self) -> &str {
            "abc123"
        }

        fn title(&self) -> &str {
            "title"
        }

        fn subreddit(&self) -> &str {
            "test"
        }
    }

    fn button_action(button: &InlineKeyboardButton) -> RepostAction {
        let InlineKeyboardButtonKind::CallbackData(data) = &button.kind else {
            panic!("expected callback button")
        };
        crate::types::decode_repost_callback(data).unwrap().action
    }

    #[test]
    fn test_format_html_anchor() {
        assert_eq!(
            format_html_anchor("https://example.com", "<hello></world>"),
            r#"<a href="https://example.com">&lt;hello&gt;&lt;/world&gt;</a>"#
        )
    }

    #[test]
    fn media_repost_buttons_offer_all_caption_actions() {
        let keyboard = format_media_repost_buttons(&TestPost, true);
        let buttons = &keyboard.inline_keyboard[0];

        assert_eq!(
            buttons
                .iter()
                .map(|button| button.text.as_str())
                .collect_vec(),
            ["Post", "Post (no caption)", "Edit caption"]
        );
        assert_eq!(
            buttons.iter().map(button_action).collect_vec(),
            [
                RepostAction::Post,
                RepostAction::PostWithoutCaption,
                RepostAction::EditCaption,
            ]
        );
    }

    #[test]
    fn non_media_repost_buttons_only_offer_post() {
        let keyboard = format_post_button(&TestPost);
        let buttons = &keyboard.inline_keyboard[0];

        assert_eq!(buttons.len(), 1);
        assert_eq!(buttons[0].text, "Post");
        assert_eq!(button_action(&buttons[0]), RepostAction::Post);
    }

    #[test]
    fn confirmation_buttons_offer_publish_and_cancel() {
        let keyboard = format_caption_confirmation_buttons();
        let buttons = &keyboard.inline_keyboard[0];

        assert_eq!(
            buttons.iter().map(button_action).collect_vec(),
            [RepostAction::PublishCaption, RepostAction::CancelCaption]
        );
    }

    #[test]
    fn test_format_subscription_list() {
        assert_eq!(
            format_subscription_list(&[
                Subscription {
                    chat_id: 1,
                    subreddit: "foo".to_owned(),
                    limit: None,
                    time: None,
                    filter: None,
                },
                Subscription {
                    chat_id: 1,
                    subreddit: "bar".to_owned(),
                    limit: Some(1),
                    time: Some(TopPostsTimePeriod::Week),
                    filter: None,
                },
            ]),
            "foo\nbar (time=week, limit=1)"
        )
    }
}
