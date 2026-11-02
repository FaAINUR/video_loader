//! Общие типы данных. Не зависит от интерфейса и внешних процессов.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const BROWSERS: [&str; 5] = ["firefox", "chrome", "chromium", "edge", "brave"];

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Quality {
    Best,
    H1080,
    H720,
    H480,
    AudioMp3,
}

impl Quality {
    pub fn label(self) -> &'static str {
        match self {
            Self::Best => "Максимальное качество",
            Self::H1080 => "1080p или ниже",
            Self::H720 => "720p или ниже",
            Self::H480 => "480p или ниже",
            Self::AudioMp3 => "Только MP3",
        }
    }

    pub fn height_filter(self) -> &'static str {
        match self {
            Self::H1080 => "[height<=1080]",
            Self::H720 => "[height<=720]",
            Self::H480 => "[height<=480]",
            _ => "",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum VideoContainer {
    Mkv,
    Mp4,
    Webm,
}

impl VideoContainer {
    pub fn label(self) -> &'static str {
        match self {
            Self::Mkv => "MKV",
            Self::Mp4 => "MP4",
            Self::Webm => "WebM",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Mkv => "mkv",
            Self::Mp4 => "mp4",
            Self::Webm => "webm",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum ProxyMode {
    Auto,
    Always,
    Never,
}

/// Настройки, которые сохраняются между запусками.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub output_dir: String,
    pub quality: Quality,
    pub container: VideoContainer,
    pub use_browser_cookies: bool,
    pub browser: String,
    pub download_playlists: bool,
    pub proxy_mode: ProxyMode,
    pub proxy_url: String,
    pub embed_cover: bool,
    pub save_cover: bool,
}

impl Default for Settings {
    fn default() -> Self {
        let directory = dirs::video_dir()
            .unwrap_or_else(|| dirs::download_dir().unwrap_or_else(|| PathBuf::from("downloads")));

        Self {
            output_dir: directory.to_string_lossy().into_owned(),
            quality: Quality::Best,
            container: VideoContainer::Mkv,
            use_browser_cookies: true,
            browser: "firefox".to_owned(),
            download_playlists: false,
            proxy_mode: ProxyMode::Auto,
            proxy_url: String::new(),
            embed_cover: false,
            save_cover: false,
        }
    }
}

impl Settings {
    /// Исправляет значения, которые не должны попадать в работу.
    pub fn sanitize(&mut self) {
        if self.output_dir.trim().is_empty() {
            self.output_dir = Settings::default().output_dir;
        }

        if !BROWSERS.contains(&self.browser.as_str()) {
            self.browser = "firefox".to_owned();
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Status {
    Waiting,
    Downloading,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::Waiting => "В очереди",
            Self::Downloading => "Загрузка / обработка",
            Self::Completed => "Завершено",
            Self::Failed => "Ошибка",
            Self::Cancelled => "Отменено",
            Self::Interrupted => "Прервано (ждёт возобновления)",
        }
    }

    /// Задача запущена или стоит в очереди на запуск.
    pub fn active(self) -> bool {
        matches!(self, Self::Waiting | Self::Downloading)
    }

    /// Задача ещё не выполнена (включая прерванные).
    pub fn pending(self) -> bool {
        matches!(self, Self::Waiting | Self::Downloading | Self::Interrupted)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AudioOption {
    pub id: String,
    pub label: String,
    pub webm_compatible: bool,
}

/// Снимок параметров одной загрузки. Не меняется после добавления.
#[derive(Clone, Debug, PartialEq)]
pub struct DownloadSpec {
    pub url: String,
    pub quality: Quality,
    pub container: VideoContainer,
    pub playlist: bool,
    pub audio_id: Option<String>,
    pub format_override: Option<String>,
    pub browser: Option<String>,
    // None — не передавать --proxy; Some("") — напрямую; Some(адрес) — прокси.
    pub proxy: Option<String>,
    pub embed_cover: bool,
    pub save_cover: bool,
    pub output_dir: PathBuf,
}

/// Формат хранения задач. Совместим с версией 0.8.0.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedJob {
    pub id: u64,
    pub url: String,
    pub quality: Quality,
    pub container: VideoContainer,
    pub download_playlist: bool,
    pub audio_id: Option<String>,
    pub audio_label: String,
    #[serde(default)]
    pub format_override: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    pub browser: Option<String>,
    #[serde(default)]
    pub proxy: Option<String>,
    #[serde(default)]
    pub embed_cover: bool,
    #[serde(default)]
    pub save_cover: bool,
    #[serde(default)]
    pub cover_embedded: bool,
    pub output_dir: String,
    pub status: Status,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub logs: Vec<String>,
}

#[cfg(test)]
pub fn sample_spec() -> DownloadSpec {
    DownloadSpec {
        url: "https://example.org/video".to_owned(),
        quality: Quality::Best,
        container: VideoContainer::Mkv,
        playlist: false,
        audio_id: None,
        format_override: None,
        browser: None,
        proxy: None,
        embed_cover: false,
        save_cover: false,
        output_dir: PathBuf::from("/tmp/videos"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn height_filters_match_presets() {
        assert_eq!(Quality::Best.height_filter(), "");
        assert_eq!(Quality::AudioMp3.height_filter(), "");
        assert_eq!(Quality::H720.height_filter(), "[height<=720]");
        assert_eq!(Quality::H1080.height_filter(), "[height<=1080]");
        assert_eq!(Quality::H480.height_filter(), "[height<=480]");
    }

    #[test]
    fn status_flags() {
        assert!(Status::Waiting.active());
        assert!(Status::Downloading.active());
        assert!(!Status::Interrupted.active());
        assert!(Status::Interrupted.pending());
        assert!(!Status::Completed.pending());
        assert!(!Status::Failed.pending());
    }

    #[test]
    fn partial_settings_use_defaults() {
        let settings: Settings = serde_json::from_str(r#"{"quality":"H720"}"#).unwrap();

        assert_eq!(settings.quality, Quality::H720);
        assert_eq!(settings.container, VideoContainer::Mkv);
        assert!(settings.use_browser_cookies);
        assert!(!settings.output_dir.is_empty());
    }

    #[test]
    fn sanitize_fixes_bad_values() {
        let mut settings = Settings::default();
        settings.output_dir = "   ".to_owned();
        settings.browser = "netscape".to_owned();

        settings.sanitize();

        assert!(!settings.output_dir.trim().is_empty());
        assert_eq!(settings.browser, "firefox");
    }
}
