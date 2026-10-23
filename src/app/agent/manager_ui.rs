//! Session manager window; operations are applied after rendering.

use super::*;
use eframe::egui::{RichText, vec2};

#[derive(Clone)]
pub(super) enum Action {
    Create(CreateForm),
    Focus(u64),
    Stop(u64),
    Close(u64),
    Restart(u64),
    Rename(u64, String),
    Dispatch(u64, String),
    GroupStop(u64),
    Resume(u64),
    Open(String, bool),
    Archive(String, bool),
    Refresh,
    Clone(u64),
    ClearQueue(u64),
}
#[derive(Clone)]
pub(super) struct CreateForm {
    name: String,
    model: String,
    role: Role,
    parent: Option<u64>,
    task: String,
}

pub fn show(ctx: &egui::Context, app: &mut App) {
    // Expire hidden-agent requests too, and keep the manager's waiting badges honest.
    for id in agents(app).map(|a| a.id).collect::<Vec<_>>() {
        if let Some(a) = by_id(app, id) {
            a.expire_requests(ctx);
        }
    }
    if !app.agent_manager.open {
        return;
    }
    if app.agent_manager.history_dir != history_dir(app) {
        refresh(app);
    }
    let mut state = std::mem::take(&mut app.agent_manager);
    let mut open = true;
    let mut actions = Vec::new();
    let available = ctx.content_rect();
    egui::Window::new(t!("Agent sessions"))
        .id(egui::Id::unique("agent_manager"))
        .open(&mut open)
        .collapsible(false)
        .default_size(vec2((available.width() - 64.0).clamp(320.0, 1060.0), (available.height() - 80.0).clamp(240.0, 720.0)))
        .vscroll(true)
        .show(ctx, |ui| {
            ui.label(RichText::new(t!("Independent sessions and coordinator groups")).strong().size(18.0));
            ui.label(
                RichText::new(t!("Each agent has its own conversation. Group workers share project files; assign separate file scopes."))
                    .small()
                    .color(theme::MUTED_TEXT),
            );
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(t!("Name"));
                ui.add(egui::TextEdit::singleline(&mut state.name).desired_width(180.0));
                ui.label(t!("Model"));
                ui.add(egui::TextEdit::singleline(&mut state.model).hint_text(t!("Default model")).desired_width(240.0));
                let catalog = agents(app).find(|a| !a.models.is_empty()).map(|a| a.models.as_slice()).unwrap_or(&[]);
                egui::ComboBox::from_id_salt("new_agent_model").selected_text(t!("Choose model")).show_ui(ui, |ui| {
                    if ui.selectable_label(state.model.is_empty(), t!("Default model")).clicked() {
                        state.model.clear();
                    }
                    let filter = state.model.to_lowercase();
                    for model in catalog
                        .iter()
                        .filter(|m| {
                            format!(
                                "{}/{} {}",
                                m["provider"].as_str().unwrap_or(""),
                                m["id"].as_str().unwrap_or(""),
                                AgentTab::model_label(m)
                            )
                            .to_lowercase()
                            .contains(&filter)
                        })
                        .take(100)
                    {
                        let key = format!("{}/{}", model["provider"].as_str().unwrap_or(""), model["id"].as_str().unwrap_or(""));
                        if ui.selectable_label(state.model == key, &key).clicked() {
                            state.model = key;
                        }
                    }
                });
            });
            ui.add(
                egui::TextEdit::multiline(&mut state.task)
                    .desired_rows(2)
                    .desired_width(f32::INFINITY)
                    .hint_text(t!("Task or goal (optional for a new session)")),
            );
            let selected = state.selected.and_then(|id| agents(app).find(|a| a.id == id));
            let parent = selected.and_then(|a| if a.role == Role::Coordinator { Some(a.id) } else { a.coordinator });
            ui.horizontal_wrapped(|ui| {
                for (label, role) in [(t!("New independent session"), Role::Independent), (t!("New coordinator group"), Role::Coordinator)]
                {
                    if ui.button(label).clicked() {
                        actions.push(Action::Create(CreateForm {
                            name: state.name.clone(),
                            model: state.model.clone(),
                            role,
                            parent: None,
                            task: state.task.clone(),
                        }));
                    }
                }
                if ui.add_enabled(parent.is_some(), egui::Button::new(t!("Add worker to selected group"))).clicked() {
                    actions.push(Action::Create(CreateForm {
                        name: state.name.clone(),
                        model: state.model.clone(),
                        role: Role::Worker,
                        parent,
                        task: state.task.clone(),
                    }));
                }
                if ui.add_enabled(selected.is_some(), egui::Button::new(t!("Assign task to selected agent"))).clicked() {
                    actions.push(Action::Dispatch(state.selected.unwrap(), state.task.clone()));
                }
            });
            ui.separator();
            ui.label(RichText::new(t!("Open agents")).strong());
            let mut ordered = agents(app).collect::<Vec<_>>();
            ordered.sort_by_key(|a| (a.group.as_deref().unwrap_or(""), a.role == Role::Worker, a.id));
            for agent in ordered {
                if agent.role == Role::Coordinator {
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&agent.title).strong().color(theme::ACCENT));
                        if ui.small_button(t!("Stop group")).clicked() {
                            actions.push(Action::GroupStop(agent.id));
                        }
                        if agent.team_paused && ui.small_button(t!("Resume group")).clicked() {
                            actions.push(Action::Resume(agent.id));
                        }
                    });
                }
                ui.push_id(agent.id, |ui| {
                    egui::Frame::new()
                        .inner_margin(8)
                        .fill(if state.selected == Some(agent.id) { theme::ACCENT_DIM } else { theme::SURFACE })
                        .corner_radius(6)
                        .show(ui, |ui| {
                            ui.set_min_width((ui.available_width() - 16.0).max(0.0));
                            ui.horizontal_wrapped(|ui| {
                                if ui
                                    .selectable_label(state.selected == Some(agent.id), format!("#{}  {}", agent.id, agent.title))
                                    .clicked()
                                {
                                    state.selected = Some(agent.id);
                                }
                                ui.label(RichText::new(agent.role.label()).small().color(theme::MUTED_TEXT));
                                let color = if !agent.requests.is_empty() {
                                    theme::WARNING
                                } else if !agent.is_running() {
                                    theme::ERROR
                                } else if agent.busy() {
                                    theme::SUCCESS
                                } else {
                                    theme::MUTED_TEXT
                                };
                                ui.label(RichText::new(agent.state_label()).color(color));
                                if let Some(parent) = agent.coordinator {
                                    ui.label(format!("→ #{parent}"));
                                }
                                if ui.small_button(t!("Open")).clicked() {
                                    actions.push(Action::Focus(agent.id));
                                }
                                if ui.add_enabled(agent.is_running(), egui::Button::new(t!("Stop"))).clicked() {
                                    actions.push(Action::Stop(agent.id));
                                }
                                ui.menu_button(t!("Actions"), |ui| {
                                    if (!agent.steering_queue.is_empty() || !agent.follow_up_queue.is_empty())
                                        && ui.button(t!("Clear queued messages")).clicked()
                                    {
                                        actions.push(Action::ClearQueue(agent.id));
                                        ui.close();
                                    }
                                    if ui.button(t!("Rename session")).clicked() {
                                        state.rename = Some((agent.id, agent.title.trim_start_matches("π ").into()));
                                        ui.close();
                                    }
                                    if ui.add_enabled(!agent.busy(), egui::Button::new(t!("Restart pi"))).clicked() {
                                        actions.push(Action::Restart(agent.id));
                                        ui.close();
                                    }
                                    if ui
                                        .add_enabled(
                                            !agent.busy() && agent.session_path().is_some_and(|path| Path::new(path).is_file()),
                                            egui::Button::new(t!("Clone conversation")),
                                        )
                                        .clicked()
                                    {
                                        actions.push(Action::Clone(agent.id));
                                        ui.close();
                                    }
                                    if agent.role == Role::Coordinator {
                                        if ui.button(t!("Stop group")).clicked() {
                                            actions.push(Action::GroupStop(agent.id));
                                            ui.close();
                                        }
                                        if agent.team_paused && ui.button(t!("Resume group")).clicked() {
                                            actions.push(Action::Resume(agent.id));
                                            ui.close();
                                        }
                                    }
                                    if ui.button(t!("Close agent (history is kept)")).clicked() {
                                        actions.push(Action::Close(agent.id));
                                        ui.close();
                                    }
                                });
                            });
                            if let Some(stats) = &agent.stats {
                                let tokens = stats["tokens"]["total"].as_u64().unwrap_or(0);
                                let cost = stats["cost"].as_f64().unwrap_or(0.0);
                                let context =
                                    stats["contextUsage"]["percent"].as_f64().map(|p| format!("{p:.0}%")).unwrap_or_else(|| "—".into());
                                ui.label(
                                    RichText::new(t!(
                                        "Session: {tokens} tokens · ${cost} · context {context}",
                                        tokens = tokens,
                                        cost = format!("{cost:.3}"),
                                        context = context
                                    ))
                                    .small()
                                    .color(theme::FAINT_TEXT),
                                );
                            }
                            if let Some(task) = &agent.task {
                                ui.label(RichText::new(crate::app::util::truncate_middle(&task.text, 160)).small());
                                if let Some(result) = &task.result {
                                    egui::CollapsingHeader::new(t!("Task result")).show(ui, |ui| {
                                        ui.label(result);
                                    });
                                }
                            }
                            if !agent.steering_queue.is_empty() || !agent.follow_up_queue.is_empty() {
                                egui::CollapsingHeader::new(t!("Queued messages")).show(ui, |ui| {
                                    for text in agent.steering_queue.iter().chain(&agent.follow_up_queue) {
                                        ui.label(text);
                                    }
                                });
                            }
                        });
                });
                ui.add_space(5.0);
            }
            if let Some((id, name)) = &mut state.rename {
                ui.horizontal_wrapped(|ui| {
                    ui.label(t!("Rename session"));
                    ui.text_edit_singleline(name);
                    if ui.button(t!("Save")).clicked() {
                        actions.push(Action::Rename(*id, name.clone()));
                    }
                });
            }
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(t!("Saved conversations")).strong());
                ui.add(egui::TextEdit::singleline(&mut state.filter).hint_text(t!("search sessions")).desired_width(200.0));
                if ui.checkbox(&mut state.archived, t!("Archived")).changed() {
                    actions.push(Action::Refresh);
                }
                if ui.button(t!("Refresh")).clicked() {
                    actions.push(Action::Refresh);
                }
            });
            let history = state.history.lock().unwrap().clone();
            match history {
                None => {
                    ui.spinner();
                }
                Some(Err(error)) => {
                    ui.colored_label(theme::ERROR, error);
                }
                Some(Ok(history)) => {
                    let filter = state.filter.to_lowercase();
                    let opened: HashSet<&str> = agents(app).filter_map(AgentTab::session_path).collect();
                    for session in history.iter().filter(|s| {
                        format!("{} {}", s.name.as_deref().unwrap_or(""), s.first_message.as_deref().unwrap_or(""))
                            .to_lowercase()
                            .contains(&filter)
                    }) {
                        ui.push_id(&session.path, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                let title = session.name.as_deref().or(session.first_message.as_deref()).unwrap_or(t!("(no messages)"));
                                if ui
                                    .add_enabled(!state.archived, egui::Button::new(crate::app::util::truncate_middle(title, 65)))
                                    .on_hover_text(&session.path)
                                    .clicked()
                                {
                                    actions.push(Action::Open(session.path.clone(), false));
                                }
                                ui.label(
                                    RichText::new(t!(
                                        "{n1} · messages: {n2}",
                                        n1 = crate::app::util::time_ago(session.modified),
                                        n2 = session.messages
                                    ))
                                    .small()
                                    .color(theme::FAINT_TEXT),
                                );
                                if opened.contains(session.path.as_str()) {
                                    ui.label(t!("Open"));
                                } else if ui.small_button(if state.archived { t!("Restore") } else { t!("Archive") }).clicked() {
                                    actions.push(Action::Archive(session.path.clone(), state.archived));
                                }
                                if !state.archived && ui.small_button(t!("Clone")).clicked() {
                                    actions.push(Action::Open(session.path.clone(), true));
                                }
                            });
                        });
                    }
                }
            }
        });
    state.open = open;
    app.agent_manager = state;
    for action in actions {
        apply(app, action);
    }
}

