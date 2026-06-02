use crate::extractor::{self, ArchiveEntry, ArchiveFormat};
use crate::formats;
use crate::ui::theme::Theme;
use eframe::egui;
use log::{error, info};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;

/// Application state
pub struct ArchiveExtractorApp {
    archive_path: Option<PathBuf>,
    archive_format: Option<ArchiveFormat>,
    archive_entries: Vec<ArchiveEntry>,
    destination_path: Option<PathBuf>,
    search_query: String,
    is_extracting: bool,
    extraction_progress: f32,
    extraction_status: String,
    extraction_handle: Option<thread::JoinHandle<()>>,
    progress_current: Arc<AtomicUsize>,
    progress_total: Arc<AtomicUsize>,
    cancel_flag: Arc<AtomicBool>,
    show_dark_theme: bool,
    status_message: String,
    destination_edit: String,
    is_encrypted: bool,
    password: String,
    password_error: bool,
    show_password: bool,
}

impl Default for ArchiveExtractorApp {
    fn default() -> Self {
        Self {
            archive_path: None,
            archive_format: None,
            archive_entries: Vec::new(),
            destination_path: None,
            search_query: String::new(),
            is_extracting: false,
            extraction_progress: 0.0,
            extraction_status: String::from("Ready"),
            extraction_handle: None,
            progress_current: Arc::new(AtomicUsize::new(0)),
            progress_total: Arc::new(AtomicUsize::new(0)),
            cancel_flag: Arc::new(AtomicBool::new(false)),
            show_dark_theme: true,
            status_message: String::from("Drop an archive file to begin"),
            destination_edit: String::new(),
            is_encrypted: false,
            password: String::new(),
            password_error: false,
            show_password: false,
        }
    }
}

