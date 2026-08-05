//! Shared UI colors and egui dark theme setup.
use eframe::egui::{
    Align2, Button, Color32, Context, CornerRadius, FontId, Frame, Margin, Sense, Stroke, Ui,
    Visuals, vec2,
};

pub const CORNER_RADIUS: f32 = 8.0;

pub fn accent() -> Color32 {
    Color32::from_rgb(70, 130, 220)
}

pub fn accent_bright() -> Color32 {
    Color32::from_rgb(100, 200, 255)
}

pub fn surface() -> Color32 {
    Color32::from_gray(25)
}

pub fn surface_track() -> Color32 {
    Color32::from_gray(30)
}

pub fn surface_dim() -> Color32 {
    Color32::from_rgba_unmultiplied(10, 10, 10, 200)
}

pub fn keep_tint() -> Color32 {
    Color32::from_rgba_unmultiplied(70, 130, 220, 40)
}

pub fn text_muted() -> Color32 {
    Color32::from_gray(140)
}

pub fn text_muted_light() -> Color32 {
    Color32::from_gray(150)
}

pub fn error() -> Color32 {
    Color32::from_rgb(220, 80, 80)
}

pub fn button_disabled() -> Color32 {
    Color32::from_gray(55)
}

pub fn status_running() -> Color32 {
    Color32::from_rgb(80, 200, 120)
}

pub fn stroke_subtle() -> Color32 {
    Color32::from_gray(48)
}

pub fn section_frame() -> Frame {
    Frame::default()
        .fill(surface())
        .corner_radius(CORNER_RADIUS)
        .inner_margin(Margin::same(20))
        .stroke(Stroke::new(1.0_f32, stroke_subtle()))
}

/// Compact card for clip grid tiles.
pub fn card_frame() -> Frame {
    Frame::default()
        .fill(surface())
        .corner_radius(CORNER_RADIUS)
        .inner_margin(Margin::same(14))
        .stroke(Stroke::new(1.0_f32, stroke_subtle()))
}

pub fn primary_button(text: &str) -> Button<'static> {
    Button::new(text).fill(accent())
}

pub fn secondary_button(text: &str) -> Button<'static> {
    Button::new(text)
}

/// Sidebar nav row with hover and selected states.
pub fn nav_item(ui: &mut Ui, label: &str, selected: bool) -> bool {
    let width = ui.available_width();
    let height = 36.0;
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());

    if ui.is_rect_visible(rect) {
        let fill = if selected {
            accent().gamma_multiply(0.22)
        } else if response.hovered() {
            Color32::from_gray(38)
        } else {
            Color32::TRANSPARENT
        };
        if fill != Color32::TRANSPARENT {
            ui.painter().rect_filled(rect, 6.0, fill);
        }

        let text_color = if selected {
            accent_bright()
        } else {
            Color32::from_gray(180)
        };
        ui.painter().text(
            rect.left_center() + vec2(12.0, 0.0),
            Align2::LEFT_CENTER,
            label,
            FontId::proportional(15.0),
            text_color,
        );
    }

    response.clicked()
}

pub fn apply_theme(ctx: &Context) {
    let mut visuals = Visuals::dark();
    let radius = CornerRadius::same(8);
    visuals.window_corner_radius = radius;
    visuals.menu_corner_radius = radius;
    visuals.panel_fill = Color32::from_gray(22);
    visuals.extreme_bg_color = Color32::from_gray(18);
    visuals.widgets.noninteractive.bg_fill = Color32::from_gray(28);
    visuals.widgets.inactive.corner_radius = CornerRadius::same(6);
    visuals.widgets.active.corner_radius = CornerRadius::same(6);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(6);
    visuals.widgets.active.bg_fill = accent().gamma_multiply(0.85);
    visuals.widgets.hovered.bg_fill = Color32::from_gray(45);
    visuals.selection.bg_fill = accent().gamma_multiply(0.35);
    ctx.set_visuals(visuals);

    ctx.style_mut(|style| {
        style.spacing.item_spacing = vec2(10.0, 10.0);
        style.spacing.button_padding = vec2(14.0, 8.0);
    });
}
