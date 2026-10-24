//! The editor area (tabs + content) and the bottom panel (terminals,
//! problems, output, references).

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, RichText, Sense, Stroke, pos2, vec2};

use super::{App, PanelTab, Tab, diffview, editor, lsp_manager, util};
use crate::{host, lsp::Severity, theme};

pub fn editor_area(ui: &mut egui::Ui, app: &mut App) {
    if !app.agent_manager.open
        && let Some(Tab::Agent(agent)) = app.active_tab.and_then(|i| app.tabs.get(i))
    {
        let id = agent.id;
        super::agent::manager::focus(app, id);
        super::agent::manager::open(app);
    }
    tab_bar(ui, app);
    if app.agent_manager.open {
        super::agent::manager::show(ui, app);
        return;
    }
    let Some(index) = app.active_tab.filter(|i| *i < app.tabs.len()) else {
        empty_state(ui, app);
        return;
    };
    match &app.tabs[index] {
        Tab::Doc(_) => {
            breadcrumb(ui, app, index);
            if markdown_toggle(ui, app, index) {
                return;
            }
            let mut actions = Vec::new();
            let Tab::Doc(doc) = &app.tabs[index] else { return };
            let (completion_triggers, signature_triggers) = app.lsp.triggers(doc);
            let has_lsp = app.lsp.has_clients(doc);
            let settings = editor::EditorSettings {
                font_size: app.config.editor_font_size,
                show_whitespace: app.config.show_whitespace,
                inline_blame: app.config.inline_blame,
                completion_triggers: &completion_triggers,
                signature_triggers: &signature_triggers,
                has_lsp,
            };
            if let Tab::Doc(doc) = &mut app.tabs[index] {
                editor::show(ui, doc, &settings, &mut actions);
            }
            crate::inline_completion::tick(app, ui.ctx());
            if !actions.is_empty() {
                lsp_manager::run_actions(app, index, actions);
            }
        }
        Tab::Diff(_) => diffview::show_diff(ui, app, index),
        Tab::Log(_) => diffview::show_log(ui, app, index),
        Tab::Agent(_) => super::agent::show(ui, app, super::agent::Loc::Tab(index)),
        Tab::Teleport(_) => super::teleport::show(ui, app, index),
        Tab::Image(_) => super::media::show_image(ui, app, index),
        Tab::Settings => super::status::settings_tab_ui(ui, app),
    }
}

