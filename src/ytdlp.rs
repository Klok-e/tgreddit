use anyhow::{Context, Result};
use duct::cmd;
use lazy_static::lazy_static;
use log::{info, warn};
use std::{
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    process::Output,
};

use crate::types::*;

use regex::Regex;
use tempfile::TempDir;
use url::Url;

const YOUTUBE_FORMATS: &[&str] = &[
    "bv[height<=1080]+ba/b[height<=1080]",
    "bv[height<=720]+ba/b[height<=720]",
    "bv[height<=480]+ba/b[height<=480]",
    "18",
];
const GENERIC_FORMATS: &[&str] = &["bv[height<=1080]+ba/best"];

fn format_selectors(url: &str) -> &'static [&'static str] {
    let is_youtube = Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .is_some_and(|host| {
            let host = host.to_ascii_lowercase();
            host == "youtube.com" || host.ends_with(".youtube.com") || host == "youtu.be"
        });

    if is_youtube {
        YOUTUBE_FORMATS
    } else {
        GENERIC_FORMATS
    }
}

fn fallback_reason(output: &str) -> Option<&str> {
    output.lines().rev().find(|line| line.starts_with("ERROR:"))
}

fn make_ytdlp_args(
    output: &Path,
    url: &str,
    format_selector: &str,
    is_direct_media_submission: bool,
) -> Vec<OsString> {
    let mut args = vec![
        "--impersonate".into(),
        "Firefox-135".into(),
        "--paths".into(),
        output.into(),
        "--output".into(),
        // To get telegram show correct aspect ratio for video, we need the dimensions and simplest
        // way to make that happens is have yt-dlp write them in the filename.
        "%(title).200B_[%(id)s]_%(width)sx%(height)s.%(ext)s".into(),
        "-f".into(),
        format_selector.into(),
        "-S".into(),
        "res,ext:mp4:m4a".into(),
        "--recode".into(),
        "mp4".into(),
    ];
    if is_direct_media_submission {
        args.extend(["--playlist-items".into(), "1".into()]);
    } else {
        args.push("--no-playlist".into());
    }
    args.push(url.into());
    args
}

trait YtdlpRunner {
    fn run(&mut self, args: &[OsString]) -> io::Result<Output>;
}

struct CommandYtdlpRunner;

impl YtdlpRunner for CommandYtdlpRunner {
    fn run(&mut self, args: &[OsString]) -> io::Result<Output> {
        cmd("yt-dlp", args.iter().cloned())
            .stderr_to_stdout()
            .stdout_capture()
            .unchecked()
            .run()
    }
}

/// Downloads media attached to a Reddit post with yt-dlp and returns its video.
pub fn download(url: &str) -> Result<Video> {
    download_with_runner(url, false, &mut CommandYtdlpRunner)
}

/// Downloads a Direct Media Submission and returns its first extractor item.
pub fn download_direct(url: &str) -> Result<Video> {
    download_with_runner(url, true, &mut CommandYtdlpRunner)
}

fn download_with_runner(
    url: &str,
    is_direct_media_submission: bool,
    runner: &mut impl YtdlpRunner,
) -> Result<Video> {
    let tmp_dir = TempDir::with_prefix("tgreddit")?;
    let selectors = format_selectors(url);

    for (attempt, selector) in selectors.iter().enumerate() {
        let attempt_path = tmp_dir.path().join(format!("attempt-{attempt}"));
        fs::create_dir(&attempt_path).context("Could not create yt-dlp attempt directory")?;
        let ytdlp_args = make_ytdlp_args(&attempt_path, url, selector, is_direct_media_submission);

        info!("running yt-dlp with arguments {ytdlp_args:?}");
        let output = runner.run(&ytdlp_args).context("Failed to run yt-dlp")?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            info!("{line}");
        }

        if output.status.success() {
            return video_from_download(url, tmp_dir, attempt_path);
        }

        let error_line = fallback_reason(&stdout);
        let reason = error_line
            .map(|line| format!(": {line}"))
            .unwrap_or_default();
        if let Some(next_selector) = selectors.get(attempt + 1) {
            warn!(
                "yt-dlp format {selector:?} failed with {}{reason}; falling back to {next_selector:?}",
                output.status
            );
        } else {
            warn!(
                "yt-dlp format {selector:?} failed with {}{reason}; no fallback remains",
                output.status
            );
            anyhow::bail!(
                "yt-dlp format {selector:?} failed with {}: {}",
                output.status,
                stdout.trim()
            );
        }
    }

    anyhow::bail!("yt-dlp exhausted all format selectors")
}

