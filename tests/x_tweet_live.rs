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
    assert_eq!(tweet.media[0].width, 1516);
    assert_eq!(tweet.media[0].height, 1038);
    assert!(
        tweet.media[0]
            .url
            .starts_with("https://pbs.twimg.com/media/")
    );

    let (path, _temp_dir) = download_url_to_tmp(&tweet.media[0].url)
        .await
        .expect("the fixture photo URL downloads from pbs.twimg.com");
    let bytes = std::fs::read(&path).expect("the downloaded fixture photo can be read");
    assert!(bytes.starts_with(&[0xff, 0xd8, 0xff]));
    assert_eq!(
        jpeg_dimensions(&bytes),
        Some((tweet.media[0].width, tweet.media[0].height))
    );
}

fn jpeg_dimensions(bytes: &[u8]) -> Option<(u16, u16)> {
    if bytes.get(0..2)? != [0xff, 0xd8] {
        return None;
    }

    let mut offset = 2;
    loop {
        while bytes.get(offset) == Some(&0xff) {
            offset += 1;
        }
        let marker = *bytes.get(offset)?;
        offset += 1;

        if marker == 0xd9 || marker == 0xda {
            return None;
        }
        if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
            continue;
        }

        let segment_length = u16::from_be_bytes(bytes.get(offset..offset + 2)?.try_into().ok()?);
        if segment_length < 2 {
            return None;
        }
        if matches!(
            marker,
            0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf
        ) {
            let height = u16::from_be_bytes(bytes.get(offset + 3..offset + 5)?.try_into().ok()?);
            let width = u16::from_be_bytes(bytes.get(offset + 5..offset + 7)?.try_into().ok()?);
            return Some((width, height));
        }
        offset = offset.checked_add(segment_length.into())?;
    }
}
