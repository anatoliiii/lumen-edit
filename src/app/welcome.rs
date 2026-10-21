//! Welcome screen, SSH connection flow and the remote folder picker.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use eframe::egui::{self, Align2, FontId, RichText, Sense, pos2, vec2};
use lumen_shared::fsops::DirEntry;

use super::{App, Bus, Command, Msg};
use crate::{
    config::{Config, RecentProject},
    host::{self, Host, Remote, ssh, ssh_config},
    terminal::Terminal,
    theme,
};

pub enum Stage {
    Input,
    Connecting { target: String, terminal: Box<Terminal>, alive: Arc<AtomicBool> },
    PickFolder { host: Host, path: String, input: String, entries: Option<Result<Vec<DirEntry>, String>> },
}

pub struct ConnectState {
    pub stage: Stage,
    pub target: String,
    open_path: Option<String>,
    hosts: Vec<ssh_config::SshHostEntry>,
    saved_hosts: Vec<String>,
    pub error: Option<String>,
    focus: bool,
    /// Connect for Teleport only: remember the host, open no project.
    pub register_only: bool,
}

impl ConnectState {
    pub fn input(config: &Config) -> Self {
        Self {
            stage: Stage::Input,
            target: String::new(),
            open_path: None,
            hosts: ssh_config::read_hosts(),
            saved_hosts: config.ssh_hosts.clone(),
            error: None,
            focus: true,
            register_only: false,
        }
    }

    pub fn connecting(target: String, open_path: Option<String>, use_agent: bool, bus: &Bus, ctx: &egui::Context) -> Self {
        let mut state = Self {
            stage: Stage::Input,
            target: target.clone(),
            open_path,
            hosts: Vec::new(),
            saved_hosts: Vec::new(),
            error: None,
            focus: false,
            register_only: false,
        };
        let alive = Arc::new(AtomicBool::new(true));
        let mut command = portable_pty::CommandBuilder::new("ssh");
        command.args(ssh::master_args(&target));
        command.env("TERM", "xterm-256color");
        match Terminal::spawn(format!("ssh {target}"), String::new(), command, ctx.clone(), Some(alive.clone())) {
            Ok(terminal) => {
                let flag = alive.clone();
                let thread_target = target.clone();
                let bus = bus.clone();
                std::thread::spawn(move || {
                    let result = ssh::wait_and_establish(thread_target.clone(), flag, use_agent);
                    bus.send(Msg::Connected { target: thread_target, result });
                });
                state.stage = Stage::Connecting { target, terminal: Box::new(terminal), alive };
            }
            Err(error) => state.error = Some(t!("Could not start ssh: {error}", error = error)),
        }
        state
    }

    pub fn pick_folder(host: Host, path: String, bus: &Bus) -> Self {
        let state = Self {
            stage: Stage::PickFolder { host: host.clone(), path: path.clone(), input: path.clone(), entries: None },
            target: String::new(),
            open_path: None,
            hosts: Vec::new(),
            saved_hosts: Vec::new(),
            error: None,
            focus: false,
            register_only: false,
        };
        list_remote(bus, host, path);
        state
    }
}

fn list_remote(bus: &Bus, host: Host, path: String) {
    bus.spawn(move || {
        let result = host.read_dir(&path).map(|entries| entries.into_iter().filter(|e| e.is_dir).collect());
        Msg::RemoteDir { path, result }
    });
}

pub fn on_remote_dir(app: &mut App, path: String, result: Result<Vec<DirEntry>, String>) {
    if let Some(ConnectState { stage: Stage::PickFolder { path: current, entries, .. }, .. }) = &mut app.connect
        && *current == path
    {
        *entries = Some(result);
    }
}

