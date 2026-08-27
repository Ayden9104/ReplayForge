//! Shared UI colors and egui theme chrome (Classic, ArmA 3, Night Ops, Pirate).
use crate::config::AppTheme;
use eframe::egui::{
    Align2, Button, Color32, Context, CornerRadius, FontData, FontDefinitions, FontFamily, FontId,
    Frame, Margin, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui, Visuals, vec2,
};
use std::sync::Mutex;

static ACTIVE: Mutex<AppTheme> = Mutex::new(AppTheme::Classic);

#[derive(Clone, Copy)]
struct ThemeTokens {
    style: AppTheme,
    corner_radius: f32,
    accent: Color32,
    accent_bright: Color32,
    surface: Color32,
    surface_track: Color32,
    surface_dim: Color32,
    keep_tint: Color32,
    text_primary: Color32,
    text_muted: Color32,
    text_muted_light: Color32,
    error: Color32,
    button_disabled: Color32,
    status_running: Color32,
    success: Color32,
    stroke_subtle: Color32,
    panel_fill: Color32,
    extreme_bg: Color32,
    hover_fill: Color32,
    active_fill: Color32,
    noninteractive_bg: Color32,
    selection_bg: Color32,
}

fn uses_flat_chrome(style: AppTheme) -> bool {
    matches!(
        style,
        AppTheme::Arma3 | AppTheme::NightOps | AppTheme::Pirate
    )
}

fn classic_tokens() -> ThemeTokens {
    ThemeTokens {
        style: AppTheme::Classic,
        corner_radius: 8.0,
        accent: Color32::from_rgb(70, 130, 220),
        accent_bright: Color32::from_rgb(100, 200, 255),
        surface: Color32::from_gray(25),
        surface_track: Color32::from_gray(30),
        surface_dim: Color32::from_rgba_unmultiplied(10, 10, 10, 200),
        keep_tint: Color32::from_rgba_unmultiplied(70, 130, 220, 40),
        text_primary: Color32::from_gray(230),
        text_muted: Color32::from_gray(140),
        text_muted_light: Color32::from_gray(150),
        error: Color32::from_rgb(220, 80, 80),
        button_disabled: Color32::from_gray(55),
        status_running: Color32::from_rgb(80, 200, 120),
        success: Color32::from_rgb(80, 200, 120),
        stroke_subtle: Color32::from_gray(48),
        panel_fill: Color32::from_gray(22),
        extreme_bg: Color32::from_gray(18),
        hover_fill: Color32::from_gray(45),
        active_fill: Color32::from_rgb(60, 110, 187),
        noninteractive_bg: Color32::from_gray(28),
        selection_bg: Color32::from_rgba_unmultiplied(70, 130, 220, 90),
    }
}

fn arma3_tokens() -> ThemeTokens {
    ThemeTokens {
        style: AppTheme::Arma3,
        corner_radius: 3.0,
        accent: Color32::from_rgb(196, 122, 32),
        accent_bright: Color32::from_rgb(224, 144, 48),
        surface: Color32::from_rgb(32, 34, 30),
        surface_track: Color32::from_rgb(40, 42, 38),
        surface_dim: Color32::from_rgba_unmultiplied(10, 11, 9, 200),
        keep_tint: Color32::from_rgba_unmultiplied(196, 122, 32, 28),
        text_primary: Color32::from_rgb(230, 230, 226),
        text_muted: Color32::from_rgb(150, 150, 146),
        text_muted_light: Color32::from_rgb(170, 170, 166),
        error: Color32::from_rgb(180, 72, 60),
        button_disabled: Color32::from_rgb(48, 50, 46),
        status_running: Color32::from_rgb(210, 210, 204),
        success: Color32::from_rgb(90, 130, 70),
        stroke_subtle: Color32::from_rgb(72, 74, 68),
        panel_fill: Color32::from_rgb(18, 19, 16),
        extreme_bg: Color32::from_rgb(14, 15, 12),
        hover_fill: Color32::from_rgb(48, 50, 46),
        active_fill: Color32::from_rgb(52, 54, 50),
        noninteractive_bg: Color32::from_rgb(28, 30, 26),
        selection_bg: Color32::from_rgba_unmultiplied(196, 122, 32, 55),
    }
}

