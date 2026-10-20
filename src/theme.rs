//! Colors and global egui style. One dark theme, tuned for long sessions.

use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Vec2};

pub const BACKGROUND: Color32 = Color32::from_rgb(24, 25, 30);
pub const EDITOR_BACKGROUND: Color32 = Color32::from_rgb(28, 30, 35);
pub const SIDEBAR_BACKGROUND: Color32 = Color32::from_rgb(23, 24, 29);
pub const PANEL_BACKGROUND: Color32 = Color32::from_rgb(23, 24, 29);
pub const ACTIVITY_BACKGROUND: Color32 = Color32::from_rgb(19, 20, 24);
pub const TAB_BACKGROUND: Color32 = Color32::from_rgb(23, 24, 29);
pub const ACTIVE_TAB_BACKGROUND: Color32 = EDITOR_BACKGROUND;
pub const SURFACE: Color32 = Color32::from_rgb(36, 38, 45);
pub const SURFACE_HOVER: Color32 = Color32::from_rgb(45, 48, 57);
pub const BORDER: Color32 = Color32::from_rgb(40, 42, 50);
pub const TEXT: Color32 = Color32::from_rgb(212, 216, 224);
pub const MUTED_TEXT: Color32 = Color32::from_rgb(128, 135, 150);
pub const FAINT_TEXT: Color32 = Color32::from_rgb(88, 94, 108);
pub const ACCENT: Color32 = Color32::from_rgb(82, 156, 236);
pub const ACCENT_DIM: Color32 = Color32::from_rgb(38, 64, 98);
pub const SELECTION: Color32 = Color32::from_rgb(44, 70, 108);
pub const SELECTION_INACTIVE: Color32 = Color32::from_rgb(52, 58, 72);
pub const CURRENT_LINE: Color32 = Color32::from_rgb(33, 35, 41);
pub const CURSOR: Color32 = Color32::from_rgb(120, 180, 250);
pub const MATCH_HIGHLIGHT: Color32 = Color32::from_rgb(70, 68, 40);
pub const LINE_NUMBER: Color32 = Color32::from_rgb(78, 84, 98);
pub const LINE_NUMBER_ACTIVE: Color32 = Color32::from_rgb(190, 196, 208);
pub const INDENT_GUIDE: Color32 = Color32::from_rgb(40, 43, 51);

pub const ERROR: Color32 = Color32::from_rgb(240, 98, 98);
pub const WARNING: Color32 = Color32::from_rgb(230, 180, 80);
pub const INFO: Color32 = Color32::from_rgb(90, 170, 240);
pub const HINT: Color32 = Color32::from_rgb(120, 130, 145);
pub const SUCCESS: Color32 = Color32::from_rgb(110, 190, 120);

pub const GIT_ADDED: Color32 = Color32::from_rgb(98, 178, 110);
pub const GIT_MODIFIED: Color32 = Color32::from_rgb(90, 155, 230);
pub const GIT_DELETED: Color32 = Color32::from_rgb(225, 95, 95);
pub const GIT_UNTRACKED: Color32 = Color32::from_rgb(120, 190, 130);
pub const GIT_CONFLICT: Color32 = Color32::from_rgb(230, 120, 70);

pub const DIFF_ADDED_BG: Color32 = Color32::from_rgb(30, 52, 38);
pub const DIFF_REMOVED_BG: Color32 = Color32::from_rgb(62, 32, 36);
pub const DIFF_HUNK_BG: Color32 = Color32::from_rgb(34, 40, 56);

pub const STATUS_BACKGROUND: Color32 = Color32::from_rgb(19, 20, 24);
pub const REMOTE_BADGE: Color32 = Color32::from_rgb(38, 120, 88);

pub const TERMINAL_BACKGROUND: Color32 = Color32::from_rgb(22, 23, 28);
pub const TERMINAL_FOREGROUND: Color32 = Color32::from_rgb(208, 212, 220);
pub const TERMINAL_CURSOR: Color32 = Color32::from_rgb(200, 210, 230);