pub fn on_connected(app: &mut App, target: String, result: Result<Arc<Remote>, String>) {
    let Some(state) = &mut app.connect else {
        // Dialog was closed; drop the connection.
        if let Ok(remote) = result {
            std::thread::spawn(move || remote.disconnect());
        }
        return;
    };
    match result {
        Ok(remote) => {
            if !app.config.ssh_hosts.contains(&target) && !state.hosts.iter().any(|h| h.alias == target) {
                app.config.ssh_hosts.insert(0, target.clone());
                app.config.ssh_hosts.truncate(20);
                app.config.save();
            }
            let status = remote.agent_status.lock().unwrap().clone();
            app.output
                .push((t!("SSH {target}: connected ({n1}, {status})", target = target, n1 = remote.platform(), status = status), false));
            let host = Host::Remote(remote);
            if !app.known_hosts.iter().any(|h| h.same(&host)) {
                app.known_hosts.push(host.clone());
            }
            if state.register_only {
                let old = std::mem::replace(&mut state.stage, Stage::Input);
                if let Stage::Connecting { terminal, .. } = old {
                    app.ssh_masters.push(*terminal);
                }
                app.connect = None;
                app.notify(t!("{target} connected — pick it in the Teleport pane", target = target), false);
                return;
            }
            let open_path = state.open_path.take();
            // Keep the master terminal alive: it owns the connection.
            let old = std::mem::replace(&mut state.stage, Stage::Input);
            if let Stage::Connecting { terminal, .. } = old {
                app.ssh_masters.push(*terminal);
            }
            match open_path {
                Some(path) => {
                    app.connect = None;
                    app.open_workspace(host, path);
                }
                None => {
                    let home = host.home();
                    app.connect = Some(ConnectState::pick_folder(host, home, &app.bus));
                }
            }
        }
        Err(error) => state.error = Some(error),
    }
}

pub fn open_recent(app: &mut App, recent: RecentProject) {
    if recent.host == "local" {
        app.open_workspace(Host::Local, recent.path);
        return;
    }
    let target = recent.host.trim_start_matches("ssh:").to_string();
    // Reuse the current connection to the same host.
    if let Some(Host::Remote(remote)) = app.host()
        && remote.target == target
        && ssh::master_ready(&target)
    {
        app.open_workspace(Host::Remote(remote), recent.path);
        return;
    }
    app.start_connect(target, Some(recent.path));
}

