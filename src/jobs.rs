//! Очередь: состояние одной задачи, сохранение и восстановление.

use crate::models::*;
use crate::progress;
use crate::ytdlp::RunningDownload;
use std::path::PathBuf;

pub const MAX_LOG_LINES: usize = 500;
pub const MAX_HISTORY: usize = 200;
pub const SAVED_LOG_LINES: usize = 60;

pub struct DownloadItem {
    pub id: u64,
    pub spec: DownloadSpec,
    pub audio_label: String,

    pub status: Status,
    pub percent: f32,
    pub speed: String,
    pub eta: String,
    pub message: String,
    pub logs: Vec<String>,

    pub cover_embedded: bool,
    pub cover_problem: bool,

    pub running: Option<RunningDownload>,
}

fn trim_logs(logs: &mut Vec<String>) {
    if logs.len() > MAX_LOG_LINES {
        let excess = logs.len() - MAX_LOG_LINES;
        logs.drain(..excess);
    }
}

impl DownloadItem {
    pub fn new(id: u64, spec: DownloadSpec, audio_label: String) -> Self {
        Self {
            id,
            spec,
            audio_label,
            status: Status::Waiting,
            percent: 0.0,
            speed: String::new(),
            eta: String::new(),
            message: "Ожидает запуска.".to_owned(),
            logs: Vec::new(),
            cover_embedded: false,
            cover_problem: false,
            running: None,
        }
    }

    /// Обрабатывает одну строку вывода yt-dlp.
    pub fn apply_line(&mut self, line: String) {
        if let Some(update) = progress::parse_progress(&line) {
            if update.reset {
                self.percent = 0.0;
                self.speed.clear();
                self.eta.clear();
            }

            if let Some(percent) = update.percent {
                self.percent = percent;
            }

            if let Some(speed) = update.speed {
                self.speed = speed;
            }

            if let Some(eta) = update.eta {
                self.eta = eta;
            }
        }

        if progress::is_cover_embedded(&line) {
            self.cover_embedded = true;
        }

        if progress::is_cover_warning(&line) {
            self.cover_problem = true;
        }

        self.message = line.clone();
        self.logs.push(line);
        trim_logs(&mut self.logs);
    }

    pub fn mark_started(&mut self, running: RunningDownload) {
        self.running = Some(running);
        self.status = Status::Downloading;
        self.message = "Получение информации и запуск загрузки...".to_owned();
    }

    pub fn fail(&mut self, error: String) {
        self.running = None;
        self.status = Status::Failed;
        self.message = crate::diagnostics::failure_message(&[], &error);
        self.logs.push(error);
        trim_logs(&mut self.logs);
    }

    /// Завершает задачу по результату процесса.
    pub fn finish(&mut self, result: Result<(), String>) {
        self.running = None;

        match result {
            Ok(()) => {
                self.status = Status::Completed;
                self.percent = 1.0;

                self.message = if self.spec.embed_cover && !self.cover_embedded {
                    "Видео сохранено, но yt-dlp не подтвердил \
                     встраивание обложки. Проверьте журнал."
                        .to_owned()
                } else if self.spec.playlist {
                    "Обработка плейлиста завершена. Проверьте журнал.".to_owned()
                } else {
                    "Файл сохранён.".to_owned()
                };
            }

            Err(error) => {
                self.status = Status::Failed;
                self.message = crate::diagnostics::failure_message(&self.logs, &error);
                self.logs.push(error);
                trim_logs(&mut self.logs);
            }
        }
    }

    pub fn cancel(&mut self) {
        if let Some(running) = self.running.as_mut() {
            if let Err(error) = running.stop() {
                self.message = error.clone();
                self.logs.push(error);
                return;
            }

            let rest: Vec<String> = running.lines.try_iter().collect();
            self.logs.extend(rest);
            trim_logs(&mut self.logs);
        }

        self.running = None;
        self.status = Status::Cancelled;
        self.message = "Отменено. Скачанные и незавершённые файлы оставлены.".to_owned();
    }