impl ArchiveExtractorApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self::default()
    }

    fn set_archive(&mut self, path: PathBuf) {
        info!("Opening archive: {:?}", path);

        self.archive_path = Some(path.clone());
        self.archive_format = ArchiveFormat::detect(&path);
        self.archive_entries.clear();
        self.extraction_progress = 0.0;
        self.password.clear();
        self.password_error = false;
        self.is_encrypted = false;

        match self.archive_format {
            Some(format) => {
                // Check for encryption
                if format == ArchiveFormat::Zip {
                    self.is_encrypted = extractor::is_zip_encrypted(&path);
                }

                self.status_message = format!(
                    "Loaded {} · {}",
                    path.file_name().unwrap_or_default().to_string_lossy(),
                    formats::format_name(format)
                );

                if self.is_encrypted {
                    self.status_message.push_str(" (password protected)");
                }

                match extractor::list_archive(&path) {
                    Ok(entries) => {
                        self.archive_entries = entries;
                        self.status_message = format!(
                            "{} files · {}",
                            self.archive_entries.len(),
                            formats::format_size(self.total_size())
                        );
                    }
                    Err(e) => {
                        error!("Failed to list archive entries: {}", e);
                        self.status_message = format!("Error: {}", e);
                    }
                }
            }
            None => {
                self.status_message = format!("Unknown format: {}", path.display());
            }
        }

        // Set default destination
        if let Some(parent) = path.parent() {
            let mut dest = parent.to_path_buf();
            if let Some(name) = path.file_stem() {
                dest.push(name);
            }
            self.destination_path = Some(dest.clone());
            self.destination_edit = dest.display().to_string();
        }
    }

    fn total_size(&self) -> u64 {
        self.archive_entries.iter().map(|e| e.size).sum()
    }

    fn filtered_entries_owned(&self) -> Vec<&ArchiveEntry> {
        if self.search_query.is_empty() {
            return self.archive_entries.iter().collect();
        }
        let search_lower = self.search_query.to_lowercase();
        self.archive_entries
            .iter()
            .filter(|e| e.name.to_lowercase().contains(&search_lower))
            .collect()
    }

    fn start_extraction(&mut self) {
        if self.archive_path.is_none() || self.destination_path.is_none() {
            return;
        }

        let archive_path = self.archive_path.clone().unwrap();
        let dest_path = self.destination_path.clone().unwrap();
        let password = if self.is_encrypted && !self.password.is_empty() {
            Some(self.password.clone())
        } else if self.is_encrypted {
            return; // Need password
        } else {
            None
        };

        let progress_current = Arc::clone(&self.progress_current);
        let progress_total = Arc::clone(&self.progress_total);
        let cancel_flag = Arc::clone(&self.cancel_flag);

        self.is_extracting = true;
        self.extraction_progress = 0.0;
        self.extraction_status = String::from("Extracting...");
        self.cancel_flag.store(false, Ordering::Relaxed);
        self.progress_current.store(0, Ordering::Relaxed);
        self.progress_total.store(0, Ordering::Relaxed);
        self.password_error = false;

        info!("Starting extraction to {:?}", dest_path);

        let handle = thread::spawn(move || {
            let _ = extractor::extract_archive(
                &archive_path,
                &dest_path,
                progress_current,
                progress_total,
                cancel_flag,
                password.as_deref(),
            );
        });

        self.extraction_handle = Some(handle);
    }

    fn update_extraction_status(&mut self) {
        if !self.is_extracting {
            return;
        }

        let current = self.progress_current.load(Ordering::Relaxed);
        let total = self.progress_total.load(Ordering::Relaxed);

        if total > 0 {
            self.extraction_progress = (current as f32 / total as f32) * 100.0;
            self.extraction_status = format!("{} / {} files", current, total);
        }

        if let Some(handle) = &self.extraction_handle {
            if handle.is_finished() {
                self.is_extracting = false;
                self.extraction_handle = None;
                self.extraction_progress = 100.0;
                self.extraction_status = String::from("Done!");
                self.status_message = String::from("Extraction complete");
            }
        }
    }

    fn ui_header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Archive Extractor")
                    .size(22.0)
                    .color(egui::Color32::WHITE),
            );

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let theme_label = if self.show_dark_theme {
                    "Dark"
                } else {
                    "Light"
                };
                let btn = egui::Button::new(theme_label).min_size(egui::vec2(50.0, 24.0));
                if ui.add(btn).clicked() {
                    self.show_dark_theme = !self.show_dark_theme;
                    let theme = if self.show_dark_theme {
                        Theme::Dark
                    } else {
                        Theme::Light
                    };
                    theme.apply(ui.ctx());
                }
            });
        });
    }

    fn ui_main(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            ui.add_space(8.0);

            // Top section: Archive info and destination
            ui.horizontal(|ui| {
                // Left: Archive info
                ui.vertical(|ui| {
                    if let Some(ref path) = self.archive_path {
                        ui.label(
                            egui::RichText::new(
                                path.file_name().unwrap_or_default().to_string_lossy(),
                            )
                            .size(16.0)
                            .color(egui::Color32::WHITE),
                        );
                        if let Some(fmt) = self.archive_format {
                            let mut info_text = format!(
                                "{} {}",
                                formats::format_icon(fmt),
                                formats::format_name(fmt)
                            );
                            if self.is_encrypted {
                                info_text.push_str(" 🔒");
                            }
                            ui.label(
                                egui::RichText::new(info_text)
                                    .size(12.0)
                                    .color(egui::Color32::GRAY),
                            );
                        }
                    } else {
                        ui.label(
                            egui::RichText::new("No archive loaded")
                                .size(16.0)
                                .color(egui::Color32::GRAY),
                        );
                    }
                });

                ui.add_space(20.0);

                // Right: Destination (only show when archive is loaded and not extracting)
                if self.archive_path.is_some()
                    && !self.is_extracting
                    && self.extraction_progress < 100.0
                {
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new("Extract to:")
                                .size(11.0)
                                .color(egui::Color32::GRAY),
                        );

                        ui.horizontal(|ui| {
                            let text_edit = egui::TextEdit::singleline(&mut self.destination_edit)
                                .desired_width(250.0)
                                .font(egui::TextStyle::Monospace);
                            ui.add(text_edit);

                            if ui.button("Browse").clicked() {
                                if let Some(path) = rfd::FileDialog::new().pick_folder() {
                                    self.destination_path = Some(path.clone());
                                    self.destination_edit = path.display().to_string();
                                }
                            }
                        });

                        // Update destination when text changes
                        self.destination_path = Some(PathBuf::from(&self.destination_edit));
                    });
                }
            });

            ui.add_space(12.0);

            // Password section (only for encrypted archives)
            if self.is_encrypted && !self.is_extracting && self.extraction_progress < 100.0 {
                let bg = if self.password_error {
                    egui::Color32::from_rgba_premultiplied(80, 30, 30, 60)
                } else {
                    egui::Color32::from_rgba_premultiplied(70, 60, 20, 50)
                };
                let frame = egui::Frame::none()
                    .fill(bg)
                    .rounding(egui::Rounding::same(8.0))
                    .inner_margin(egui::Margin::symmetric(12.0, 10.0));
                frame.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("🔒").size(16.0));
                        let pass_label = if self.password_error {
                            "Password required (incorrect)"
                        } else {
                            "This archive is password protected"
                        };
                        let pass_color = if self.password_error {
                            egui::Color32::from_rgb(240, 120, 120)
                        } else {
                            egui::Color32::from_rgb(220, 190, 80)
                        };
                        ui.label(
                            egui::RichText::new(pass_label)
                                .size(13.0)
                                .color(pass_color),
                        );
                    });

                    ui.add_space(6.0);

                    ui.horizontal(|ui| {
                        let mut password_edit =
                            egui::TextEdit::singleline(&mut self.password)
                                .password(!self.show_password)
                                .desired_width(280.0)
                                .hint_text("Enter archive password");

                        if self.password_error {
                            password_edit = password_edit
                                .text_color_opt(Some(egui::Color32::from_rgb(255, 160, 160)));
                        }

                        ui.add(password_edit);

                        let toggle_label = if self.show_password { "🙈" } else { "👁️" };
                        let toggle_tip = if self.show_password { "Hide password" } else { "Show password" };
                        if ui.add(
                            egui::Button::new(toggle_label)
                                .min_size(egui::vec2(32.0, 24.0))
                                .rounding(egui::Rounding::same(4.0)),
                        )
                        .on_hover_text(toggle_tip)
                        .clicked()
                        {
                            self.show_password = !self.show_password;
                        }
                    });
                });
                ui.add_space(10.0);
            }

            // Progress bar (during extraction)
            if self.is_extracting || self.extraction_progress > 0.0 {
                egui::Frame::none()
                    .fill(egui::Color32::from_rgba_premultiplied(40, 50, 60, 40))
                    .rounding(egui::Rounding::same(8.0))
                    .inner_margin(egui::Margin::symmetric(12.0, 8.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.add_sized(
                                ui.available_size() - egui::vec2(if self.is_extracting { 90.0 } else { 0.0 }, 0.0),
                                egui::ProgressBar::new(self.extraction_progress / 100.0)
                                    .desired_width(ui.available_size().x)
                                    .show_percentage()
                                    .text(&self.extraction_status)
                                    .rounding(egui::Rounding::same(4.0)),
                            );

                            if self.is_extracting
                                && ui.add(
                                    egui::Button::new("✕ Cancel")
                                        .min_size(egui::vec2(80.0, 24.0))
                                        .rounding(egui::Rounding::same(4.0)),
                                )
                                .on_hover_text("Esc")
                                .clicked()
                            {
                                self.cancel_flag.store(true, Ordering::Relaxed);
                            }
                        });
                    });
                ui.add_space(10.0);
            }

            ui.add_space(8.0);

            // Action buttons
            ui.horizontal(|ui| {
                if self.archive_path.is_none() {
                    if ui.add(
                        egui::Button::new("📂 Open Archive")
                            .min_size(egui::vec2(130.0, 32.0))
                            .rounding(egui::Rounding::same(6.0)),
                    )
                    .on_hover_text("Ctrl+O")
                    .clicked()
                    {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Archive", formats::supported_extensions())
                            .pick_file()
                        {
                            self.set_archive(path);
                        }
                    }
                } else if !self.is_extracting && self.extraction_progress < 100.0 {
                    if ui.add(
                        egui::Button::new("📂 Change Archive")
                            .min_size(egui::vec2(130.0, 32.0))
                            .rounding(egui::Rounding::same(6.0)),
                    )
                    .on_hover_text("Ctrl+O")
                    .clicked()
                    {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Archive", formats::supported_extensions())
                            .pick_file()
                        {
                            self.set_archive(path);
                        }
                    }

                    ui.add_space(8.0);

                    if ui.add(
                        egui::Button::new("📁 Destination")
                            .min_size(egui::vec2(110.0, 32.0))
                            .rounding(egui::Rounding::same(6.0)),
                    )
                    .on_hover_text("Ctrl+D")
                    .clicked()
                    {
                        if let Some(path) = rfd::FileDialog::new().pick_folder() {
                            self.destination_path = Some(path.clone());
                            self.destination_edit = path.display().to_string();
                        }
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let needs_password = self.is_encrypted && self.password.is_empty();
                        let extract_label = if needs_password {
                            "🔒 Extract"
                        } else {
                            "▶ Extract"
                        };
                        let btn = egui::Button::new(
                            egui::RichText::new(extract_label)
                                .size(14.0)
                                .color(egui::Color32::WHITE),
                        )
                        .min_size(egui::vec2(110.0, 36.0))
                        .rounding(egui::Rounding::same(6.0))
                        .fill(if needs_password {
                            egui::Color32::from_rgb(80, 80, 90)
                        } else {
                            egui::Color32::from_rgb(60, 140, 80)
                        });

                        if ui.add(btn)
                            .on_hover_text(if needs_password {
                                "Enter password first"
                            } else {
                                "Ctrl+E"
                            })
                            .clicked()
                            && !needs_password
                        {
                            self.start_extraction();
                        }
                    });
                } else if self.extraction_progress >= 100.0 {
                    if ui.add(
                        egui::Button::new("📂 Open Another")
                            .min_size(egui::vec2(130.0, 32.0))
                            .rounding(egui::Rounding::same(6.0)),
                    )
                    .clicked()
                    {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Archive", formats::supported_extensions())
                            .pick_file()
                        {
                            self.set_archive(path);
                        }
                    }

                    ui.add_space(8.0);

                    if let Some(ref dest) = self.destination_path {
                        if ui.add(
                            egui::Button::new("📂 Open Destination")
                                .min_size(egui::vec2(140.0, 32.0))
                                .rounding(egui::Rounding::same(6.0)),
                        )
                        .clicked()
                        {
                            let _ = open::that(dest);
                        }
                    }
                }
            });

            ui.add_space(16.0);

            // File list header
            if !self.archive_entries.is_empty() {
                ui.horizontal(|ui| {
                    let total = self.archive_entries.len();
                    let showing = if self.search_query.is_empty() {
                        format!("Contents  ·  {} files", total)
                    } else {
                        let count = self.filtered_entries_owned().len();
                        format!("Contents  ·  {} / {} files", count, total)
                    };
                    ui.label(
                        egui::RichText::new(showing)
                            .size(13.0)
                            .color(egui::Color32::from_rgb(180, 180, 190)),
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let _search_resp = ui.add(
                            egui::TextEdit::singleline(&mut self.search_query)
                                .hint_text("🔍 Search files...")
                                .desired_width(200.0),
                        );

                        if !self.search_query.is_empty()
                            && ui.add(
                                egui::Button::new("✕")
                                    .min_size(egui::vec2(20.0, 20.0))
                                    .rounding(egui::Rounding::same(4.0)),
                            )
                            .clicked()
                        {
                            self.search_query.clear();
                        }
                    });
                });

                ui.add_space(6.0);

                // File list
                egui::ScrollArea::vertical()
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        let filtered = self.filtered_entries_owned();

                        if filtered.is_empty() {
                            ui.add_space(20.0);
                            ui.horizontal(|ui| {
                                ui.add_space(10.0);
                                ui.label(
                                    egui::RichText::new("🔍 No files match your search")
                                        .size(13.0)
                                        .color(egui::Color32::from_rgb(140, 140, 150)),
                                );
                            });
                            ui.add_space(10.0);
                        } else {
                            for (idx, entry) in filtered.iter().enumerate() {
                                let bg = if idx % 2 == 0 {
                                    egui::Color32::from_rgba_premultiplied(30, 30, 38, 80)
                                } else {
                                    egui::Color32::TRANSPARENT
                                };

                                egui::Frame::none()
                                    .fill(bg)
                                    .rounding(egui::Rounding::same(4.0))
                                    .inner_margin(egui::Margin::symmetric(8.0, 4.0))
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                egui::RichText::new(formats::file_icon(entry))
                                                    .size(14.0),
                                            );
                                            ui.label(
                                                egui::RichText::new(&entry.name)
                                                    .size(13.0)
                                                    .color(if entry.is_dir {
                                                        egui::Color32::from_rgb(160, 180, 210)
                                                    } else {
                                                        egui::Color32::WHITE
                                                    }),
                                            );
                                            ui.with_layout(
                                                egui::Layout::right_to_left(
                                                    egui::Align::Center,
                                                ),
                                                |ui| {
                                                    ui.label(
                                                        egui::RichText::new(formats::format_size(
                                                            entry.size,
                                                        ))
                                                        .size(11.0)
                                                        .color(egui::Color32::from_rgb(
                                                            120, 120, 130,
                                                        )),
                                                    );
                                                },
                                            );
                                        });
                                    });
                            }
                        }
                    });
            }
        });
    }

    fn ui_drop_zone(&mut self, ui: &mut egui::Ui) {
        ui.centered_and_justified(|ui| {
            egui::Frame::none()
                .fill(egui::Color32::from_rgba_premultiplied(35, 35, 45, 50))
                .rounding(egui::Rounding::same(16.0))
                .inner_margin(egui::Margin::symmetric(40.0, 30.0))
                .show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.add_space(20.0);

                        // Large archive icon
                        ui.label(
                            egui::RichText::new("📦")
                                .size(48.0)
                                .color(egui::Color32::from_rgb(150, 180, 220)),
                        );

                        ui.add_space(12.0);

                        ui.label(
                            egui::RichText::new("Drop an archive file here")
                                .size(20.0)
                                .color(egui::Color32::WHITE),
                        );

                        ui.add_space(4.0);

                        ui.label(
                            egui::RichText::new("or click to browse")
                                .size(14.0)
                                .color(egui::Color32::from_rgb(140, 140, 150)),
                        );

                        ui.add_space(16.0);

                        if ui.add(
                            egui::Button::new(
                                egui::RichText::new("📂 Browse Files")
                                    .size(14.0)
                                    .color(egui::Color32::WHITE),
                            )
                            .min_size(egui::vec2(140.0, 36.0))
                            .rounding(egui::Rounding::same(8.0))
                            .fill(egui::Color32::from_rgb(60, 100, 160)),
                        )
                        .on_hover_text("Ctrl+O")
                        .clicked()
                        {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("Archive", formats::supported_extensions())
                                .pick_file()
                            {
                                self.set_archive(path);
                            }
                        }

                        ui.add_space(16.0);

                        ui.label(
                            egui::RichText::new(
                                "Supported: ZIP, TAR, GZ, BZ2, XZ, RAR, 7z, ZST, BR, LZ4",
                            )
                            .size(12.0)
                            .color(egui::Color32::from_rgb(100, 100, 110)),
                        );

                        ui.add_space(10.0);
                    });
                });
        });
    }

    fn ui_footer(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(&self.status_message)
                    .size(11.0)
                    .color(egui::Color32::from_rgb(140, 140, 140)),
            );
        });
    }
}