pub fn connect_window(ctx: &egui::Context, app: &mut App) {
    let Some(mut state) = app.connect.take() else { return };
    let mut close = false;
    let mut connect: Option<String> = None;
    let mut open_folder: Option<(Host, String)> = None;
    let mut navigate: Option<String> = None;

    let title = match &state.stage {
        Stage::Input => t!("Connect via SSH").to_string(),
        Stage::Connecting { target, .. } => t!("Connecting to {target}", target = target),
        Stage::PickFolder { host, .. } => t!("Folder on {n1}", n1 = host.label()),
    };
    egui::Window::new(title)
        .id(egui::Id::new("connect_window"))
        .collapsible(false)
        .resizable(true)
        .default_width(640.0)
        .anchor(egui::Align2::CENTER_TOP, vec2(0.0, 80.0))
        .show(ctx, |ui| match &mut state.stage {
            Stage::Input => {
                ui.label(t!("A host from ~/.ssh/config or user@host[:port]. Your ssh is used: keys, agent, ProxyJump."));
                ui.add_space(6.0);
                let input =
                    ui.add(egui::TextEdit::singleline(&mut state.target).hint_text("user@example.com").desired_width(f32::INFINITY));
                if state.focus {
                    input.request_focus();
                    state.focus = false;
                }
                if input.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !state.target.trim().is_empty() {
                    connect = Some(state.target.trim().to_string());
                }
                ui.add_space(8.0);
                let filter = state.target.to_lowercase();
                egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                    let saved = state.saved_hosts.iter().map(|h| (h.clone(), String::new()));
                    let from_config = state.hosts.iter().map(|h| {
                        let detail = match (&h.user, &h.hostname) {
                            (Some(u), Some(n)) => format!("{u}@{n}"),
                            (None, Some(n)) => n.clone(),
                            _ => String::new(),
                        };
                        (h.alias.clone(), detail)
                    });
                    for (alias, detail) in saved.chain(from_config) {
                        if !filter.is_empty() && !alias.to_lowercase().contains(&filter) && !detail.to_lowercase().contains(&filter) {
                            continue;
                        }
                        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
                        if response.hovered() {
                            ui.painter().rect_filled(rect, 4.0, theme::SURFACE_HOVER);
                        }
                        ui.painter().text(
                            rect.left_center() + vec2(10.0, 0.0),
                            Align2::LEFT_CENTER,
                            format!("⇄  {alias}"),
                            FontId::proportional(13.5),
                            theme::TEXT,
                        );
                        ui.painter().text(
                            rect.right_center() - vec2(10.0, 0.0),
                            Align2::RIGHT_CENTER,
                            detail,
                            FontId::proportional(12.0),
                            theme::MUTED_TEXT,
                        );
                        if response.clicked() {
                            connect = Some(alias);
                        }
                    }
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.add_enabled(!state.target.trim().is_empty(), egui::Button::new(t!("Connect"))).clicked() {
                        connect = Some(state.target.trim().to_string());
                    }
                    if ui.button(t!("Cancel")).clicked() {
                        close = true;
                    }
                });
            }
            Stage::Connecting { target, terminal, alive } => {
                let running = alive.load(Ordering::Relaxed);
                ui.horizontal(|ui| {
                    if running && state.error.is_none() {
                        ui.spinner();
                        ui.label(t!("If ssh asks for a password or code, type it in the window below."));
                    }
                });
                if let Some(error) = &state.error {
                    ui.label(RichText::new(error).color(theme::ERROR));
                }
                ui.add_space(4.0);
                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width().max(600.0), 260.0), Sense::hover());
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                let id = egui::Id::new(("ssh_master", target.as_str()));
                if !child.memory(|m| m.has_focus(id)) && running {
                    child.memory_mut(|m| m.request_focus(id));
                }
                terminal.show(&mut child, id, 13.0);
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if !running && ui.button(t!("Retry")).clicked() {
                        connect = Some(target.clone());
                    }
                    if ui.button(t!("Cancel")).clicked() {
                        close = true;
                    }
                });
            }
            Stage::PickFolder { host, path, input, entries } => {
                ui.horizontal(|ui| {
                    if ui.small_button("⇧").on_hover_text(t!("Up")).clicked() {
                        navigate = Some(host::parent(path));
                    }
                    if ui.small_button("⌂").on_hover_text(t!("Home folder")).clicked() {
                        navigate = Some(host.home());
                    }
                    let field = ui.add(egui::TextEdit::singleline(input).desired_width(ui.available_width()));
                    if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        navigate = Some(input.trim().to_string());
                    }
                });
                ui.add_space(6.0);
                egui::ScrollArea::vertical().max_height(360.0).auto_shrink([false, true]).show(ui, |ui| match entries {
                    None => {
                        ui.spinner();
                    }
                    Some(Err(error)) => {
                        ui.label(RichText::new(error.as_str()).color(theme::ERROR));
                    }
                    Some(Ok(list)) => {
                        if list.is_empty() {
                            ui.label(RichText::new(t!("No subfolders")).color(theme::MUTED_TEXT));
                        }
                        for entry in list.iter() {
                            if entry.name.starts_with('.') && entry.name != ".config" {
                                continue;
                            }
                            let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
                            if response.hovered() {
                                ui.painter().rect_filled(rect, 4.0, theme::SURFACE_HOVER);
                            }
                            ui.painter().text(
                                rect.left_center() + vec2(10.0, 0.0),
                                Align2::LEFT_CENTER,
                                format!("▸  {}", entry.name),
                                FontId::proportional(13.5),
                                theme::TEXT,
                            );
                            if response.double_clicked() {
                                open_folder = Some((host.clone(), host::join(path, &entry.name)));
                            } else if response.clicked() {
                                navigate = Some(host::join(path, &entry.name));
                            }
                        }
                    }
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(egui::Button::new(RichText::new(t!("Open {n1}", n1 = host::file_name(path))).strong()).fill(theme::ACCENT_DIM))
                        .clicked()
                    {
                        open_folder = Some((host.clone(), path.clone()));
                    }
                    if ui.button(t!("Cancel")).clicked() {
                        close = true;
                    }
                    ui.label(RichText::new(t!("Double-click opens a folder right away")).small().color(theme::FAINT_TEXT));
                });
            }
        });

    if let Some(path) = navigate
        && let Stage::PickFolder { host, path: current, input, entries } = &mut state.stage
    {
        *current = path.clone();
        *input = path.clone();
        *entries = None;
        list_remote(&app.bus, host.clone(), path);
    }
    if let Some((host, path)) = open_folder {
        app.open_workspace(host, path);
        return;
    }
    if let Some(target) = connect {
        let open_path = state.open_path.take();
        let mut next = ConnectState::connecting(target, open_path, app.config.use_agent, &app.bus, ctx);
        next.register_only = state.register_only;
        app.connect = Some(next);
        return;
    }
    if !close {
        app.connect = Some(state);
    }
}

