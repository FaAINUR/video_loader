//! Всё, что связано с yt-dlp: аргументы, разбор JSON, запуск процесса.

use crate::models::*;
use crate::platform;
use serde_json::Value;
use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};

const YT_DLP: &str = "yt-dlp";

// --------------------------------------------------
// Селектор форматов и аргументы
// --------------------------------------------------

pub fn build_format(quality: Quality, audio_id: Option<&str>, container: VideoContainer) -> String {
    if quality == Quality::AudioMp3 {
        return audio_id
            .map(str::to_owned)
            .unwrap_or_else(|| "bestaudio/best".to_owned());
    }

    let height = quality.height_filter();

    match container {
        VideoContainer::Mkv | VideoContainer::Mp4 => match audio_id {
            Some(id) => format!("bv{height}+{id}"),
            None => format!("bv*{height}+ba/b{height}"),
        },

        VideoContainer::Webm => {
            let video = format!("bv{height}[ext=webm]");

            match audio_id {
                Some(id) => format!("{video}+{id}"),

                None => format!("{video}+ba[ext=webm]/b{height}[ext=webm]"),
            }
        }
    }
}

fn output_template(playlist: bool) -> &'static str {
    if playlist {
        "%(playlist_title|Плейлист)s/\
         %(playlist_index|0)03d - %(title)s [%(id)s].%(ext)s"
    } else {
        "%(title)s [%(id)s].%(ext)s"
    }
}

fn push_connection(args: &mut Vec<String>, browser: Option<&str>, proxy: Option<&str>) {
    if let Some(browser) = browser {
        args.push("--cookies-from-browser".to_owned());
        args.push(browser.to_owned());
    }

    if let Some(proxy) = proxy {
        args.push("--proxy".to_owned());
        args.push(proxy.to_owned());
    }
}

/// Аргументы для получения списка форматов (без скачивания).
pub fn info_args(url: &str, browser: Option<&str>, proxy: Option<&str>) -> Vec<String> {
    let mut args: Vec<String> = [
        "--ignore-config",
        "--no-playlist",
        "--skip-download",
        "--dump-single-json",
        "--socket-timeout",
        "30",
        "--extractor-retries",
        "1",
    ]
    .iter()
    .map(|part| (*part).to_owned())
    .collect();

    push_connection(&mut args, browser, proxy);

    args.push("--".to_owned());
    args.push(url.to_owned());
    args
}

/// Аргументы для скачивания. URL всегда последний, после «--».
pub fn download_args(spec: &DownloadSpec) -> Vec<String> {
    let mut args: Vec<String> = [
        "--ignore-config",
        "--newline",
        "--no-colors",
        "--progress",
        "--socket-timeout",
        "30",
        "--extractor-retries",
        "1",
    ]
    .iter()
    .map(|part| (*part).to_owned())
    .collect();

    if spec.playlist {
        args.push("--yes-playlist".to_owned());
        args.push("--no-abort-on-error".to_owned());
    } else {
        args.push("--no-playlist".to_owned());
    }

    push_connection(&mut args, spec.browser.as_deref(), spec.proxy.as_deref());

    if spec.embed_cover {
        args.push("--embed-thumbnail".to_owned());
    }

    if spec.save_cover {
        args.push("--write-thumbnail".to_owned());
    }

    if spec.embed_cover || spec.save_cover {
        args.push("--convert-thumbnails".to_owned());
        args.push("jpg".to_owned());
    }

    if spec.quality == Quality::AudioMp3 {
        for part in ["-x", "--audio-format", "mp3", "--audio-quality", "0"] {
            args.push(part.to_owned());
        }
    } else {
        args.push("--merge-output-format".to_owned());
        args.push(spec.container.extension().to_owned());
        args.push("--remux-video".to_owned());
        args.push(spec.container.extension().to_owned());
    }

    args.push("-f".to_owned());
    args.push(build_format(
        spec.quality,
        spec.audio_id.as_deref(),
        spec.container,
    ));

    args.push("-P".to_owned());
    args.push(spec.output_dir.to_string_lossy().into_owned());

    args.push("-o".to_owned());
    args.push(output_template(spec.playlist).to_owned());

    args.push("--".to_owned());
    args.push(spec.url.clone());
    args
}

// --------------------------------------------------
// Аудиодорожки
// --------------------------------------------------

fn field<'a>(value: &'a Value, name: &str) -> &'a str {
    value.get(name).and_then(Value::as_str).unwrap_or("")
}

