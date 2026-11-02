//! Короткие объяснения; исходный вывод остаётся в журнале.
pub const UNKNOWN: &str = "Загрузка завершилась с ошибкой. Подробности — в журнале.";

pub fn explain(text: &str) -> Option<&'static str> {
    let s = text.to_lowercase();
    if s.contains("not a bot") || s.contains("confirm you’re not a bot") {
        Some("YouTube просит подтвердить, что вы не бот. Проверьте вход в аккаунт и настройку cookies.")
    } else if (s.contains("cookie")
        && [
            "could not",
            "couldn't",
            "failed to",
            "unable to",
            "permission denied",
        ]
        .iter()
        .any(|p| s.contains(p)))
        || s.contains("could not find firefox cookies database")
    {
        Some("Не удалось прочитать cookies браузера. Проверьте профиль браузера. Подробности — в журнале.")
    } else if (s.contains("ffmpeg") || s.contains("ffprobe"))
        && ["not found", "not installed", "could not find"]
            .iter()
            .any(|p| s.contains(p))
    {
        Some("Не найден FFmpeg или FFprobe. Проверьте установку и доступность в PATH.")
    } else if s.contains("requested format is not available") {
        Some("Выбранное качество или озвучка недоступны. Попробуйте автоматическую озвучку или другое качество.")
    } else if s.contains("proxyerror")
        || s.contains("unable to connect to proxy")
        || s.contains("cannot connect to proxy")
        || s.contains("error connecting to socks")
    {
        Some("Не удалось подключиться к прокси. Проверьте адрес и запущен ли прокси-клиент.")
    } else if s.contains("http error 429") || s.contains("too many requests") {
        Some("Сайт ограничил количество запросов. Подождите перед повторной попыткой.")
    } else if s.contains("http error 403") {
        Some("Сайт отказал в доступе (403). Причина не определена; подробности — в журнале.")
    } else if s.contains("timed out") || s.contains("timeout") {
        Some("Превышено время ожидания. Проверьте подключение и настройки прокси.")
    } else {
        None
    }
}

/// Не используем сообщения прошлой попытки после нажатия «Повторить».
pub fn failure_message(logs: &[String], fallback: &str) -> String {
    for line in logs.iter().rev() {
        if line == "--- Повторный запуск ---" {
            break;
        }
        if line.trim_start().starts_with("ERROR:") {
            if let Some(message) = explain(line) {
                return message.to_owned();
            }
        }
    }
    explain(fallback).unwrap_or(UNKNOWN).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn known_errors() {
        for (raw, expected) in [
            ("ERROR: Sign in to confirm you’re not a bot", "YouTube"),
            ("ERROR: could not find firefox cookies database", "cookies"),
            ("ERROR: ffmpeg not found", "FFmpeg"),
            ("ERROR: Requested format is not available", "качество"),
            ("ERROR: ProxyError: Unable to connect to proxy", "прокси"),
            ("ERROR: HTTP Error 429: Too Many Requests", "ограничил"),
            ("ERROR: HTTP Error 403: Forbidden", "403"),
            ("ERROR: Connection timed out", "ожидания"),
        ] {
            assert!(explain(raw).unwrap().contains(expected), "{raw}");
        }
        assert_eq!(explain("ERROR: something new"), None);
    }
    #[test]
    fn ignores_old_attempt_and_warnings() {
        let logs = vec![
            "ERROR: Requested format is not available".into(),
            "--- Повторный запуск ---".into(),
            "WARNING: ffmpeg not found".into(),
        ];
        assert_eq!(failure_message(&logs, "exit 1"), UNKNOWN);
    }
    #[test]
    fn preserves_specific_message_after_later_output() {
        let logs = vec![
            "ERROR: Requested format is not available".into(),
            "[info] finished".into(),
        ];
        assert!(failure_message(&logs, "exit 1").contains("качество"));
    }
}