fn night_ops_tokens() -> ThemeTokens {
    ThemeTokens {
        style: AppTheme::NightOps,
        corner_radius: 3.0,
        accent: Color32::from_rgb(78, 180, 220),
        accent_bright: Color32::from_rgb(120, 210, 240),
        surface: Color32::from_rgb(22, 26, 32),
        surface_track: Color32::from_rgb(28, 34, 42),
        surface_dim: Color32::from_rgba_unmultiplied(8, 10, 14, 200),
        keep_tint: Color32::from_rgba_unmultiplied(78, 180, 220, 28),
        text_primary: Color32::from_rgb(220, 228, 236),
        text_muted: Color32::from_rgb(130, 140, 152),
        text_muted_light: Color32::from_rgb(150, 160, 172),
        error: Color32::from_rgb(200, 80, 90),
        button_disabled: Color32::from_rgb(40, 46, 54),
        status_running: Color32::from_rgb(200, 208, 216),
        success: Color32::from_rgb(70, 140, 120),
        stroke_subtle: Color32::from_rgb(48, 56, 68),
        panel_fill: Color32::from_rgb(12, 14, 18),
        extreme_bg: Color32::from_rgb(8, 10, 14),
        hover_fill: Color32::from_rgb(36, 44, 54),
        active_fill: Color32::from_rgb(40, 48, 58),
        noninteractive_bg: Color32::from_rgb(18, 22, 28),
        selection_bg: Color32::from_rgba_unmultiplied(78, 180, 220, 55),
    }
}

fn pirate_tokens() -> ThemeTokens {
    ThemeTokens {
        style: AppTheme::Pirate,
        corner_radius: 3.0,
        // Metallic gold: classic #D4AF37 base + bright highlight.
        accent: Color32::from_rgb(212, 175, 55),
        accent_bright: Color32::from_rgb(240, 208, 96),
        surface: Color32::from_rgb(42, 28, 20),
        surface_track: Color32::from_rgb(52, 36, 26),
        surface_dim: Color32::from_rgba_unmultiplied(10, 8, 6, 200),
        keep_tint: Color32::from_rgba_unmultiplied(212, 175, 55, 36),
        text_primary: Color32::from_rgb(242, 230, 200),
        text_muted: Color32::from_rgb(160, 148, 128),
        text_muted_light: Color32::from_rgb(180, 168, 148),
        error: Color32::from_rgb(168, 28, 28),
        button_disabled: Color32::from_rgb(48, 36, 28),
        status_running: Color32::from_rgb(61, 155, 143),
        success: Color32::from_rgb(70, 140, 120),
        stroke_subtle: Color32::from_rgb(72, 52, 38),
        panel_fill: Color32::from_rgb(22, 16, 12),
        extreme_bg: Color32::from_rgb(14, 12, 10),
        hover_fill: Color32::from_rgb(58, 40, 30),
        active_fill: Color32::from_rgb(62, 44, 32),
        noninteractive_bg: Color32::from_rgb(28, 20, 14),
        selection_bg: Color32::from_rgba_unmultiplied(212, 175, 55, 70),
    }
}

fn set_theme(theme: AppTheme) {
    if let Ok(mut guard) = ACTIVE.lock() {
        *guard = theme;
    }
}

fn tokens() -> ThemeTokens {
    let theme = ACTIVE.lock().map(|g| *g).unwrap_or(AppTheme::Classic);
    match theme {
        AppTheme::Classic => classic_tokens(),
        AppTheme::Arma3 => arma3_tokens(),
        AppTheme::NightOps => night_ops_tokens(),
        AppTheme::Pirate => pirate_tokens(),
    }
}

/// Corner radius for the active theme (used by trim preview chrome).
pub fn corner_radius() -> f32 {
    tokens().corner_radius
}

pub fn accent() -> Color32 {
    tokens().accent
}

pub fn accent_bright() -> Color32 {
    tokens().accent_bright
}

pub fn surface() -> Color32 {
    tokens().surface
}

pub fn surface_track() -> Color32 {
    tokens().surface_track
}

pub fn surface_dim() -> Color32 {
    tokens().surface_dim
}

pub fn keep_tint() -> Color32 {
    tokens().keep_tint
}

pub fn text_primary() -> Color32 {
    tokens().text_primary
}

pub fn text_muted() -> Color32 {
    tokens().text_muted
}

pub fn text_muted_light() -> Color32 {
    tokens().text_muted_light
}

pub fn error() -> Color32 {
    tokens().error
}

pub fn button_disabled() -> Color32 {
    tokens().button_disabled
}