fn tab_bar(ui: &mut egui::Ui, app: &mut App) {
    let mut activate = None;
    let mut close = None;
    let mut close_others = None;
    let mut close_all = false;
    let mut copy_path = None;
    let mut reveal = None;
    let mut send_to_agent: Option<String> = None;
    let height = 36.0;
    let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    ui.painter().rect_filled(bar, 0.0, theme::TAB_BACKGROUND);
    ui.painter().line_segment([bar.left_bottom(), bar.right_bottom()], Stroke::new(1.0, theme::BORDER));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(bar).layout(egui::Layout::left_to_right(egui::Align::Center)));
    egui::ScrollArea::horizontal().id_salt("tabs_scroll").scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(
        &mut child,
        |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let workspace_tab = ui.add(egui::Button::selectable(app.agent_manager.open, t!("Agent sessions")));
            if workspace_tab.clicked() {
                super::agent::manager::open(app);
            }
            if app.agent_manager.open && ui.small_button("×").on_hover_text(t!("Close")).clicked() {
                super::agent::manager::hide(app);
            }
            for (index, tab) in app.tabs.iter().enumerate() {
                if matches!(tab, Tab::Agent(_)) {
                    continue;
                }
                let active = !app.agent_manager.open && app.active_tab == Some(index);
                let mut title = tab.title();
                // Same file name in different folders: add the folder, like `about.png · ru`.
                if let Some(path) = tab.path()
                    && app
                        .tabs
                        .iter()
                        .filter_map(Tab::path)
                        .any(|other| other != path && crate::host::file_name(other) == crate::host::file_name(path))
                {
                    title = format!("{title} · {}", crate::host::file_name(&crate::host::parent(path)));
                }
                let (icon, icon_color) = match tab {
                    Tab::Doc(doc) => util::path_icon(&doc.path),
                    Tab::Diff(_) => ("±", theme::GIT_MODIFIED),
                    Tab::Log(_) => ("⎇", theme::ACCENT),
                    Tab::Agent(agent) if agent.streaming => ("●", theme::SUCCESS),
                    Tab::Agent(_) => ("π", theme::ACCENT),
                    Tab::Teleport(_) => ("⇄", theme::GIT_ADDED),
                    Tab::Image(_) => ("▣", Color32::from_rgb(180, 120, 200)),
                    Tab::Settings => ("⚙", theme::ACCENT),
                };
                let errors = match tab {
                    Tab::Doc(doc) => doc.diagnostic_counts().0,
                    _ => 0,
                };
                let title_color = if errors > 0 {
                    theme::ERROR
                } else if active {
                    theme::TEXT
                } else {
                    theme::MUTED_TEXT
                };
                let galley = ui.painter().layout_no_wrap(title.clone(), FontId::proportional(13.0), title_color);
                let width = galley.size().x + 64.0;
                let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
                if active {
                    ui.painter().rect_filled(rect, 0.0, theme::ACTIVE_TAB_BACKGROUND);
                    ui.painter().line_segment([rect.left_top(), rect.right_top()], Stroke::new(2.0, theme::ACCENT));
                } else if response.hovered() {
                    ui.painter().rect_filled(rect, 0.0, theme::SURFACE);
                }
                ui.painter().line_segment([rect.right_top(), rect.right_bottom()], Stroke::new(1.0, theme::BORDER));
                match tab {
                    Tab::Doc(doc) => super::icons::file(ui.painter(), rect.left_center() + vec2(16.0, 0.0), &doc.path),
                    _ => {
                        ui.painter().text(
                            rect.left_center() + vec2(16.0, 0.0),
                            Align2::CENTER_CENTER,
                            icon,
                            FontId::monospace(11.5),
                            icon_color,
                        );
                    }
                }
                ui.painter().galley(pos2(rect.left() + 28.0, rect.center().y - galley.size().y / 2.0), galley, title_color);
                let close_rect = egui::Rect::from_center_size(pos2(rect.right() - 16.0, rect.center().y), vec2(18.0, 18.0));
                let close_response = ui.interact(close_rect, ui.scope_id().with(("tab_close", index)), Sense::click());
                let dirty = tab.dirty();
                if close_response.hovered() {
                    ui.painter().rect_filled(close_rect, CornerRadius::same(3), theme::SURFACE_HOVER);
                }
                let close_glyph = if dirty && !close_response.hovered() { "●" } else { "×" };
                if active || response.hovered() || dirty || close_response.hovered() {
                    ui.painter().text(
                        close_rect.center(),
                        Align2::CENTER_CENTER,
                        close_glyph,
                        FontId::proportional(if close_glyph == "●" { 10.0 } else { 15.0 }),
                        theme::MUTED_TEXT,
                    );
                }
                if close_response.clicked() || response.middle_clicked() {
                    close = Some(index);
                } else if response.clicked() {
                    activate = Some(index);
                }
                if let Tab::Doc(doc) = tab {
                    let path = doc.path.clone();
                    response.on_hover_text(path.clone()).context_menu(|ui| {
                        if ui.button(t!("Close")).clicked() {
                            close = Some(index);
                            ui.close();
                        }
                        if ui.button(t!("Close Others")).clicked() {
                            close_others = Some(index);
                            ui.close();
                        }
                        if ui.button(t!("Close All")).clicked() {
                            close_all = true;
                            ui.close();
                        }
                        ui.separator();
                        if ui.button(t!("Copy path")).clicked() {
                            copy_path = Some(path.clone());
                            ui.close();
                        }
                        if ui.button(t!("Reveal in Explorer")).clicked() {
                            reveal = Some(path.clone());
                            ui.close();
                        }
                        if ui.button(RichText::new(t!("π  Send to the agent")).color(theme::ACCENT)).clicked() {
                            send_to_agent = Some(path.clone());
                            ui.close();
                        }
                    });
                }
                if active {
                    ui.scroll_to_rect(rect, None);
                }
            }
        },
    );
    if let Some(index) = activate {
        app.agent_manager.open = false;
        app.activate_tab(index);
    }
    if let Some(index) = close {
        app.request_close_tab(index);
    }
    if let Some(keep) = close_others {
        for index in (0..app.tabs.len()).rev() {
            if index != keep && !app.tabs[index].dirty() {
                app.close_tab(index);
            }
        }
    }
    if close_all {
        for index in (0..app.tabs.len()).rev() {
            if !app.tabs[index].dirty() {
                app.close_tab(index);
            }
        }
    }
    if let Some(path) = send_to_agent {
        super::agent::attach_file(app, &path);
    }
    if let Some(path) = copy_path {
        app.ctx.copy_text(path);
    }
    if let Some(path) = reveal
        && let Some(workspace) = &mut app.workspace
    {
        let root = workspace.root.clone();
        let missing = workspace.explorer.reveal(&path, &root);
        for dir in missing {
            app.load_dir(dir);
        }
        app.side_view = super::SideView::Explorer;
        app.sidebar_open = true;
    }
}

