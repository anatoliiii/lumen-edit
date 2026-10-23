//! Agents live in the project workspace: session navigation beside one conversation.
use super::*;
use eframe::egui::RichText;

#[derive(Clone)]
pub(super) enum Action {
    Create(CreateForm),
    Focus(u64),
    Stop(u64),
    Close(u64),
    Restart(u64),
    Rename(u64, String),
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

pub fn tick(ctx: &egui::Context, app: &mut App) {
    for id in agents(app).map(|a| a.id).collect::<Vec<_>>() {
        if let Some(agent) = by_id(app, id) {
            agent.expire_requests(ctx);
        }
    }
}

pub fn show(ui: &mut egui::Ui, app: &mut App) {
    if app.agent_manager.history_dir != history_dir(app) {
        refresh(app);
    }
    if !agents(app).any(|a| Some(a.id) == app.agent_manager.selected) {
        let selected = agents(app).next().map(|a| a.id);
        app.agent_manager.selected = selected;
    }
    let mut state = std::mem::take(&mut app.agent_manager);
    let mut actions = Vec::new();
    egui::Panel::left("agent_session_navigation")
        .resizable(true)
        .default_size(270.0)
        .size_range(210.0..=420.0)
        .frame(egui::Frame::new().fill(theme::SIDEBAR_BACKGROUND).inner_margin(10))
        .show(ui, |ui| {
            ui.label(RichText::new(t!("Agent sessions")).strong());
            ui.horizontal_wrapped(|ui| {
                if ui.button(t!("New session")).clicked() {
                    state.creating = Some(Role::Independent);
                    state.name.clear();
                    state.task.clear();
                }
                if ui.button(t!("New group")).clicked() {
                    state.creating = Some(Role::Coordinator);
                    state.name.clear();
                    state.task.clear();
                }
            });
            ui.add_space(6.0);
            ui.add(egui::TextEdit::singleline(&mut state.filter).hint_text(t!("search sessions")).desired_width(f32::INFINITY));
            ui.horizontal(|ui| {
                ui.selectable_value(&mut state.history_view, false, t!("Open agents"));
                if ui.selectable_value(&mut state.history_view, true, t!("Saved conversations")).changed() {
                    actions.push(Action::Refresh);
                }
            });
            ui.separator();
            egui::ScrollArea::vertical().id_salt("workspace_session_list").auto_shrink([false, false]).show(ui, |ui| {
                if state.history_view {
                    history(ui, app, &mut state, &mut actions);
                } else {
                    live_list(ui, app, &mut state, &mut actions);
                }
            });
        });
    app.agent_manager = state;
    for action in actions {
        apply(app, action);
    }
    egui::CentralPanel::default().frame(egui::Frame::new().fill(theme::EDITOR_BACKGROUND)).show(ui, |ui| {
        if app.agent_manager.creating.is_some() {
            create_form(ui, app);
        } else if let Some(id) = app.agent_manager.selected.filter(|id| agents(app).any(|a| a.id == *id)) {
            conversation(ui, app, id);
        } else {
            ui.vertical_centered(|ui| {
                ui.add_space(56.0);
                ui.label(RichText::new(t!("Your agents, in this project")).strong().size(22.0));
                ui.add_space(10.0);
                ui.label(t!("Choose a session on the left, or start a new conversation."));
                ui.add_space(16.0);
                if ui.button(t!("New independent session")).clicked() {
                    app.agent_manager.creating = Some(Role::Independent);
                }
                if ui.button(t!("New coordinator group")).clicked() {
                    app.agent_manager.creating = Some(Role::Coordinator);
                }
            });
        }
    });
}

fn live_list(ui: &mut egui::Ui, app: &App, state: &mut Manager, actions: &mut Vec<Action>) {
    let filter = state.filter.to_lowercase();
    let mut ordered = agents(app).collect::<Vec<_>>();
    ordered.sort_by_key(|a| (a.group.as_deref().unwrap_or(""), a.role == Role::Worker, a.id));
    for agent in ordered.into_iter().filter(|a| a.title.to_lowercase().contains(&filter)) {
        if agent.role == Role::Coordinator {
            ui.add_space(8.0);
            ui.label(RichText::new(t!("Coordinator group")).small().strong().color(theme::ACCENT));
        }
        ui.push_id(agent.id, |ui| {
            let selected = state.selected == Some(agent.id) && state.creating.is_none();
            let title = agent.title.trim_start_matches("π ");
            let frame = egui::Frame::new().inner_margin(8).corner_radius(5).fill(if selected {
                theme::ACCENT_DIM
            } else {
                egui::Color32::TRANSPARENT
            });
            let response = frame
                .show(ui, |ui| {
                    ui.set_min_width((ui.available_width() - 16.0).max(0.0));
                    ui.add(egui::Label::new(RichText::new(title).strong()).truncate());
                    ui.horizontal(|ui| {
                        let color = if !agent.requests.is_empty() {
                            theme::WARNING
                        } else if agent.exited.is_some() {
                            theme::ERROR
                        } else {
                            theme::MUTED_TEXT
                        };
                        ui.label(RichText::new(agent.state_label()).small().color(color));
                        if agent.role != Role::Independent {
                            ui.label(RichText::new(agent.role.label()).small().color(theme::FAINT_TEXT));
                        }
                    });
                })
                .response
                .interact(egui::Sense::click());
            if response.clicked() {
                actions.push(Action::Focus(agent.id));
            }
            response.on_hover_text(title);
        });
        ui.add_space(3.0);
    }
}

fn history(ui: &mut egui::Ui, app: &App, state: &mut Manager, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        if ui.checkbox(&mut state.archived, t!("Archived")).changed() {
            actions.push(Action::Refresh);
        }
        if ui.small_button(t!("Refresh")).clicked() {
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
            let opened: HashSet<&str> = agents(app).filter_map(AgentTab::session_path).collect();
            let filter = state.filter.to_lowercase();
            for session in history.iter().filter(|s| {
                format!("{} {}", s.name.as_deref().unwrap_or(""), s.first_message.as_deref().unwrap_or("")).to_lowercase().contains(&filter)
            }) {
                ui.push_id(&session.path, |ui| {
                    let title = session.name.as_deref().or(session.first_message.as_deref()).unwrap_or(t!("(no messages)"));
                    ui.horizontal(|ui| {
                        let label = egui::Label::new(title).truncate().sense(egui::Sense::click());
                        if ui.add_sized([ui.available_width() - 32.0, 24.0], label).on_hover_text(title).clicked() && !state.archived {
                            actions.push(Action::Open(session.path.clone(), false));
                        }
                        ui.menu_button("...", |ui| {
                            if state.archived {
                                if ui.button(t!("Restore")).clicked() {
                                    actions.push(Action::Archive(session.path.clone(), true));
                                    ui.close();
                                }
                            } else {
                                if ui.button(t!("Open")).clicked() {
                                    actions.push(Action::Open(session.path.clone(), false));
                                    ui.close();
                                }
                                if ui.button(t!("Clone")).clicked() {
                                    actions.push(Action::Open(session.path.clone(), true));
                                    ui.close();
                                }
                                if !opened.contains(session.path.as_str()) && ui.button(t!("Archive")).clicked() {
                                    actions.push(Action::Archive(session.path.clone(), false));
                                    ui.close();
                                }
                            }
                        });
                    });
                    ui.label(
                        RichText::new(t!(
                            "{n1} · messages: {n2}",
                            n1 = crate::app::util::time_ago(session.modified),
                            n2 = session.messages
                        ))
                        .small()
                        .color(theme::FAINT_TEXT),
                    );
                    ui.separator();
                });
            }
        }
    }
}