pub fn status_running() -> Color32 {
    tokens().status_running
}

pub fn success() -> Color32 {
    tokens().success
}

pub fn stroke_subtle() -> Color32 {
    tokens().stroke_subtle
}

pub fn panel_fill() -> Color32 {
    tokens().panel_fill
}

/// Panel / accent / accent_bright chips for Appearance picker (does not depend on active theme).
pub fn swatch_colors(theme: AppTheme) -> [Color32; 3] {
    match theme {
        AppTheme::Classic => [
            Color32::from_gray(25),
            Color32::from_rgb(70, 130, 220),
            Color32::from_rgb(100, 200, 255),
        ],
        AppTheme::Arma3 => [
            Color32::from_rgb(32, 34, 30),
            Color32::from_rgb(196, 122, 32),
            Color32::from_rgb(224, 144, 48),
        ],
        AppTheme::NightOps => [
            Color32::from_rgb(22, 26, 32),
            Color32::from_rgb(78, 180, 220),
            Color32::from_rgb(120, 210, 240),
        ],
        AppTheme::Pirate => [
            Color32::from_rgb(42, 28, 20),
            Color32::from_rgb(212, 175, 55),
            Color32::from_rgb(240, 208, 96),
        ],
    }
}

pub fn paint_swatch(ui: &mut Ui, colors: [Color32; 3]) {
    let size = vec2(10.0, 10.0);
    let gap = 3.0;
    let total = vec2(size.x * 3.0 + gap * 2.0, size.y);
    let (rect, _) = ui.allocate_exact_size(total, Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let stroke = Stroke::new(1.0_f32, stroke_subtle());
    for (i, color) in colors.iter().enumerate() {
        let x = rect.min.x + i as f32 * (size.x + gap);
        let r = Rect::from_min_size(Pos2::new(x, rect.min.y), size);
        ui.painter().rect_filled(r, 2.0, *color);
        ui.painter().rect_stroke(r, 2.0, stroke, StrokeKind::Outside);
    }
}

pub fn section_frame() -> Frame {
    let t = tokens();
    Frame::default()
        .fill(t.surface)
        .corner_radius(t.corner_radius)
        .inner_margin(Margin::same(20))
        .stroke(Stroke::new(1.0_f32, t.stroke_subtle))
}

pub fn home_section_frame(running: bool) -> Frame {
    let t = tokens();
    if uses_flat_chrome(t.style) {
        return Frame::default()
            .fill(t.surface)
            .corner_radius(t.corner_radius)
            .inner_margin(Margin::same(24))
            .stroke(Stroke::new(
                1.0_f32,
                if running { t.accent } else { t.stroke_subtle },
            ));
    }

    let fill = if running {
        Color32::from_rgba_unmultiplied(
            t.status_running.r(),
            t.status_running.g(),
            t.status_running.b(),
            18,
        )
    } else {
        t.surface
    };
    Frame::default()
        .fill(fill)
        .corner_radius(t.corner_radius)
        .inner_margin(Margin::same(24))
        .stroke(Stroke::new(
            1.0_f32,
            if running {
                Color32::from_rgba_unmultiplied(
                    t.status_running.r(),
                    t.status_running.g(),
                    t.status_running.b(),
                    55,
                )
            } else {
                t.stroke_subtle
            },
        ))
}

pub fn home_last_clip_frame() -> Frame {
    let t = tokens();
    Frame::default()
        .fill(t.surface_track)
        .corner_radius(t.corner_radius)
        .inner_margin(Margin::same(10))
        .stroke(Stroke::new(1.0_f32, t.stroke_subtle))
}

pub fn card_frame() -> Frame {
    let t = tokens();
    Frame::default()
        .fill(t.surface)
        .corner_radius(t.corner_radius)
        .inner_margin(Margin::same(14))
        .stroke(Stroke::new(1.0_f32, t.stroke_subtle))
}

pub fn card_frame_focused() -> Frame {
    let t = tokens();
    let stroke_w = if uses_flat_chrome(t.style) {
        1.0_f32
    } else {
        2.0_f32
    };
    Frame::default()
        .fill(t.surface)
        .corner_radius(t.corner_radius)
        .inner_margin(Margin::same(14))
        .stroke(Stroke::new(stroke_w, t.accent))
}

pub fn primary_button(text: &str) -> Button<'static> {
    let t = tokens();
    if uses_flat_chrome(t.style) {
        Button::new(RichText::new(text).color(t.text_primary))
            .fill(t.surface_track)
            .stroke(Stroke::new(1.0_f32, t.stroke_subtle))
            .corner_radius(t.corner_radius)
    } else {
        Button::new(text)
            .fill(t.accent)
            .corner_radius(t.corner_radius)
    }
}