/// Preview/source switch for Markdown, SVG and CSV/TSV files. Returns true
/// when the preview took the editor area.
fn markdown_toggle(ui: &mut egui::Ui, app: &mut App, index: usize) -> bool {
    let font_size = app.config.editor_font_size;
    let Some(Tab::Doc(doc)) = app.tabs.get_mut(index) else { return false };
    let media = super::media::kind(&doc.path);
    if doc.lang != crate::syntax::Lang::Markdown && media.is_none() {
        return false;
    }
    let preview_label = if media == Some(super::media::Media::Table) { t!("Table") } else { t!("Preview") };
    ui.horizontal(|ui| {
        ui.add_space(14.0);
        if ui.selectable_label(doc.preview, preview_label).on_hover_text("Ctrl+Alt+P").clicked() {
            doc.preview = true;
        }
        if ui.selectable_label(!doc.preview, t!("Source")).on_hover_text("Ctrl+Alt+P").clicked() {
            doc.preview = false;
            doc.view.focus_request = true;
        }
    });
    if !doc.preview {
        return false;
    }
    match media {
        Some(super::media::Media::Svg) => {
            super::media::show_svg(ui, doc);
            return true;
        }
        Some(super::media::Media::Table) => {
            super::media::show_table(ui, doc);
            return true;
        }
        _ => {}
    }
    let text = doc.buffer.rope.to_string();
    let images = super::markdown::Images { base_dir: crate::host::parent(&doc.path), cache: &app.image_cache };
    let mut output = None;
    egui::Frame::new().fill(theme::EDITOR_BACKGROUND).show(ui, |ui| {
        egui::ScrollArea::vertical().id_salt(("md_preview", doc.id)).auto_shrink([false, false]).show(ui, |ui| {
            // Comfortable reading width, centered.
            let width = (ui.available_width() - 40.0).clamp(200.0, 880.0);
            let margin = ((ui.available_width() - width) / 2.0).max(8.0);
            ui.horizontal_top(|ui| {
                ui.add_space(margin);
                ui.vertical(|ui| {
                    ui.set_max_width(width);
                    ui.add_space(12.0);
                    output = Some(super::markdown::render_with(ui, &text, font_size + 1.0, Some(&images)));
                    ui.add_space(40.0);
                });
            });
        });
    });
    if let Some(output) = output {
        if !output.missing.is_empty() {
            app.request_images(output.missing);
        }
        if let Some(target) = output.clicked {
            super::agent::open_link(app, &target);
        }
    }
    true
}

fn breadcrumb(ui: &mut egui::Ui, app: &App, index: usize) {
    let Some(Tab::Doc(doc)) = app.tabs.get(index) else { return };
    let Some(workspace) = &app.workspace else { return };
    let relative = workspace.relative(&doc.path);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
    let mut x = rect.left() + 14.0;
    let parts: Vec<&str> = relative.split('/').collect();
    for (i, part) in parts.iter().enumerate() {
        let last = i + 1 == parts.len();
        let color = if last { theme::TEXT } else { theme::MUTED_TEXT };
        let galley = ui.painter().layout_no_wrap(part.to_string(), FontId::proportional(12.5), color);
        let w = galley.size().x;
        ui.painter().galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley, color);
        x += w + 6.0;
        if !last {
            ui.painter().text(pos2(x, rect.center().y), Align2::LEFT_CENTER, "›", FontId::proportional(12.5), theme::FAINT_TEXT);
            x += 12.0;
        }
    }
}

