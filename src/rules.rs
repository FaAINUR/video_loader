//! Правила: проверка ссылок, прокси, путей и параметров новых задач.

use crate::models::*;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub const PROXY_ERROR: &str = "Адрес прокси должен начинаться с http://, https://, \
     socks4://, socks5:// или socks5h://.";

pub fn valid_url(url: &str) -> bool {
    let remainder = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"));

    match remainder {
        Some(rest) => {
            !rest.is_empty()
                && !url.chars().any(char::is_whitespace)
                && !url.contains('[')
                && !url.contains(']')
        }

        None => false,
    }
}

/// Разбирает многострочное поле. Пустые строки пропускаются.
pub fn parse_url_list(input: &str) -> Result<Vec<String>, String> {
    let mut urls = Vec::new();
    let mut invalid = Vec::new();

    for (index, line) in input.lines().enumerate() {
        let line = line.trim();

        if line.is_empty() {
            continue;
        }

        if valid_url(line) {
            urls.push(line.to_owned());
        } else {
            invalid.push((index + 1).to_string());
        }
    }

    if !invalid.is_empty() {
        return Err(format!(
            "Неверные ссылки в строках: {}. \
             Используйте адреса с http:// или https://, \
             по одному на строку.",
            invalid.join(", ")
        ));
    }

    if urls.is_empty() {
        return Err("Вставьте хотя бы одну ссылку.".to_owned());
    }

    Ok(urls)
}

/// Убирает точные повторы. Возвращает список и число пропущенных.
pub fn dedupe_urls(urls: Vec<String>) -> (Vec<String>, usize) {
    let mut seen = HashSet::new();
    let mut unique = Vec::new();
    let mut skipped = 0;

    for url in urls {
        if seen.insert(url.clone()) {
            unique.push(url);
        } else {
            skipped += 1;
        }
    }

    (unique, skipped)
}

pub fn is_youtube(url: &str) -> bool {
    let rest = url.split("://").nth(1).unwrap_or("");
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit('@').next().unwrap_or(host);
    let host = host.split(':').next().unwrap_or(host).to_lowercase();

    ["youtube.com", "youtu.be", "youtube-nocookie.com"]
        .iter()
        .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
}

pub fn valid_proxy(value: &str) -> bool {
    value.is_empty()
        || [
            "http://",
            "https://",
            "socks4://",
            "socks5://",
            "socks5h://",
        ]
        .iter()
        .any(|prefix| value.starts_with(prefix))
}

/// None — не передавать --proxy. Some("") — прямое подключение.
pub fn proxy_for(settings: &Settings, url: &str) -> Option<String> {
    let proxy = settings.proxy_url.trim();

    match settings.proxy_mode {
        ProxyMode::Never => Some(String::new()),

        ProxyMode::Always => {
            if proxy.is_empty() {
                None
            } else {
                Some(proxy.to_owned())
            }
        }

        ProxyMode::Auto => {
            if is_youtube(url) && !proxy.is_empty() {
                Some(proxy.to_owned())
            } else {
                Some(String::new())
            }
        }
    }
}

/// Проверяет сочетание настроек перед добавлением задач.
pub fn validate_request(
    settings: &Settings,
    url_count: usize,
    audio: Option<&AudioOption>,
) -> Result<(), String> {
    if !valid_proxy(settings.proxy_url.trim()) {
        return Err(PROXY_ERROR.to_owned());
    }

    let webm_video =
        settings.quality != Quality::AudioMp3 && settings.container == VideoContainer::Webm;

    if settings.embed_cover && webm_video {
        return Err("Для WebM обложка встраивается ненадёжно. Выберите \
             MKV/MP4 или сохраняйте обложку отдельным файлом."
            .to_owned());
    }

    if (url_count > 1 || settings.download_playlists) && audio.is_some() {
        return Err("Для группы ссылок и плейлистов нужна \
             автоматическая озвучка. Нажмите «Сбросить»."
            .to_owned());
    }

    if webm_video {
        if let Some(audio) = audio {
            if !audio.webm_compatible {
                return Err("Выбранная дорожка не подходит для WebM. \
                     Выберите другую, автоматическую озвучку \
                     или контейнер MKV."
                    .to_owned());
            }
        }
    }

    Ok(())
}

