use eframe::egui;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

// --------------------------------------------------
// Настройки и модели
// --------------------------------------------------

const SETTINGS_KEY: &str = "video_loader_settings_v1";

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Downloads,
    Settings,
}

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
enum Quality {
    Best,
    H1080,
    H720,
    H480,
    AudioMp3,
}

impl Quality {
    fn label(self) -> &'static str {
        match self {
            Self::Best => "Максимальное качество",
            Self::H1080 => "1080p или ниже",
            Self::H720 => "720p или ниже",
            Self::H480 => "480p или ниже",
            Self::AudioMp3 => "Только MP3",
        }
    }

    fn height_filter(self) -> &'static str {
        match self {
            Self::H1080 => "[height<=1080]",
            Self::H720 => "[height<=720]",
            Self::H480 => "[height<=480]",
            _ => "",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
enum VideoContainer {
    Mkv,
    Mp4,
    Webm,
}

impl VideoContainer {
    fn label(self) -> &'static str {
        match self {
            Self::Mkv => "MKV",
            Self::Mp4 => "MP4",
            Self::Webm => "WebM",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Mkv => "mkv",
            Self::Mp4 => "mp4",
            Self::Webm => "webm",
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
struct SavedSettings {
    output_dir: String,
    quality: Quality,
    container: VideoContainer,
    use_browser_cookies: bool,
    browser: String,
    download_playlists: bool,
}

impl Default for SavedSettings {
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
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Status {
    Waiting,
    Downloading,
    Completed,
    Failed,
    Cancelled,
}

impl Status {
    fn label(self) -> &'static str {
        match self {
            Self::Waiting => "В очереди",
            Self::Downloading => "Загрузка / обработка",
            Self::Completed => "Завершено",
            Self::Failed => "Ошибка",
            Self::Cancelled => "Отменено",
        }
    }

    fn active(self) -> bool {
        matches!(self, Self::Waiting | Self::Downloading)
    }
}

#[derive(Clone)]
struct AudioOption {
    id: String,
    label: String,
    webm_compatible: bool,
}

struct AudioResult {
    url: String,
    result: Result<Vec<AudioOption>, String>,
}

struct DownloadItem {
    id: u64,
    url: String,
    quality: Quality,
    container: VideoContainer,
    download_playlist: bool,

    audio_id: Option<String>,
    audio_label: String,
    browser: Option<String>,
    output_dir: PathBuf,

    status: Status,
    percent: f32,
    speed: String,
    eta: String,
    message: String,
    logs: Vec<String>,

    running: Option<RunningDownload>,
}

struct RunningDownload {
    child: Child,
    lines: Receiver<String>,
    readers: Vec<thread::JoinHandle<()>>,
    finished: bool,
}

struct App {
    page: Page,
    settings: SavedSettings,
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

impl Default for App {
    fn default() -> Self {
        Self {
            page: Page::Downloads,
            settings: SavedSettings::default(),
            urls: String::new(),
            notice: String::new(),

            folder_receiver: None,

            audio_options: Vec::new(),
            selected_audio: None,
            temporary_audio_id: None,
            audio_receiver: None,
            audio_status: String::new(),
            show_audio_window: false,

            next_id: 1,
            items: Vec::new(),

            dependency_receiver: None,
            dependencies: None,
        }
    }
}

// --------------------------------------------------
// Создание приложения и добавление задач
// --------------------------------------------------

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut app = Self::default();

        app.settings = cc
            .storage
            .and_then(|storage| eframe::get_value(storage, SETTINGS_KEY))
            .unwrap_or_default();

        if app.settings.output_dir.trim().is_empty() {
            app.settings.output_dir = SavedSettings::default().output_dir;
        }

        if !["firefox", "chrome", "chromium", "edge", "brave"]
            .contains(&app.settings.browser.as_str())
        {
            app.settings.browser = "firefox".to_owned();
        }

        app
    }

    fn browser_setting(&self) -> Option<String> {
        self.settings
            .use_browser_cookies
            .then(|| self.settings.browser.clone())
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

        if !valid_url(&url) {
            self.notice = "Вставьте ссылку с http:// или https://.".to_owned();
            return;
        }

        let browser = self.browser_setting();
        let (sender, receiver) = mpsc::channel();

        self.audio_options.clear();
        self.temporary_audio_id = None;
        self.audio_receiver = Some(receiver);
        self.audio_status = "Получаем список аудиодорожек...".to_owned();
        self.show_audio_window = true;
        self.notice.clear();

        thread::spawn(move || {
            let result = load_audio_options(&url, browser.as_deref());

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

                    Err(error) => self.audio_status = error,
                }
            }

            Err(mpsc::TryRecvError::Empty) => {}

            Err(mpsc::TryRecvError::Disconnected) => {
                self.audio_receiver = None;
                self.audio_status = "Получение аудиодорожек завершилось без результата.".to_owned();
            }
        }
    }

    fn add_downloads(&mut self) {
        if self.audio_receiver.is_some() {
            self.notice = "Дождитесь получения аудиодорожек.".to_owned();
            return;
        }

        let urls: Vec<String> = self
            .urls
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect();

        if urls.is_empty() {
            self.notice = "Вставьте хотя бы одну ссылку.".to_owned();
            return;
        }

        let invalid_lines: Vec<String> = self
            .urls
            .lines()
            .enumerate()
            .filter_map(|(index, line)| {
                let line = line.trim();

                if line.is_empty() || valid_url(line) {
                    None
                } else {
                    Some((index + 1).to_string())
                }
            })
            .collect();

        if !invalid_lines.is_empty() {
            self.notice = format!(
                "Неверные ссылки в строках: {}. \
                 Используйте адреса с http:// или https://, \
                 по одному на строку.",
                invalid_lines.join(", ")
            );
            return;
        }

        let multiple = urls.len() > 1;

        if (multiple || self.settings.download_playlists) && self.selected_audio.is_some() {
            self.notice = "Для группы ссылок и плейлистов нужна \
                 автоматическая озвучка. Нажмите «Сбросить»."
                .to_owned();
            return;
        }

        if self.settings.quality != Quality::AudioMp3
            && self.settings.container == VideoContainer::Webm
        {
            if let Some(audio) = &self.selected_audio {
                if !audio.webm_compatible {
                    self.notice = "Выбранная дорожка не подходит для WebM. \
                         Выберите другую, автоматическую озвучку \
                         или контейнер MKV."
                        .to_owned();
                    return;
                }
            }
        }

        let output_dir = match resolve_output_dir(&self.settings.output_dir) {
            Ok(path) => path,

            Err(error) => {
                self.notice = error;
                return;
            }
        };

        let browser = self.browser_setting();

        let audio_id = self.selected_audio.as_ref().map(|audio| audio.id.clone());

        let audio_label = self
            .selected_audio
            .as_ref()
            .map(|audio| audio.label.clone())
            .unwrap_or_else(|| "Автоматически".to_owned());

        let mut seen = HashSet::new();
        let mut added = 0;
        let mut skipped = 0;

        for url in urls {
            if !seen.insert(url.clone()) {
                skipped += 1;
                continue;
            }

            let already_active = self.items.iter().any(|item| {
                item.url == url
                    && item.download_playlist == self.settings.download_playlists
                    && item.status.active()
            });

            if already_active {
                skipped += 1;
                continue;
            }

            self.items.push(DownloadItem {
                id: self.next_id,
                url,
                quality: self.settings.quality,
                container: self.settings.container,
                download_playlist: self.settings.download_playlists,

                audio_id: audio_id.clone(),
                audio_label: audio_label.clone(),
                browser: browser.clone(),
                output_dir: output_dir.clone(),

                status: Status::Waiting,
                percent: 0.0,
                speed: String::new(),
                eta: String::new(),
                message: "Ожидает запуска.".to_owned(),
                logs: Vec::new(),

                running: None,
            });

            self.next_id += 1;
            added += 1;
        }

        self.urls.clear();
        self.reset_audio();
        self.show_audio_window = false;

        self.notice = if skipped > 0 {
            format!(
                "Добавлено задач: {added}. \
                 Пропущено повторных или активных ссылок: {skipped}."
            )
        } else {
            format!("Добавлено задач: {added}.")
        };
    }
}

// --------------------------------------------------
// Очередь и отмена
// --------------------------------------------------

impl App {
    fn poll_downloads(&mut self) {
        for item in &mut self.items {
            let Some(running) = item.running.as_mut() else {
                continue;
            };

            for line in running.lines.try_iter() {
                update_progress(&line, &mut item.percent, &mut item.speed, &mut item.eta);

                item.message = line.clone();
                item.logs.push(line);
            }

            trim_logs(&mut item.logs);

            let completion = match running.child.try_wait() {
                Ok(Some(status)) => {
                    let readers_finished =
                        running.readers.iter().all(|reader| reader.is_finished());

                    if readers_finished {
                        running.finished = true;

                        Some(if status.success() {
                            Ok(())
                        } else {
                            Err(format!(
                                "yt-dlp завершился с ошибкой: {status}. \
                                 Подробности в журнале."
                            ))
                        })
                    } else {
                        None
                    }
                }

                Ok(None) => None,

                Err(error) => Some(Err(format!("Ошибка проверки процесса: {error}"))),
            };

            if let Some(result) = completion {
                if let Some(running) = item.running.as_ref() {
                    item.logs.extend(running.lines.try_iter());
                }

                trim_logs(&mut item.logs);
                item.running = None;

                match result {
                    Ok(()) => {
                        item.status = Status::Completed;
                        item.percent = 1.0;

                        item.message = if item.download_playlist {
                            "Обработка плейлиста завершена. \
                             Проверьте журнал."
                                .to_owned()
                        } else {
                            "Файл сохранён.".to_owned()
                        };
                    }

                    Err(error) => {
                        item.status = Status::Failed;
                        item.message = error;
                    }
                }
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
            match RunningDownload::start(item) {
                Ok(running) => {
                    item.running = Some(running);
                    item.status = Status::Downloading;
                    item.message = "Получение информации и запуск загрузки...".to_owned();
                }

                Err(error) => {
                    item.status = Status::Failed;
                    item.message = error.clone();
                    item.logs.push(error);
                }
            }
        }
    }

    fn cancel(&mut self, id: u64) {
        let Some(item) = self.items.iter_mut().find(|item| item.id == id) else {
            return;
        };

        if let Some(running) = item.running.as_mut() {
            if let Err(error) = running.stop() {
                item.message = error.clone();
                item.logs.push(error);
                return;
            }

            item.logs.extend(running.lines.try_iter());
        }

        item.running = None;
        item.status = Status::Cancelled;
        item.message = "Отменено. Скачанные и незавершённые файлы оставлены.".to_owned();
    }
}

// --------------------------------------------------
// Папки и проверка зависимостей
// --------------------------------------------------

impl App {
    fn browse_output_folder(&mut self) {
        if self.folder_receiver.is_some() {
            return;
        }

        let initial_dir = resolve_output_dir(&self.settings.output_dir)
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
        let directory = match resolve_output_dir(&self.settings.output_dir) {
            Ok(path) => path,

            Err(error) => {
                self.notice = error;
                return;
            }
        };

        if let Err(error) = std::fs::create_dir_all(&directory) {
            self.notice = format!("Не удалось создать папку: {error}");
            return;
        }

        #[cfg(target_os = "linux")]
        let mut command = Command::new("xdg-open");

        #[cfg(target_os = "windows")]
        let mut command = Command::new("explorer.exe");

        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            match command
                .arg(directory)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(mut child) => {
                    thread::spawn(move || {
                        let _ = child.wait();
                    });
                }

                Err(error) => {
                    self.notice = format!("Не удалось открыть папку: {error}");
                }
            }
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
                check_command("yt-dlp", "--version"),
                check_command("ffmpeg", "-version"),
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

// --------------------------------------------------
// Главный интерфейс
// --------------------------------------------------

impl eframe::App for App {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, SETTINGS_KEY, &self.settings);
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

        egui::SidePanel::left("sidebar")
            .exact_width(200.0)
            .show(ctx, |ui| {
                ui.add_space(16.0);
                ui.heading("VideoLoader");
                ui.add_space(20.0);

                ui.selectable_value(&mut self.page, Page::Downloads, "Загрузки");

                ui.selectable_value(&mut self.page, Page::Settings, "Настройки");

                ui.add_space(20.0);

                if ui.button("Открыть папку").clicked() {
                    self.open_folder();
                }

                if ui.button("Очистить завершённые").clicked() {
                    self.items.retain(|item| item.status.active());
                }

                let waiting = self
                    .items
                    .iter()
                    .filter(|item| item.status == Status::Waiting)
                    .count();

                let completed = self
                    .items
                    .iter()
                    .filter(|item| item.status == Status::Completed)
                    .count();

                ui.add_space(20.0);
                ui.small(format!("Всего задач: {}", self.items.len()));
                ui.small(format!("Ожидают: {waiting}"));
                ui.small(format!("Завершены: {completed}"));
            });

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

impl App {
    fn show_downloads(&mut self, ui: &mut egui::Ui) {
        ui.heading("Новые загрузки");
        ui.add_space(12.0);

        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label("Ссылки — каждая с новой строки:");

            let response = ui.add_enabled(
                self.audio_receiver.is_none(),
                egui::TextEdit::multiline(&mut self.urls)
                    .hint_text(
                        "https://youtu.be/...\n\
                         https://www.youtube.com/playlist?list=...\n\
                         https://vkvideo.ru/video...",
                    )
                    .desired_rows(5)
                    .desired_width(f32::INFINITY),
            );

            if response.changed() {
                self.reset_audio();
                self.show_audio_window = false;
            }

            let link_count = self
                .urls
                .lines()
                .filter(|line| !line.trim().is_empty())
                .count();

            ui.small(format!("Строк со ссылками: {link_count}"));
            ui.add_space(10.0);

            let playlist_mode_changed = ui
                .add_enabled(
                    self.audio_receiver.is_none(),
                    egui::Checkbox::new(
                        &mut self.settings.download_playlists,
                        "Скачивать плейлисты целиком",
                    ),
                )
                .changed();

            if playlist_mode_changed {
                self.reset_audio();
                self.show_audio_window = false;
            }

            if self.settings.download_playlists {
                ui.colored_label(
                    egui::Color32::from_rgb(240, 180, 90),
                    "Режим плейлистов включён: могут быть скачаны \
                     все доступные видео списка.",
                );

                ui.small(
                    "Один плейлист — одна задача. \
                     Озвучка автоматическая. \
                     Для первого теста возьмите небольшой список.",
                );
            } else {
                ui.small(
                    "Для ссылки на видео с параметром плейлиста \
                     скачивается только выбранное видео.",
                );
            }

            ui.add_space(10.0);

            ui.horizontal_wrapped(|ui| {
                ui.label("Качество:");

                for quality in [
                    Quality::Best,
                    Quality::H1080,
                    Quality::H720,
                    Quality::H480,
                    Quality::AudioMp3,
                ] {
                    ui.selectable_value(&mut self.settings.quality, quality, quality.label());
                }
            });

            ui.add_space(10.0);

            ui.add_enabled_ui(self.settings.quality != Quality::AudioMp3, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label("Контейнер:");

                    for container in [
                        VideoContainer::Mkv,
                        VideoContainer::Mp4,
                        VideoContainer::Webm,
                    ] {
                        ui.selectable_value(
                            &mut self.settings.container,
                            container,
                            container.label(),
                        );
                    }
                });
            });

            if self.settings.quality == Quality::AudioMp3 {
                ui.small("Итоговый аудиофайл: MP3.");
            } else {
                ui.small(match self.settings.container {
                    VideoContainer::Mkv => "MKV: объединение без перекодирования.",

                    VideoContainer::Mp4 => {
                        "MP4: перепаковка без перекодирования. \
                         Совместимость плеера зависит от кодеков."
                    }

                    VideoContainer::Webm => {
                        "WebM: нужны совместимые видео- и аудиопотоки. \
                         Если их нет, выберите MKV."
                    }
                });
            }

            ui.add_space(10.0);

            let audio_label = self
                .selected_audio
                .as_ref()
                .map(|audio| audio.label.as_str())
                .unwrap_or("Автоматически")
                .to_owned();

            ui.horizontal_wrapped(|ui| {
                ui.label(format!("Озвучка: {audio_label}"));

                if ui
                    .add_enabled(
                        self.audio_receiver.is_none()
                            && link_count == 1
                            && !self.settings.download_playlists,
                        egui::Button::new("Выбрать озвучку"),
                    )
                    .clicked()
                {
                    self.start_audio_selection();
                }

                if self.selected_audio.is_some() && ui.button("Сбросить").clicked() {
                    self.reset_audio();
                }
            });

            if link_count > 1 || self.settings.download_playlists {
                ui.small(
                    "Для группы и плейлистов озвучка автоматическая. \
                     Качество, контейнер и папка общие.",
                );
            }

            if self.audio_receiver.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Получаем аудиодорожки...");
                });

                if ui.button("Показать окно озвучки").clicked() {
                    self.show_audio_window = true;
                }
            }

            ui.add_space(12.0);

            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        link_count > 0 && self.audio_receiver.is_none(),
                        egui::Button::new(format!("Добавить в очередь ({link_count})")),
                    )
                    .clicked()
                {
                    self.add_downloads();
                }

                if ui
                    .add_enabled(
                        !self.urls.is_empty() && self.audio_receiver.is_none(),
                        egui::Button::new("Очистить поле"),
                    )
                    .clicked()
                {
                    self.urls.clear();
                    self.reset_audio();
                    self.show_audio_window = false;
                }
            });
        });

        ui.add_space(18.0);
        ui.heading("Очередь");
        ui.small(
            "Задачи выполняются последовательно. \
             Видео внутри плейлиста тоже скачиваются по порядку.",
        );
        ui.add_space(10.0);

        self.show_queue(ui);
    }

    fn show_queue(&mut self, ui: &mut egui::Ui) {
        let mut cancel_id = None;

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if self.items.is_empty() {
                    ui.label("Пока нет загрузок.");
                }

                for item in &self.items {
                    ui.push_id(item.id, |ui| {
                        egui::Frame::group(ui.style()).show(ui, |ui| {
                            ui.set_min_width(ui.available_width());

                            ui.label(egui::RichText::new(&item.url).strong());

                            ui.horizontal_wrapped(|ui| {
                                ui.label(item.status.label());
                                ui.separator();

                                ui.label(if item.download_playlist {
                                    "Режим: плейлист"
                                } else {
                                    "Режим: отдельное видео"
                                });

                                ui.separator();
                                ui.label(item.quality.label());
                                ui.separator();

                                ui.label(if item.quality == Quality::AudioMp3 {
                                    "MP3"
                                } else {
                                    item.container.label()
                                });
                            });

                            ui.label(format!("Озвучка: {}", item.audio_label));

                            if item.status == Status::Downloading {
                                ui.add_space(8.0);

                                ui.add(egui::ProgressBar::new(item.percent).show_percentage());

                                if item.download_playlist {
                                    ui.small(
                                        "Прогресс текущего потока, \
                                         не всего плейлиста.",
                                    );
                                }

                                ui.horizontal_wrapped(|ui| {
                                    if !item.speed.is_empty() {
                                        ui.label(format!("Скорость: {}", item.speed));
                                    }

                                    if !item.eta.is_empty() {
                                        ui.label(format!("Осталось: {}", item.eta));
                                    }
                                });
                            }

                            ui.label(&item.message);

                            if item.status.active() && ui.button("Отменить").clicked() {
                                cancel_id = Some(item.id);
                            }

                            ui.collapsing("Журнал", |ui| {
                                egui::ScrollArea::vertical()
                                    .max_height(180.0)
                                    .stick_to_bottom(true)
                                    .show(ui, |ui| {
                                        for line in &item.logs {
                                            ui.label(egui::RichText::new(line).monospace());
                                        }
                                    });
                            });
                        });
                    });

                    ui.add_space(10.0);
                }
            });

        if let Some(id) = cancel_id {
            self.cancel(id);
        }
    }

    fn show_audio_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_audio_window {
            return;
        }

        let mut open = true;
        let mut apply = false;
        let mut cancel = false;

        egui::Window::new("Выбор озвучки")
            .open(&mut open)
            .default_width(700.0)
            .show(ctx, |ui| {
                if self.audio_receiver.is_some() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Получаем список аудиодорожек...");
                    });
                    return;
                }

                egui::ScrollArea::vertical()
                    .id_salt("audio_status")
                    .max_height(140.0)
                    .show(ui, |ui| {
                        ui.label(&self.audio_status);
                    });

                ui.add_space(10.0);

                ui.small(
                    "Один язык может иметь несколько вариантов \
                     качества и кодека. Для WebM выбирайте \
                     совместимую дорожку.",
                );

                ui.add_space(10.0);

                egui::ScrollArea::vertical()
                    .id_salt("audio_options")
                    .max_height(330.0)
                    .show(ui, |ui| {
                        ui.radio_value(&mut self.temporary_audio_id, None, "Автоматически");

                        for option in &self.audio_options {
                            ui.radio_value(
                                &mut self.temporary_audio_id,
                                Some(option.id.clone()),
                                &option.label,
                            );
                        }
                    });

                ui.add_space(12.0);

                ui.horizontal(|ui| {
                    if ui.button("Применить").clicked() {
                        apply = true;
                    }

                    if ui.button("Отмена").clicked() {
                        cancel = true;
                    }
                });
            });

        if apply {
            self.selected_audio = self
                .temporary_audio_id
                .as_ref()
                .and_then(|id| self.audio_options.iter().find(|option| &option.id == id))
                .cloned();
        }

        self.show_audio_window = open && !apply && !cancel;
    }

    fn show_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Настройки");
        ui.add_space(16.0);
        ui.label("Папка сохранения:");

        let choosing = self.folder_receiver.is_some();

        ui.horizontal(|ui| {
            let width = (ui.available_width() - 130.0).max(100.0);

            ui.add_enabled(
                !choosing,
                egui::TextEdit::singleline(&mut self.settings.output_dir).desired_width(width),
            );

            if ui
                .add_enabled(!choosing, egui::Button::new("Обзор…"))
                .clicked()
            {
                self.browse_output_folder();
            }

            if choosing {
                ui.spinner();
            }
        });

        ui.small("Новая папка применяется к следующим добавленным задачам.");

        ui.add_space(16.0);
        ui.separator();
        ui.heading("Сессия браузера");

        let mut changed = false;

        ui.add_enabled_ui(self.audio_receiver.is_none(), |ui| {
            changed |= ui
                .checkbox(
                    &mut self.settings.use_browser_cookies,
                    "Использовать cookies браузера",
                )
                .changed();

            ui.add_enabled_ui(self.settings.use_browser_cookies, |ui| {
                egui::ComboBox::from_label("Браузер")
                    .selected_text(&self.settings.browser)
                    .show_ui(ui, |ui| {
                        for browser in ["firefox", "chrome", "chromium", "edge", "brave"] {
                            changed |= ui
                                .selectable_value(
                                    &mut self.settings.browser,
                                    browser.to_owned(),
                                    browser,
                                )
                                .changed();
                        }
                    });
            });
        });

        if changed {
            self.reset_audio();
            self.show_audio_window = false;
        }

        ui.small(
            "Cookies читает yt-dlp из локального браузера. \
             Сами cookies в настройках не сохраняются.",
        );

        ui.add_space(16.0);
        ui.separator();
        ui.heading("Подключение");

        ui.label("Используется окружение, из которого запущено приложение.");

        ui.small(
            "Адрес Hiddify не прописан в программе. \
             Запуск из меню рабочего стола может иметь \
             другое окружение, чем запуск из терминала.",
        );

        ui.add_space(16.0);
        ui.separator();

        if ui
            .add_enabled(
                self.dependency_receiver.is_none(),
                egui::Button::new("Проверить yt-dlp и FFmpeg"),
            )
            .clicked()
        {
            self.check_dependencies();
        }

        if self.dependency_receiver.is_some() {
            ui.spinner();
        }

        if let Some((yt_dlp, ffmpeg)) = self.dependencies {
            ui.label(format!(
                "yt-dlp: {}",
                if yt_dlp {
                    "найден"
                } else {
                    "не найден"
                }
            ));

            ui.label(format!(
                "FFmpeg: {}",
                if ffmpeg {
                    "найден"
                } else {
                    "не найден"
                }
            ));
        }

        ui.add_space(16.0);
        ui.small(
            "Настройки сохраняются автоматически \
             и при штатном закрытии приложения.",
        );

        ui.small("Очередь загрузок пока не сохраняется между запусками.");
    }
}

