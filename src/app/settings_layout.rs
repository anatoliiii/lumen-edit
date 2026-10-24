//! Shared spacing and responsive layout for the settings pages only.
use eframe::egui::{self, RichText};

pub(super) const HELP_TEXT: egui::Color32 = egui::Color32::from_rgb(170, 178, 193);

pub(super) fn apply_style(ui: &mut egui::Ui) {
    let style = ui.style_mut();
    for (kind, font) in &mut style.text_styles {
        let minimum = match kind {
            egui::TextStyle::Small => 13.0,
            egui::TextStyle::Heading => 21.0,
            _ => 15.0,
        };
        font.size = font.size.max(minimum);
    }
    style.wrap_mode = Some(egui::TextWrapMode::Wrap);
    style.spacing.item_spacing = egui::vec2(12.0, 10.0);
    style.spacing.button_padding = egui::vec2(12.0, 7.0);
    style.spacing.interact_size.y = 30.0;
    style.spacing.slider_width = 170.0;
    style.spacing.text_edit_width = 340.0;
    style.spacing.combo_width = 200.0;
    style.visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, crate::theme::MUTED_TEXT.gamma_multiply(0.45));
}

pub(super) fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(18.0);
    ui.label(RichText::new(title).size(18.0).strong());
    ui.separator();
    ui.add_space(6.0);
}

/// Keep the picker compact, leaving most of the page for the form.
/// On narrow windows the picker goes above the form.
pub(super) fn master_detail(ui: &mut egui::Ui, mut content: impl FnMut(&mut egui::Ui, bool)) {
    if ui.available_width() < 740.0 {
        content(ui, true);
        ui.add_space(12.0);
        ui.separator();
        content(ui, false);
    } else {
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(egui::vec2(220.0, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width(220.0);
                content(ui, true);
            });
            ui.separator();
            ui.vertical(|ui| {
                ui.set_width(ui.available_width());
                content(ui, false);
            });
        });
    }
}