const NO_HOME: &str = "Не удалось определить домашнюю папку.";

pub fn resolve_output_dir_with(
    input: &str,
    home: Option<&Path>,
    cwd: &Path,
) -> Result<PathBuf, String> {
    let input = input.trim();

    if input.is_empty() {
        return Err("Укажите папку сохранения в настройках.".to_owned());
    }

    let path = if input == "~" {
        home.ok_or(NO_HOME)?.to_path_buf()
    } else if let Some(relative) = input.strip_prefix("~/") {
        home.ok_or(NO_HOME)?.join(relative)
    } else {
        PathBuf::from(input)
    };

    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(cwd.join(path))
    }
}

pub fn resolve_output_dir(input: &str) -> Result<PathBuf, String> {
    let home = dirs::home_dir();
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    resolve_output_dir_with(input, home.as_deref(), &cwd)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        Settings::default()
    }

    fn audio(webm: bool) -> AudioOption {
        AudioOption {
            id: "251".to_owned(),
            label: "test".to_owned(),
            webm_compatible: webm,
        }
    }

    #[test]
    fn valid_urls() {
        assert!(valid_url("https://youtu.be/abc"));
        assert!(valid_url("http://example.org"));
        assert!(!valid_url("youtu.be/abc"));
        assert!(!valid_url("https://"));
        assert!(!valid_url("https://a b"));
        assert!(!valid_url("[https://a.b](https://a.b)"));
        assert!(!valid_url("ftp://example.org"));
    }

    #[test]
    fn url_list_skips_blank_lines() {
        let urls = parse_url_list("\n https://a.org/1 \n\nhttps://b.org/2\n").unwrap();

        assert_eq!(urls, vec!["https://a.org/1", "https://b.org/2"]);
    }

    #[test]
    fn url_list_reports_line_numbers() {
        let error = parse_url_list("https://a.org\n\nплохо\nhttps://b.org\nещё").unwrap_err();

        assert!(error.contains("3, 5"), "{error}");
    }

    #[test]
    fn url_list_rejects_empty_input() {
        assert!(parse_url_list("").is_err());
        assert!(parse_url_list("  \n \n").is_err());
    }

    #[test]
    fn dedupe_keeps_order_and_counts() {
        let input = vec![
            "https://a.org".to_owned(),
            "https://b.org".to_owned(),
            "https://a.org".to_owned(),
        ];

        let (unique, skipped) = dedupe_urls(input);

        assert_eq!(unique, vec!["https://a.org", "https://b.org"]);
        assert_eq!(skipped, 1);
    }

    #[test]
    fn youtube_hosts() {
        assert!(is_youtube("https://youtu.be/x"));
        assert!(is_youtube("https://www.youtube.com/watch?v=x"));
        assert!(is_youtube("https://music.youtube.com/watch?v=x"));
        assert!(is_youtube("https://youtube.com:443/x"));
        assert!(is_youtube("https://user@youtube.com/x"));
        assert!(is_youtube("https://www.youtube-nocookie.com/embed/x"));
    }

    #[test]
    fn non_youtube_hosts() {
        assert!(!is_youtube("https://vkvideo.ru/video-1_2"));
        assert!(!is_youtube("https://rutube.ru/video/abc/"));
        assert!(!is_youtube("https://notyoutube.com/x"));
        assert!(!is_youtube("https://youtube.com.evil.example/x"));
        assert!(!is_youtube("https://example.org/?u=youtube.com"));
    }

    #[test]
    fn proxy_schemes() {
        assert!(valid_proxy(""));
        assert!(valid_proxy("socks5h://127.0.0.1:12334"));
        assert!(valid_proxy("http://127.0.0.1:8080"));
        assert!(!valid_proxy("127.0.0.1:8080"));
        assert!(!valid_proxy("ftp://host"));
    }

    #[test]
    fn auto_proxy_only_for_youtube() {
        let mut s = settings();
        s.proxy_mode = ProxyMode::Auto;
        s.proxy_url = "socks5h://127.0.0.1:12334".to_owned();

        assert_eq!(
            proxy_for(&s, "https://youtu.be/x").as_deref(),
            Some("socks5h://127.0.0.1:12334")
        );
        assert_eq!(
            proxy_for(&s, "https://rutube.ru/video/x").as_deref(),
            Some("")
        );
    }

    #[test]
    fn auto_proxy_without_address_is_direct() {
        let mut s = settings();
        s.proxy_mode = ProxyMode::Auto;
        s.proxy_url = "  ".to_owned();

        assert_eq!(proxy_for(&s, "https://youtu.be/x").as_deref(), Some(""));
    }

    #[test]
    fn always_and_never_proxy() {
        let mut s = settings();
        s.proxy_url = "http://127.0.0.1:1".to_owned();

        s.proxy_mode = ProxyMode::Always;
        assert_eq!(
            proxy_for(&s, "https://rutube.ru/x").as_deref(),
            Some("http://127.0.0.1:1")
        );

        s.proxy_mode = ProxyMode::Never;
        assert_eq!(proxy_for(&s, "https://youtu.be/x").as_deref(), Some(""));

        s.proxy_mode = ProxyMode::Always;
        s.proxy_url.clear();
        assert_eq!(proxy_for(&s, "https://youtu.be/x"), None);
    }

    #[test]
    fn bad_proxy_blocks_request() {
        let mut s = settings();
        s.proxy_url = "localhost:80".to_owned();

        assert!(validate_request(&s, 1, None).is_err());
    }

    #[test]
    fn webm_blocks_embedded_cover() {
        let mut s = settings();
        s.container = VideoContainer::Webm;
        s.embed_cover = true;

        assert!(validate_request(&s, 1, None).is_err());

        s.quality = Quality::AudioMp3;
        assert!(validate_request(&s, 1, None).is_ok());
    }

    #[test]
    fn audio_choice_not_allowed_for_groups() {
        let mut s = settings();
        let a = audio(true);

        assert!(validate_request(&s, 1, Some(&a)).is_ok());
        assert!(validate_request(&s, 2, Some(&a)).is_err());

        s.download_playlists = true;
        assert!(validate_request(&s, 1, Some(&a)).is_err());
    }

    #[test]
    fn webm_requires_compatible_audio() {
        let mut s = settings();
        s.container = VideoContainer::Webm;

        assert!(validate_request(&s, 1, Some(&audio(false))).is_err());
        assert!(validate_request(&s, 1, Some(&audio(true))).is_ok());
    }

    #[test]
    fn empty_output_dir_is_error() {
        assert!(resolve_output_dir_with("  ", None, Path::new("/")).is_err());
    }

    #[test]
    fn tilde_needs_home() {
        assert!(resolve_output_dir_with("~", None, Path::new("/")).is_err());
        assert!(resolve_output_dir_with("~/x", None, Path::new("/")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn paths_are_resolved() {
        let home = Path::new("/home/user");
        let cwd = Path::new("/work");

        assert_eq!(
            resolve_output_dir_with("~", Some(home), cwd).unwrap(),
            PathBuf::from("/home/user")
        );
        assert_eq!(
            resolve_output_dir_with("~/Видео", Some(home), cwd).unwrap(),
            PathBuf::from("/home/user/Видео")
        );
        assert_eq!(
            resolve_output_dir_with("/data/v", Some(home), cwd).unwrap(),
            PathBuf::from("/data/v")
        );
        assert_eq!(
            resolve_output_dir_with("downloads", Some(home), cwd).unwrap(),
            PathBuf::from("/work/downloads")
        );
    }
}

/// Cookies браузера используются только для ссылок YouTube.
pub fn browser_for(settings: &Settings, url: &str) -> Option<String> {
    (settings.use_browser_cookies && is_youtube(url)).then(|| settings.browser.clone())
}

#[cfg(test)]
mod cookies_tests {
    use super::*;

    #[test]
    fn cookies_only_for_youtube() {
        let mut s = Settings::default();
        s.use_browser_cookies = true;
        assert_eq!(
            browser_for(&s, "https://youtu.be/x"),
            Some(s.browser.clone())
        );
        for url in [
            "https://vkvideo.ru/x",
            "https://rutube.ru/x",
            "https://youtube.com.evil.example/x",
        ] {
            assert_eq!(browser_for(&s, url), None);
        }
        s.use_browser_cookies = false;
        assert_eq!(browser_for(&s, "https://youtube.com/watch?v=x"), None);
    }
}