fn empty_state(ui: &mut egui::Ui, app: &mut App) {
    let rect = ui.available_rect_before_wrap();
    ui.allocate_rect(rect, Sense::hover());
    let center = rect.center();
    let lines = [
        (t!("Go to file"), "Ctrl+P"),
        (t!("Command Palette"), "Ctrl+Shift+P"),
        (t!("Search in Project"), "Ctrl+Shift+F"),
        (t!("Run"), "F5"),
        (t!("Terminal"), "Ctrl+`"),
        ("Git", "Ctrl+Shift+G"),
    ];
    let painter = ui.painter();
    painter.text(center - vec2(0.0, 110.0), Align2::CENTER_CENTER, "◆", FontId::proportional(56.0), theme::SURFACE_HOVER);
    for (i, (label, keys)) in lines.iter().enumerate() {
        let y = center.y - 30.0 + i as f32 * 26.0;
        painter.text(pos2(center.x - 8.0, y), Align2::RIGHT_CENTER, *label, FontId::proportional(13.5), theme::MUTED_TEXT);
        painter.text(pos2(center.x + 8.0, y), Align2::LEFT_CENTER, *keys, FontId::monospace(12.5), theme::FAINT_TEXT);
    }
    let _ = app;
}

// ------------------------------------------------------------ bottom panel

pub fn show(ui: &mut egui::Ui, app: &mut App) {
    let (errors, warnings) = problem_counts(app);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.spacing_mut().item_spacing.x = 2.0;
        let tabs = [
            (PanelTab::Terminal, t!("TERMINAL").to_string()),
            (PanelTab::Problems, if errors + warnings > 0 { t!("PROBLEMS  {n1}", n1 = errors + warnings) } else { t!("PROBLEMS").into() }),
            (PanelTab::Output, t!("OUTPUT").into()),
            (
                PanelTab::References,
                if app.references.is_empty() { t!("REFERENCES").into() } else { t!("REFERENCES  {n1}", n1 = app.references.len()) },
            ),
            (PanelTab::Pulse, t!("PULSE").into()),
        ];
        for (tab, label) in tabs {
            let active = app.panel_tab == tab;
            let text = RichText::new(label).size(11.5).color(if active { theme::TEXT } else { theme::MUTED_TEXT });
            let response = ui.add(egui::Button::new(text).frame(false).min_size(vec2(0.0, 30.0)));
            if active {
                let r = response.rect;
                ui.painter().line_segment(
                    [pos2(r.left() + 4.0, r.bottom() - 1.0), pos2(r.right() - 4.0, r.bottom() - 1.0)],
                    Stroke::new(1.5, theme::ACCENT),
                );
            }
            if response.clicked() {
                app.panel_tab = tab;
            }
            ui.add_space(10.0);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(8.0);
            if ui.small_button("×").on_hover_text(t!("Hide the panel (Ctrl+J)")).clicked() {
                app.panel_open = false;
            }
            if app.panel_tab == PanelTab::Terminal {
                if ui.small_button(t!("Copy last output")).on_hover_text(t!("Copy output since the most recent Enter")).clicked()
                    && app.active_terminal < app.terminals.len()
                {
                    app.terminals[app.active_terminal].copy_last_command(ui.ctx());
                }
                if ui
                    .small_button(t!("Copy output"))
                    .on_hover_text(t!("Copy the retained terminal scrollback and visible output"))
                    .clicked()
                    && app.active_terminal < app.terminals.len()
                {
                    app.terminals[app.active_terminal].copy_scrollback(ui.ctx());
                }
                if ui.small_button("🗑").on_hover_text(t!("Close terminal")).clicked() && app.active_terminal < app.terminals.len() {
                    app.terminals.remove(app.active_terminal);
                    app.active_terminal = app.active_terminal.saturating_sub(1);
                }
                if ui.small_button("+").on_hover_text(t!("New terminal (Ctrl+Shift+`)")).clicked() {
                    app.new_terminal(None);
                }
                // Terminal switcher.
                for index in (0..app.terminals.len()).rev() {
                    let terminal = &app.terminals[index];
                    let alive = terminal.is_alive();
                    let mut label = terminal.window_title().filter(|t| !t.is_empty()).unwrap_or_else(|| terminal.title.clone());
                    label = util::truncate_middle(&label, 28);
                    if !alive {
                        let code = terminal.exit_code.lock().unwrap_or_else(|e| e.into_inner()).unwrap_or(0);
                        label = format!("{label} [{code}]");
                    }
                    let selected = index == app.active_terminal;
                    let color = if !alive {
                        theme::MUTED_TEXT
                    } else if selected {
                        theme::TEXT
                    } else {
                        theme::MUTED_TEXT
                    };
                    if ui.selectable_label(selected, RichText::new(label).size(12.0).color(color)).clicked() {
                        app.active_terminal = index;
                        app.terminal_focus = true;
                    }
                }
            }
        });
    });
    ui.add_space(2.0);
    match app.panel_tab {
        PanelTab::Terminal => terminal_view(ui, app),
        PanelTab::Problems => problems(ui, app),
        PanelTab::Output => output(ui, app),
        PanelTab::References => references(ui, app),
        PanelTab::Pulse => super::pulse::show(ui, app),
    }
}

