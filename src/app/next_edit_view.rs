//! Review-only surface for a versioned prediction. Code rows never wrap.
use super::{document::Document, editor::accept_next_edit};
use crate::theme;
use eframe::egui::{self, Align2, FontId, RichText, Sense, pos2, vec2};

pub fn show(ctx: &egui::Context, doc: &mut Document) {
    let Some(preview) = doc.view.next_edit_preview.as_ref() else { return };
    let mut open = true;
    let mut accept = false;
    let mut dismiss = false;
    let screen = ctx.content_rect().size();
    egui::Window::new(t!("Next Edit Preview"))
        .id(egui::Id::unique(("next_edit_review", doc.id)))
        .open(&mut open)
        .collapsible(false)
        .default_pos(ctx.content_rect().center() - vec2(screen.x.min(960.0), screen.y.min(520.0)) * 0.5)
        .default_size(vec2((screen.x - 48.0).clamp(280.0, 960.0), (screen.y - 100.0).clamp(200.0, 520.0)))
        .show(ctx, |ui| {
            let added = preview.rows.iter().filter(|r| r.tag == similar::ChangeTag::Insert).count();
            let removed = preview.rows.iter().filter(|r| r.tag == similar::ChangeTag::Delete).count();
            ui.horizontal(|ui| {
                let name = std::path::Path::new(&doc.path).file_name().unwrap_or_default().to_string_lossy();
                ui.label(RichText::new(name).strong()).on_hover_text(&doc.path);
                ui.colored_label(theme::SUCCESS, format!("+{added}"));
                ui.colored_label(theme::ERROR, format!("−{removed}"));
            });
            if let Some(warning) = &preview.warning {
                ui.colored_label(theme::WARNING, format!("⚠ {warning}"));
            }
            ui.horizontal(|ui| {
                accept = ui
                    .add_enabled(
                        preview.is_current(doc.id, &doc.buffer),
                        egui::Button::new(t!("Accept Next Edit")).fill(theme::DIFF_ADDED_BG),
                    )
                    .clicked();
                dismiss = ui.button(t!("Dismiss")).clicked();
            });
            ui.collapsing(t!("Request details"), |ui| {
                let m = &preview.metrics;
                ui.small(format!(
                    "context {} ms · HTTP {} ms · diff {} ms · total {} ms",
                    m.context_prepare_ms, m.http_latency_ms, m.diff_prepare_ms, m.total_latency_ms
                ));
                ui.small(format!("tokens {} / {} · cached {}", m.prompt_tokens, m.completion_tokens, m.cached_tokens));
            });
            ui.separator();
            ui.small(t!("Old / New · proposed changes"));
            let mono = FontId::monospace(14.0);
            let row_h = ui.fonts_mut(|f| f.row_height(&mono)) + 4.0;
            egui::ScrollArea::both().id_salt(("prediction_rows", preview.version)).auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for row in &preview.rows {
                    let (bg, fg, sign) = match row.tag {
                        similar::ChangeTag::Insert => (theme::DIFF_ADDED_BG, theme::SUCCESS, "+"),
                        similar::ChangeTag::Delete => (theme::DIFF_REMOVED_BG, theme::ERROR, "−"),
                        similar::ChangeTag::Equal => (egui::Color32::TRANSPARENT, theme::TEXT, " "),
                    };
                    let text = format!("{sign} {}", row.text.replace('\t', "    "));
                    let galley = ui.painter().layout_no_wrap(text, mono.clone(), fg);
                    let (rect, _) =
                        ui.allocate_exact_size(vec2((galley.size().x + 112.0).max(ui.available_width()), row_h), Sense::hover());
                    ui.painter().rect_filled(rect, 0.0, bg);
                    for (line, x) in [(row.old_line, 42.0), (row.new_line, 88.0)] {
                        if let Some(line) = line {
                            ui.painter().text(
                                rect.left_center() + vec2(x, 0.0),
                                Align2::RIGHT_CENTER,
                                line,
                                mono.clone(),
                                theme::LINE_NUMBER,
                            );
                        }
                    }
                    ui.painter().galley(pos2(rect.left() + 100.0, rect.top() + 2.0), galley, fg);
                }
            });
        });
    if accept {
        accept_next_edit(doc);
        doc.view.focus_request = true;
    } else if dismiss || !open {
        doc.view.next_edit_preview = None;
        doc.view.focus_request = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inline_completion::next_edit::{DiffRow, Preview};

    #[test]
    fn review_renders_long_code_without_wrapping_at_multiple_window_sizes() {
        for size in [vec2(800.0, 500.0), vec2(1440.0, 900.0)] {
            let ctx = egui::Context::default();
            let mut doc = Document::new(1, "/tmp/cli.rs".into(), "old\n", 4, true);
            let code = format!("    close({});", "argument, ".repeat(40));
            doc.view.next_edit_preview = Some(Preview {
                doc_id: 1,
                version: doc.buffer.version,
                cursor: 0,
                range: 0..3,
                expected: "old".into(),
                replacement: code.clone(),
                rows: vec![DiffRow { old_line: None, new_line: Some(40), tag: similar::ChangeTag::Insert, text: code.clone() }],
                metrics: Default::default(),
                warning: None,
            });
            let mut found = false;
            for _ in 0..3 {
                let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)), ..Default::default() };
                let mut output = ctx.run_ui(input, |ui| show(ui.ctx(), &mut doc));
                output.textures_delta.clear();
                for shape in output.shapes {
                    if let egui::epaint::Shape::Text(text) = shape.shape
                        && text.galley.text().contains(&code)
                    {
                        assert_eq!(text.galley.rows.len(), 1);
                        assert!(text.pos.y >= 0.0 && text.pos.y < size.y);
                        assert!(shape.clip_rect.right() <= size.x);
                        found = true;
                    }
                }
            }
            assert!(found, "code must be rendered at {size:?}");
        }
    }
}
