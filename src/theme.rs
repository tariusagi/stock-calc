//! Shared look & feel for both apps: palette, fonts, style and small widgets.

use eframe::egui::{
    self, Align, Color32, CornerRadius, FontFamily, FontId, Layout, Margin, RichText, Shadow,
    Stroke, TextStyle, Ui, Vec2,
};

pub const BG: Color32 = Color32::from_rgb(0xF3, 0xF5, 0xFA);
pub const CARD: Color32 = Color32::WHITE;
pub const TEXT: Color32 = Color32::from_rgb(0x1F, 0x29, 0x37);
pub const MUTED: Color32 = Color32::from_rgb(0x6B, 0x72, 0x80);
pub const BORDER: Color32 = Color32::from_rgb(0xE3, 0xE7, 0xEF);
pub const ACCENT: Color32 = Color32::from_rgb(0x4F, 0x46, 0xE5);
pub const ACCENT_DARK: Color32 = Color32::from_rgb(0x43, 0x38, 0xCA);
pub const ACCENT_SOFT: Color32 = Color32::from_rgb(0xEE, 0xF2, 0xFF);
pub const ACCENT_MUTED: Color32 = Color32::from_rgb(0xC7, 0xD2, 0xFE);
pub const GREEN: Color32 = Color32::from_rgb(0x05, 0x96, 0x69);
pub const GREEN_SOFT: Color32 = Color32::from_rgb(0xEC, 0xFD, 0xF5);
pub const RED: Color32 = Color32::from_rgb(0xDC, 0x26, 0x26);
pub const RED_SOFT: Color32 = Color32::from_rgb(0xFE, 0xF2, 0xF2);
pub const INPUT_BG: Color32 = Color32::from_rgb(0xF8, 0xF9, 0xFC);

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semibold".into()))
}

pub fn signed_color(v: f64) -> Color32 {
    if v > 0.0 {
        GREEN
    } else if v < 0.0 {
        RED
    } else {
        MUTED
    }
}

/// The window/taskbar icon: the largest image in `stock_calc.ico`.
pub fn app_icon() -> Option<std::sync::Arc<egui::IconData>> {
    const ICO: &[u8] = include_bytes!("../stock_calc.ico");
    // ICO layout: 6-byte header, then 16-byte directory entries of
    // (width, height, .., .., size: u32 @8, offset: u32 @12); 0 means 256 px.
    let count = u16::from_le_bytes(ICO.get(4..6)?.try_into().ok()?) as usize;
    let (offset, size) = (0..count)
        .filter_map(|i| {
            let e = ICO.get(6 + 16 * i..22 + 16 * i)?;
            let px = if e[0] == 0 { 256 } else { e[0] as u32 };
            let size = u32::from_le_bytes(e[8..12].try_into().ok()?) as usize;
            let offset = u32::from_le_bytes(e[12..16].try_into().ok()?) as usize;
            Some((px, offset, size))
        })
        .max_by_key(|&(px, ..)| px)
        .map(|(_, offset, size)| (offset, size))?;
    let png = ICO.get(offset..offset + size)?;
    eframe::icon_data::from_png_bytes(png).ok().map(std::sync::Arc::new)
}

/// Applies fonts and style; call once at app creation.
pub fn setup(ctx: &egui::Context) {
    setup_fonts(ctx);
    setup_style(ctx);
}

pub fn card_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(CARD)
        .corner_radius(14)
        .stroke(Stroke::new(1.0, BORDER))
        .inner_margin(Margin::same(14))
        .shadow(Shadow { offset: [0, 2], blur: 10, spread: 0, color: Color32::from_black_alpha(14) })
}

pub fn window_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(CARD)
        .corner_radius(16)
        .stroke(Stroke::new(1.0, BORDER))
        .inner_margin(Margin::same(20))
        .shadow(Shadow { offset: [0, 8], blur: 28, spread: 0, color: Color32::from_black_alpha(40) })
}

pub fn field_label(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(14.5).color(MUTED));
}

/// A label/value row inside a two-column `Grid`.
pub fn result_line(ui: &mut Ui, label: &str, value: String, color: Color32) {
    ui.label(RichText::new(label).size(13.5).color(MUTED));
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.label(RichText::new(value).font(semibold(14.0)).color(color));
    });
    ui.end_row();
}

fn setup_fonts(ctx: &egui::Context) {
    use std::sync::Arc;
    let mut fonts = egui::FontDefinitions::default();
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
    let load = |file: &str| std::fs::read(format!(r"{windir}\Fonts\{file}")).ok();

    let mut semibold_family = fonts.families[&FontFamily::Proportional].clone();

    if let Some(bytes) = load("segoeui.ttf") {
        fonts.font_data.insert("segoe".into(), Arc::new(egui::FontData::from_owned(bytes)));
        fonts.families.get_mut(&FontFamily::Proportional).unwrap().insert(0, "segoe".into());
        semibold_family.insert(0, "segoe".into());
    }
    if let Some(bytes) = load("seguisb.ttf") {
        fonts.font_data.insert("segoe-sb".into(), Arc::new(egui::FontData::from_owned(bytes)));
        semibold_family.insert(0, "segoe-sb".into());
    }
    // Segoe UI Symbol covers pictographs like the calculator glyph.
    if let Some(bytes) = load("seguisym.ttf") {
        fonts.font_data.insert("segoe-sym".into(), Arc::new(egui::FontData::from_owned(bytes)));
        fonts.families.get_mut(&FontFamily::Proportional).unwrap().push("segoe-sym".into());
        semibold_family.push("segoe-sym".into());
    }
    fonts.families.insert(FontFamily::Name("semibold".into()), semibold_family);
    ctx.set_fonts(fonts);
}

fn setup_style(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Light);
    ctx.style_mut_of(egui::Theme::Light, |style| {
        use TextStyle::*;
        style.text_styles = [
            (Heading, FontId::proportional(22.0)),
            (Body, FontId::proportional(15.0)),
            (Button, FontId::proportional(15.0)),
            (Small, FontId::proportional(12.0)),
            (Monospace, FontId::monospace(14.0)),
        ]
        .into();

        style.spacing.item_spacing = Vec2::new(8.0, 6.0);
        style.spacing.button_padding = Vec2::new(12.0, 5.0);
        style.spacing.interact_size.y = 28.0;

        let v = &mut style.visuals;
        v.panel_fill = BG;
        v.window_fill = CARD;
        v.extreme_bg_color = INPUT_BG;
        v.faint_bg_color = Color32::from_rgb(0xF8, 0xF9, 0xFD);
        v.override_text_color = Some(TEXT);
        v.selection.bg_fill = ACCENT_MUTED;
        v.selection.stroke = Stroke::new(1.5, ACCENT);
        v.hyperlink_color = ACCENT;
        v.window_corner_radius = CornerRadius::same(16);
        v.text_cursor.stroke = Stroke::new(2.0, ACCENT);

        let r = CornerRadius::same(8);
        v.widgets.noninteractive.corner_radius = r;
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
        v.widgets.inactive.corner_radius = r;
        v.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
        v.widgets.inactive.weak_bg_fill = Color32::from_rgb(0xF1, 0xF3, 0xF8);
        v.widgets.hovered.corner_radius = r;
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0xA5, 0xB4, 0xFC));
        v.widgets.hovered.weak_bg_fill = ACCENT_SOFT;
        v.widgets.active.corner_radius = r;
        v.widgets.active.bg_stroke = Stroke::new(1.5, ACCENT);
        v.widgets.open.corner_radius = r;
        v.widgets.open.weak_bg_fill = ACCENT_SOFT; // active window title bar
    });
}
