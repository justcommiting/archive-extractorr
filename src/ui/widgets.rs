#![allow(dead_code)]

use egui::*;

/// A styled button with icon support
pub struct StyledButton<'a> {
    text: &'a str,
    icon: Option<&'a str>,
    enabled: bool,
    min_size: Option<Vec2>,
}

impl<'a> StyledButton<'a> {
    pub fn new(text: &'a str) -> Self {
        Self {
            text,
            icon: None,
            enabled: true,
            min_size: None,
        }
    }

    pub fn icon(mut self, icon: &'a str) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn min_size(mut self, size: Vec2) -> Self {
        self.min_size = Some(size);
        self
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let mut button = if let Some(icon) = self.icon {
            Button::new(RichText::new(format!("{} {}", icon, self.text)))
        } else {
            Button::new(self.text)
        };

        if !self.enabled {
            button = button.sense(Sense::focusable_noninteractive());
        }

        if let Some(size) = self.min_size {
            button = button.min_size(size);
        }

        ui.add(button)
    }
}

/// Progress bar widget
pub fn progress_bar(ui: &mut Ui, progress: f32, label: Option<&str>) {
    let progress = progress.clamp(0.0, 100.0) / 100.0;

    ui.add(
        ProgressBar::new(progress)
            .show_percentage()
            .text(label.unwrap_or(""))
    );
}

/// Card widget for displaying file info
pub fn file_card(ui: &mut Ui, icon: &str, name: &str, size: &str, selected: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), 50.0),
        Sense::click(),
    );

    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);

        let bg_color = if selected {
            ui.visuals().selection.bg_fill
        } else if response.hovered() {
            visuals.bg_fill
        } else {
            Color32::TRANSPARENT
        };

        let rounding = Rounding::same(8.0);
        ui.painter().rect(
            rect.shrink(4.0),
            rounding,
            bg_color,
            Stroke::NONE,
        );

        // Icon
        let icon_pos = rect.min + Vec2::new(12.0, 26.0);
        ui.painter().text(
            icon_pos,
            Align2::CENTER_CENTER,
            icon,
            FontId::new(24.0, FontFamily::Proportional),
            Color32::WHITE,
        );

        // Name
        let name_pos = rect.min + Vec2::new(50.0, 18.0);
        ui.painter().text(
            name_pos,
            Align2::LEFT_TOP,
            name,
            FontId::new(14.0, FontFamily::Proportional),
            Color32::WHITE,
        );

        // Size
        let size_pos = rect.min + Vec2::new(50.0, 35.0);
        ui.painter().text(
            size_pos,
            Align2::LEFT_TOP,
            size,
            FontId::new(12.0, FontFamily::Monospace),
            Color32::GRAY,
        );
    }

    response
}

/// Status indicator
pub fn status_indicator(ui: &mut Ui, status: &str, is_error: bool) {
    let color = if is_error {
        Color32::from_rgb(220, 80, 80)
    } else {
        Color32::from_rgb(80, 180, 100)
    };

    ui.horizontal(|ui| {
        let indicator = RichText::new("●").size(10.0).color(color);
        ui.label(indicator);
        ui.label(RichText::new(status).size(12.0).color(color));
    });
}

/// Drop zone widget
pub fn drop_zone(ui: &mut Ui, is_dragging: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), 150.0),
        Sense::hover(),
    );

    if ui.is_rect_visible(rect) {
        let visuals = ui.visuals();

        let stroke_color = if is_dragging {
            visuals.selection.bg_fill
        } else {
            Color32::from_rgb(80, 80, 100)
        };

        let fill_color = if is_dragging {
            Color32::from_rgba_unmultiplied(80, 140, 220, 30)
        } else {
            Color32::from_rgba_unmultiplied(40, 40, 50, 50)
        };

        let rounding = Rounding::same(12.0);

        // Draw dashed border manually
        ui.painter().rect(
            rect,
            rounding,
            fill_color,
            Stroke::new(2.0, stroke_color),
        );

        // Icon and text
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            "📥",
            FontId::new(48.0, FontFamily::Proportional),
            Color32::WHITE,
        );

        let text_pos = rect.center() + Vec2::new(0.0, 35.0);
        ui.painter().text(
            text_pos,
            Align2::CENTER_CENTER,
            "Drop archive here",
            FontId::new(16.0, FontFamily::Proportional),
            Color32::WHITE,
        );
    }

    response
}
