//! Состояние приложения и действия. Отрисовка находится в app/ui.rs.

mod ui;

use crate::jobs::{self, DownloadItem, MAX_HISTORY, SAVED_LOG_LINES};
use crate::models::*;
use crate::platform;
use crate::rules;
use crate::ytdlp::{self, RunningDownload};
use eframe::egui;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

const SETTINGS_KEY: &str = "video_loader_settings_v1";
const JOBS_KEY: &str = "video_loader_jobs_v1";

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Downloads,
    Settings,
}

enum CardAction {
    Cancel(u64),
    Resume(u64),
    Remove(u64),
}

struct AudioResult {
    url: String,
    result: Result<Vec<AudioOption>, String>,
}

pub struct App {
    page: Page,
    settings: Settings,
    urls: String,
    notice: String,

    folder_receiver: Option<Receiver<Option<PathBuf>>>,

    audio_options: Vec<AudioOption>,
    selected_audio: Option<AudioOption>,
    temporary_audio_id: Option<String>,
    audio_receiver: Option<Receiver<AudioResult>>,
    audio_status: String,
    show_audio_window: bool,

    next_id: u64,
    items: Vec<DownloadItem>,

    dependency_receiver: Option<Receiver<(bool, bool)>>,
    dependencies: Option<(bool, bool)>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut settings: Settings = cc
            .storage
            .and_then(|storage| eframe::get_value(storage, SETTINGS_KEY))
            .unwrap_or_default();

        settings.sanitize();

        let saved: Vec<SavedJob> = cc
            .storage
            .and_then(|storage| eframe::get_value(storage, JOBS_KEY))
            .unwrap_or_default();

        let (items, next_id) = jobs::restore_items(saved);