fn video_from_download(url: &str, tmp_dir: TempDir, successful_path: PathBuf) -> Result<Video> {
    // yt-dlp is expected to write a single file, which is the video, to its attempt directory.
    let video_path = get_video_path(&successful_path)?;

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

/// Pick the path of the recoded yt-dlp video output file in `dir`.
///
/// Metadata sidecars and transient downloader files are deliberately excluded.
/// When yt-dlp writes more than one MP4 file, this selects the file with the
/// oldest modification timestamp; if timestamps are equal or unavailable, paths
/// are used as a deterministic tiebreaker.
fn get_video_path(dir: &Path) -> Result<PathBuf> {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .context("Could not read files in temp dir")?
        .map(|entry| entry.map(|e| e.path()))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|path| {
            path.is_file() && path.extension().is_some_and(|extension| extension == "mp4")
        })
        .collect();

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
        .context("No recoded MP4 video file in temp dir")
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
    use super::{
        GENERIC_FORMATS, YOUTUBE_FORMATS, YtdlpRunner, download_with_runner, fallback_reason,
        format_selectors, get_video_path, make_ytdlp_args, parse_metadata_from_path,
    };
    use std::collections::VecDeque;
    use std::ffi::OsString;
    use std::fs::{self, File};
    use std::io;
    use std::os::unix::process::ExitStatusExt;
    use std::path::Path;
    use std::process::{ExitStatus, Output};
    use std::time::{Duration, SystemTime};
    use tempfile::TempDir;

    struct FakeRunner {
        statuses: VecDeque<i32>,
        selectors: Vec<String>,
        output_paths: Vec<std::path::PathBuf>,
    }

    impl FakeRunner {
        fn with_statuses(statuses: impl IntoIterator<Item = i32>) -> Self {
            Self {
                statuses: statuses.into_iter().collect(),
                selectors: Vec::new(),
                output_paths: Vec::new(),
            }
        }
    }

    impl YtdlpRunner for FakeRunner {
        fn run(&mut self, args: &[OsString]) -> io::Result<Output> {
            let format_index = args.iter().position(|arg| arg == "-f").unwrap();
            self.selectors
                .push(args[format_index + 1].to_string_lossy().into_owned());
            let status = self.statuses.pop_front().unwrap();
            let paths_index = args.iter().position(|arg| arg == "--paths").unwrap();
            let output_path = std::path::PathBuf::from(&args[paths_index + 1]);
            self.output_paths.push(output_path.clone());
            if status == 0 {
                fs::write(output_path.join("video_[id]_1280x720.mp4"), [])?;
            } else {
                fs::write(output_path.join("partial.part"), [])?;
            }
            Ok(Output {
                status: ExitStatus::from_raw(status << 8),
                stdout: if status == 0 {
                    b"download complete".to_vec()
                } else {
                    b"ERROR: requested format failed".to_vec()
                },
                stderr: Vec::new(),
            })
        }
    }

    #[test]
    fn youtube_urls_use_the_quality_fallback_ladder() {
        assert_eq!(
            format_selectors("https://www.youtube.com/watch?v=video"),
            [
                "bv[height<=1080]+ba/b[height<=1080]",
                "bv[height<=720]+ba/b[height<=720]",
                "bv[height<=480]+ba/b[height<=480]",
                "18",
            ]
        );
    }

    #[test]
    fn only_supported_youtube_hostnames_use_the_fallback_ladder() {
        for url in [
            "https://youtube.com/watch?v=video",
            "https://m.youtube.com/watch?v=video",
            "https://YOUTUBE.COM/watch?v=video",
            "https://youtu.be/video",
        ] {
            assert_eq!(format_selectors(url), YOUTUBE_FORMATS, "{url}");
        }

        for url in [
            "https://youtube.com.evil.example/watch?v=video",
            "https://notyoutube.com/watch?v=video",
            "https://sub.youtu.be/video",
            "not a URL",
        ] {
            assert_eq!(format_selectors(url), GENERIC_FORMATS, "{url}");
        }
    }

    #[test]
    fn youtube_download_falls_back_after_a_failed_attempt() {
        let mut runner = FakeRunner::with_statuses([1, 0]);

        let video = download_with_runner("https://youtu.be/video", false, &mut runner).unwrap();

        assert_eq!(video.title, "video");
        assert_eq!(
            runner.selectors,
            [
                "bv[height<=1080]+ba/b[height<=1080]",
                "bv[height<=720]+ba/b[height<=720]",
            ]
        );
    }

    struct LaunchFailingRunner {
        calls: usize,
    }

    impl YtdlpRunner for LaunchFailingRunner {
        fn run(&mut self, _args: &[OsString]) -> io::Result<Output> {
            self.calls += 1;
            Err(io::Error::new(io::ErrorKind::NotFound, "missing yt-dlp"))
        }
    }

    #[test]
    fn yt_dlp_launch_failure_stops_without_trying_another_format() {
        let mut runner = LaunchFailingRunner { calls: 0 };

        let error = download_with_runner("https://youtube.com/watch?v=video", false, &mut runner)
            .unwrap_err();

        assert_eq!(runner.calls, 1);
        assert!(error.to_string().contains("Failed to run yt-dlp"));
    }

    #[test]
    fn fallback_attempts_use_isolated_output_directories() {
        let mut runner = FakeRunner::with_statuses([1, 0]);

        let video =
            download_with_runner("https://youtube.com/watch?v=video", false, &mut runner).unwrap();

        assert_ne!(runner.output_paths[0], runner.output_paths[1]);
        assert_eq!(video.path.parent(), Some(runner.output_paths[1].as_path()));
    }

    #[test]
    fn non_youtube_download_uses_the_existing_selector_once() {
        let mut runner = FakeRunner::with_statuses([1, 0]);

        let error =
            download_with_runner("https://v.redd.it/video", false, &mut runner).unwrap_err();

        assert_eq!(runner.selectors, ["bv[height<=1080]+ba/best"]);
        assert!(error.to_string().contains("requested format failed"));
    }

    #[test]
    fn youtube_download_reports_the_final_failure_after_exhausting_the_ladder() {
        let mut runner = FakeRunner::with_statuses([1, 2, 3, 4]);

        let error = download_with_runner("https://youtu.be/video", false, &mut runner).unwrap_err();

        assert_eq!(runner.selectors, YOUTUBE_FORMATS);
        assert!(error.to_string().contains("exit status: 4"));
        assert!(error.to_string().contains("ERROR: requested format failed"));
    }

    #[test]
    fn fallback_diagnostic_uses_the_final_error_line() {
        let output = "ERROR: first cause\nprogress detail\nERROR: final cause\ncleanup detail";

        assert_eq!(fallback_reason(output), Some("ERROR: final cause"));
    }

    #[test]
    fn test_ytdlp_args_use_the_supported_firefox_target_and_first_playlist_item() {
        let args = make_ytdlp_args(
            Path::new("/tmp/output"),
            "https://example.com/video",
            GENERIC_FORMATS[0],
            true,
        );
        let args = args
            .iter()
            .map(|arg| arg.to_string_lossy())
            .collect::<Vec<_>>();

        assert_eq!(&args[..2], ["--impersonate", "Firefox-135"]);
        let playlist_items = args
            .iter()
            .position(|arg| arg == "--playlist-items")
            .expect("yt-dlp arguments should limit a direct submission to its first item");
        assert_eq!(args[playlist_items + 1], "1");
        assert!(!args.contains(&"--no-playlist".into()));
        assert_eq!(args.last().unwrap(), "https://example.com/video");
    }

    #[test]
    fn non_direct_media_keeps_the_existing_single_item_selection() {
        let args = make_ytdlp_args(
            Path::new("/tmp/output"),
            "https://example.com/video",
            GENERIC_FORMATS[0],
            false,
        );
        let args = args
            .iter()
            .map(|arg| arg.to_string_lossy())
            .collect::<Vec<_>>();

        assert!(args.contains(&"--no-playlist".into()));
        assert!(!args.contains(&"--playlist-items".into()));
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
    fn test_get_video_path_ignores_metadata_and_downloader_artifacts() {
        let dir = TempDir::new().expect("create tempdir");
        let video = dir.path().join("video_[id]_1280x720.mp4");
        let sidecar = dir.path().join("video_[id]_1280x720.info.json");
        let partial = dir.path().join("video_[id]_1280x720.mp4.part");
        write_empty_file(&video);
        write_empty_file(&sidecar);
        write_empty_file(&partial);

        assert_eq!(get_video_path(dir.path()).unwrap(), video);
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
