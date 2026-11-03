use egui::{Color32, CornerRadius, Frame, Margin, Stroke};

pub const BG: Color32 = Color32::from_rgb(0x0f, 0x11, 0x15);
pub const PANEL: Color32 = Color32::from_rgb(0x15, 0x18, 0x1f);
pub const CARD: Color32 = Color32::from_rgb(0x1b, 0x1f, 0x28);
pub const CARD_HOVER: Color32 = Color32::from_rgb(0x22, 0x27, 0x32);
pub const BORDER: Color32 = Color32::from_rgb(0x2b, 0x31, 0x3d);
pub const TEXT: Color32 = Color32::from_rgb(0xe7, 0xea, 0xf1);
pub const MUTED: Color32 = Color32::from_rgb(0x8b, 0x93, 0xa7);
pub const ACCENT: Color32 = Color32::from_rgb(0x5b, 0x8d, 0xff);
pub const ACCENT_SOFT: Color32 = Color32::from_rgb(0x1e, 0x2c, 0x4d);
pub const SUCCESS: Color32 = Color32::from_rgb(0x35, 0xc7, 0x8a);
pub const WARN: Color32 = Color32::from_rgb(0xf0, 0xb7, 0x3f);
pub const ERROR: Color32 = Color32::from_rgb(0xef, 0x5f, 0x6b);

pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.all_styles_mut(|style| {
        let mut visuals = egui::Visuals::dark();

        visuals.panel_fill = PANEL;
        visuals.window_fill = PANEL;
        visuals.extreme_bg_color = BG;
        visuals.faint_bg_color = CARD;
        visuals.override_text_color = Some(TEXT);
        visuals.hyperlink_color = ACCENT;
        visuals.warn_fg_color = WARN;
        visuals.error_fg_color = ERROR;
        visuals.selection.bg_fill = ACCENT.gamma_multiply(0.45);
        visuals.selection.stroke = Stroke::new(1.0, TEXT);
        visuals.window_stroke = Stroke::new(1.0, BORDER);
        visuals.window_corner_radius = CornerRadius::same(12);

        visuals.widgets.noninteractive.bg_fill = PANEL;
        visuals.widgets.noninteractive.weak_bg_fill = PANEL;
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
        visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, MUTED);
        visuals.widgets.noninteractive.corner_radius = CornerRadius::same(8);

        visuals.widgets.inactive.bg_fill = CARD;
        visuals.widgets.inactive.weak_bg_fill = CARD;
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
        visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
        visuals.widgets.inactive.corner_radius = CornerRadius::same(8);

        visuals.widgets.hovered.bg_fill = CARD_HOVER;
        visuals.widgets.hovered.weak_bg_fill = CARD_HOVER;
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT.gamma_multiply(0.6));
        visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT);
        visuals.widgets.hovered.corner_radius = CornerRadius::same(8);

        visuals.widgets.active.bg_fill = ACCENT_SOFT;
        visuals.widgets.active.weak_bg_fill = ACCENT_SOFT;
        visuals.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
        visuals.widgets.active.fg_stroke = Stroke::new(1.0, TEXT);
        visuals.widgets.active.corner_radius = CornerRadius::same(8);

        visuals.widgets.open.bg_fill = CARD_HOVER;
        visuals.widgets.open.weak_bg_fill = CARD_HOVER;
        visuals.widgets.open.bg_stroke = Stroke::new(1.0, BORDER);
        visuals.widgets.open.corner_radius = CornerRadius::same(8);

        style.visuals = visuals;
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(12.0, 7.0);
        style.spacing.window_margin = Margin::same(16);
        style.spacing.menu_margin = Margin::same(10);
        style.spacing.interact_size.y = 26.0;

        style.text_styles.insert(
            egui::TextStyle::Body,
            egui::FontId::new(14.0, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Button,
            egui::FontId::new(14.0, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Small,
            egui::FontId::new(12.0, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Heading,
            egui::FontId::new(19.0, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Monospace,
            egui::FontId::new(12.5, egui::FontFamily::Monospace),
        );
    });
}

pub fn card() -> Frame {
    Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::same(12))
}

