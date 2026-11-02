//! Разбор строк вывода yt-dlp. Чистые функции без побочных эффектов.

#[derive(Debug, Default, PartialEq)]
pub struct ProgressUpdate {
    /// Начался новый поток или элемент плейлиста: прогресс сбрасывается.
    pub reset: bool,
    pub percent: Option<f32>,
    pub speed: Option<String>,
    pub eta: Option<String>,
}

pub fn parse_progress(line: &str) -> Option<ProgressUpdate> {
    if !line.starts_with("[download]") {
        return None;
    }

    let reset = line.contains("Destination:") || line.contains("Downloading item ");

    let percent = line
        .split_whitespace()
        .find_map(|part| {
            part.strip_suffix('%')
                .and_then(|text| text.parse::<f32>().ok())
        })
        .map(|value| (value / 100.0).clamp(0.0, 1.0));

    let speed = line
        .split(" at ")
        .nth(1)
        .and_then(|part| part.split_whitespace().next())
        .map(str::to_owned);

    let eta = line
        .split(" ETA ")
        .nth(1)
        .and_then(|part| part.split_whitespace().next())
        .map(str::to_owned);

    Some(ProgressUpdate {
        reset,
        percent,
        speed,
        eta,
    })
}

pub fn is_cover_embedded(line: &str) -> bool {
    line.starts_with("[EmbedThumbnail]") && line.contains("Adding thumbnail")
}

pub fn is_cover_warning(line: &str) -> bool {
    let lower = line.to_lowercase();

    (lower.starts_with("error:") || lower.starts_with("warning:")) && lower.contains("thumbnail")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regular_progress_line() {
        let update =
            parse_progress("[download]  57.3% of   16.78MiB at    2.90MiB/s ETA 00:02").unwrap();

        assert!(!update.reset);
        assert!((update.percent.unwrap() - 0.573).abs() < 1e-5);
        assert_eq!(update.speed.as_deref(), Some("2.90MiB/s"));
        assert_eq!(update.eta.as_deref(), Some("00:02"));
    }

    #[test]
    fn finished_stream_line() {
        let update =
            parse_progress("[download] 100% of    4.76MiB in 00:00:03 at 1.28MiB/s").unwrap();

        assert_eq!(update.percent, Some(1.0));
        assert_eq!(update.speed.as_deref(), Some("1.28MiB/s"));
        assert_eq!(update.eta, None);
    }

    #[test]
    fn zero_percent_is_kept() {
        let update = parse_progress("[download]   0.0% of 10MiB").unwrap();

        assert_eq!(update.percent, Some(0.0));
    }

    #[test]
    fn destination_and_item_reset() {
        let destination = parse_progress("[download] Destination: /tmp/a.f398.mp4").unwrap();
        let item = parse_progress("[download] Downloading item 2 of 5").unwrap();

        assert!(destination.reset);
        assert!(item.reset);
        assert_eq!(destination.percent, None);
    }

    #[test]
    fn other_lines_are_ignored() {
        assert_eq!(parse_progress("[Merger] Merging formats"), None);
        assert_eq!(parse_progress("WARNING: something"), None);
        assert_eq!(parse_progress(""), None);
    }

    #[test]
    fn unknown_eta_is_text() {
        let update = parse_progress("[download]  10.0% of ~ 5MiB at 1MiB/s ETA Unknown").unwrap();

        assert_eq!(update.eta.as_deref(), Some("Unknown"));
    }

    #[test]
    fn cover_embedded_marker() {
        assert!(is_cover_embedded(
            "[EmbedThumbnail] ffmpeg: Adding thumbnail to \"a.mkv\""
        ));
        assert!(is_cover_embedded(
            "[EmbedThumbnail] mutagen: Adding thumbnail to \"a.mp4\""
        ));
        assert!(!is_cover_embedded("[download] Adding thumbnail"));
        assert!(!is_cover_embedded("[EmbedThumbnail] nothing"));
    }

    #[test]
    fn cover_warning_marker() {
        assert!(is_cover_warning("WARNING: Unable to embed thumbnail"));
        assert!(is_cover_warning("ERROR: thumbnail failed"));
        assert!(!is_cover_warning("WARNING: unrelated"));
        assert!(!is_cover_warning("[info] thumbnail ok"));
    }
}