fn terminal_view(ui: &mut egui::Ui, app: &mut App) {
    if app.terminals.is_empty() {
        ui.vertical_centered(|ui| {
            ui.add_space(30.0);
            if ui.button(t!("Open terminal")).clicked() {
                app.new_terminal(None);
            }
        });
        return;
    }
    app.active_terminal = app.active_terminal.min(app.terminals.len() - 1);
    let font_size = app.config.terminal_font_size;
    let id = egui::Id::unique(("terminal", app.active_terminal));
    if app.terminal_focus {
        ui.memory_mut(|m| m.request_focus(id));
        app.terminal_focus = false;
    }
    let output = app.terminals[app.active_terminal].show(ui, id, font_size);
    if let Some((path, line, col)) = output.open_location {
        let cwd = app.terminals[app.active_terminal].cwd.clone();
        let full = if crate::host::is_absolute(&path) {
            path
        } else if let Some(rest) = path.strip_prefix("~/") {
            host::join(&app.host().map(|h| h.home()).unwrap_or_default(), rest)
        } else {
            host::join(&cwd, path.trim_start_matches("./"))
        };
        app.open_file(full, Some((line.saturating_sub(1), col.saturating_sub(1))));
    }
}

fn problem_counts(app: &App) -> (usize, usize) {
    let mut errors = 0;
    let mut warnings = 0;
    for tab in &app.tabs {
        if let Tab::Doc(doc) = tab {
            let (e, w) = doc.diagnostic_counts();
            errors += e;
            warnings += w;
        }
    }
    for list in app.lsp.closed_diagnostics.values() {
        for (_, d) in list {
            match d.severity {
                Some(lsp_types::DiagnosticSeverity::ERROR) | None => errors += 1,
                Some(lsp_types::DiagnosticSeverity::WARNING) => warnings += 1,
                _ => {}
            }
        }
    }
    (errors, warnings)
}

pub fn status_counts(app: &App) -> (usize, usize) {
    problem_counts(app)
}

