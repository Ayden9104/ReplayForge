//! Shared UI colors and egui ArmA-inspired olive/amber theme.
use eframe::egui::{
    Align2, Button, Color32, Context, CornerRadius, FontData, FontDefinitions, FontFamily, FontId,
    Frame, Margin, Sense, Stroke, Ui, Visuals, vec2,
};

pub const CORNER_RADIUS: f32 = 8.0;

pub fn accent() -> Color32 {
    Color32::from_rgb(224, 160, 32)
}

pub fn accent_bright() -> Color32 {
    Color32::from_rgb(240, 192, 64)
}

pub fn surface() -> Color32 {
    Color32::from_rgb(35, 38, 30)
}

pub fn surface_track() -> Color32 {
    Color32::from_rgb(42, 46, 36)
}

pub fn surface_dim() -> Color32 {
    Color32::from_rgba_unmultiplied(12, 14, 10, 200)
}

pub fn keep_tint() -> Color32 {
    Color32::from_rgba_unmultiplied(224, 160, 32, 40)
}

pub fn text_muted() -> Color32 {
    Color32::from_rgb(148, 156, 138)
}

pub fn text_muted_light() -> Color32 {
    Color32::from_rgb(160, 168, 150)
}

pub fn error() -> Color32 {
    Color32::from_rgb(192, 80, 64)
}

pub fn button_disabled() -> Color32 {
    Color32::from_rgb(55, 58, 48)
}

pub fn status_running() -> Color32 {
    Color32::from_rgb(122, 158, 74)
}

pub fn stroke_subtle() -> Color32 {
    Color32::from_rgb(58, 64, 50)
}

pub fn panel_fill() -> Color32 {
    Color32::from_rgb(20, 22, 16)
}

pub fn extreme_bg() -> Color32 {
    Color32::from_rgb(16, 18, 12)
}

pub fn section_frame() -> Frame {
    Frame::default()
        .fill(surface())
        .corner_radius(CORNER_RADIUS)
        .inner_margin(Margin::same(20))
        .stroke(Stroke::new(1.0_f32, stroke_subtle()))
}

/// Home command card: subtle green tint while the replay buffer is live.
pub fn home_section_frame(running: bool) -> Frame {
    let live = status_running();
    let fill = if running {
        Color32::from_rgba_unmultiplied(live.r(), live.g(), live.b(), 18)
    } else {
        surface()
    };
    Frame::default()
        .fill(fill)
        .corner_radius(CORNER_RADIUS)
        .inner_margin(Margin::same(24))
        .stroke(Stroke::new(
            1.0_f32,
            if running {
                Color32::from_rgba_unmultiplied(live.r(), live.g(), live.b(), 55)
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
        .stroke(Stroke::new(2.0_f32, accent()))
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
            Color32::from_rgb(38, 42, 32)
        } else {
            Color32::TRANSPARENT
        };
        if fill != Color32::TRANSPARENT {
            ui.painter().rect_filled(rect, 6.0, fill);
        }

        let text_color = if selected {
            accent_bright()
        } else {
            Color32::from_rgb(180, 186, 170)
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

fn install_fonts(ctx: &Context) {
    let mut fonts = FontDefinitions::default();

    fonts.font_data.insert(
        "Rajdhani".to_owned(),
        FontData::from_owned(
            include_bytes!("../assets/fonts/Rajdhani-Regular.ttf").to_vec(),
        )
        .into(),
    );
    fonts.font_data.insert(
        "RajdhaniSemiBold".to_owned(),
        FontData::from_owned(
            include_bytes!("../assets/fonts/Rajdhani-SemiBold.ttf").to_vec(),
        )
        .into(),
    );

    // Prefer condensed tactical type; keep egui defaults as fallback for missing glyphs.
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "Rajdhani".to_owned());
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(1, "RajdhaniSemiBold".to_owned());

    ctx.set_fonts(fonts);
}

pub fn apply_theme(ctx: &Context) {
    install_fonts(ctx);

    let mut visuals = Visuals::dark();
    let radius = CornerRadius::same(8);
    visuals.window_corner_radius = radius;
    visuals.menu_corner_radius = radius;
    visuals.panel_fill = panel_fill();
    visuals.extreme_bg_color = extreme_bg();
    visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(32, 36, 28);
    visuals.widgets.inactive.corner_radius = CornerRadius::same(6);
    visuals.widgets.active.corner_radius = CornerRadius::same(6);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(6);
    visuals.widgets.active.bg_fill = accent().gamma_multiply(0.85);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(48, 52, 40);
    visuals.selection.bg_fill = accent().gamma_multiply(0.35);
    ctx.set_visuals(visuals);

    ctx.style_mut(|style| {
        style.spacing.item_spacing = vec2(10.0, 10.0);
        style.spacing.button_padding = vec2(14.0, 8.0);
    });
}