fn conversation(ui: &mut egui::Ui, app: &mut App, id: u64) {
    let mut actions = Vec::new();
    let agent = agents(app).find(|a| a.id == id).unwrap();
    let parent = if agent.role == Role::Coordinator { Some(id) } else { agent.coordinator };
    let mut rename = None;
    let mut add_worker = false;
    egui::Frame::new().fill(theme::TAB_BACKGROUND).inner_margin(12).show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(agent.title.trim_start_matches("π ")).strong().size(18.0));
            ui.label(RichText::new(agent.state_label()).small().color(theme::MUTED_TEXT));
            if ui.add_enabled(agent.busy(), egui::Button::new(t!("Stop"))).clicked() {
                actions.push(Action::Stop(id));
            }
            if parent.is_some() && ui.button(t!("Add worker")).clicked() {
                add_worker = true;
            }
            if agent.role == Role::Coordinator {
                if ui.button(t!("Stop group")).clicked() {
                    actions.push(Action::GroupStop(id));
                }
                if agent.team_paused && ui.button(t!("Resume group")).clicked() {
                    actions.push(Action::Resume(id));
                }
            }
            ui.menu_button(t!("Actions"), |ui| {
                if ui.button(t!("Rename session")).clicked() {
                    rename = Some((id, agent.title.trim_start_matches("π ").into()));
                    ui.close();
                }
                if ui.add_enabled(!agent.busy(), egui::Button::new(t!("Restart pi"))).clicked() {
                    actions.push(Action::Restart(id));
                    ui.close();
                }
                if ui
                    .add_enabled(
                        !agent.busy() && agent.session_path().is_some_and(|p| Path::new(p).is_file()),
                        egui::Button::new(t!("Clone conversation")),
                    )
                    .clicked()
                {
                    actions.push(Action::Clone(id));
                    ui.close();
                }
                if ui.button(t!("Clear queued messages")).clicked() {
                    actions.push(Action::ClearQueue(id));
                    ui.close();
                }
                if ui.button(t!("Close agent (history is kept)")).clicked() {
                    actions.push(Action::Close(id));
                    ui.close();
                }
            });
        });
        if let Some(stats) = &agent.stats {
            let tokens = stats["tokens"]["total"].as_u64().unwrap_or(0);
            let cost = stats["cost"].as_f64().unwrap_or(0.0);
            let context = stats["contextUsage"]["percent"].as_f64().map(|p| format!("{p:.0}%")).unwrap_or_else(|| "—".into());
            ui.label(
                RichText::new(t!(
                    "Session: {tokens} tokens · ${cost} · context {context}",
                    tokens = tokens,
                    cost = format!("{cost:.3}"),
                    context = context
                ))
                .small()
                .color(theme::MUTED_TEXT),
            );
        }
        if let Some(task) = &agent.task
            && let Some(result) = &task.result
        {
            egui::CollapsingHeader::new(t!("Task result")).show(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("workspace_task_result").max_height(120.0).show(ui, |ui| {
                    ui.label(result);
                });
            });
        }
        if !agent.steering_queue.is_empty() || !agent.follow_up_queue.is_empty() {
            egui::CollapsingHeader::new(t!("Queued messages")).show(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("workspace_task_queue").max_height(100.0).show(ui, |ui| {
                    for text in agent.steering_queue.iter().chain(&agent.follow_up_queue) {
                        ui.label(text);
                    }
                });
            });
        }
    });
    if let Some(rename) = rename {
        app.agent_manager.rename = Some(rename);
    }
    if add_worker {
        app.agent_manager.creating = Some(Role::Worker);
    }
    let mut cancel_rename = false;
    if let Some((id, name)) = &mut app.agent_manager.rename {
        ui.horizontal(|ui| {
            let field = ui.text_edit_singleline(name);
            if ui.button(t!("Save")).clicked() || (field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
                actions.push(Action::Rename(*id, name.clone()));
            }
            if ui.button(t!("Cancel")).clicked() {
                cancel_rename = true;
            }
        });
    }
    if cancel_rename {
        app.agent_manager.rename = None;
    }
    for action in actions {
        apply(app, action);
    }
    if let Some(loc) = location(app, id) {
        crate::app::agent::show(ui, app, loc);
    }
}

