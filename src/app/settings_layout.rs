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

/// A setting owns its row; the control never determines the label width.
pub(super) fn row(ui: &mut egui::Ui, label: &str, control: impl FnOnce(&mut egui::Ui)) {
    let width = ui.available_width();
    if width < 560.0 {
        ui.label(RichText::new(label).strong());
        control(ui);
        ui.add_space(8.0);
    } else {
        ui.horizontal_top(|ui| {
            let label_width = (width * 0.32).clamp(200.0, 260.0);
            ui.allocate_ui_with_layout(egui::vec2(label_width, 30.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width(label_width);
                ui.add(egui::Label::new(label).wrap());
            });
            let control_width = ui.available_width();
            ui.allocate_ui_with_layout(egui::vec2(control_width, 30.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width(control_width);
                control(ui);
            });
        });
    }
}

/// Each server is a readable block even when its command is long.
pub(super) fn server(ui: &mut egui::Ui, enabled: &mut bool, name: &str, command: &str, languages: &str) -> bool {
    egui::Frame::new()
        .fill(crate::theme::ACTIVITY_BACKGROUND)
        .stroke(egui::Stroke::new(1.0, crate::theme::BORDER))
        .corner_radius(6.0)
        .inner_margin(14.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let changed = ui.checkbox(enabled, name).changed();
            ui.label(RichText::new(command).monospace().size(14.0));
            ui.label(RichText::new(languages).small().color(HELP_TEXT));
            changed
        })
        .inner
}

/// Keep a bounded picker beside a form with an explicit remaining width.
pub(super) fn master_detail(ui: &mut egui::Ui, mut content: impl FnMut(&mut egui::Ui, bool)) {
    let width = ui.available_width();
    if width < 760.0 {
        content(ui, true);
        ui.add_space(12.0);
        ui.separator();
        content(ui, false);
    } else {
        ui.horizontal_top(|ui| {
            let picker_width = 220.0;
            let detail_width = width - picker_width - ui.spacing().item_spacing.x * 2.0 - 1.0;
            ui.allocate_ui_with_layout(egui::vec2(picker_width, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width(picker_width);
                content(ui, true);
            });
            ui.separator();
            ui.allocate_ui_with_layout(egui::vec2(detail_width, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width(detail_width);
                content(ui, false);
            });
        });
    }
}

/// Wide screens have a quiet navigation rail; small windows use wrapped tabs.
pub(super) fn page(ui: &mut egui::Ui, selected: &mut usize, content: impl FnOnce(&mut egui::Ui, usize)) {
    let titles = [t!("Editor"), t!("AI Completion"), t!("Pi Models & Settings"), t!("Servers & Agents")];
    let width = ui.available_width();
    let height = (ui.available_height() - 60.0).max(100.0);
    if width < 960.0 {
        ui.heading(t!("Settings"));
        ui.horizontal_wrapped(|ui| {
            for (index, title) in titles.iter().enumerate() {
                ui.selectable_value(selected, index, *title);
            }
        });
        ui.separator();
        let height = (ui.available_height() - 60.0).max(100.0);
        ui.allocate_ui_with_layout(egui::vec2(width, height), egui::Layout::top_down(egui::Align::Min), |ui| content(ui, *selected));
    } else {
        ui.allocate_ui_with_layout(egui::vec2(width, height), egui::Layout::left_to_right(egui::Align::TOP), |ui| {
            ui.allocate_ui_with_layout(egui::vec2(210.0, height), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width(210.0);
                ui.heading(t!("Settings"));
                ui.add_space(20.0);
                for (index, title) in titles.iter().enumerate() {
                    let response = ui.add_sized([210.0, 46.0], egui::Button::selectable(*selected == index, *title));
                    if response.clicked() {
                        *selected = index;
                    }
                }
            });
            ui.separator();
            let body_width = ui.available_width();
            ui.allocate_ui_with_layout(egui::vec2(body_width, height), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width(body_width);
                ui.heading(titles[*selected]);
                ui.add_space(12.0);
                content(ui, *selected);
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn russian_form_labels_and_server_commands_stay_readable_after_resizing() {
        let ctx = egui::Context::default();
        let russian_label = crate::i18n::russian_of("Maximum output (tokens)");
        for width in [1500.0, 820.0, 480.0, 1500.0] {
            let size = egui::vec2(width, 1000.0);
            let mut found_label = false;
            let mut found_server = false;
            for _ in 0..3 {
                let mut output = ctx.run_ui(
                    egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)), ..Default::default() },
                    |ui| {
                        apply_style(ui);
                        page(ui, &mut 3, |ui, _| {
                            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                                row(ui, russian_label, |ui| {
                                    ui.add(egui::TextEdit::singleline(&mut "131072".to_string()).desired_width(160.0));
                                });
                                server(ui, &mut true, "rust-analyzer", "rust-analyzer --background-index", "rust");
                            });
                        });
                        ui.label("Footer");
                    },
                );
                output.textures_delta.clear();
                for shape in output.shapes {
                    if let egui::epaint::Shape::Text(text) = shape.shape {
                        let label = text.galley.text();
                        if label == "Footer" {
                            assert!(text.pos.y + text.galley.size().y <= size.y, "footer outside window: {:?}", text.pos);
                        }
                        if label == russian_label || label == "rust-analyzer --background-index" {
                            assert!(text.galley.rows.len() <= 2, "label collapsed at width {width}: {label}");
                            let rect = egui::Rect::from_min_size(text.pos, text.galley.size());
                            assert!(rect.right() <= width + 1.0, "text outside window at width {width}: {rect:?}");
                            found_label |= label == russian_label;
                            found_server |= label.starts_with("rust-analyzer");
                        }
                    }
                }
            }
            assert!(found_label && found_server, "missing form content at width {width}");
        }
    }
}