pub fn parse_audio_options(json: &[u8]) -> Result<Vec<AudioOption>, String> {
    let data: Value =
        serde_json::from_slice(json).map_err(|error| format!("Ошибка разбора JSON: {error}"))?;

    let formats = data
        .get("formats")
        .and_then(Value::as_array)
        .ok_or_else(|| "yt-dlp не вернул список форматов.".to_owned())?;

    let mut options = Vec::new();

    for format in formats {
        let id = field(format, "format_id");
        let vcodec = field(format, "vcodec");
        let acodec = field(format, "acodec");

        if id.is_empty()
            || acodec.is_empty()
            || acodec == "none"
            || (!vcodec.is_empty() && vcodec != "none")
        {
            continue;
        }

        let language = field(format, "language");

        let language_name = if language.starts_with("ru") {
            "Русский"
        } else if language.starts_with("en") {
            "Английский"
        } else if language.is_empty() {
            "Язык не указан"
        } else {
            language
        };

        let webm_compatible = acodec.starts_with("opus") || acodec.starts_with("vorbis");

        let mut parts = vec![language_name.to_owned()];

        let note = field(format, "format_note");

        if !note.is_empty() {
            parts.push(note.to_owned());
        }

        parts.push(acodec.to_owned());

        if let Some(abr) = format.get("abr").and_then(Value::as_f64) {
            parts.push(format!("{abr:.0} кбит/с"));
        }

        if webm_compatible {
            parts.push("подходит для WebM".to_owned());
        }

        parts.push(format!("ID: {id}"));

        options.push(AudioOption {
            id: id.to_owned(),
            label: parts.join(" • "),
            webm_compatible,
        });
    }

    options.reverse();
    Ok(options)
}

pub fn load_audio_options(
    url: &str,
    browser: Option<&str>,
    proxy: Option<&str>,
) -> Result<Vec<AudioOption>, String> {
    let mut command = Command::new(YT_DLP);

    command.args(info_args(url, browser, proxy));
    command.stdin(Stdio::null());
    platform::hide_console(&mut command);

    let output = command
        .output()
        .map_err(|error| format!("Не удалось запустить yt-dlp: {error}"))?;

    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr).into_owned();

        return Err(if error.trim().is_empty() {
            format!("yt-dlp завершился с кодом: {}", output.status)
        } else {
            error
        });
    }

    parse_audio_options(&output.stdout)
}

// --------------------------------------------------
// Запущенная загрузка
// --------------------------------------------------

pub struct RunningDownload {
    child: Child,
    pub lines: Receiver<String>,
    readers: Vec<JoinHandle<()>>,
    finished: bool,
}

impl RunningDownload {
    pub fn start(spec: &DownloadSpec) -> Result<Self, String> {
        std::fs::create_dir_all(&spec.output_dir)
            .map_err(|error| format!("Не удалось создать папку: {error}"))?;

        let mut command = Command::new(YT_DLP);

        command
            .args(download_args(spec))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        platform::prepare_process(&mut command);

        let mut child = command
            .spawn()
            .map_err(|error| format!("Не удалось запустить yt-dlp: {error}"))?;

        let (sender, lines) = mpsc::channel();
        let mut readers = Vec::new();

        if let Some(stdout) = child.stdout.take() {
            readers.push(spawn_reader(stdout, sender.clone()));
        }

        if let Some(stderr) = child.stderr.take() {
            readers.push(spawn_reader(stderr, sender.clone()));
        }

        drop(sender);

        Ok(Self {
            child,
            lines,
            readers,
            finished: false,
        })
    }

    /// Проверяет завершение без блокировки. Some — процесс закончился.
    pub fn poll(&mut self) -> Option<Result<(), String>> {
        match self.child.try_wait() {
            Ok(Some(status)) => {
                if !self.readers.iter().all(|reader| reader.is_finished()) {
                    return None;
                }

                self.finished = true;

                Some(if status.success() {
                    Ok(())
                } else {
                    Err(format!(
                        "yt-dlp завершился с ошибкой: {status}. \
                         Подробности в журнале."
                    ))
                })
            }

            Ok(None) => None,

            Err(error) => Some(Err(format!("Ошибка проверки процесса: {error}"))),
        }
    }

    pub fn stop(&mut self) -> Result<(), String> {
        if self.finished {
            return Ok(());
        }

        match self.child.try_wait() {
            Ok(Some(_)) => {
                self.finished = true;
                return Ok(());
            }

            Ok(None) => {}

            Err(error) => return Err(error.to_string()),
        }

        if let Err(error) = platform::kill_process_tree(self.child.id()) {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                self.finished = true;
                return Ok(());
            }

            return Err(error);
        }

