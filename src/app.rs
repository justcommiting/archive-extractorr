use crate::extractor::{self, ArchiveEntry, ArchiveFormat};
use crate::formats;
use crate::ui::theme::Theme;
use eframe::egui;
use log::{error, info};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

#[cfg(feature = "gui")]
fn pick_file() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("Archive", formats::supported_extensions())
        .pick_file()
}

#[cfg(not(feature = "gui"))]
fn pick_file() -> Option<PathBuf> {
    log::warn!("File dialog unavailable (GUI feature disabled)");
    None
}

#[cfg(feature = "gui")]
fn pick_folder() -> Option<PathBuf> {
    rfd::FileDialog::new().pick_folder()
}

#[cfg(not(feature = "gui"))]
fn pick_folder() -> Option<PathBuf> {
    log::warn!("File dialog unavailable (GUI feature disabled)");
    None
}

#[cfg(feature = "gui")]
fn open_path(path: &Path) {
    let _ = open::that(path);
}

#[cfg(not(feature = "gui"))]
fn open_path(_path: &Path) {}

/// Result of loading an archive in a background thread
/// (encryption check + entry listing, off the UI thread).
struct LoadedArchive {
    is_encrypted: bool,
    entries: Vec<ArchiveEntry>,
}

#[derive(Default, Clone, Copy, Debug, PartialEq)]
pub enum SortBy {
    #[default]
    Name,
    Size,
    Type,
}

