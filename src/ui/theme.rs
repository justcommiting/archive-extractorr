use egui::{Color32, Rounding, Spacing, Stroke, Vec2, Visuals};

/// Application theme
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Theme {
    Dark,
    Light,
    #[allow(dead_code)]
    System,
}

impl Theme {
    pub fn apply(self, ctx: &egui::Context) {
        match self {
            Theme::Dark => apply_dark_theme(ctx),
            Theme::Light => apply_light_theme(ctx),
            Theme::System => apply_dark_theme(ctx),
        }
    }
}

fn apply_dark_theme(ctx: &egui::Context) {
    let mut visuals = Visuals::dark();

    // Clean dark theme - no boxes, just subtle colors
    visuals.panel_fill = Color32::from_rgb(24, 24, 30);
    visuals.window_fill = Color32::from_rgb(24, 24, 30);
    visuals.selection.bg_fill = Color32::from_rgb(70, 130, 200);
    visuals.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(24, 24, 30);
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(40, 40, 48);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(55, 55, 65);
    visuals.widgets.active.bg_fill = Color32::from_rgb(70, 70, 82);
    visuals.extreme_bg_color = Color32::from_rgb(18, 18, 22);

    // Accent color
    let accent = Color32::from_rgb(90, 150, 220);
    visuals.widgets.inactive.fg_stroke.color = accent;
    visuals.widgets.hovered.fg_stroke.color = Color32::WHITE;
    visuals.widgets.active.fg_stroke.color = Color32::WHITE;

    // Smooth rounding
    visuals.window_rounding = Rounding::same(10.0);

    ctx.set_visuals(visuals);

    // Clean spacing - no box borders
    let mut style = (*ctx.style()).clone();
    style.spacing = Spacing {
        item_spacing: Vec2::new(10.0, 10.0),
        button_padding: Vec2::new(10.0, 5.0),
        menu_spacing: 4.0,
        ..Default::default()
    };
    style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(40, 40, 48);
    style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(55, 55, 65);
    ctx.set_style(style);
}

fn apply_light_theme(ctx: &egui::Context) {
    let mut visuals = Visuals::light();

    visuals.panel_fill = Color32::from_rgb(250, 250, 252);
    visuals.window_fill = Color32::from_rgb(250, 250, 252);
    visuals.selection.bg_fill = Color32::from_rgb(60, 110, 190);
    visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(250, 250, 252);
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(235, 235, 240);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(220, 220, 230);
    visuals.widgets.active.bg_fill = Color32::from_rgb(200, 200, 215);
    visuals.extreme_bg_color = Color32::from_rgb(245, 245, 248);

    let accent = Color32::from_rgb(50, 90, 170);
    visuals.widgets.inactive.fg_stroke.color = accent;

    visuals.window_rounding = Rounding::same(10.0);

    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing = Spacing {
        item_spacing: Vec2::new(10.0, 10.0),
        button_padding: Vec2::new(10.0, 5.0),
        menu_spacing: 4.0,
        ..Default::default()
    };
    ctx.set_style(style);
}
