use anyhow::Result;
use duct::cmd;
use lazy_static::lazy_static;
use log::{error, info};
use std::{
    ffi::OsString,
    fs,
    io::{BufRead, BufReader},
    path::Path,
};

use crate::types::*;

use regex::Regex;
use tempdir::TempDir;

fn make_ytdlp_args(output: &Path, url: &str) -> Vec<OsString> {
    vec![
        "--paths".into(),
        output.into(),
        "--output".into(),
        // To get telegram show correct aspect ratio for video, we need the dimensions and simplest
        // way to make that happens is have yt-dlp write them in the filename.
        "%(title)s_[%(id)s]_%(width)sx%(height)s.%(ext)s".into(),
        "-f".into(),
        "bv[height<=1080]+ba/best".into(),
        "-S".into(),
        "res,ext:mp4:m4a".into(),
        "--recode".into(),
        "mp4".into(),
        "--no-playlist".into(),
        url.into(),
    ]
}

/// Downloads given url with yt-dlp and returns path to video
pub fn download(url: &str) -> Result<Video> {
    let tmp_dir = TempDir::new("tgreddit")?;
    // Convert to path to avoid tmp dir from getting deleted when it goes out of scope
    let tmp_path = tmp_dir.into_path();
    let ytdlp_args = make_ytdlp_args(&tmp_path, url);

    info!("running yt-dlp with arguments {:?}", ytdlp_args);
    let duct_exp = cmd("yt-dlp", ytdlp_args).stderr_to_stdout();
    let reader = match duct_exp.reader() {
        Ok(child) => child,
        Err(err) => {
            error!("failed to run yt-dlp:\n{}", err);
            return Err(anyhow::anyhow!(err));
        }
    };

    let lines = BufReader::new(reader).lines();
    for line_result in lines {
        match line_result {
            Ok(line) => info!("{line}"),
            Err(_) => panic!("failed to read line"),
        }
    }

    // yt-dlp is expected to write a single file, which is the video, to tmp_path
    let video_path = fs::read_dir(tmp_path)
        .expect("could not read files in temp dir")
        .map(|de| de.unwrap().path())
        .next()
        .expect("video file in temp dir");

    let metadata =
        parse_metadata_from_path(&video_path).expect("video filename should have dimensions");

    let video = Video {
        path: video_path,
        url: url.to_owned(),
        title: metadata.0,
        id: metadata.1,
        width: metadata.2,
        height: metadata.3,
    };

    Ok(video)
}

fn parse_metadata_from_path(path: &Path) -> Option<(String, String, u16, u16)> {
    lazy_static! {
        static ref RE: Regex =
            Regex::new(r"(?P<title>.*)_\[(?P<id>.*)\]_(?P<width>\d+)x(?P<height>\d+)\.").unwrap();
    }

    let filename_str = path
        .file_name()
        .expect("file should have a stem")
        .to_string_lossy();

    let caps = RE.captures(&filename_str)?;

    let id = caps.name("id")?.as_str().to_string();
    let title = caps.name("title")?.as_str().to_string();
    let width = caps.name("width")?.as_str().parse::<u16>().ok()?;
    let height = caps.name("height")?.as_str().parse::<u16>().ok()?;

    Some((title, id, width, height))
}

#[cfg(test)]
mod tests {
    use super::parse_metadata_from_path;
    use std::path::Path;

    #[test]
    fn test_parse_metadata_from_path() {
        assert_eq!(
            parse_metadata_from_path(Path::new("/foo/bar/video_[dummyid]_1920x1080.mp4")),
            Some(("video".into(), "dummyid".into(), 1920, 1080))
        );

        // This test should fail now because the filename format is incorrect
        assert_eq!(
            parse_metadata_from_path(Path::new("/foo/bar/someothervideo_asdfax1080.mp4")),
            None,
        );

        // Testing a case where title includes underscores
        assert_eq!(
            parse_metadata_from_path(Path::new("/foo/bar/cool_video_[dummyid]_1280x720.mp4")),
            Some(("cool_video".into(), "dummyid".into(), 1280, 720))
        );

        // Testing a case where title includes special characters
        assert_eq!(
            parse_metadata_from_path(Path::new("/foo/bar/awesome#video!_[dummyid]_640x480.mp4")),
            Some(("awesome#video!".into(), "dummyid".into(), 640, 480))
        );

        // Testing a case where dimensions are not in the standard format
        assert_eq!(
            parse_metadata_from_path(Path::new("/foo/bar/video_1920_1080.mp4")),
            None,
        );

        // Testing a case where there is no title
        assert_eq!(
            parse_metadata_from_path(Path::new("/foo/bar/_[dummyid]_1920x1080.mp4")),
            Some(("".into(), "dummyid".into(), 1920, 1080))
        );

        // Testing a case where ID is an empty string
        assert_eq!(
            parse_metadata_from_path(Path::new("/foo/bar/video_[]_1920x1080.mp4")),
            Some(("video".into(), "".into(), 1920, 1080))
        );
    }
}
