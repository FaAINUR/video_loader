//! Метаданные и выбор конкретных форматов из результата анализа.
use crate::models::{AudioOption, Quality, VideoContainer};
use serde_json::Value;

#[derive(Clone, Debug)]
pub struct Format {
    pub id: String,
    pub height: u64,
    pub fps: u64,
    pub video: bool,
    pub audio: bool,
    pub ext: String,
    pub codec: String,
    pub language: String,
    pub track: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolution {
    pub height: u64,
    pub fps: u64,
}
impl Resolution {
    pub fn label(&self) -> String {
        if self.fps == 0 {
            format!("{}p", self.height)
        } else {
            format!("{}p · {} кадров/с", self.height, self.fps)
        }
    }
}
#[derive(Clone, Debug)]
pub struct MediaInfo {
    pub title: String,
    pub author: String,
    pub duration: String,
    pub site: String,
    pub formats: Vec<Format>,
    pub resolutions: Vec<Resolution>,
}
fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_owned()
}
fn nonempty(v: String) -> String {
    if v.trim().is_empty() {
        "Не указано".into()
    } else {
        v
    }
}
pub fn duration(seconds: Option<f64>) -> String {
    match seconds.filter(|s| s.is_finite() && *s >= 0.0) {
        Some(s) => {
            let s = s as u64;
            if s >= 3600 {
                format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
            } else {
                format!("{}:{:02}", s / 60, s % 60)
            }
        }
        None => "Не указано / прямой эфир".into(),
    }
}
fn track_name(note: &str) -> String {
    // Снимаем только известный суффикс качества; смысл названия не угадываем.
    let mut words: Vec<&str> = note.split_whitespace().collect();
    if words
        .last()
        .is_some_and(|s| ["low", "medium", "high", "tiny"].contains(s))
    {
        words.pop();
    }
    let name = words.join(" ");
    if name.is_empty() {
        "Название не указано".into()
    } else {
        name
    }
}
pub fn parse(bytes: &[u8]) -> Result<MediaInfo, String> {
    let root: Value = serde_json::from_slice(bytes).map_err(|e| format!("Ошибка JSON: {e}"))?;
    if text(&root, "_type") == "playlist" {
        return Err(
            "Это плейлист. Включите режим плейлистов; подробный анализ доступен для одного видео."
                .into(),
        );
    }
    let raw = root
        .get("formats")
        .and_then(Value::as_array)
        .ok_or("Источник не вернул список форматов.")?;
    let mut formats = Vec::new();
    for f in raw {
        if f.get("has_drm").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let id = text(f, "format_id");
        if id.is_empty()
            || !id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
        {
            continue;
        }
        let vc = text(f, "vcodec");
        let ac = text(f, "acodec");
        let video = !vc.is_empty() && vc != "none";
        let audio = !ac.is_empty() && ac != "none";
        if !video && !audio {
            continue;
        }
        formats.push(Format {
            id,
            video,
            audio,
            height: f.get("height").and_then(Value::as_u64).unwrap_or(0),
            fps: f.get("fps").and_then(Value::as_f64).unwrap_or(0.0).round() as u64,
            ext: text(f, "ext"),
            codec: ac,
            language: text(f, "language"),
            track: track_name(&text(f, "format_note")),
        });
    }
    if formats.is_empty() {
        return Err("Не найдены поддерживаемые форматы видео или аудио.".into());
    }
    let mut resolutions: Vec<Resolution> = formats
        .iter()
        .filter(|f| f.video && f.height > 0)
        .map(|f| Resolution {
            height: f.height,
            fps: f.fps,
        })
        .collect();
    resolutions.sort_by(|a, b| (b.height, b.fps).cmp(&(a.height, a.fps)));
    resolutions.dedup();
    let mut author = text(&root, "channel");
    if author.is_empty() {
        author = text(&root, "uploader");
    }
    Ok(MediaInfo {
        title: nonempty(text(&root, "title")),
        author: nonempty(author),
        duration: duration(root.get("duration").and_then(Value::as_f64)),
        site: nonempty(text(&root, "extractor_key")),
        formats,
        resolutions,
    })
}
impl Format {
    fn webm_audio(&self) -> bool {
        self.codec.starts_with("opus") || self.codec.starts_with("vorbis")
    }
}
impl MediaInfo {
    pub fn languages(&self) -> Vec<String> {
        let mut result: Vec<String> = self
            .formats
            .iter()
            .filter(|f| f.audio && !f.video)
            .map(|f| f.language.clone())
            .collect();
        result.sort();
        result.dedup();
        result
    }
    pub fn tracks(&self, language: &str) -> Vec<String> {
        let mut result: Vec<String> = self
            .formats
            .iter()
            .filter(|f| f.audio && !f.video && f.language == language)
            .map(|f| f.track.clone())
            .collect();
        result.sort();
        result.dedup();
        result
    }
    pub fn audio_option(
        &self,
        language: Option<&str>,
        track: Option<&str>,
        webm: bool,
    ) -> Result<Option<AudioOption>, String> {
        let Some(language) = language else {
            return Ok(None);
        };
        let format = self.formats.iter().rev().find(|f| f.audio && !f.video && f.language == language
            && track.map_or(true, |t| f.track == t) && (!webm || f.webm_audio()))
            .ok_or("Для выбранной озвучки нет совместимого аудиоформата. Выберите другую дорожку или MKV.")?;
        Ok(Some(AudioOption {
            id: format.id.clone(),
            label: format!("{} · {}", language_label(language), format.track),
            webm_compatible: format.webm_audio(),
        }))
    }
    pub fn selector(
        &self,
        resolution: Option<&Resolution>,
        quality: Quality,
        container: VideoContainer,
        audio: Option<&AudioOption>,
    ) -> Result<String, String> {
        if quality == Quality::AudioMp3 {
            return Ok(audio
                .map(|a| a.id.clone())
                .unwrap_or_else(|| "bestaudio/best".into()));
        }
        let compatible = |f: &&Format| {
            f.video
                && (container != VideoContainer::Webm || f.ext == "webm")
                && resolution.map_or(true, |r| f.height == r.height && f.fps == r.fps)
        };
        let rank = |f: &&Format| (f.height, f.fps);
        if audio.is_none() {
            if let Some(best) = self.formats.iter().filter(compatible).max_by_key(rank) {
                if best.audio {
                    return Ok(best.id.clone());
                }
            }
        }
        let video_only = self
            .formats
            .iter()
            .filter(compatible)
            .filter(|f| !f.audio)
            .max_by_key(rank);
        let auto_audio =
            self.formats.iter().rev().find(|f| {
                f.audio && !f.video && (container != VideoContainer::Webm || f.webm_audio())
            });
        if let Some(video) = video_only {
            if let Some(id) = audio
                .map(|a| a.id.as_str())
                .or_else(|| auto_audio.map(|f| f.id.as_str()))
            {
                return Ok(format!("{}+{}", video.id, id));
            }
        }
        if audio.is_none() {
            if let Some(combined) = self
                .formats
                .iter()
                .rev()
                .filter(compatible)
                .find(|f| f.audio)
            {
                return Ok(combined.id.clone());
            }
        }
        Err("Выбранное качество и озвучка недоступны вместе в этом контейнере. Попробуйте MKV или другой вариант.".into())
    }
}
pub fn language_label(code: &str) -> String {
    match code {
        "" => "Язык не указан".into(),
        "ru" => "Русский".into(),
        "en" => "Английский".into(),
        _ => code.to_owned(),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn info() -> MediaInfo {
        parse(&serde_json::to_vec(&json!({"title":"Test", "uploader":"Author", "duration":3661,
            "formats":[
                {"format_id":"v1","vcodec":"vp9","acodec":"none","height":720,"fps":30,"ext":"webm"},
                {"format_id":"v2","vcodec":"vp9","acodec":"none","height":1080,"fps":60,"ext":"webm"},
                {"format_id":"a1","vcodec":"none","acodec":"opus","language":"ru","format_note":"dub low"},
                {"format_id":"a2","vcodec":"none","acodec":"opus","language":"ru","format_note":"dub medium"},
                {"format_id":"a3","vcodec":"none","acodec":"mp4a","language":"en","format_note":"original"}
            ]})).unwrap()).unwrap()
    }
    #[test]
    fn metadata_and_resolutions() {
        let i = info();
        assert_eq!(i.author, "Author");
        assert_eq!(i.duration, "1:01:01");
        assert_eq!(i.resolutions[0].height, 1080);
        assert_eq!(i.resolutions.len(), 2);
    }
    #[test]
    fn audio_grouping() {
        let i = info();
        assert_eq!(i.tracks("ru"), vec!["dub"]);
        assert_eq!(
            i.audio_option(Some("ru"), Some("dub"), false)
                .unwrap()
                .unwrap()
                .id,
            "a2"
        );
        assert!(i.audio_option(Some("en"), Some("original"), true).is_err());
    }
    #[test]
    fn exact_selection() {
        let i = info();
        let a = i.audio_option(Some("ru"), Some("dub"), false).unwrap();
        assert_eq!(
            i.selector(
                Some(&i.resolutions[1]),
                Quality::Best,
                VideoContainer::Mkv,
                a.as_ref()
            )
            .unwrap(),
            "v1+a2"
        );
    }
    #[test]
    fn missing_quality_is_error() {
        let i = info();
        assert!(i
            .selector(
                Some(&Resolution {
                    height: 2160,
                    fps: 60
                }),
                Quality::Best,
                VideoContainer::Mkv,
                None
            )
            .is_err());
    }
    #[test]
    fn bad_and_missing_json() {
        assert!(parse(b"bad").is_err());
        assert!(parse(b"{}").is_err());
        assert!(parse(br#"{"_type":"playlist"}"#).is_err());
    }
    #[test]
    fn unknown_duration() {
        assert!(duration(None).contains("Не указано"));
    }
    #[test]
    fn combined_stream() {
        let i = parse(
            br#"{"formats":[{"format_id":"18","vcodec":"avc1","acodec":"aac","height":360}]}"#,
        )
        .unwrap();
        assert_eq!(
            i.selector(None, Quality::Best, VideoContainer::Mkv, None)
                .unwrap(),
            "18"
        );
    }
}

// Проверки бывшего аудиопарсера перенесены на рабочую реализацию MediaInfo.
#[cfg(test)]
mod audio_regression_tests {
    use super::*;
    use serde_json::json;

    fn parse_value(value: Value) -> MediaInfo {
        parse(&serde_json::to_vec(&value).unwrap()).unwrap()
    }

    #[test]
    fn audio_selection_ignores_video_and_muxed_formats() {
        let info = parse_value(json!({"formats":[
            {"format_id":"137","vcodec":"avc1","acodec":"none"},
            {"format_id":"140","vcodec":"none","acodec":"mp4a.40.2","language":"en"},
            {"format_id":"251","vcodec":"none","acodec":"opus","language":"ru"},
            {"format_id":"18","vcodec":"avc1","acodec":"mp4a.40.2","language":"de"},
            {"vcodec":"none","acodec":"opus","language":"fr"}
        ]}));
        assert_eq!(info.languages(), vec!["en", "ru"]);
        let ru = info.audio_option(Some("ru"), None, false).unwrap().unwrap();
        assert_eq!(ru.id, "251");
        assert!(ru.webm_compatible);
        assert!(ru.label.contains("Русский"));
        let en = info.audio_option(Some("en"), None, false).unwrap().unwrap();
        assert_eq!(en.id, "140");
        assert!(!en.webm_compatible);
        assert!(en.label.contains("Английский"));
    }

    #[test]
    fn video_without_separate_audio_has_no_audio_groups() {
        let info = parse_value(json!({"formats":[
            {"format_id":"18","vcodec":"avc1","acodec":"aac"}
        ]}));
        assert!(info.languages().is_empty());
        assert!(info.audio_option(None, None, false).unwrap().is_none());
    }

    #[test]
    fn malformed_audio_json_is_rejected() {
        assert!(parse(b"not json").is_err());
    }

    #[test]
    fn missing_formats_is_rejected() {
        assert!(parse(br#"{"title":"x"}"#).is_err());
    }

    #[test]
    fn unknown_and_missing_languages_remain_selectable() {
        let info = parse_value(json!({"formats":[
            {"format_id":"a1","vcodec":"none","acodec":"opus"},
            {"format_id":"a2","vcodec":"none","acodec":"opus","language":"de"}
        ]}));
        assert_eq!(info.languages(), vec!["", "de"]);
        let unknown = info.audio_option(Some(""), None, false).unwrap().unwrap();
        let german = info.audio_option(Some("de"), None, false).unwrap().unwrap();
        assert_eq!(unknown.id, "a1");
        assert!(unknown.label.contains("Язык не указан"));
        assert_eq!(german.id, "a2");
        assert!(german.label.contains("de"));
    }
}