/// Precomputed sort keys to avoid per-comparison heap allocations when sorting archive entries.
struct SortKey {
    name_lower: String,
    ext_lower: String,
    size: u64,
    is_dir: bool,
}

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
    request_password_focus: bool,
    error_message: Arc<Mutex<Option<String>>>,
    sort_keys: Vec<SortKey>,
    sorted_indices: Vec<usize>,

    // Sorting state
    sort_by: SortBy,
    sort_ascending: bool,

    // Extraction metrics
    extraction_start_time: Option<std::time::Instant>,
    extraction_speed: String,
    extraction_elapsed: String,

    // Background loading: encryption check + listing off the UI thread
    is_loading: bool,
    loading_handle: Option<thread::JoinHandle<()>>,
    loading_result: Arc<Mutex<Option<anyhow::Result<LoadedArchive>>>>,

    // Generation counters for stale thread detection
    load_generation: Arc<AtomicU64>,
    extract_generation: Arc<AtomicU64>,
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
            request_password_focus: false,
            error_message: Arc::new(Mutex::new(None)),
            sort_keys: Vec::new(),
            sorted_indices: Vec::new(),
            sort_by: SortBy::Name,
            sort_ascending: true,
            extraction_start_time: None,
            extraction_speed: String::new(),
            extraction_elapsed: String::new(),
            is_loading: false,
            loading_handle: None,
            loading_result: Arc::new(Mutex::new(None)),
            load_generation: Arc::new(AtomicU64::new(0)),
            extract_generation: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl ArchiveExtractorApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self::default()
    }

    fn set_archive(&mut self, path: PathBuf) {
        info!("Opening archive: {:?}", path);

        // Extract what we need from the owned path before moving it into self.
        let format = ArchiveFormat::detect(&path);
        let file_name = path.file_name().unwrap_or_default().to_string_lossy().to_string();

        // Set default destination (reads self.archive_format, so set it first)
        self.archive_format = format;
        self.set_default_destination(&path);

        // Move path into self — saves 1 heap-allocated clone vs the original
        self.archive_path = Some(path);
        self.archive_entries.clear();
        self.sorted_indices.clear();
        self.extraction_start_time = None;
        self.extraction_progress = 0.0;
        self.password.clear();
        self.password_error = false;
        self.is_encrypted = false;
        self.request_password_focus = false;

        match format {
            Some(format) => {
                self.status_message = format!(
                    "Loading {} · {} …",
                    file_name,
                    formats::format_name(format)
                );

                // Spawn a background thread for encryption check + listing
                // so the UI stays responsive during I/O.
                let path_clone = self.archive_path.as_ref().unwrap().clone();
                let result = Arc::clone(&self.loading_result);
                let gen = self.load_generation.fetch_add(1, Ordering::Relaxed) + 1;
                let gen_arc = Arc::clone(&self.load_generation);
                self.is_loading = true;
                self.loading_handle = Some(thread::spawn(move || {
                    let is_encrypted = match format {
                        ArchiveFormat::Zip => extractor::is_zip_encrypted(&path_clone),
                        ArchiveFormat::Rar => extractor::is_rar_encrypted(&path_clone),
                        ArchiveFormat::SevenZip => extractor::is_sevenzip_encrypted(&path_clone),
                        _ => false,
                    };

                    let list_result = extractor::list_archive(&path_clone);
                    let outcome = match list_result {
                        Ok(entries) => Ok(LoadedArchive {
                            is_encrypted,
                            entries,
                        }),
                        Err(e) => {
                            let err_text = e.to_string();
                            let err_lower = err_text.to_lowercase();
                            let is_password_err = err_lower.contains("password")
                                || err_lower.contains("decrypt")
                                || err_lower.contains("badpassword")
                                || err_lower.contains("passwordrequired");

                            if is_password_err {
                                // Listing failed because the archive is password-protected.
                                Ok(LoadedArchive {
                                    is_encrypted: true,
                                    entries: Vec::new(),
                                })
                            } else {
                                Err(e)
                            }
                        }
                    };

                    if gen_arc.load(Ordering::Relaxed) == gen {
                        *result.lock().unwrap() = Some(outcome);
                    }
                }));
            }
            None => {
                self.status_message = format!("Unknown format: {}", file_name);
            }
        }
    }

    fn set_default_destination(&mut self, path: &Path) {
        if let Some(parent) = path.parent() {
            let mut dest = parent.to_path_buf();
            if let Some(format) = self.archive_format {
                if !format.is_single_file() {
                    if let Some(name) = path.file_stem() {
                        dest.push(name);
                    }
                }
            } else {
                if let Some(name) = path.file_stem() {
                    dest.push(name);
                }
            }
            self.destination_path = Some(dest.clone());
            self.destination_edit = dest.display().to_string();
        }
    }

    fn total_size(&self) -> u64 {
        self.archive_entries.iter().map(|e| e.size).sum()
    }

    fn rebuild_sort_keys(&mut self) {
        self.sort_keys = self
            .archive_entries
            .iter()
            .map(|e| SortKey {
                name_lower: e.name.to_lowercase(),
                ext_lower: e
                    .path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_lowercase(),
                size: e.size,
                is_dir: e.is_dir,
            })
            .collect();
    }

    fn filter_entries(&mut self) {
        if self.search_query.is_empty() {
            self.sort_entries();
        } else {
            let search_lower = self.search_query.to_lowercase();
            self.sorted_indices.retain(|&i| {
                self.sort_keys[i].name_lower.contains(&search_lower)
            });
        }
    }

    fn sort_entries(&mut self) {
        self.sorted_indices = (0..self.archive_entries.len()).collect();
        let sort_keys = &self.sort_keys;
        let sort_by = self.sort_by;
        let sort_ascending = self.sort_ascending;
        self.sorted_indices.sort_by(|&a, &b| {
            let ka = &sort_keys[a];
            let kb = &sort_keys[b];
            let ord = match sort_by {
                SortBy::Name => ka.name_lower.cmp(&kb.name_lower),
                SortBy::Size => ka.size.cmp(&kb.size),
                SortBy::Type => {
                    if ka.is_dir != kb.is_dir {
                        kb.is_dir.cmp(&ka.is_dir)
                    } else {
                        ka.ext_lower.cmp(&kb.ext_lower)
                    }
                }
            };
            if sort_ascending {
                ord
            } else {
                ord.reverse()
            }
        });
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
            self.password_error = true; // <-- ADDED: Show error if empty
            return; // Need password
        } else {
            None
        };

        let progress_current = Arc::clone(&self.progress_current);
        let progress_total = Arc::clone(&self.progress_total);
        let cancel_flag = Arc::clone(&self.cancel_flag);
        let error_message = Arc::clone(&self.error_message);

        self.is_extracting = true;
        self.extraction_progress = 0.0;
        self.extraction_status = String::from("Extracting...");
        self.cancel_flag.store(false, Ordering::Relaxed);
        self.progress_current.store(0, Ordering::Relaxed);
        self.progress_total.store(0, Ordering::Relaxed);
        self.password_error = false;
        self.extraction_start_time = Some(std::time::Instant::now());
        self.extraction_speed = String::new();
        self.extraction_elapsed = String::new();
        // Clear any previous errors
        *error_message.lock().unwrap() = None;

        info!("Starting extraction to {:?}", dest_path);

        let gen = self.extract_generation.fetch_add(1, Ordering::Relaxed) + 1;
        let gen_arc = Arc::clone(&self.extract_generation);

        let handle = thread::spawn(move || {
            let ctx = extractor::ExtractionContext {
                path: &archive_path,
                dest: &dest_path,
                progress: progress_current,
                total: progress_total,
                cancel_flag,
                password: password.as_deref(),
            };
            let result = extractor::extract_archive(&ctx);
            if let Err(e) = result {
                let err_str = format!("{:#}", e);
                error!("Extraction failed: {}", err_str);
                if gen_arc.load(Ordering::Relaxed) == gen {
                    *error_message.lock().unwrap() = Some(err_str);
                }
            }
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
            if self.archive_format.is_some_and(|f| f.is_single_file()) {
                self.extraction_status = format!(
                    "{} / {}",
                    formats::format_size(current as u64),
                    formats::format_size(total as u64)
                );
            } else {
                self.extraction_status = format!("{} / {} files ", current, total);
            }
        }

        if let Some(start_time) = self.extraction_start_time {
            let elapsed = start_time.elapsed();
            let elapsed_secs = elapsed.as_secs_f64();
            self.extraction_elapsed = format!("{:.1}s", elapsed_secs);

            if elapsed_secs > 0.1 && current > 0 {
                let speed = current as f64 / elapsed_secs;
                self.extraction_speed = format!("{:.1} files/s", speed);
            }
        }

        if let Some(handle) = &self.extraction_handle {
            if handle.is_finished() {
                self.is_extracting = false;
                self.extraction_handle = None;

                let total_time_str = if let Some(start_time) = self.extraction_start_time {
                    format!(" in {:.2}s", start_time.elapsed().as_secs_f64())
                } else {
                    String::new()
                };

                // Check if there was an error
                let err = self.error_message.lock().unwrap().take();
                if let Some(err_msg) = err {
                    let err_lower = err_msg.to_lowercase();
                    let is_password_err = err_lower.contains("password")
                        || err_lower.contains("invalid password")
                        || err_lower.contains("decrypt");

                    if is_password_err {
                        // Password error — allow retry
                        self.password_error = true;
                        self.extraction_progress = 0.0;
                        self.extraction_status = String::from("Extraction failed");
                        self.status_message = format!("✗ Error: {}", err_msg);
                        error!("Password error: {}", err_msg);
                    } else {
                        // Other error — show message but keep progress visible
                        self.extraction_progress = 100.0;
                        self.extraction_status = String::from("Failed");
                        self.status_message = format!("✗ Error: {}", err_msg);
                        error!("Extraction failed: {}", err_msg);
                    }
                } else {
                    // Success
                    self.extraction_progress = 100.0;
                    self.extraction_status = String::from("Done!");
                    self.status_message = format!("Extraction complete{}", total_time_str);
                }
            }
        }
    }

    /// Poll for completion of the background archive loading thread and
    /// merge the result (encryption status + entry list) into UI state.
    fn update_loading_status(&mut self) {
        if !self.is_loading {
            return;
        }

        if let Some(handle) = &self.loading_handle {
            if !handle.is_finished() {
                return;
            }
        }

        self.is_loading = false;
        self.loading_handle = None;

        let outcome = self.loading_result.lock().unwrap().take();
        match outcome {
            Some(Ok(loaded)) => {
                self.is_encrypted = loaded.is_encrypted;
                self.archive_entries = loaded.entries;
                self.rebuild_sort_keys();
                self.sort_entries();

                if self.archive_entries.is_empty() && !self.is_encrypted {
                    self.status_message = String::from("No files found in archive");
                } else if self.is_encrypted {
                    self.request_password_focus = true;
                    self.status_message = if self.archive_entries.is_empty() {
                        String::from("Archive is password protected")
                    } else {
                        format!(
                            "{} files · {}  (password protected)",
                            self.archive_entries.len(),
                            formats::format_size(self.total_size())
                        )
                    };
                } else {
                    self.status_message = format!(
                        "{} files · {} ",
                        self.archive_entries.len(),
                        formats::format_size(self.total_size())
                    );
                }
            }
            Some(Err(e)) => {
                let err_text = e.to_string();
                error!("Failed to load archive: {}", err_text);
                let err_lower = err_text.to_lowercase();
                if err_lower.contains("password")
                    || err_lower.contains("decrypt")
                    || err_lower.contains("badpassword")
                    || err_lower.contains("passwordrequired")
                {
                    self.is_encrypted = true;
                    self.request_password_focus = true;
                    self.status_message = String::from("Archive is password protected");
                } else {
                    self.status_message = format!("Error: {}", err_text);
                }
            }
            None => {
                // Thread finished but didn't set a result (shouldn't happen)
                self.status_message = String::from("Failed to load archive");
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
                                "{} {} ",
                                formats::format_icon(fmt),
                                formats::format_name(fmt)
                            );
                            if self.is_encrypted {
                                info_text.push_str(" [Encrypted] ");
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
                            let response = ui.add(text_edit);

                            if response.changed() {
                                self.destination_path = Some(PathBuf::from(&self.destination_edit));
                            }

                            if ui.button("Browse").clicked() {
                                if let Some(path) = pick_folder() {
                                    self.destination_path = Some(path.clone());
                                    self.destination_edit = path.display().to_string();
                                }
                            }
                        });
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
                    let pass_color = if self.password_error {
                        egui::Color32::from_rgb(240, 120, 120)
                    } else {
                        egui::Color32::from_rgb(220, 190, 80)
                    };
                    let pass_label = if self.password_error {
                        "Password required (incorrect)"
                    } else {
                        "This archive is password protected"
                    };

                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("[LOCKED] ")
                                .size(13.0)
                                .color(pass_color),
                        );
                        ui.label(egui::RichText::new(pass_label).size(13.0).color(pass_color));
                    });

                    ui.add_space(6.0);

                    ui.horizontal(|ui| {
                        // Generate a persistent ID for the text field so we can request focus
                        let password_id = ui.make_persistent_id("password_input");

                        // Auto-focus the password field when a new encrypted archive is loaded
                        if self.request_password_focus {
                            ui.memory_mut(|mem| mem.request_focus(password_id));
                            self.request_password_focus = false;
                        }

                        let mut password_edit = egui::TextEdit::singleline(&mut self.password)
                            .id(password_id)
                            .password(!self.show_password)
                            .desired_width(ui.available_width() - 80.0) // <-- IMPROVED: Responsive width
                            .hint_text("Enter archive password");

                        if self.password_error {
                            password_edit = password_edit
                                .text_color_opt(Some(egui::Color32::from_rgb(255, 160, 160)));
                        }

                        let response = ui.add(password_edit);

                        // Clear error state immediately when the user starts typing a new password
                        if response.changed() {
                            self.password_error = false;
                        }

                        // Allow pressing Enter to trigger extraction
                        if response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            self.start_extraction();
                        }

                        let toggle_label = if self.show_password { "Hide" } else { "Show" };
                        let toggle_tip = if self.show_password {
                            "Hide password"
                        } else {
                            "Show password"
                        };
                        if ui
                            .add(
                                egui::Button::new(toggle_label)
                                    .min_size(egui::vec2(60.0, 24.0))
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
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                ui.add_sized(
                                    ui.available_size()
                                        - egui::vec2(
                                            if self.is_extracting { 90.0 } else { 0.0 },
                                            0.0,
                                        ),
                                    egui::ProgressBar::new(self.extraction_progress / 100.0)
                                        .desired_width(ui.available_size().x)
                                        .show_percentage()
                                        .text(&self.extraction_status)
                                        .rounding(egui::Rounding::same(4.0)),
                                );

                                if self.is_extracting
                                    && ui
                                        .add(
                                            egui::Button::new("Cancel")
                                                .min_size(egui::vec2(80.0, 24.0))
                                                .rounding(egui::Rounding::same(4.0)),
                                        )
                                        .on_hover_text("Esc")
                                        .clicked()
                                {
                                    self.cancel_flag.store(true, Ordering::Relaxed);
                                }
                            });

                            if self.is_extracting {
                                ui.add_space(4.0);
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "Elapsed: {}",
                                            self.extraction_elapsed
                                        ))
                                        .size(11.0)
                                        .color(egui::Color32::GRAY),
                                    );
                                    ui.add_space(16.0);
                                    if !self.extraction_speed.is_empty() {
                                        ui.label(
                                            egui::RichText::new(format!(
                                                "Speed: {}",
                                                self.extraction_speed
                                            ))
                                            .size(11.0)
                                            .color(egui::Color32::GRAY),
                                        );
                                    }
                                });
                            }
                        });
                    });
                ui.add_space(10.0);
            }

            ui.add_space(8.0);

            // Action buttons
            ui.horizontal(|ui| {
                if self.archive_path.is_none() {
                    if ui
                        .add(
                            egui::Button::new("Open Archive")
                                .min_size(egui::vec2(130.0, 32.0))
                                .rounding(egui::Rounding::same(6.0)),
                        )
                        .on_hover_text("Ctrl+O")
                        .clicked()
                    {
                        if let Some(path) = pick_file() {
                            self.set_archive(path);
                        }
                    }
                } else if !self.is_extracting && self.extraction_progress < 100.0 {
                    if ui
                        .add(
                            egui::Button::new("Change Archive")
                                .min_size(egui::vec2(130.0, 32.0))
                                .rounding(egui::Rounding::same(6.0)),
                        )
                        .on_hover_text("Ctrl+O")
                        .clicked()
                    {
                        if let Some(path) = pick_file() {
                            self.set_archive(path);
                        }
                    }

                    ui.add_space(8.0);

                    if ui
                        .add(
                            egui::Button::new("Destination")
                                .min_size(egui::vec2(110.0, 32.0))
                                .rounding(egui::Rounding::same(6.0)),
                        )
                        .on_hover_text("Ctrl+D")
                        .clicked()
                    {
                        if let Some(path) = pick_folder() {
                            self.destination_path = Some(path.clone());
                            self.destination_edit = path.display().to_string();
                        }
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let needs_password = self.is_encrypted && self.password.is_empty();
                        let extract_label = if needs_password {
                            "Locked Extract"
                        } else {
                            "Extract"
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

                        if ui
                            .add(btn)
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
                    if ui
                        .add(
                            egui::Button::new("Open Another")
                                .min_size(egui::vec2(130.0, 32.0))
                                .rounding(egui::Rounding::same(6.0)),
                        )
                        .clicked()
                    {
                        if let Some(path) = pick_file() {
                            self.set_archive(path);
                        }
                    }

                    ui.add_space(8.0);

                    if let Some(ref dest) = self.destination_path {
                        if ui
                            .add(
                                egui::Button::new("Open Destination")
                                    .min_size(egui::vec2(140.0, 32.0))
                                    .rounding(egui::Rounding::same(6.0)),
                            )
                            .clicked()
                        {
                            open_path(dest);
                        }
                    }
                }
            });

            ui.add_space(16.0);

            // File list header — show spinner while background loading is active
            if self.is_loading && self.archive_entries.is_empty() {
                ui.add_space(20.0);
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    ui.label(
                        egui::RichText::new("Loading archive…")
                            .size(14.0)
                            .color(egui::Color32::from_rgb(160, 160, 170)),
                    );
                });
                ui.add_space(10.0);
            } else if !self.archive_entries.is_empty() {
                ui.horizontal(|ui| {
                    let total = self.archive_entries.len();
                    let showing = if self.search_query.is_empty() {
                        format!("Contents  ·  {} files ", total)
                    } else {
                        let count = self.sorted_indices.len();
                        format!("Contents  ·  {} / {} files ", count, total)
                    };
                    ui.label(
                        egui::RichText::new(showing)
                            .size(13.0)
                            .color(egui::Color32::from_rgb(180, 180, 190)),
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let search_resp = ui.add(
                            egui::TextEdit::singleline(&mut self.search_query)
                                .hint_text("Search files...")
                                .desired_width(200.0),
                        );
                        if search_resp.changed() {
                            self.filter_entries();
                        }

                        if !self.search_query.is_empty()
                            && ui
                                .add(
                                    egui::Button::new("X")
                                        .min_size(egui::vec2(20.0, 20.0))
                                        .rounding(egui::Rounding::same(4.0)),
                                )
                                .clicked()
                        {
                            self.search_query.clear();
                            self.filter_entries();
                        }
                    });
                });

                ui.add_space(6.0);

                // Sorting row
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Sort by:")
                            .size(11.0)
                            .color(egui::Color32::from_rgb(140, 140, 150)),
                    );

                    let mut sort_clicked = None;
                    if ui
                        .selectable_label(self.sort_by == SortBy::Name, "Name")
                        .clicked()
                    {
                        sort_clicked = Some(SortBy::Name);
                    }
                    if ui
                        .selectable_label(self.sort_by == SortBy::Size, "Size")
                        .clicked()
                    {
                        sort_clicked = Some(SortBy::Size);
                    }
                    if ui
                        .selectable_label(self.sort_by == SortBy::Type, "Type")
                        .clicked()
                    {
                        sort_clicked = Some(SortBy::Type);
                    }

                    if let Some(clicked_sort) = sort_clicked {
                        if self.sort_by == clicked_sort {
                            self.sort_ascending = !self.sort_ascending;
                        } else {
                            self.sort_by = clicked_sort;
                            self.sort_ascending = true;
                        }
                        self.sort_entries();
                    }

                    let arrow = if self.sort_ascending {
                        "(asc)"
                    } else {
                        "(desc)"
                    };
                    ui.label(
                        egui::RichText::new(arrow)
                            .size(11.0)
                            .color(egui::Color32::from_rgb(120, 120, 130)),
                    );
                });

                ui.add_space(6.0);

                // File list
                egui::ScrollArea::vertical()
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        if self.sorted_indices.is_empty() {
                            ui.add_space(20.0);
                            ui.horizontal(|ui| {
                                ui.add_space(10.0);
                                ui.label(
                                    egui::RichText::new("No files match your search")
                                        .size(13.0)
                                        .color(egui::Color32::from_rgb(140, 140, 150)),
                                );
                            });
                            ui.add_space(10.0);
                        } else {
                            for (idx, &entry_idx) in self.sorted_indices.iter().enumerate() {
                                let entry = &self.archive_entries[entry_idx];
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
                                                egui::RichText::new(formats::entry_type_label(
                                                    entry,
                                                ))
                                                .size(14.0),
                                            );
                                            ui.label(
                                                egui::RichText::new(&entry.name).size(13.0).color(
                                                    if entry.is_dir {
                                                        egui::Color32::from_rgb(160, 180, 210)
                                                    } else {
                                                        egui::Color32::WHITE
                                                    },
                                                ),
                                            );
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    let size_text =
                                                        if entry.size > 0 || entry.is_dir {
                                                            formats::format_size(entry.size)
                                                        } else {
                                                            String::from("?")
                                                        };
                                                    ui.label(
                                                        egui::RichText::new(size_text)
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

                        // Large text heading
                        ui.label(
                            egui::RichText::new("[ ARCHIVE EXTRACTOR ]")
                                .size(28.0)
                                .color(egui::Color32::from_rgb(150, 180, 220))
                                .strong(),
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

                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new("Browse Files")
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
                            if let Some(path) = pick_file() {
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

impl Drop for ArchiveExtractorApp {
    fn drop(&mut self) {
        self.cancel_flag.store(true, Ordering::Relaxed);
        if let Some(handle) = self.extraction_handle.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.loading_handle.take() {
            let _ = handle.join();
        }
    }
}

impl eframe::App for ArchiveExtractorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.update_extraction_status();
        self.update_loading_status();
        if self.is_extracting || self.is_loading {
            ctx.request_repaint();
        }

        // Ctrl/Cmd+O: Open archive
        if ctx.input(|i| i.key_pressed(egui::Key::O) && (i.modifiers.ctrl || i.modifiers.command)) {
            if let Some(path) = pick_file() {
                self.set_archive(path);
            }
        }

        // Ctrl/Cmd+D: Select destination
        if ctx.input(|i| i.key_pressed(egui::Key::D) && (i.modifiers.ctrl || i.modifiers.command))
            && self.archive_path.is_some()
            && !self.is_extracting
        {
            if let Some(path) = pick_folder() {
                self.destination_path = Some(path.clone());
                self.destination_edit = path.display().to_string();
            }
        }

        // Ctrl/Cmd+E: Extract
        if ctx.input(|i| i.key_pressed(egui::Key::E) && (i.modifiers.ctrl || i.modifiers.command))
            && self.archive_path.is_some()
            && !self.is_extracting
        {
            self.start_extraction();
        }

        // Ctrl/Cmd+Q: Quit
        if ctx.input(|i| i.key_pressed(egui::Key::Q) && (i.modifiers.ctrl || i.modifiers.command)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        // Escape: Cancel extraction
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && self.is_extracting {
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
