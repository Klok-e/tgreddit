use anyhow::{Context, Result};
use duct::cmd;
use lazy_static::lazy_static;
use log::info;
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

use crate::types::*;

use regex::Regex;
use tempfile::TempDir;

fn make_ytdlp_args(output: &Path, url: &str) -> Vec<OsString> {
    vec![
        "--impersonate".into(),
        "Firefox-135".into(),
        "--paths".into(),
        output.into(),
        "--output".into(),
        // To get telegram show correct aspect ratio for video, we need the dimensions and simplest
        // way to make that happens is have yt-dlp write them in the filename.
        "%(title).200B_[%(id)s]_%(width)sx%(height)s.%(ext)s".into(),
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
    let tmp_dir = TempDir::with_prefix("tgreddit")?;
    let tmp_path = tmp_dir.path();
    let ytdlp_args = make_ytdlp_args(tmp_path, url);

    info!("running yt-dlp with arguments {ytdlp_args:?}");
    let output = cmd("yt-dlp", ytdlp_args)
        .stderr_to_stdout()
        .stdout_capture()
        .unchecked()
        .run()
        .context("Failed to run yt-dlp")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        info!("{line}");
    }

    if !output.status.success() {
        anyhow::bail!("yt-dlp failed with {}: {}", output.status, stdout.trim());
    }

    // yt-dlp is expected to write a single file, which is the video, to tmp_path
    let video_path = get_video_path(tmp_path)?;

    let (title, id, width, height) =
        parse_metadata_from_path(&video_path).context("Video filename should have dimensions")?;

    let video = Video {
        path: video_path,
        url: url.to_owned(),
        title,
        id,
        width,
        height,
        // return temp dir with the video so that when Video goes out of scope tempdir is deleted
        // but not at the end of this scope
        _video_tempdir: tmp_dir,
    };

    Ok(video)
}

/// Pick the path of the yt-dlp output file in `dir`.
///
/// When yt-dlp writes more than one file, this selects the file with the
/// oldest modification timestamp; if timestamps are equal or unavailable,
/// paths are used as a deterministic tiebreaker.
fn get_video_path(dir: &Path) -> Result<PathBuf> {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .context("Could not read files in temp dir")?
        .map(|entry| entry.map(|e| e.path()))
        .collect::<Result<Vec<_>, _>>()?;

    // Sort by (modified time, path) so the oldest modified file wins and ties
    // resolve to a deterministic ordering independent of the filesystem.
    entries.sort_by(|a, b| {
        let ma = fs::metadata(a).and_then(|m| m.modified()).ok();
        let mb = fs::metadata(b).and_then(|m| m.modified()).ok();
        ma.cmp(&mb).then_with(|| a.cmp(b))
    });

    entries
        .into_iter()
        .next()
        .context("No video file in temp dir")
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
    use super::{get_video_path, make_ytdlp_args, parse_metadata_from_path};
    use std::fs::File;
    use std::path::Path;
    use std::time::{Duration, SystemTime};
    use tempfile::TempDir;

    #[test]
    fn test_ytdlp_args_use_the_supported_firefox_impersonation_target() {
        let args = make_ytdlp_args(Path::new("/tmp/output"), "https://example.com/video");
        let args = args
            .iter()
            .map(|arg| arg.to_string_lossy())
            .collect::<Vec<_>>();

        assert_eq!(&args[..2], ["--impersonate", "Firefox-135"]);
        assert_eq!(args.last().unwrap(), "https://example.com/video");
    }

    fn write_empty_file(path: &Path) {
        File::create(path).expect("create empty test file");
    }

    fn set_mtime(path: &Path, mtime: SystemTime) {
        let file = File::options()
            .write(true)
            .open(path)
            .expect("open for mtime set");
        file.set_modified(mtime).expect("set modified time");
    }

    #[test]
    fn test_get_video_path_returns_only_file() {
        let dir = TempDir::new().expect("create tempdir");
        let only = dir.path().join("only.mp4");
        write_empty_file(&only);
        assert_eq!(get_video_path(dir.path()).unwrap(), only);
    }

    #[test]
    fn test_get_video_path_picks_oldest_modified_file() {
        let dir = TempDir::new().expect("create tempdir");
        let newer = dir.path().join("newer.mp4");
        let oldest = dir.path().join("oldest.mp4");
        let middle = dir.path().join("middle.mp4");
        write_empty_file(&newer);
        write_empty_file(&middle);
        write_empty_file(&oldest);
        // Use distinct mtimes so the test cannot rely on creation order.
        let base = SystemTime::now();
        set_mtime(&newer, base + Duration::from_secs(30));
        set_mtime(&middle, base + Duration::from_secs(15));
        set_mtime(&oldest, base);
        assert_eq!(get_video_path(dir.path()).unwrap(), oldest);
    }

    #[test]
    fn test_get_video_path_falls_back_to_path_order_when_timestamps_tie() {
        let dir = TempDir::new().expect("create tempdir");
        let alpha = dir.path().join("alpha.mp4");
        let bravo = dir.path().join("bravo.mp4");
        let charlie = dir.path().join("charlie.mp4");
        write_empty_file(&alpha);
        write_empty_file(&bravo);
        write_empty_file(&charlie);
        // All files share the same mtime.
        let mtime = SystemTime::now();
        set_mtime(&alpha, mtime);
        set_mtime(&bravo, mtime);
        set_mtime(&charlie, mtime);
        // With equal mtimes, deterministic path ordering picks the
        // alphabetically first path.
        assert_eq!(get_video_path(dir.path()).unwrap(), alpha);
    }

    #[test]
    fn test_get_video_path_errors_on_empty_directory() {
        let dir = TempDir::new().expect("create tempdir");
        let result = get_video_path(dir.path());
        assert!(result.is_err());
    }

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
