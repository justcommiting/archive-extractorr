#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod cli;
mod extractor;
mod formats;
mod ui;

fn main() {
    use clap::Parser;

    // Initialize logging
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp(None)
        .format_target(false)
        .init();

    let args: Vec<String> = std::env::args().collect();

    // If no CLI arguments, run GUI mode
    if args.len() == 1 {
        #[cfg(feature = "gui")]
        {
            if let Err(e) = run_gui() {
                eprintln!("GUI Error: {}", e);
                std::process::exit(1);
            }
        }
        #[cfg(not(feature = "gui"))]
        {
            eprintln!("GUI feature not enabled. Use --help for CLI options.");
            std::process::exit(1);
        }
    } else {
        // Run CLI mode
        let cli = cli::Cli::parse();

        if let Err(e) = cli::run(cli) {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }
}

#[cfg(feature = "gui")]
fn run_gui() -> eframe::Result<()> {
    use app::ArchiveExtractorApp;
    use eframe::egui;

    log::info!("Starting Archive Extractor GUI");

    // Try to load icon, fall back to default if not available
    let icon_data = include_bytes!("../assets/icon.png");
    let app_icon = eframe::icon_data::from_png_bytes(icon_data).ok();

    // Native options for better cross-platform support
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([900.0, 700.0])
            .with_min_inner_size([600.0, 400.0])
            .with_icon(app_icon.unwrap_or_default())
            .with_title("Archive Extractor")
            .with_drag_and_drop(true),
        vsync: true,
        ..Default::default()
    };

    eframe::run_native(
        "Archive Extractor",
        native_options,
        Box::new(|cc| {
            // Apply custom style
            let mut style = (*cc.egui_ctx.style()).clone();
            style.spacing.item_spacing = egui::vec2(8.0, 8.0);
            style.spacing.button_padding = egui::Vec2::new(8.0, 4.0);
            cc.egui_ctx.set_style(style);

            // Apply dark theme
            let theme = ui::theme::Theme::Dark;
            theme.apply(&cc.egui_ctx);

            Ok(Box::new(ArchiveExtractorApp::new(cc)))
        }),
    )
}