/// Color for a highlight name index from `syntax::HIGHLIGHT_NAMES`.
pub fn syntax_color(index: u8) -> Color32 {
    let name = crate::syntax::HIGHLIGHT_NAMES.get(index as usize).copied().unwrap_or("");
    match name {
        "comment" => Color32::from_rgb(106, 115, 130),
        "keyword" | "conditional" | "repeat" | "include" | "exception" | "storageclass" | "keyword.function" | "keyword.return"
        | "keyword.operator" => Color32::from_rgb(198, 120, 221),
        "string" | "string.special" | "character" | "text.literal" => Color32::from_rgb(152, 195, 121),
        "escape" | "string.escape" => Color32::from_rgb(86, 182, 194),
        "number" | "boolean" | "constant.builtin" => Color32::from_rgb(209, 154, 102),
        "constant" => Color32::from_rgb(229, 192, 123),
        "function" | "function.method" | "method" => Color32::from_rgb(97, 175, 239),
        "function.builtin" => Color32::from_rgb(86, 182, 194),
        "function.macro" => Color32::from_rgb(86, 182, 194),
        "type" | "constructor" => Color32::from_rgb(229, 192, 123),
        "type.builtin" => Color32::from_rgb(224, 170, 100),
        "attribute" | "label" => Color32::from_rgb(209, 154, 102),
        "module" | "namespace" => Color32::from_rgb(229, 192, 123),
        "property" | "field" | "property.definition" => Color32::from_rgb(224, 108, 117),
        "variable.builtin" => Color32::from_rgb(224, 108, 117),
        "variable.parameter" | "parameter" => Color32::from_rgb(220, 190, 160),
        "tag" => Color32::from_rgb(224, 108, 117),
        "operator" => Color32::from_rgb(171, 178, 191),
        "punctuation" | "punctuation.bracket" | "punctuation.delimiter" | "delimiter" => Color32::from_rgb(150, 156, 170),
        "punctuation.special" | "embedded" => Color32::from_rgb(198, 120, 221),
        "text.title" | "markup.heading" => Color32::from_rgb(224, 108, 117),
        "text.uri" | "markup.link" => Color32::from_rgb(86, 182, 194),
        _ => TEXT,
    }
}

pub fn apply(ctx: &egui::Context, ui_font_size: f32) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BACKGROUND;
    visuals.window_fill = SURFACE;
    visuals.window_stroke = Stroke::new(1.0, Color32::from_rgb(55, 58, 68));
    visuals.extreme_bg_color = Color32::from_rgb(20, 21, 26);
    visuals.code_bg_color = EDITOR_BACKGROUND;
    visuals.faint_bg_color = Color32::from_rgb(30, 32, 38);
    visuals.text_edit_bg_color = Some(Color32::from_rgb(20, 21, 26));
    visuals.weak_text_color = Some(MUTED_TEXT);
    visuals.hyperlink_color = ACCENT;
    visuals.selection.bg_fill = SELECTION;
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    visuals.window_corner_radius = CornerRadius::same(8);
    visuals.menu_corner_radius = CornerRadius::same(6);
    visuals.popup_shadow = egui::Shadow { offset: [0, 4], blur: 16, spread: 0, color: Color32::from_black_alpha(120) };
    visuals.window_shadow = egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(140) };

    let radius = CornerRadius::same(4);
    visuals.widgets.noninteractive.bg_fill = BACKGROUND;
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    visuals.widgets.noninteractive.corner_radius = radius;
    visuals.widgets.inactive.bg_fill = SURFACE;
    visuals.widgets.inactive.weak_bg_fill = SURFACE;
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.inactive.bg_stroke = Stroke::NONE;
    visuals.widgets.inactive.corner_radius = radius;
    visuals.widgets.hovered.bg_fill = SURFACE_HOVER;
    visuals.widgets.hovered.weak_bg_fill = SURFACE_HOVER;
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    visuals.widgets.hovered.bg_stroke = Stroke::NONE;
    visuals.widgets.hovered.corner_radius = radius;
    visuals.widgets.active.bg_fill = ACCENT_DIM;
    visuals.widgets.active.weak_bg_fill = ACCENT_DIM;
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    visuals.widgets.active.bg_stroke = Stroke::NONE;
    visuals.widgets.active.corner_radius = radius;
    visuals.widgets.open.weak_bg_fill = SURFACE_HOVER;
    visuals.widgets.open.corner_radius = radius;

    ctx.set_visuals_of(egui::Theme::Dark, visuals);
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.spacing.item_spacing = Vec2::new(6.0, 4.0);
        style.spacing.button_padding = Vec2::new(8.0, 3.0);
        style.spacing.interact_size = Vec2::new(22.0, 22.0);
        style.spacing.menu_margin = egui::Margin::same(6);
        style.spacing.window_margin = egui::Margin::same(12);
        style.spacing.scroll = egui::style::ScrollStyle::thin();
        style.interaction.tooltip_delay = 0.4;
        style.text_styles = [
            (TextStyle::Small, FontId::new(ui_font_size - 2.0, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(ui_font_size, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(ui_font_size, FontFamily::Proportional)),
            (TextStyle::Heading, FontId::new(ui_font_size + 5.0, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(ui_font_size, FontFamily::Monospace)),
        ]
        .into();
    });
}