pub fn secondary_button(text: &str) -> Button<'static> {
    let t = tokens();
    if uses_flat_chrome(t.style) {
        Button::new(RichText::new(text).color(t.text_primary))
            .fill(t.surface)
            .stroke(Stroke::new(1.0_f32, t.stroke_subtle))
            .corner_radius(t.corner_radius)
    } else {
        Button::new(text).corner_radius(t.corner_radius)
    }
}

pub fn nav_item(ui: &mut Ui, label: &str, selected: bool) -> bool {
    let t = tokens();
    let width = ui.available_width();
    let height = 36.0;
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());

    if ui.is_rect_visible(rect) {
        if uses_flat_chrome(t.style) {
            if response.hovered() && !selected {
                ui.painter()
                    .rect_filled(rect, t.corner_radius, t.hover_fill);
            }
            if selected {
                let bar = Rect::from_min_size(rect.min, vec2(3.0, rect.height()));
                ui.painter().rect_filled(bar, 0.0, t.accent);
            }
            let text_color = if selected {
                t.text_primary
            } else {
                t.text_muted
            };
            ui.painter().text(
                rect.left_center() + vec2(14.0, 0.0),
                Align2::LEFT_CENTER,
                label,
                FontId::proportional(15.0),
                text_color,
            );
        } else {
            let fill = if selected {
                t.accent.gamma_multiply(0.22)
            } else if response.hovered() {
                Color32::from_gray(38)
            } else {
                Color32::TRANSPARENT
            };
            if fill != Color32::TRANSPARENT {
                ui.painter().rect_filled(rect, 6.0, fill);
            }
            let text_color = if selected {
                t.accent_bright
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
    }

    response.clicked()
}

fn install_fonts_condensed(ctx: &Context) {
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

fn install_fonts_pirate_serif(ctx: &Context) {
    let mut fonts = FontDefinitions::default();

    fonts.font_data.insert(
        "LibreBaskerville".to_owned(),
        FontData::from_owned(
            include_bytes!("../assets/fonts/LibreBaskerville-Regular.ttf").to_vec(),
        )
        .into(),
    );
    fonts.font_data.insert(
        "LibreBaskervilleBold".to_owned(),
        FontData::from_owned(
            include_bytes!("../assets/fonts/LibreBaskerville-Bold.ttf").to_vec(),
        )
        .into(),
    );

    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "LibreBaskerville".to_owned());
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(1, "LibreBaskervilleBold".to_owned());

    ctx.set_fonts(fonts);
}

pub fn apply_theme(ctx: &Context, theme: AppTheme) {
    set_theme(theme);

    match theme {
        AppTheme::Classic => ctx.set_fonts(FontDefinitions::default()),
        AppTheme::Arma3 | AppTheme::NightOps => install_fonts_condensed(ctx),
        AppTheme::Pirate => install_fonts_pirate_serif(ctx),
    }

    let t = tokens();
    let mut visuals = Visuals::dark();
    let radius = CornerRadius::same(t.corner_radius as u8);
    visuals.window_corner_radius = radius;
    visuals.menu_corner_radius = radius;
    visuals.panel_fill = t.panel_fill;
    visuals.extreme_bg_color = t.extreme_bg;
    visuals.widgets.noninteractive.bg_fill = t.noninteractive_bg;
    visuals.widgets.inactive.corner_radius = radius;
    visuals.widgets.active.corner_radius = radius;
    visuals.widgets.hovered.corner_radius = radius;
    visuals.widgets.inactive.bg_fill = t.surface;
    visuals.widgets.hovered.bg_fill = t.hover_fill;
    visuals.widgets.active.bg_fill = if uses_flat_chrome(theme) {
        t.active_fill
    } else {
        t.accent.gamma_multiply(0.85)
    };
    visuals.selection.bg_fill = if uses_flat_chrome(theme) {
        t.selection_bg
    } else {
        t.accent.gamma_multiply(0.35)
    };
    ctx.set_visuals(visuals);

    ctx.style_mut(|style| {
        style.spacing.item_spacing = vec2(10.0, 10.0);
        style.spacing.button_padding = vec2(14.0, 8.0);
    });
}
