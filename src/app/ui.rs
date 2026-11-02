//! Отрисовка интерфейса. Логика находится в app.rs и других модулях.

use super::{App, CardAction, Page};
use crate::jobs::{MAX_HISTORY, SAVED_LOG_LINES};
use crate::models::*;
use crate::rules;
use eframe::egui;

impl App {
    pub(super) fn show_sidebar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("sidebar")
            .exact_width(210.0)
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

                let interrupted = self
                    .items
                    .iter()
                    .filter(|item| item.status == Status::Interrupted)
                    .count();

                if interrupted > 0
                    && ui
                        .button(format!("Возобновить всё ({interrupted})"))
                        .clicked()
                {
                    self.resume_all();
                }

                if ui.button("Очистить завершённые").clicked() {
                    self.items.retain(|item| item.status.pending());
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
                ui.small(format!("Прервано: {interrupted}"));
                ui.small(format!("Завершены: {completed}"));
            });
    }

    pub(super) fn show_downloads(&mut self, ui: &mut egui::Ui) {
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
                    "Один плейлист — одна задача. Озвучка автоматическая. \
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

            let webm_video = self.settings.quality != Quality::AudioMp3
                && self.settings.container == VideoContainer::Webm;

            ui.add_enabled(
                !webm_video,
                egui::Checkbox::new(&mut self.settings.embed_cover, "Встроить обложку в файл"),
            );

            ui.checkbox(
                &mut self.settings.save_cover,
                "Сохранить обложку отдельным файлом (jpg)",
            );

            if webm_video {
                ui.small("Для WebM встраивание обложки отключено.");
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
        ui.heading("Очередь и история");
        ui.small(
            "Задачи выполняются последовательно. Очередь сохраняется \
             между запусками; прерванные задачи нужно возобновить вручную.",
        );
        ui.add_space(10.0);

        self.show_queue(ui);
    }

    fn show_queue(&mut self, ui: &mut egui::Ui) {
        let mut action: Option<CardAction> = None;

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

                            ui.label(egui::RichText::new(&item.spec.url).strong());

                            ui.horizontal_wrapped(|ui| {
                                ui.label(item.status.label());
                                ui.separator();

                                ui.label(if item.spec.playlist {
                                    "Режим: плейлист"
                                } else {
                                    "Режим: отдельное видео"
                                });

                                ui.separator();
                                ui.label(item.spec.quality.label());
                                ui.separator();

                                ui.label(if item.spec.quality == Quality::AudioMp3 {
                                    "MP3"
                                } else {
                                    item.spec.container.label()
                                });
                            });

                            ui.label(format!("Озвучка: {}", item.audio_label));

                            ui.small(format!("Папка: {}", item.spec.output_dir.display()));

                            ui.small(match item.spec.proxy.as_deref() {
                                Some("") => "Подключение: напрямую",
                                Some(_) => "Подключение: через прокси",
                                None => "Подключение: по окружению",
                            });

                            ui.small(match (item.spec.embed_cover, item.spec.save_cover) {
                                (true, true) => "Обложка: встроить и сохранить файлом",
                                (true, false) => "Обложка: встроить",
                                (false, true) => "Обложка: отдельным файлом",
                                (false, false) => "Обложка: нет",
                            });

                            if item.spec.embed_cover && item.status == Status::Completed {
                                if item.cover_embedded {
                                    ui.small(
                                        "Встраивание обложки: \
                                         подтверждено журналом yt-dlp.",
                                    );
                                } else {
                                    ui.colored_label(
                                        egui::Color32::from_rgb(240, 120, 90),
                                        "Встраивание обложки не \
                                         подтверждено журналом yt-dlp.",
                                    );
                                }
                            }

                            if item.cover_problem {
                                ui.colored_label(
                                    egui::Color32::from_rgb(240, 180, 90),
                                    "В журнале есть предупреждение \
                                     про обложку.",
                                );
                            }

                            if item.status == Status::Downloading {
                                ui.add_space(8.0);

                                ui.add(egui::ProgressBar::new(item.percent).show_percentage());

                                if item.spec.playlist {
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

                            ui.horizontal_wrapped(|ui| {
                                if item.status.active() && ui.button("Отменить").clicked() {
                                    action = Some(CardAction::Cancel(item.id));
                                }

                                match item.status {
                                    Status::Interrupted => {
                                        if ui.button("Возобновить").clicked() {
                                            action = Some(CardAction::Resume(item.id));
                                        }
                                    }

                                    Status::Failed | Status::Cancelled => {
                                        if ui.button("Повторить").clicked() {
                                            action = Some(CardAction::Resume(item.id));
                                        }
                                    }

                                    _ => {}
                                }

                                if !item.status.active() && ui.button("Удалить из списка").clicked()
                                {
                                    action = Some(CardAction::Remove(item.id));
                                }
                            });

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

        match action {
            Some(CardAction::Cancel(id)) => self.cancel(id),
            Some(CardAction::Resume(id)) => self.resume(id),

            Some(CardAction::Remove(id)) => {
                self.items.retain(|item| item.id != id);
            }

            None => {}
        }
    }

    pub(super) fn show_audio_dialog(&mut self, ctx: &egui::Context) {
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
                    "Один язык может иметь несколько вариантов качества \
                     и кодека. Для WebM выбирайте совместимую дорожку.",
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

    pub(super) fn show_settings(&mut self, ui: &mut egui::Ui) {
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
                        for browser in BROWSERS {
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

        let mut proxy_changed = false;

        for (mode, label) in [
            (ProxyMode::Auto, "Авто: прокси только для YouTube"),
            (ProxyMode::Always, "Всегда через прокси"),
            (ProxyMode::Never, "Без прокси"),
        ] {
            proxy_changed |= ui
                .radio_value(&mut self.settings.proxy_mode, mode, label)
                .changed();
        }

        ui.add_enabled_ui(self.settings.proxy_mode != ProxyMode::Never, |ui| {
            ui.label("Адрес прокси:");

            proxy_changed |= ui
                .add(
                    egui::TextEdit::singleline(&mut self.settings.proxy_url)
                        .hint_text("socks5h://127.0.0.1:12334")
                        .desired_width(350.0),
                )
                .changed();
        });

        if proxy_changed {
            self.reset_audio();
            self.show_audio_window = false;
        }

        if !rules::valid_proxy(self.settings.proxy_url.trim()) {
            ui.colored_label(egui::Color32::from_rgb(240, 120, 90), rules::PROXY_ERROR);
        }

        ui.small(
            "В режиме «Авто» VK, Rutube и другие сайты идут напрямую. \
             Если у вас системный VPN, оставьте адрес пустым.",
        );

        ui.small(
            "Прямое подключение передаётся как --proxy \"\", поэтому \
             переменная ALL_PROXY не должна влиять на такие задачи.",
        );

        ui.add_space(16.0);
        ui.separator();
        ui.heading("Обложки");

        ui.small(
            "MKV: yt-dlp прикрепляет jpg как вложение. Не все плееры \
             и файловые менеджеры показывают его.",
        );

        ui.small(
            "MP4: yt-dlp пробует mutagen, AtomicParsley и ffmpeg. \
             Если встраивание не подтверждено, проверьте журнал.",
        );

        ui.small(
            "Проверка файла: ffprobe -hide_banner файл.mkv \
             (без -select_streams v: вложения иначе не видны).",
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
            "Настройки и очередь сохраняются автоматически \
             и при штатном закрытии приложения.",
        );

        ui.small(format!(
            "История хранит не более {MAX_HISTORY} завершённых задач; \
             для каждой — последние {SAVED_LOG_LINES} строк журнала."
        ));
    }
}
