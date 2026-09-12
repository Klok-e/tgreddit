use tgreddit::{
    config::DEFAULT_X_TWEET_API_BASE_URL, download::download_url_to_tmp, types::MediaKind, x_tweet,
};
use url::Url;

const PHOTO_TWEET: &str = "https://x.com/TrueSlazac/status/2098362080696946984";

#[tokio::test]
#[ignore = "calls the live FxTwitter service"]
async fn fxtwitter_returns_the_linked_tweets_text_and_photo() {
    let url = Url::parse(PHOTO_TWEET).expect("fixture Tweet URL is valid");
    let tweet = x_tweet::fetch(&url, DEFAULT_X_TWEET_API_BASE_URL)
        .await
        .expect("FxTwitter returned the fixture Tweet");

    assert_eq!(tweet.id, "2098362080696946984");
    assert!(
        tweet
            .text
            .starts_with("I don't like where things are going")
    );
    assert_eq!(tweet.media.len(), 1);
    assert_eq!(tweet.media[0].kind, MediaKind::Photo);
    assert!(
        tweet.media[0]
            .url
            .starts_with("https://pbs.twimg.com/media/")
    );

    let (path, _temp_dir) = download_url_to_tmp(&tweet.media[0].url)
        .await
        .expect("the fixture photo URL downloads from pbs.twimg.com");
    assert!(
        std::fs::metadata(path)
            .expect("the downloaded fixture photo exists")
            .len()
            > 1_000
    );
}