        self.child
            .wait()
            .map_err(|error| format!("Ошибка ожидания остановки: {error}"))?;

        self.finished = true;
        Ok(())
    }
}

impl Drop for RunningDownload {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.stop();
        }
    }
}

fn spawn_reader<R>(reader: R, sender: Sender<String>) -> JoinHandle<()>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        for line in BufReader::new(reader).lines() {
            match line {
                Ok(line) => {
                    if sender.send(line).is_err() {
                        break;
                    }
                }

                Err(error) => {
                    let _ = sender.send(format!("Ошибка чтения журнала: {error}"));
                    break;
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn value_after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
        let index = args.iter().position(|arg| arg == flag)?;
        args.get(index + 1).map(String::as_str)
    }

    fn has(args: &[String], flag: &str) -> bool {
        args.iter().any(|arg| arg == flag)
    }

    // ---------- селектор форматов ----------

    #[test]
    fn best_quality_without_audio() {
        assert_eq!(
            build_format(Quality::Best, None, VideoContainer::Mkv),
            "bv*+ba/b"
        );
    }

    #[test]
    fn limit_applies_to_both_branches() {
        assert_eq!(
            build_format(Quality::H720, None, VideoContainer::Mp4),
            "bv*[height<=720]+ba/b[height<=720]"
        );
    }

    #[test]
    fn explicit_audio_has_no_fallback() {
        assert_eq!(
            build_format(Quality::H720, Some("251"), VideoContainer::Mkv),
            "bv[height<=720]+251"
        );
    }

    #[test]
    fn webm_filters_streams() {
        assert_eq!(
            build_format(Quality::H720, None, VideoContainer::Webm),
            "bv[height<=720][ext=webm]+ba[ext=webm]/b[height<=720][ext=webm]"
        );
        assert_eq!(
            build_format(Quality::Best, Some("251"), VideoContainer::Webm),
            "bv[ext=webm]+251"
        );
    }

    #[test]
    fn mp3_format() {
        assert_eq!(
            build_format(Quality::AudioMp3, None, VideoContainer::Mkv),
            "bestaudio/best"
        );
        assert_eq!(
            build_format(Quality::AudioMp3, Some("140"), VideoContainer::Mkv),
            "140"
        );
    }

    // ---------- аргументы загрузки ----------

    #[test]
    fn url_is_last_after_separator() {
        let args = download_args(&sample_spec());
        let n = args.len();

        assert_eq!(args[n - 2], "--");
        assert_eq!(args[n - 1], "https://example.org/video");
    }

    #[test]
    fn default_args() {
        let args = download_args(&sample_spec());

        assert!(has(&args, "--no-playlist"));
        assert!(!has(&args, "--yes-playlist"));
        assert!(!has(&args, "--proxy"));
        assert!(!has(&args, "--cookies-from-browser"));
        assert!(!has(&args, "--embed-thumbnail"));
        assert!(!has(&args, "--write-thumbnail"));
        assert!(!has(&args, "--convert-thumbnails"));
        assert_eq!(value_after(&args, "--merge-output-format"), Some("mkv"));
        assert_eq!(value_after(&args, "--remux-video"), Some("mkv"));
        assert_eq!(value_after(&args, "-f"), Some("bv*+ba/b"));
        assert_eq!(value_after(&args, "-P"), Some("/tmp/videos"));
    }

    #[test]
    fn direct_connection_passes_empty_proxy() {
        let mut spec = sample_spec();
        spec.proxy = Some(String::new());

        let args = download_args(&spec);

        assert_eq!(value_after(&args, "--proxy"), Some(""));
    }

    #[test]
    fn proxy_and_cookies_are_passed() {
        let mut spec = sample_spec();
        spec.proxy = Some("socks5h://127.0.0.1:12334".to_owned());
        spec.browser = Some("firefox".to_owned());

        let args = download_args(&spec);

        assert_eq!(
            value_after(&args, "--proxy"),
            Some("socks5h://127.0.0.1:12334")
        );
        assert_eq!(
            value_after(&args, "--cookies-from-browser"),
            Some("firefox")
        );
    }

    #[test]
    fn cover_flags() {
        let mut spec = sample_spec();
        spec.embed_cover = true;

        let args = download_args(&spec);
        assert!(has(&args, "--embed-thumbnail"));
        assert!(!has(&args, "--write-thumbnail"));
        assert_eq!(value_after(&args, "--convert-thumbnails"), Some("jpg"));

        spec.embed_cover = false;
        spec.save_cover = true;

        let args = download_args(&spec);
        assert!(!has(&args, "--embed-thumbnail"));
        assert!(has(&args, "--write-thumbnail"));
        assert_eq!(value_after(&args, "--convert-thumbnails"), Some("jpg"));
    }

    #[test]
    fn playlist_args() {
        let mut spec = sample_spec();
        spec.playlist = true;

        let args = download_args(&spec);

        assert!(has(&args, "--yes-playlist"));
        assert!(has(&args, "--no-abort-on-error"));
        assert!(!has(&args, "--no-playlist"));
        assert!(value_after(&args, "-o").unwrap().contains("playlist_index"));
    }

    #[test]
    fn single_video_template_has_no_folder() {
        let args = download_args(&sample_spec());

        assert!(!value_after(&args, "-o").unwrap().contains('/'));
    }

    #[test]
    fn mp3_args() {
        let mut spec = sample_spec();
        spec.quality = Quality::AudioMp3;

        let args = download_args(&spec);

        assert!(has(&args, "-x"));
        assert_eq!(value_after(&args, "--audio-format"), Some("mp3"));
        assert!(!has(&args, "--merge-output-format"));
        assert!(!has(&args, "--remux-video"));
    }

    #[test]
    fn container_extension_is_passed() {
        let mut spec = sample_spec();
        spec.container = VideoContainer::Mp4;

        let args = download_args(&spec);

        assert_eq!(value_after(&args, "--merge-output-format"), Some("mp4"));
        assert_eq!(value_after(&args, "--remux-video"), Some("mp4"));
    }

    #[test]
    fn info_args_end_with_url() {
        let args = info_args("https://example.org/v", Some("firefox"), Some(""));
        let n = args.len();

        assert!(has(&args, "--dump-single-json"));
        assert!(has(&args, "--skip-download"));
        assert_eq!(value_after(&args, "--proxy"), Some(""));
        assert_eq!(args[n - 2], "--");
        assert_eq!(args[n - 1], "https://example.org/v");
    }

    // ---------- разбор аудиоформатов ----------

    #[test]
    fn audio_only_formats_are_selected() {
        let data = json!({
            "formats": [
                {"format_id": "137", "vcodec": "avc1", "acodec": "none"},
                {
                    "format_id": "140", "vcodec": "none",
                    "acodec": "mp4a.40.2", "language": "en",
                    "format_note": "medium", "abr": 129.5
                },
                {
                    "format_id": "251", "vcodec": "none",
                    "acodec": "opus", "language": "ru",
                    "format_note": "medium", "abr": 130.0
                },
                {"format_id": "18", "vcodec": "avc1", "acodec": "mp4a.40.2"},
                {"vcodec": "none", "acodec": "opus"}
            ]
        });

        let bytes = serde_json::to_vec(&data).unwrap();
        let options = parse_audio_options(&bytes).unwrap();

        let ids: Vec<&str> = options.iter().map(|option| option.id.as_str()).collect();

        // Порядок обратный порядку yt-dlp.
        assert_eq!(ids, vec!["251", "140"]);

        assert!(options[0].webm_compatible);
        assert!(options[0].label.contains("Русский"));
        assert!(options[0].label.contains("ID: 251"));

        assert!(!options[1].webm_compatible);
        assert!(options[1].label.contains("Английский"));
    }

    #[test]
    fn no_audio_formats_gives_empty_list() {
        let bytes = serde_json::to_vec(&json!({"formats": []})).unwrap();

        assert!(parse_audio_options(&bytes).unwrap().is_empty());
    }

    #[test]
    fn broken_json_is_error() {
        assert!(parse_audio_options(b"not json").is_err());
    }

    #[test]
    fn missing_formats_is_error() {
        let bytes = serde_json::to_vec(&json!({"title": "x"})).unwrap();

        assert!(parse_audio_options(&bytes).is_err());
    }

    #[test]
    fn language_is_shown_when_unknown() {
        let data = json!({"formats": [
            {"format_id": "a1", "vcodec": "none", "acodec": "opus"},
            {"format_id": "a2", "vcodec": "none", "acodec": "opus",
             "language": "de"}
        ]});

        let bytes = serde_json::to_vec(&data).unwrap();
        let options = parse_audio_options(&bytes).unwrap();

        assert!(options[0].label.contains("de"));
        assert!(options[1].label.contains("Язык не указан"));
    }
}