    /// Возвращает задачу в очередь. false — состояние не подходит.
    pub fn resume(&mut self) -> bool {
        if !matches!(
            self.status,
            Status::Failed | Status::Cancelled | Status::Interrupted
        ) {
            return false;
        }

        self.running = None;
        self.status = Status::Waiting;
        self.percent = 0.0;
        self.speed.clear();
        self.eta.clear();
        self.cover_embedded = false;
        self.cover_problem = false;
        self.message = "Ожидает повторного запуска.".to_owned();
        self.logs.push("--- Повторный запуск ---".to_owned());
        trim_logs(&mut self.logs);
        true
    }

    pub fn to_saved(&self, log_lines: usize) -> SavedJob {
        let start = self.logs.len().saturating_sub(log_lines);

        SavedJob {
            id: self.id,
            url: self.spec.url.clone(),
            quality: self.spec.quality,
            container: self.spec.container,
            download_playlist: self.spec.playlist,
            audio_id: self.spec.audio_id.clone(),
            audio_label: self.audio_label.clone(),
            browser: self.spec.browser.clone(),
            proxy: self.spec.proxy.clone(),
            embed_cover: self.spec.embed_cover,
            save_cover: self.spec.save_cover,
            cover_embedded: self.cover_embedded,
            output_dir: self.spec.output_dir.to_string_lossy().into_owned(),
            status: self.status,
            message: self.message.clone(),
            logs: self.logs[start..].to_vec(),
        }
    }

    pub fn from_saved(job: SavedJob) -> Self {
        let (status, message) = match job.status {
            Status::Waiting | Status::Downloading => (
                Status::Interrupted,
                "Прервано при закрытии программы. \
                 Нажмите «Возобновить»."
                    .to_owned(),
            ),

            other => (other, job.message),
        };

        Self {
            id: job.id,
            spec: DownloadSpec {
                url: job.url,
                quality: job.quality,
                container: job.container,
                playlist: job.download_playlist,
                audio_id: job.audio_id,
                browser: job.browser,
                proxy: job.proxy,
                embed_cover: job.embed_cover,
                save_cover: job.save_cover,
                output_dir: PathBuf::from(job.output_dir),
            },
            audio_label: job.audio_label,
            status,
            percent: if status == Status::Completed {
                1.0
            } else {
                0.0
            },
            speed: String::new(),
            eta: String::new(),
            message,
            logs: job.logs,
            cover_embedded: job.cover_embedded,
            cover_problem: false,
            running: None,
        }
    }
}

/// Выбирает задачи для сохранения. Старые завершённые отбрасываются,
/// если их больше max_history. Незавершённые сохраняются всегда.
pub fn saved_jobs(items: &[DownloadItem], max_history: usize, log_lines: usize) -> Vec<SavedJob> {
    let finished = |status: Status| {
        matches!(
            status,
            Status::Completed | Status::Failed | Status::Cancelled
        )
    };

    let finished_total = items.iter().filter(|item| finished(item.status)).count();

    let mut to_skip = finished_total.saturating_sub(max_history);
    let mut result = Vec::new();

    for item in items {
        if finished(item.status) && to_skip > 0 {
            to_skip -= 1;
            continue;
        }

        result.push(item.to_saved(log_lines));
    }

    result
}