fn create_form(ui: &mut egui::Ui, app: &mut App) {
    let mut state = std::mem::take(&mut app.agent_manager);
    let role = state.creating.unwrap_or_default();
    let parent = state
        .selected
        .and_then(|id| agents(app).find(|a| a.id == id))
        .and_then(|a| if a.role == Role::Coordinator { Some(a.id) } else { a.coordinator });
    let mut action = None;
    egui::ScrollArea::vertical().id_salt("workspace_new_agent").show(ui, |ui| {
        egui::Frame::new().inner_margin(24).show(ui, |ui| {
            ui.label(
                RichText::new(match role {
                    Role::Independent => t!("New independent session"),
                    Role::Coordinator => t!("New coordinator group"),
                    Role::Worker => t!("Add worker"),
                })
                .strong()
                .size(22.0),
            );
            ui.add_space(12.0);
            ui.label(t!("Name"));
            ui.add(egui::TextEdit::singleline(&mut state.name).desired_width(ui.available_width().min(460.0)));
            ui.add_space(10.0);
            ui.label(t!("Model"));
            let catalog = agents(app).find(|a| !a.models.is_empty()).map(|a| a.models.as_slice()).unwrap_or(&[]);
            let selected = catalog
                .iter()
                .find(|m| format!("{}/{}", m["provider"].as_str().unwrap_or(""), m["id"].as_str().unwrap_or("")) == state.model)
                .map(AgentTab::model_label)
                .unwrap_or_else(|| t!("Default model").into());
            egui::ComboBox::from_id_salt("workspace_agent_model").selected_text(selected).width(ui.available_width().min(460.0)).show_ui(
                ui,
                |ui| {
                    ui.add(egui::TextEdit::singleline(&mut state.model_filter).hint_text(t!("Search models")));
                    if ui.selectable_label(state.model.is_empty(), t!("Default model")).clicked() {
                        state.model.clear();
                    }
                    let filter = state.model_filter.to_lowercase();
                    for model in catalog
                        .iter()
                        .filter(|m| {
                            format!(
                                "{} {} {}",
                                AgentTab::model_label(m),
                                m["provider"].as_str().unwrap_or(""),
                                m["id"].as_str().unwrap_or("")
                            )
                            .to_lowercase()
                            .contains(&filter)
                        })
                        .take(100)
                    {
                        let key = format!("{}/{}", model["provider"].as_str().unwrap_or(""), model["id"].as_str().unwrap_or(""));
                        let label = format!("{} · {}", AgentTab::model_label(model), model["provider"].as_str().unwrap_or(""));
                        if ui
                            .selectable_label(state.model == key, crate::app::util::truncate_middle(&label, 70))
                            .on_hover_text(&key)
                            .clicked()
                        {
                            state.model = key;
                        }
                    }
                },
            );
            ui.add_space(12.0);
            ui.label(t!("Task or goal (optional for a new session)"));
            ui.add(egui::TextEdit::multiline(&mut state.task).desired_rows(6).desired_width(f32::INFINITY));
            if role != Role::Independent {
                ui.add_space(8.0);
                ui.label(
                    RichText::new(t!(
                        "Each agent has its own conversation. Group workers share project files; assign separate file scopes."
                    ))
                    .small()
                    .color(theme::MUTED_TEXT),
                );
            }
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                if ui.add_enabled(role != Role::Worker || parent.is_some(), egui::Button::new(t!("Create session"))).clicked() {
                    action = Some(Action::Create(CreateForm {
                        name: state.name.clone(),
                        model: state.model.clone(),
                        role,
                        parent: if role == Role::Worker { parent } else { None },
                        task: state.task.clone(),
                    }));
                }
                if ui.button(t!("Cancel")).clicked() {
                    state.creating = None;
                }
            });
        });
    });
    app.agent_manager = state;
    if let Some(action) = action {
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
                app.agent_manager.creating = None;
            }
        }
        Action::Focus(id) => {
            focus(app, id);
            app.agent_manager.creating = None;
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
        Action::Open(path, clone_session) => {
            app.agent_manager.creating = None;
            open_history(app, path, clone_session);
        }
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