// --------------------------------------------------
// Ссылки и аудиодорожки
// --------------------------------------------------

fn valid_url(url: &str) -> bool {
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

fn field<'a>(value: &'a Value, name: &str) -> &'a str {
    value.get(name).and_then(Value::as_str).unwrap_or("")
}

fn load_audio_options(url: &str, browser: Option<&str>) -> Result<Vec<AudioOption>, String> {
    let mut command = Command::new("yt-dlp");

    command.args([
        "--ignore-config",
        "--no-playlist",
        "--skip-download",
        "--dump-single-json",
        "--socket-timeout",
        "30",
        "--extractor-retries",
        "1",
    ]);

    if let Some(browser) = browser {
        command.arg("--cookies-from-browser").arg(browser);
    }

    hide_console(&mut command);

    let output = command
        .arg("--")
        .arg(url)
        .stdin(Stdio::null())
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

    let data: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("Ошибка разбора JSON: {error}"))?;

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

// --------------------------------------------------
// Селектор форматов
// --------------------------------------------------

fn build_format(quality: Quality, audio_id: Option<&str>, container: VideoContainer) -> String {
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

// --------------------------------------------------
// Запуск загрузки
// --------------------------------------------------

impl RunningDownload {
    fn start(item: &DownloadItem) -> Result<Self, String> {
        std::fs::create_dir_all(&item.output_dir)
            .map_err(|error| format!("Не удалось создать папку: {error}"))?;

        let mut command = Command::new("yt-dlp");

        command.args([
            "--ignore-config",
            "--newline",
            "--no-colors",
            "--progress",
            "--socket-timeout",
            "30",
            "--extractor-retries",
            "1",
        ]);

        if item.download_playlist {
            command.args(["--yes-playlist", "--no-abort-on-error"]);
        } else {
            command.arg("--no-playlist");
        }

        if let Some(browser) = item.browser.as_deref() {
            command.arg("--cookies-from-browser").arg(browser);
        }

        if item.quality == Quality::AudioMp3 {
            command.args(["-x", "--audio-format", "mp3", "--audio-quality", "0"]);
        } else {
            command
                .arg("--merge-output-format")
                .arg(item.container.extension())
                .arg("--remux-video")
                .arg(item.container.extension());
        }

        let output_template = if item.download_playlist {
            "%(playlist_title|Плейлист)s/\
             %(playlist_index|0)03d - %(title)s [%(id)s].%(ext)s"
        } else {
            "%(title)s [%(id)s].%(ext)s"
        };

        command
            .arg("-f")
            .arg(build_format(
                item.quality,
                item.audio_id.as_deref(),
                item.container,
            ))
            .arg("-P")
            .arg(&item.output_dir)
            .arg("-o")
            .arg(output_template)
            .arg("--")
            .arg(&item.url)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        prepare_process(&mut command);

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

    fn stop(&mut self) -> Result<(), String> {
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

        if let Err(error) = kill_process_tree(self.child.id()) {
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

fn spawn_reader<R>(reader: R, sender: mpsc::Sender<String>) -> thread::JoinHandle<()>
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

fn trim_logs(logs: &mut Vec<String>) {
    if logs.len() > 500 {
        let excess = logs.len() - 500;
        logs.drain(..excess);
    }
}

fn update_progress(line: &str, percent: &mut f32, speed: &mut String, eta: &mut String) {
    if !line.starts_with("[download]") {
        return;
    }

    if line.contains("Destination:") || line.contains("Downloading item ") {
        *percent = 0.0;
        speed.clear();
        eta.clear();
    }

    if let Some(value) = line.split_whitespace().find_map(|part| {
        part.strip_suffix('%')
            .and_then(|text| text.parse::<f32>().ok())
    }) {
        *percent = (value / 100.0).clamp(0.0, 1.0);
    }

    if let Some(value) = line
        .split(" at ")
        .nth(1)
        .and_then(|part| part.split_whitespace().next())
    {
        *speed = value.to_owned();
    }

    if let Some(value) = line
        .split(" ETA ")
        .nth(1)
        .and_then(|part| part.split_whitespace().next())
    {
        *eta = value.to_owned();
    }
}

// --------------------------------------------------
// Платформенные функции
// --------------------------------------------------

fn hide_console(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }

    #[cfg(not(windows))]
    {
        let _ = command;
    }
}

fn prepare_process(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    hide_console(command);
}

fn kill_process_tree(process_id: u32) -> Result<(), String> {
    #[cfg(unix)]
    let result = Command::new("kill")
        .arg("-KILL")
        .arg("--")
        .arg(format!("-{process_id}"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    #[cfg(windows)]
    let result = {
        let mut command = Command::new("taskkill");
        hide_console(&mut command);

        command
            .arg("/PID")
            .arg(process_id.to_string())
            .args(["/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
    };

    #[cfg(any(unix, windows))]
    {
        let status = result.map_err(|error| format!("Не удалось остановить загрузку: {error}"))?;

        if status.success() {
            Ok(())
        } else {
            Err("Не удалось завершить дерево процессов загрузки.".to_owned())
        }
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = process_id;
        Err("Остановка не поддерживается на этой ОС.".to_owned())
    }
}

fn check_command(program: &str, argument: &str) -> bool {
    let mut command = Command::new(program);
    hide_console(&mut command);

    command
        .arg(argument)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn resolve_output_dir(input: &str) -> Result<PathBuf, String> {
    let input = input.trim();

    if input.is_empty() {
        return Err("Укажите папку сохранения в настройках.".to_owned());
    }

    let path = if input == "~" {
        dirs::home_dir().ok_or_else(|| "Не удалось определить домашнюю папку.".to_owned())?
    } else if let Some(relative) = input.strip_prefix("~/") {
        dirs::home_dir()
            .ok_or_else(|| "Не удалось определить домашнюю папку.".to_owned())?
            .join(relative)
    } else {
        PathBuf::from(input)
    };

    if path.is_absolute() {
        Ok(path)
    } else {
        std::env::current_dir()
            .map(|current| current.join(path))
            .map_err(|error| format!("Ошибка текущей папки: {error}"))
    }
}

// --------------------------------------------------
// Точка входа
// --------------------------------------------------

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("VideoLoader")
            .with_inner_size([1180.0, 940.0])
            .with_min_inner_size([850.0, 760.0]),
        ..Default::default()
    };

    eframe::run_native(
        "VideoLoader",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}
