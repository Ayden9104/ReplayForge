//! Shared UI colors and egui ArmA 3–style menu chrome.
use eframe::egui::{
    Align2, Button, Color32, Context, CornerRadius, FontData, FontDefinitions, FontFamily, FontId,
    Frame, Margin, Rect, RichText, Sense, Stroke, Ui, Visuals, vec2,
};

pub const CORNER_RADIUS: f32 = 3.0;

pub fn accent() -> Color32 {
    Color32::from_rgb(196, 122, 32)
}

pub fn accent_bright() -> Color32 {
    Color32::from_rgb(224, 144, 48)
}

pub fn surface() -> Color32 {
    Color32::from_rgb(32, 34, 30)
}

pub fn surface_track() -> Color32 {
    Color32::from_rgb(40, 42, 38)
}

pub fn surface_dim() -> Color32 {
    Color32::from_rgba_unmultiplied(10, 11, 9, 200)
}

pub fn keep_tint() -> Color32 {
    Color32::from_rgba_unmultiplied(196, 122, 32, 28)
}

pub fn text_primary() -> Color32 {
    Color32::from_rgb(230, 230, 226)
}

pub fn text_muted() -> Color32 {
    Color32::from_rgb(150, 150, 146)
}

pub fn text_muted_light() -> Color32 {
    Color32::from_rgb(170, 170, 166)
}

pub fn error() -> Color32 {
    Color32::from_rgb(180, 72, 60)
}

pub fn button_disabled() -> Color32 {
    Color32::from_rgb(48, 50, 46)
}

/// Quiet live/idle indicator (not neon green).
pub fn status_running() -> Color32 {
    Color32::from_rgb(210, 210, 204)
}

/// Success flash only (e.g. Copied).
pub fn success() -> Color32 {
    Color32::from_rgb(90, 130, 70)
}

pub fn stroke_subtle() -> Color32 {
    Color32::from_rgb(72, 74, 68)
}

pub fn panel_fill() -> Color32 {
    Color32::from_rgb(18, 19, 16)
}

pub fn extreme_bg() -> Color32 {
    Color32::from_rgb(14, 15, 12)
}

pub fn section_frame() -> Frame {
    Frame::default()
        .fill(surface())
        .corner_radius(CORNER_RADIUS)
        .inner_margin(Margin::same(20))
        .stroke(Stroke::new(1.0_f32, stroke_subtle()))
}

/// Home command card — same surface idle/live; thin orange stroke while running.
pub fn home_section_frame(running: bool) -> Frame {
    Frame::default()
        .fill(surface())
        .corner_radius(CORNER_RADIUS)
        .inner_margin(Margin::same(24))
        .stroke(Stroke::new(
            1.0_f32,
            if running {
                accent()
            } else {
                stroke_subtle()
            },
        ))
}

/// Nested last-clip strip on Home.
pub fn home_last_clip_frame() -> Frame {
    Frame::default()
        .fill(surface_track())
        .corner_radius(CORNER_RADIUS)
        .inner_margin(Margin::same(10))
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

/// Emphasized card for the most recently saved clip.
pub fn card_frame_focused() -> Frame {
    Frame::default()
        .fill(surface())
        .corner_radius(CORNER_RADIUS)
        .inner_margin(Margin::same(14))
        .stroke(Stroke::new(1.0_f32, accent()))
}

pub fn primary_button(text: &str) -> Button<'static> {
    Button::new(RichText::new(text).color(text_primary()))
        .fill(surface_track())
        .stroke(Stroke::new(1.0_f32, stroke_subtle()))
        .corner_radius(CORNER_RADIUS)
}

pub fn secondary_button(text: &str) -> Button<'static> {
    Button::new(RichText::new(text).color(text_primary()))
        .fill(surface())
        .stroke(Stroke::new(1.0_f32, stroke_subtle()))
        .corner_radius(CORNER_RADIUS)
}

/// Sidebar nav row: orange left bar when selected (no amber wash).
pub fn nav_item(ui: &mut Ui, label: &str, selected: bool) -> bool {
    let width = ui.available_width();
    let height = 36.0;
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());

    if ui.is_rect_visible(rect) {
        if response.hovered() && !selected {
            ui.painter()
                .rect_filled(rect, CORNER_RADIUS, Color32::from_rgb(36, 38, 34));
        }

        if selected {
            let bar = Rect::from_min_size(rect.min, vec2(3.0, rect.height()));
            ui.painter().rect_filled(bar, 0.0, accent());
        }

        let text_color = if selected {
            text_primary()
        } else {
            text_muted()
        };
        ui.painter().text(
            rect.left_center() + vec2(14.0, 0.0),
            Align2::LEFT_CENTER,
            label,
            FontId::proportional(15.0),
            text_color,
        );
    }

    response.clicked()
}

fn install_fonts(ctx: &Context) {
    let mut fonts = FontDefinitions::default();

    fonts.font_data.insert(
        "RobotoCondensed".to_owned(),
        FontData::from_owned(
            include_bytes!("../assets/fonts/RobotoCondensed-Regular.ttf").to_vec(),
        )
        .into(),
    );
    fonts.font_data.insert(
        "RobotoCondensedBold".to_owned(),
        FontData::from_owned(include_bytes!("../assets/fonts/RobotoCondensed-Bold.ttf").to_vec())
            .into(),
    );

    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "RobotoCondensed".to_owned());
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(1, "RobotoCondensedBold".to_owned());

    ctx.set_fonts(fonts);
}

pub fn apply_theme(ctx: &Context) {
    install_fonts(ctx);

    let mut visuals = Visuals::dark();
    let radius = CornerRadius::same(CORNER_RADIUS as u8);
    visuals.window_corner_radius = radius;
    visuals.menu_corner_radius = radius;
    visuals.panel_fill = panel_fill();
    visuals.extreme_bg_color = extreme_bg();
    visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(28, 30, 26);
    visuals.widgets.inactive.corner_radius = radius;
    visuals.widgets.active.corner_radius = radius;
    visuals.widgets.hovered.corner_radius = radius;
    visuals.widgets.inactive.bg_fill = surface();
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(48, 50, 46);
    visuals.widgets.active.bg_fill = Color32::from_rgb(52, 54, 50);
    visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(196, 122, 32, 55);
    ctx.set_visuals(visuals);

    ctx.style_mut(|style| {
        style.spacing.item_spacing = vec2(10.0, 10.0);
        style.spacing.button_padding = vec2(14.0, 8.0);
    });
}
