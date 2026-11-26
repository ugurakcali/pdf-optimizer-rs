use eframe::egui::{self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, Stroke, Visuals};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemePalette {
    pub dark: bool,
    pub bg_canvas: Color32,
    pub bg_card: Color32,
    pub bg_card_alt: Color32,
    pub bg_hover: Color32,
    pub border: Color32,
    pub border_accent: Color32,
    pub text_primary: Color32,
    pub text_muted: Color32,
    pub primary: Color32,
    pub primary_hover: Color32,
    pub emerald: Color32,
    pub emerald_hover: Color32,
    pub blue: Color32,
    pub blue_hover: Color32,
    pub danger: Color32,
    pub danger_bg: Color32,
    pub pdf_pill_bg: Color32,
    pub pdf_pill_fg: Color32,
    pub img_pill_bg: Color32,
    pub img_pill_fg: Color32,
    pub success_bg: Color32,
    pub success_fg: Color32,
    pub success_border: Color32,
    pub error_bg: Color32,
    pub error_fg: Color32,
    pub error_border: Color32,
}

impl ThemePalette {
    pub fn light() -> Self {
        Self {
            dark: false,
            bg_canvas: Color32::from_rgb(243, 246, 251),
            bg_card: Color32::WHITE,
            bg_card_alt: Color32::from_rgb(248, 250, 252),
            bg_hover: Color32::from_rgb(241, 245, 249),
            border: Color32::from_rgb(226, 232, 240),
            border_accent: Color32::from_rgb(99, 102, 241),
            text_primary: Color32::from_rgb(15, 23, 42),
            text_muted: Color32::from_rgb(100, 116, 139),
            primary: Color32::from_rgb(79, 70, 229),
            primary_hover: Color32::from_rgb(67, 56, 202),
            emerald: Color32::from_rgb(16, 185, 129),
            emerald_hover: Color32::from_rgb(5, 150, 105),
            blue: Color32::from_rgb(37, 99, 235),
            blue_hover: Color32::from_rgb(29, 78, 216),
            danger: Color32::from_rgb(220, 38, 38),
            danger_bg: Color32::from_rgb(254, 226, 226),
            pdf_pill_bg: Color32::from_rgb(254, 226, 226),
            pdf_pill_fg: Color32::from_rgb(185, 28, 28),
            img_pill_bg: Color32::from_rgb(219, 234, 254),
            img_pill_fg: Color32::from_rgb(29, 78, 216),
            success_bg: Color32::from_rgb(236, 253, 245),
            success_fg: Color32::from_rgb(6, 95, 70),
            success_border: Color32::from_rgb(167, 243, 208),
            error_bg: Color32::from_rgb(254, 242, 242),
            error_fg: Color32::from_rgb(153, 27, 27),
            error_border: Color32::from_rgb(254, 202, 202),
        }
    }

    pub fn dark() -> Self {
        Self {
            dark: true,
            bg_canvas: Color32::from_rgb(14, 17, 23),
            bg_card: Color32::from_rgb(22, 27, 34),
            bg_card_alt: Color32::from_rgb(28, 34, 44),
            bg_hover: Color32::from_rgb(36, 44, 56),
            border: Color32::from_rgb(45, 55, 72),
            border_accent: Color32::from_rgb(129, 140, 248),
            text_primary: Color32::from_rgb(241, 245, 249),
            text_muted: Color32::from_rgb(148, 163, 184),
            primary: Color32::from_rgb(99, 102, 241),
            primary_hover: Color32::from_rgb(79, 70, 229),
            emerald: Color32::from_rgb(16, 185, 129),
            emerald_hover: Color32::from_rgb(5, 150, 105),
            blue: Color32::from_rgb(59, 130, 246),
            blue_hover: Color32::from_rgb(37, 99, 235),
            danger: Color32::from_rgb(248, 113, 113),
            danger_bg: Color32::from_rgb(69, 26, 26),
            pdf_pill_bg: Color32::from_rgb(76, 29, 36),
            pdf_pill_fg: Color32::from_rgb(252, 165, 165),
            img_pill_bg: Color32::from_rgb(30, 58, 95),
            img_pill_fg: Color32::from_rgb(147, 197, 253),
            success_bg: Color32::from_rgb(19, 46, 36),
            success_fg: Color32::from_rgb(110, 231, 183),
            success_border: Color32::from_rgb(22, 101, 52),
            error_bg: Color32::from_rgb(58, 22, 26),
            error_fg: Color32::from_rgb(252, 165, 165),
            error_border: Color32::from_rgb(153, 27, 27),
        }
    }
}

/// Installs Windows Segoe UI fonts (if present) so Turkish glyphs and UI typography look crisp.
pub fn configure_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();

    if let Ok(bytes) = std::fs::read(r"C:\Windows\Fonts\segoeui.ttf") {
        fonts
            .font_data
            .insert("segoe_ui".to_owned(), Arc::new(FontData::from_owned(bytes)));
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .insert(0, "segoe_ui".to_owned());
    }

    if let Ok(bytes) = std::fs::read(r"C:\Windows\Fonts\seguisym.ttf") {
        fonts
            .font_data
            .insert("segoe_sym".to_owned(), Arc::new(FontData::from_owned(bytes)));
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .push("segoe_sym".to_owned());
    }

    ctx.set_fonts(fonts);
}

pub fn apply_theme(ctx: &egui::Context, p: &ThemePalette) {
    let mut visuals = if p.dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };

    visuals.panel_fill = p.bg_canvas;
    visuals.window_fill = p.bg_card;
    visuals.extreme_bg_color = p.bg_card_alt;
    visuals.faint_bg_color = p.bg_card_alt;
    visuals.selection.bg_fill = p.primary;
    visuals.selection.stroke = Stroke::new(1.0_f32, Color32::WHITE);

    visuals.widgets.noninteractive.bg_fill = p.bg_card;
    visuals.widgets.noninteractive.weak_bg_fill = p.bg_card_alt;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, p.border);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, p.text_primary);
    visuals.widgets.noninteractive.corner_radius = CornerRadius::same(10);

    visuals.widgets.inactive.bg_fill = p.bg_card_alt;
    visuals.widgets.inactive.weak_bg_fill = p.bg_card_alt;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, p.border);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, p.text_primary);
    visuals.widgets.inactive.corner_radius = CornerRadius::same(8);

    visuals.widgets.hovered.bg_fill = p.bg_hover;
    visuals.widgets.hovered.weak_bg_fill = p.bg_hover;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, p.border_accent);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, p.text_primary);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(8);

    visuals.widgets.active.bg_fill = p.primary;
    visuals.widgets.active.weak_bg_fill = p.primary;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, p.primary);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, Color32::WHITE);
    visuals.widgets.active.corner_radius = CornerRadius::same(8);

    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 7.0);
    style.spacing.slider_width = 180.0;
    ctx.set_style(style);
}