pub fn show(ui: &mut egui::Ui, app: &mut App) {
    let rect = ui.available_rect_before_wrap();
    let width = rect.width().clamp(420.0, 860.0);
    let mut action: Option<Box<dyn FnOnce(&mut App)>> = None;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.add_space((rect.height() * 0.12).max(24.0));
        ui.vertical_centered(|ui| {
            ui.set_max_width(width);
            ui.label(RichText::new("◆").size(46.0).color(theme::ACCENT));
            ui.label(RichText::new("Lumen Edit").size(30.0).strong());
            ui.label(RichText::new(t!("Native editor: local and over SSH, LSP, terminal, git")).color(theme::MUTED_TEXT));
            ui.add_space(22.0);
            ui.horizontal(|ui| {
                let total = 2.0 * 220.0 + 12.0;
                ui.add_space(((ui.available_width() - total) / 2.0).max(0.0));
                if ui
                    .add(egui::Button::new(RichText::new(t!("🗀  Open Folder")).size(15.0)).min_size(vec2(220.0, 44.0)))
                    .on_hover_text("Ctrl+O")
                    .clicked()
                {
                    action = Some(Box::new(|app: &mut App| app.open_folder_dialog()));
                }
                ui.add_space(12.0);
                if ui
                    .add(
                        egui::Button::new(RichText::new(t!("⇄  Connect via SSH")).size(15.0))
                            .min_size(vec2(220.0, 44.0))
                            .fill(theme::REMOTE_BADGE),
                    )
                    .on_hover_text("Ctrl+Alt+O")
                    .clicked()
                {
                    action = Some(Box::new(|app: &mut App| app.run_command(super::Command::ConnectSsh)));
                }
            });
            ui.add_space(28.0);
        });

        ui.horizontal_top(|ui| {
            ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
            ui.vertical(|ui| {
                ui.set_width(width);
                ui.label(RichText::new(t!("RECENT")).small().strong().color(theme::MUTED_TEXT));
                ui.add_space(4.0);
                if app.config.recent.is_empty() {
                    ui.label(RichText::new(t!("Nothing yet")).color(theme::FAINT_TEXT));
                }
                for recent in app.config.recent.clone().into_iter().take(12) {
                    let (rect, response) = ui.allocate_exact_size(vec2(width, 34.0), Sense::click());
                    if response.hovered() {
                        ui.painter().rect_filled(rect, 6.0, theme::SURFACE);
                    }
                    let remote = recent.host != "local";
                    let (icon, color) = if remote { ("⇄", theme::GIT_ADDED) } else { ("🗀", theme::WARNING) };
                    ui.painter().text(rect.left_center() + vec2(14.0, 0.0), Align2::CENTER_CENTER, icon, FontId::proportional(14.0), color);
                    let name = host::file_name(&recent.path).to_string();
                    let galley = ui.painter().layout_no_wrap(name, FontId::proportional(14.0), theme::TEXT);
                    let w = galley.size().x;
                    ui.painter().galley(pos2(rect.left() + 32.0, rect.center().y - galley.size().y / 2.0), galley, theme::TEXT);
                    let location =
                        if remote { format!("{}:{}", recent.host.trim_start_matches("ssh:"), recent.path) } else { recent.path.clone() };
                    ui.painter().text(
                        pos2(rect.left() + 44.0 + w, rect.center().y),
                        Align2::LEFT_CENTER,
                        super::util::truncate_middle(&location, 80),
                        FontId::proportional(12.0),
                        theme::FAINT_TEXT,
                    );
                    if response.clicked() {
                        let recent = recent.clone();
                        action = Some(Box::new(move |app: &mut App| open_recent(app, recent)));
                    }
                    response.context_menu(|ui| {
                        if ui.button(t!("Remove from list")).clicked() {
                            let recent = recent.clone();
                            action = Some(Box::new(move |app: &mut App| {
                                app.config.recent.retain(|r| *r != recent);
                                app.config.save();
                            }));
                            ui.close();
                        }
                    });
                }
                ui.add_space(24.0);
                ui.label(RichText::new(t!("SHORTCUTS")).small().strong().color(theme::MUTED_TEXT));
                ui.add_space(4.0);
                egui::Grid::new("welcome_keys").num_columns(2).spacing([24.0, 6.0]).show(ui, |ui| {
                    for (commands, label) in [
                        (&[Command::QuickOpen][..], t!("go to file")),
                        (&[Command::CommandPalette], t!("command palette")),
                        (&[Command::RunDefault, Command::Build], t!("run / build")),
                        (&[Command::ToggleTerminal], t!("terminal")),
                        (&[Command::ShowGit], "git"),
                        (&[Command::GotoDefinition, Command::References, Command::Rename], t!("definition / references / rename")),
                        (&[Command::Keybindings], t!("keyboard shortcuts")),
                    ] {
                        let keys: Vec<String> = commands.iter().filter_map(|c| c.shortcut_text()).collect();
                        if keys.is_empty() {
                            continue;
                        }
                        ui.label(RichText::new(keys.join(" / ")).monospace().color(theme::TEXT));
                        ui.label(RichText::new(label).color(theme::MUTED_TEXT));
                        ui.end_row();
                    }
                });
            });
        });
    });
    if let Some(action) = action {
        action(app);
    }
}