impl eframe::App for ArchiveExtractorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.update_extraction_status();

        if self.is_extracting {
            ctx.request_repaint();
        }

        // Keyboard shortcuts
        let input = ctx.input(|i| i.clone());

        // Ctrl/Cmd+O: Open archive
        if input.key_pressed(egui::Key::O) && (input.modifiers.ctrl || input.modifiers.command) {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("Archive", formats::supported_extensions())
                .pick_file()
            {
                self.set_archive(path);
            }
        }

        // Ctrl/Cmd+D: Select destination
        if input.key_pressed(egui::Key::D)
            && (input.modifiers.ctrl || input.modifiers.command)
            && self.archive_path.is_some()
            && !self.is_extracting
        {
            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                self.destination_path = Some(path.clone());
                self.destination_edit = path.display().to_string();
            }
        }

        // Ctrl/Cmd+E: Extract
        if input.key_pressed(egui::Key::E)
            && (input.modifiers.ctrl || input.modifiers.command)
            && self.archive_path.is_some()
            && !self.is_extracting
        {
            self.start_extraction();
        }

        // Ctrl/Cmd+Q: Quit
        if input.key_pressed(egui::Key::Q) && (input.modifiers.ctrl || input.modifiers.command) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        // Escape: Cancel extraction
        if input.key_pressed(egui::Key::Escape) && self.is_extracting {
            self.cancel_flag.store(true, Ordering::Relaxed);
        }

        // Handle drag and drop
        if let Some(payload) = ctx.input(|i| i.raw.dropped_files.first().cloned()) {
            if let Some(path) = payload.path {
                if formats::is_supported_archive(&path) {
                    self.set_archive(path);
                } else {
                    self.status_message = String::from("Not a supported archive format");
                }
            }
        }

        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.add_space(8.0);
            self.ui_header(ui);
            ui.add_space(8.0);
        });

        egui::TopBottomPanel::bottom("bottom_panel").show(ctx, |ui| {
            ui.add_space(8.0);
            self.ui_footer(ui);
            ui.add_space(4.0);
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(12.0);
            if self.archive_path.is_some() {
                self.ui_main(ui);
            } else {
                self.ui_drop_zone(ui);
            }
        });
    }
}