pub(super) fn apply(app: &mut App, action: Action) {
    match action {
        Action::Create(form) => {
            let result = try_create(
                app,
                Create { name: form.name, model: form.model, role: form.role, coordinator: form.parent, ..Default::default() },
            );
            if let Some(id) = report(app, result) {
                if !form.task.trim().is_empty()
                    && let Err(error) = dispatch(app, id, form.task)
                {
                    app.notify(error, true);
                }
                app.agent_manager.name.clear();
                app.agent_manager.task.clear();
            }
        }
        Action::Focus(id) => {
            focus(app, id);
            app.agent_manager.open = false;
        }
        Action::Stop(id) => {
            stop(app, id);
            deliver(app, id);
        }
        Action::Close(id) => close(app, id),
        Action::Restart(id) => restart(app, id),
        Action::Rename(id, name) => {
            if let Some(agent) = by_id(app, id) {
                agent.send(json!({ "type": "set_session_name", "name": name.trim() }));
            }
            app.agent_manager.rename = None;
        }
        Action::Dispatch(id, text) => {
            if let Err(error) = dispatch(app, id, text) {
                app.notify(error, true);
            }
        }
        Action::GroupStop(id) => group_stop(app, id),
        Action::Resume(id) => {
            let Some(id) = agents(app).find(|a| a.id == id && a.role != Role::Independent).map(|a| a.coordinator.unwrap_or(a.id)) else {
                return;
            };
            if let Some(agent) = by_id(app, id) {
                agent.team_paused = false;
            }
            let ids: Vec<_> = agents(app).filter(|a| a.coordinator == Some(id)).map(|a| a.id).collect();
            for id in ids {
                if let Some(agent) = by_id(app, id) {
                    agent.team_paused = false;
                }
                deliver(app, id);
            }
            persist(app);
        }
        Action::Open(path, clone_session) => open_history(app, path, clone_session),
        Action::Archive(path, restore) => {
            if let Err(error) = archive(app, &path, restore) {
                app.notify(error, true);
            }
            refresh(app);
        }
        Action::Refresh => refresh(app),
        Action::ClearQueue(id) => {
            if let Some(agent) = by_id(app, id) {
                agent.send(json!({ "type": "clear_queue" }));
            }
        }
        Action::Clone(id) => {
            let source = agents(app).find(|a| a.id == id).and_then(AgentTab::session_path).map(str::to_string);
            if let Some(source) = source {
                open_history(app, source, true);
            }
        }
    }
}
