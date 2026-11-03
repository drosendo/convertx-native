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

/// Fonts shipped with the OS that cover the symbols and scripts egui's built-in
/// fonts miss (✕, ●, ⌘, and any non-Latin file name).
fn system_font_candidates() -> &'static [&'static str] {
    if cfg!(target_os = "macos") {
        &[
            "/System/Library/Fonts/Apple Symbols.ttf",
            "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
            "/System/Library/Fonts/Supplemental/Songti.ttc.ttf",
            "/System/Library/Fonts/Geneva.ttf",
        ]
    } else if cfg!(target_os = "windows") {
        &[
            "C:/Windows/Fonts/seguisym.ttf",
            "C:/Windows/Fonts/segoeui.ttf",
        ]
    } else {
        // Linux and Android. .ttc collections are skipped: epaint cannot index them.
        &[
            "/system/fonts/NotoSansSymbols-Regular.ttf",
            "/system/fonts/Roboto-Regular.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/TTF/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/noto/NotoSans-Regular.ttf",
        ]
    }
}

fn load_system_fonts() -> Vec<(String, Vec<u8>)> {
    system_font_candidates()
        .iter()
        .enumerate()
        .filter_map(|(index, path)| std::fs::read(path).ok().map(|bytes| (index, bytes)))
        .map(|(index, bytes)| (format!("system-fallback-{index}"), bytes))
        .collect()
}

/// Installs the default fonts plus every usable system font as a fallback chain.
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    for (name, bytes) in load_system_fonts() {
        fonts.font_data.insert(
            name.clone(),
            std::sync::Arc::new(egui::FontData::from_owned(bytes)),
        );
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts.families.entry(family).or_default().push(name.clone());
        }
    }

    ctx.set_fonts(fonts);
}

pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::ThemePreference::Dark);
    install_fonts(ctx);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Every non-ASCII character the interface renders must have a glyph,
    /// otherwise buttons show up blank — which is how the remove button broke.
    #[test]
    fn ui_symbols_have_glyphs() {
        const SYMBOLS: &str = "…→—●·⌘×−";

        let ctx = egui::Context::default();
        apply(&ctx);

        // Fonts are built on the first pass.
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(400.0, 300.0),
            )),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            ui.label("warm up the fonts × ● ⌘ − → … — ·");
        });
        output.textures_delta.clear();

        let font_id = egui::FontId::proportional(14.0);
        let missing: Vec<char> = ctx.fonts_mut(|fonts| {
            SYMBOLS
                .chars()
                .filter(|symbol| !fonts.has_glyph(&font_id, *symbol))
                .collect()
        });

        assert!(
            missing.is_empty(),
            "no glyph for {missing:?} — check the system font fallback in theme::apply"
        );
    }
}