        Self {
            page: Page::Downloads,
            settings,
            urls: String::new(),
            notice: String::new(),

            folder_receiver: None,

            audio_options: Vec::new(),
            selected_audio: None,
            temporary_audio_id: None,
            audio_receiver: None,
            audio_status: String::new(),
            show_audio_window: false,

            next_id,
            items,

            dependency_receiver: None,
            dependencies: None,
        }
    }

    fn reset_audio(&mut self) {
        self.audio_options.clear();
        self.selected_audio = None;
        self.temporary_audio_id = None;
        self.audio_status.clear();
    }

    fn single_url(&self) -> Option<String> {
        let mut lines = self
            .urls
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty());

        let first = lines.next()?;

        if lines.next().is_some() {
            return None;
        }

        Some(first.to_owned())
    }

    // ---------- аудиодорожки ----------

    fn start_audio_selection(&mut self) {
        if self.audio_receiver.is_some() {
            return;
        }

        if self.settings.download_playlists {
            self.notice = "Для плейлистов используется автоматическая озвучка. \
                 Для конкретной дорожки добавляйте видео отдельно."
                .to_owned();
            return;
        }

        let Some(url) = self.single_url() else {
            self.notice = "Выбор озвучки доступен для одной ссылки. \
                 Для группы используется автоматическая озвучка."
                .to_owned();
            return;
        };

        if !rules::valid_url(&url) {
            self.notice = "Вставьте ссылку с http:// или https://.".to_owned();
            return;
        }

        if !rules::valid_proxy(self.settings.proxy_url.trim()) {
            self.notice = rules::PROXY_ERROR.to_owned();
            return;
        }

        let browser = rules::browser_for(&self.settings, &url);
        let proxy = rules::proxy_for(&self.settings, &url);
        let (sender, receiver) = mpsc::channel();

        self.audio_options.clear();
        self.temporary_audio_id = None;
        self.audio_receiver = Some(receiver);
        self.audio_status = "Получаем список аудиодорожек...".to_owned();
        self.show_audio_window = true;
        self.notice.clear();

        thread::spawn(move || {
            let result = ytdlp::load_audio_options(&url, browser.as_deref(), proxy.as_deref());

            let _ = sender.send(AudioResult { url, result });
        });
    }

    fn poll_audio(&mut self) {
        let result = match self.audio_receiver.as_ref() {
            Some(receiver) => receiver.try_recv(),
            None => return,
        };

        match result {
            Ok(response) => {
                self.audio_receiver = None;

                if self.single_url().as_deref() != Some(response.url.as_str()) {
                    self.audio_status = "Ссылка изменилась. Запросите список заново.".to_owned();
                    return;
                }

                match response.result {
                    Ok(options) => {
                        self.audio_status = if options.is_empty() {
                            "Отдельные аудиоформаты не найдены. \
                             Попробуйте автоматическую озвучку."
                                .to_owned()
                        } else {
                            format!("Найдено аудиоформатов: {}.", options.len())
                        };

                        self.temporary_audio_id =
                            self.selected_audio.as_ref().map(|audio| audio.id.clone());

                        self.audio_options = options;
                    }

                    Err(error) => {
                        self.audio_status = match crate::diagnostics::explain(&error) {
                            Some(message) => format!("{message}\n\nИсходное сообщение:\n{error}"),
                            None => error,
                        };
                    }
                }
            }

            Err(mpsc::TryRecvError::Empty) => {}

            Err(mpsc::TryRecvError::Disconnected) => {
                self.audio_receiver = None;
                self.audio_status = "Получение аудиодорожек завершилось без результата.".to_owned();
            }
        }
    }

    // ---------- добавление задач ----------

    fn add_downloads(&mut self) {
        if self.audio_receiver.is_some() {
            self.notice = "Дождитесь получения аудиодорожек.".to_owned();
            return;
        }

        let urls = match rules::parse_url_list(&self.urls) {
            Ok(urls) => urls,

            Err(error) => {
                self.notice = error;
                return;
            }
        };

        if let Err(error) =
            rules::validate_request(&self.settings, urls.len(), self.selected_audio.as_ref())
        {
            self.notice = error;
            return;
        }

        let output_dir = match rules::resolve_output_dir(&self.settings.output_dir) {
            Ok(path) => path,

            Err(error) => {
                self.notice = error;
                return;
            }
        };

        let (urls, duplicates) = rules::dedupe_urls(urls);

        let audio_id = self.selected_audio.as_ref().map(|audio| audio.id.clone());

        let audio_label = self
            .selected_audio
            .as_ref()
            .map(|audio| audio.label.clone())
            .unwrap_or_else(|| "Автоматически".to_owned());

        let playlist = self.settings.download_playlists;
        let mut added = 0;
        let mut skipped = duplicates;

        for url in urls {
            let already_pending = self.items.iter().any(|item| {
                item.spec.url == url && item.spec.playlist == playlist && item.status.pending()
            });

            if already_pending {
                skipped += 1;
                continue;
            }

            let spec = DownloadSpec {
                url: url.clone(),
                quality: self.settings.quality,
                container: self.settings.container,
                playlist,
                audio_id: audio_id.clone(),
                browser: rules::browser_for(&self.settings, &url),
                proxy: rules::proxy_for(&self.settings, &url),
                embed_cover: self.settings.embed_cover,
                save_cover: self.settings.save_cover,
                output_dir: output_dir.clone(),
            };

            self.items
                .push(DownloadItem::new(self.next_id, spec, audio_label.clone()));

            self.next_id += 1;
            added += 1;
        }

        self.urls.clear();
        self.reset_audio();
        self.show_audio_window = false;

        self.notice = if skipped > 0 {
            format!(
                "Добавлено задач: {added}. \
                 Пропущено повторных или незавершённых ссылок: {skipped}."
            )
        } else {
            format!("Добавлено задач: {added}.")
        };
    }

    // ---------- очередь ----------

    fn poll_downloads(&mut self) {
        for item in &mut self.items {
            let lines: Vec<String> = match item.running.as_mut() {
                Some(running) => running.lines.try_iter().collect(),
                None => continue,
            };

            for line in lines {
                item.apply_line(line);
            }

            let completion = item.running.as_mut().and_then(RunningDownload::poll);

            if let Some(result) = completion {
                let rest: Vec<String> = item
                    .running
                    .as_ref()
                    .map(|running| running.lines.try_iter().collect())
                    .unwrap_or_default();

                for line in rest {
                    item.apply_line(line);
                }

                item.finish(result);
            }
        }

        if self.items.iter().any(|item| item.running.is_some()) {
            return;
        }

        if let Some(item) = self
            .items
            .iter_mut()
            .find(|item| item.status == Status::Waiting)
        {
            match RunningDownload::start(&item.spec) {
                Ok(running) => item.mark_started(running),
                Err(error) => item.fail(error),
            }
        }
    }

    fn cancel(&mut self, id: u64) {
        if let Some(item) = self.items.iter_mut().find(|item| item.id == id) {
            item.cancel();
        }
    }

    fn resume(&mut self, id: u64) {
        if let Some(item) = self.items.iter_mut().find(|item| item.id == id) {
            item.resume();
        }
    }

    fn resume_all(&mut self) {
        for item in self
            .items
            .iter_mut()
            .filter(|item| item.status == Status::Interrupted)
        {
            item.resume();
        }
    }

    // ---------- папки и зависимости ----------

    fn browse_output_folder(&mut self) {
        if self.folder_receiver.is_some() {
            return;
        }

        let initial_dir = rules::resolve_output_dir(&self.settings.output_dir)
            .ok()
            .filter(|path| path.is_dir())
            .or_else(dirs::home_dir);

        let (sender, receiver) = mpsc::channel();
        self.folder_receiver = Some(receiver);

        thread::spawn(move || {
            let mut dialog =
                rfd::FileDialog::new().set_title("Выберите папку для сохранения видео");

            if let Some(directory) = initial_dir {
                dialog = dialog.set_directory(directory);
            }

            let _ = sender.send(dialog.pick_folder());
        });
    }

    fn poll_folder_selection(&mut self) {
        let result = match self.folder_receiver.as_ref() {
            Some(receiver) => receiver.try_recv(),
            None => return,
        };

        match result {
            Ok(Some(path)) => {
                self.folder_receiver = None;
                self.settings.output_dir = path.to_string_lossy().into_owned();
                self.notice = "Папка сохранения изменена.".to_owned();
            }

            Ok(None) => self.folder_receiver = None,

            Err(mpsc::TryRecvError::Empty) => {}

            Err(mpsc::TryRecvError::Disconnected) => {
                self.folder_receiver = None;
                self.notice = "Диалог завершился без результата.".to_owned();
            }
        }
    }

    fn open_folder(&mut self) {
        let result = rules::resolve_output_dir(&self.settings.output_dir)
            .and_then(|path| platform::open_directory(&path));

        if let Err(error) = result {
            self.notice = error;
        }
    }

    fn check_dependencies(&mut self) {
        if self.dependency_receiver.is_some() {
            return;
        }

        let (sender, receiver) = mpsc::channel();
        self.dependency_receiver = Some(receiver);

        thread::spawn(move || {
            let result = (
                platform::check_command("yt-dlp", "--version"),
                platform::check_command("ffmpeg", "-version"),
            );

            let _ = sender.send(result);
        });
    }

    fn poll_dependencies(&mut self) {
        let result = match self.dependency_receiver.as_ref() {
            Some(receiver) => receiver.try_recv(),
            None => return,
        };

        match result {
            Ok(result) => {
                self.dependencies = Some(result);
                self.dependency_receiver = None;
            }

            Err(mpsc::TryRecvError::Empty) => {}

            Err(mpsc::TryRecvError::Disconnected) => {
                self.dependency_receiver = None;
            }
        }
    }
}

impl eframe::App for App {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, SETTINGS_KEY, &self.settings);

        eframe::set_value(
            storage,
            JOBS_KEY,
            &jobs::saved_jobs(&self.items, MAX_HISTORY, SAVED_LOG_LINES),
        );
    }

    fn auto_save_interval(&self) -> Duration {
        Duration::from_secs(5)
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_audio();
        self.poll_downloads();
        self.poll_folder_selection();
        self.poll_dependencies();

        let busy = self.audio_receiver.is_some()
            || self.folder_receiver.is_some()
            || self.dependency_receiver.is_some()
            || self.items.iter().any(|item| item.status.active());

        if busy {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        self.show_sidebar(ctx);

        egui::CentralPanel::default().show(ctx, |ui| {
            if !self.notice.is_empty() {
                ui.colored_label(egui::Color32::from_rgb(240, 180, 90), &self.notice);
                ui.add_space(10.0);
            }

            match self.page {
                Page::Downloads => self.show_downloads(ui),

                Page::Settings => {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        self.show_settings(ui);
                    });
                }
            }
        });

        self.show_audio_dialog(ctx);
    }
}