fn problems(ui: &mut egui::Ui, app: &mut App) {
    struct Item {
        path: String,
        line: usize,
        col: usize,
        severity: Severity,
        message: String,
        source: String,
    }
    let mut items: Vec<Item> = Vec::new();
    for tab in &app.tabs {
        if let Tab::Doc(doc) = tab {
            for d in doc.all_diagnostics() {
                let (line, col) = doc.buffer.line_col(d.range.start);
                items.push(Item {
                    path: doc.path.clone(),
                    line,
                    col,
                    severity: d.severity,
                    message: d.message.clone(),
                    source: d.source.clone(),
                });
            }
        }
    }
    for (path, list) in &app.lsp.closed_diagnostics {
        for (source, d) in list {
            let severity = match d.severity {
                Some(lsp_types::DiagnosticSeverity::ERROR) | None => Severity::Error,
                Some(lsp_types::DiagnosticSeverity::WARNING) => Severity::Warning,
                Some(lsp_types::DiagnosticSeverity::INFORMATION) => Severity::Info,
                _ => Severity::Hint,
            };
            items.push(Item {
                path: path.clone(),
                line: d.range.start.line as usize,
                col: d.range.start.character as usize,
                severity,
                message: d.message.clone(),
                source: d.source.clone().unwrap_or_else(|| source.clone()),
            });
        }
    }
    items.retain(|i| i.severity != Severity::Hint);
    items.sort_by(|a, b| a.severity.cmp(&b.severity).then_with(|| a.path.cmp(&b.path)).then(a.line.cmp(&b.line)));
    let root = app.workspace.as_ref().map(|w| w.root.clone()).unwrap_or_default();
    let mut open = None;
    if items.is_empty() {
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            ui.label(RichText::new(t!("No problems found")).color(theme::MUTED_TEXT));
        });
    }
    let row_h = 22.0;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, row_h, items.len(), |ui, range| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for item in &items[range] {
            let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
            if response.hovered() {
                ui.painter().rect_filled(rect, 0.0, theme::SURFACE);
            }
            let (icon, color) = match item.severity {
                Severity::Error => ("⊗", theme::ERROR),
                Severity::Warning => ("▲", theme::WARNING),
                _ => ("ℹ", theme::INFO),
            };
            let painter = ui.painter_at(rect);
            painter.text(rect.left_center() + vec2(18.0, 0.0), Align2::CENTER_CENTER, icon, FontId::proportional(12.0), color);
            let message = item.message.lines().next().unwrap_or("");
            let galley = painter.layout_no_wrap(message.to_string(), FontId::proportional(13.0), theme::TEXT);
            let w = galley.size().x;
            painter.galley(rect.left_center() + vec2(32.0, -galley.size().y / 2.0), galley, theme::TEXT);
            let relative = item.path.strip_prefix(&root).unwrap_or(&item.path).trim_start_matches('/');
            let location = format!("{}  {relative}:{}:{}", item.source, item.line + 1, item.col + 1);
            painter.text(
                rect.left_center() + vec2(44.0 + w, 0.0),
                Align2::LEFT_CENTER,
                location,
                FontId::proportional(12.0),
                theme::FAINT_TEXT,
            );
            if response.clicked() {
                open = Some((item.path.clone(), item.line, item.col));
            }
            response.on_hover_text(&item.message);
        }
    });
    if let Some((path, line, col)) = open {
        app.open_file(path, Some((line, col)));
    }
}

fn output(ui: &mut egui::Ui, app: &mut App) {
    egui::ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
        ui.add_space(4.0);
        if let Some(Some(remote)) =
            app.workspace.as_ref().map(|w| if let crate::host::Host::Remote(r) = &w.host { Some(r.clone()) } else { None })
        {
            let status = remote.agent_status.lock().unwrap_or_else(|e| e.into_inner()).clone();
            ui.label(
                RichText::new(format!("SSH {} ({}): {status}", remote.target, remote.platform()))
                    .monospace()
                    .small()
                    .color(theme::MUTED_TEXT),
            );
        }
        let unavailable = app.lsp.unavailable();
        if !unavailable.is_empty() {
            ui.label(
                RichText::new(t!("Language servers not installed: {n1}", n1 = unavailable.join(", ")))
                    .monospace()
                    .small()
                    .color(theme::MUTED_TEXT),
            );
        }
        for (text, error) in &app.output {
            if text.is_empty() {
                continue;
            }
            let color = if *error { theme::ERROR } else { theme::TEXT };
            ui.label(RichText::new(text).monospace().size(12.5).color(color));
        }
    });
}

fn references(ui: &mut egui::Ui, app: &mut App) {
    let mut open = None;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for (path, line, col, label) in &app.references {
            let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
            if response.hovered() {
                ui.painter().rect_filled(rect, 0.0, theme::SURFACE);
            }
            ui.painter().text(
                rect.left_center() + vec2(16.0, 0.0),
                Align2::LEFT_CENTER,
                format!("{label}:{}:{}", line + 1, col + 1),
                FontId::monospace(12.5),
                theme::TEXT,
            );
            if response.clicked() {
                open = Some((path.clone(), *line, *col));
            }
        }
    });
    if let Some((path, line, col)) = open {
        app.open_file(path, Some((line, col)));
    }
}

#[allow(dead_code)]
fn unused(_: Color32) {}