/// Восстанавливает очередь. Возвращает задачи и следующий свободный ID.
pub fn restore_items(saved: Vec<SavedJob>) -> (Vec<DownloadItem>, u64) {
    let next_id = saved.iter().map(|job| job.id).max().unwrap_or(0) + 1;

    let items = saved.into_iter().map(DownloadItem::from_saved).collect();

    (items, next_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: u64) -> DownloadItem {
        DownloadItem::new(id, sample_spec(), "Автоматически".to_owned())
    }

    fn with_status(id: u64, status: Status) -> DownloadItem {
        let mut item = item(id);
        item.status = status;
        item
    }

    #[test]
    fn new_item_waits() {
        let item = item(1);

        assert_eq!(item.status, Status::Waiting);
        assert_eq!(item.percent, 0.0);
        assert!(item.logs.is_empty());
        assert!(item.running.is_none());
    }

    #[test]
    fn progress_line_updates_fields() {
        let mut item = item(1);

        item.apply_line("[download]  57.3% of   16.78MiB at    2.90MiB/s ETA 00:02".to_owned());

        assert!((item.percent - 0.573).abs() < 1e-5);
        assert_eq!(item.speed, "2.90MiB/s");
        assert_eq!(item.eta, "00:02");
        assert_eq!(item.logs.len(), 1);
        assert!(item.message.starts_with("[download]"));
    }

    #[test]
    fn destination_resets_progress() {
        let mut item = item(1);

        item.apply_line("[download]  80.0% of 1MiB at 1MiB/s ETA 00:01".into());
        item.apply_line("[download] Destination: /tmp/a.webm".into());

        assert_eq!(item.percent, 0.0);
        assert!(item.speed.is_empty());
        assert!(item.eta.is_empty());
    }

    #[test]
    fn plain_line_keeps_progress() {
        let mut item = item(1);

        item.apply_line("[download]  40.0% of 1MiB at 1MiB/s ETA 00:01".into());
        item.apply_line("[Merger] Merging formats".into());

        assert!((item.percent - 0.4).abs() < 1e-5);
        assert_eq!(item.message, "[Merger] Merging formats");
    }

    #[test]
    fn logs_are_trimmed() {
        let mut item = item(1);

        for n in 0..(MAX_LOG_LINES + 25) {
            item.apply_line(format!("line {n}"));
        }

        assert_eq!(item.logs.len(), MAX_LOG_LINES);
        assert_eq!(item.logs[0], "line 25");
    }

    #[test]
    fn cover_markers_are_detected() {
        let mut item = item(1);

        item.apply_line("[EmbedThumbnail] ffmpeg: Adding thumbnail to \"a.mkv\"".into());
        item.apply_line("WARNING: thumbnail is odd".into());

        assert!(item.cover_embedded);
        assert!(item.cover_problem);
    }

    #[test]
    fn finish_success() {
        let mut item = item(1);

        item.finish(Ok(()));

        assert_eq!(item.status, Status::Completed);
        assert_eq!(item.percent, 1.0);
        assert_eq!(item.message, "Файл сохранён.");
    }

    #[test]
    fn finish_playlist_message() {
        let mut item = item(1);
        item.spec.playlist = true;

        item.finish(Ok(()));

        assert!(item.message.contains("плейлиста"));
    }

    #[test]
    fn finish_warns_when_cover_not_confirmed() {
        let mut item = item(1);
        item.spec.embed_cover = true;

        item.finish(Ok(()));

        assert_eq!(item.status, Status::Completed);
        assert!(item.message.contains("обложк"));
    }

    #[test]
    fn finish_is_quiet_when_cover_confirmed() {
        let mut item = item(1);
        item.spec.embed_cover = true;
        item.cover_embedded = true;

        item.finish(Ok(()));

        assert_eq!(item.message, "Файл сохранён.");
    }

    #[test]
    fn finish_error() {
        let mut item = item(1);

        item.finish(Err("boom".to_owned()));

        assert_eq!(item.status, Status::Failed);
        assert_eq!(item.message, crate::diagnostics::UNKNOWN);
        assert_eq!(item.logs.last().map(String::as_str), Some("boom"));
    }

    #[test]
    fn fail_records_error_in_log() {
        let mut item = item(1);

        item.fail("нет yt-dlp".to_owned());

        assert_eq!(item.status, Status::Failed);
        assert_eq!(item.logs.last().map(String::as_str), Some("нет yt-dlp"));
    }

    #[test]
    fn resume_only_from_stopped_states() {
        for status in [Status::Failed, Status::Cancelled, Status::Interrupted] {
            let mut item = with_status(1, status);
            item.cover_embedded = true;

            assert!(item.resume(), "{status:?}");
            assert_eq!(item.status, Status::Waiting);
            assert!(!item.cover_embedded);
            assert_eq!(
                item.logs.last().map(String::as_str),
                Some("--- Повторный запуск ---")
            );
        }

        for status in [Status::Waiting, Status::Downloading, Status::Completed] {
            let mut item = with_status(1, status);

            assert!(!item.resume(), "{status:?}");
            assert_eq!(item.status, status);
        }
    }

    #[test]
    fn cancel_without_process() {
        let mut item = item(1);

        item.cancel();

        assert_eq!(item.status, Status::Cancelled);
    }

    #[test]
    fn saved_log_is_limited() {
        let mut item = item(1);

        for n in 0..100 {
            item.apply_line(format!("line {n}"));
        }

        let saved = item.to_saved(10);

        assert_eq!(saved.logs.len(), 10);
        assert_eq!(saved.logs[0], "line 90");
        assert_eq!(saved.logs[9], "line 99");
    }

    #[test]
    fn saved_job_json_round_trip() {
        let mut item = item(7);
        item.spec.proxy = Some(String::new());
        item.spec.embed_cover = true;
        item.apply_line("hello".into());

        let saved = item.to_saved(10);
        let json = serde_json::to_string(&saved).unwrap();
        let back: SavedJob = serde_json::from_str(&json).unwrap();

        assert_eq!(saved, back);
    }

    #[test]
    fn old_saved_job_without_new_fields_loads() {
        let json = r#"{
            "id": 3, "url": "https://example.org/v",
            "quality": "Best", "container": "Mkv",
            "download_playlist": false, "audio_id": null,
            "audio_label": "Автоматически", "browser": null,
            "output_dir": "/tmp/v", "status": "Completed"
        }"#;

        let saved: SavedJob = serde_json::from_str(json).unwrap();

        assert_eq!(saved.proxy, None);
        assert!(!saved.embed_cover);
        assert!(saved.logs.is_empty());
    }

    #[test]
    fn restore_marks_unfinished_as_interrupted() {
        let saved = vec![
            with_status(1, Status::Waiting).to_saved(5),
            with_status(2, Status::Downloading).to_saved(5),
            with_status(3, Status::Completed).to_saved(5),
            with_status(4, Status::Failed).to_saved(5),
        ];

        let (items, next_id) = restore_items(saved);

        assert_eq!(items[0].status, Status::Interrupted);
        assert_eq!(items[1].status, Status::Interrupted);
        assert!(items[0].message.contains("Возобновить"));
        assert_eq!(items[2].status, Status::Completed);
        assert_eq!(items[2].percent, 1.0);
        assert_eq!(items[3].status, Status::Failed);
        assert_eq!(next_id, 5);
    }

    #[test]
    fn restore_empty_starts_from_one() {
        let (items, next_id) = restore_items(Vec::new());

        assert!(items.is_empty());
        assert_eq!(next_id, 1);
    }

    #[test]
    fn restored_interrupted_job_can_resume() {
        let saved = vec![with_status(1, Status::Downloading).to_saved(5)];
        let (mut items, _) = restore_items(saved);

        assert!(items[0].resume());
        assert_eq!(items[0].status, Status::Waiting);
    }

    #[test]
    fn history_keeps_newest_finished_and_all_pending() {
        let mut items: Vec<DownloadItem> = (1..=5)
            .map(|id| with_status(id, Status::Completed))
            .collect();

        items.push(with_status(6, Status::Waiting));
        items.push(with_status(7, Status::Failed));

        let saved = saved_jobs(&items, 2, 10);
        let ids: Vec<u64> = saved.iter().map(|job| job.id).collect();

        assert_eq!(ids, vec![5, 6, 7]);
    }

    #[test]
    fn history_under_limit_is_unchanged() {
        let items = vec![
            with_status(1, Status::Completed),
            with_status(2, Status::Interrupted),
        ];

        assert_eq!(saved_jobs(&items, 200, 10).len(), 2);
    }
}

#[cfg(test)]
mod friendly_error_tests {
    use super::*;
    #[test]
    fn specific_error_survives_process_exit() {
        let mut item = DownloadItem::new(1, sample_spec(), "auto".into());
        item.apply_line("ERROR: Requested format is not available".into());
        item.apply_line("[info] cleanup".into());
        item.finish(Err("exit 1".into()));
        assert!(item.message.contains("качество"));
        assert!(item.logs.iter().any(|s| s.contains("Requested format")));
        assert!(item.resume());
        item.finish(Err("new unknown error".into()));
        assert_eq!(item.message, crate::diagnostics::UNKNOWN);
    }
}
