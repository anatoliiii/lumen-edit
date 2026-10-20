//! The pi coding agent in an editor tab. Pi runs as `pi --mode rpc` with the
//! user's own configuration (~/.pi/agent: models, keys, extensions, skills,
//! MCP servers), inside a bubblewrap sandbox for local projects. It reaches
//! the editor through the `lumen` MCP server (deferred tools, discovered on
//! demand) and the `lumen-edit` CLI.

use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{Arc, Mutex},
    time::Instant,
};

use eframe::egui::{self, CornerRadius, Key, RichText, Sense, Stroke, vec2};
use serde_json::{Value, json};

use super::{App, Bus, Msg, editor::render_markdown, util};
use crate::{host::Host, theme};

pub enum Item {
    User(String),
    Assistant { blocks: BTreeMap<usize, Block>, error: Option<String> },
    Tool(ToolCall),
    Notice { text: String, error: bool },
}

pub enum Block {
    Text(String),
    Thinking(String),
}

pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub args: Value,
    pub output: String,
    pub done: bool,
    pub is_error: bool,
    pub duration_ms: Option<u64>,
    pub expanded: bool,
}

struct UiRequest {
    id: String,
    method: String,
    title: String,
    message: String,
    options: Vec<String>,
    value: String,
}

pub struct AgentTab {
    pub id: u64,
    pub title: String,
    child: Option<Child>,
    stdin: Option<Arc<Mutex<ChildStdin>>>,
    pub items: Vec<Item>,
    input: String,
    pub streaming: bool,
    started_at: Option<Instant>,
    model: Option<Value>,
    models: Vec<Value>,
    thinking: String,
    levels: Vec<String>,
    commands: Vec<(String, String)>,
    statuses: BTreeMap<String, String>,
    widgets: BTreeMap<String, Vec<String>>,
    requests: Vec<UiRequest>,
    usage: Option<Value>,
    pub sandboxed: bool,
    pub cwd: String,
    pub exited: Option<String>,
    next_request: u64,
    focus_input: bool,
    stderr_tail: Arc<Mutex<Vec<String>>>,
    modes: Vec<String>,
    model_filter: String,
    picker_open: bool,
    picker_selected: usize,
    /// Scroll the picker to the selection (after arrow keys only).
    picker_scroll: bool,
    collapsed: std::collections::HashSet<String>,
    favorites: Vec<String>,
    current_session: Option<String>,
    sessions_open: bool,
    sessions: Arc<Mutex<Option<Vec<SessionInfo>>>>,
    session_filter: String,
    renaming: Option<String>,
}

#[cfg(test)]
mod tests {
    #[test]
    fn session_dir_matches_pi_layout() {
        let dir = super::session_dir("/home/user/projects/demo-app");
        assert!(dir.ends_with("--home-user-projects-demo-app--"), "{}", dir.display());
    }

    #[test]
    fn splits_locations() {
        assert_eq!(super::split_location("make.bash:76"), ("make.bash".into(), Some(76), None));
        assert_eq!(super::split_location("a/b.sh:65-73"), ("a/b.sh".into(), Some(65), Some(73)));
        assert_eq!(super::split_location("src/main.rs:4:18"), ("src/main.rs".into(), Some(4), None));
        assert_eq!(super::split_location("Cargo.toml"), ("Cargo.toml".into(), None, None));
    }

    #[test]
    fn strips_ansi() {
        assert_eq!(super::strip_ansi("\x1b[39m\x1b[2mask\x1b[22;39m"), "ask");
    }
}

#[derive(Clone)]
pub struct SessionInfo {
    path: String,
    modified: i64,
    name: Option<String>,
    first_message: Option<String>,
    messages: usize,
}

/// pi groups sessions by working directory: `/a/b` -> `--a-b--`.
fn session_dir(cwd: &str) -> PathBuf {
    let base = std::env::var_os("PI_CODING_AGENT_SESSION_DIR").map(PathBuf::from).unwrap_or_else(|| home().join(".pi/agent/sessions"));
    base.join(format!("--{}--", cwd.trim_start_matches('/').replace('/', "-")))
}

/// Reads session files of `cwd`, newest first (runs on a worker thread).
fn list_sessions(cwd: &str) -> Vec<SessionInfo> {
    let mut files: Vec<(PathBuf, std::time::SystemTime)> = std::fs::read_dir(session_dir(cwd))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
        .filter_map(|e| Some((e.path(), e.metadata().ok()?.modified().ok()?)))
        .collect();
    files.sort_by_key(|f| std::cmp::Reverse(f.1));
    files
        .into_iter()
        .take(200)
        .map(|(path, modified)| {
            let mut info = SessionInfo {
                path: path.to_string_lossy().into_owned(),
                modified: modified.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0),
                name: None,
                first_message: None,
                messages: 0,
            };
            if let Ok(file) = std::fs::File::open(&path) {
                for line in BufReader::new(file).lines().map_while(Result::ok) {
                    if line.contains("\"type\":\"session_info\"") {
                        if let Ok(entry) = serde_json::from_str::<Value>(&line) {
                            info.name = entry["name"].as_str().map(str::to_string).filter(|n| !n.is_empty());
                        }
                    } else if line.contains("\"role\":\"user\"") {
                        info.messages += 1;
                        if info.first_message.is_none()
                            && let Ok(entry) = serde_json::from_str::<Value>(&line)
                        {
                            let text = content_text(&entry["message"]["content"]);
                            info.first_message = Some(text.split_whitespace().collect::<Vec<_>>().join(" "));
                        }
                    } else if line.contains("\"role\":\"assistant\"") {
                        info.messages += 1;
                    }
                }
            }
            info
        })
        .filter(|info| info.messages > 0)
        .collect()
}

impl Drop for AgentTab {
    fn drop(&mut self) {
        // Closing stdin asks pi to shut down; kill as a fallback.
        self.stdin = None;
        if let Some(mut child) = self.child.take() {
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(3));
                let _ = child.kill();
                let _ = child.wait();
            });
        }
    }
}

/// Strips ANSI escape sequences that extensions put into status texts.
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            } else if chars.peek() == Some(&']') {
                for c in chars.by_ref() {
                    if c == '\x07' {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

/// Ensures pi knows the `lumen` MCP server with deferred (on-demand) tools.
/// The server connects to the editor named by `$LUMEN_SOCKET`.
pub fn ensure_pi_mcp_entry() -> Result<bool, String> {
    let path = home().join(".pi/agent/mcp.json");
    let mut config: Value = match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?,
        Err(_) => json!({ "mcpServers": {} }),
    };
    // Entry written under the editor's former name.
    let legacy = config.get_mut("mcpServers").and_then(Value::as_object_mut).and_then(|servers| servers.remove("codium")).is_some();
    if config.pointer("/mcpServers/lumen").is_some() && !legacy {
        return Ok(false);
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let installed = home().join(".local/bin/lumen-edit");
    let command = if installed.exists() { installed } else { exe };
    if !config.get("mcpServers").is_some_and(Value::is_object) {
        config["mcpServers"] = json!({});
    }
    config["mcpServers"]["lumen"] = json!({
        "command": command.to_string_lossy(),
        "args": ["mcp", "--granular"],
        "env": { "LUMEN_SOCKET": "${LUMEN_SOCKET}", "LUMEN_MCP_NO_LAUNCH": "1" },
        "exposure": "deferred",
        "description": "The Lumen Edit code editor that hosts this session: open/read/edit files in the editor buffers, project search, LSP (hover, definition, references, diagnostics), git, terminals, tasks, and shell commands on the project host (local or SSH).",
    });
    if path.exists() {
        let _ = std::fs::copy(&path, path.with_extension("json.lumen-backup"));
    }
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&path, serde_json::to_string_pretty(&config).unwrap()).map_err(|e| e.to_string())?;
    Ok(true)
}

pub struct LaunchOptions {
    pub pi_command: String,
    pub sandbox: bool,
    pub extra_args: Vec<String>,
    pub resume: bool,
    pub model: String,
    pub mode: String,
}

impl LaunchOptions {
    pub fn from_config(config: &crate::config::Config, resume: bool) -> Self {
        Self {
            pi_command: config.pi_command.clone(),
            sandbox: config.agent_sandbox,
            extra_args: config.pi_args.clone(),
            resume,
            model: config.agent_model.clone(),
            mode: config.agent_mode.clone(),
        }
    }
}

/// `enabledModels` from pi's settings (`provider/id`), shown as favorites.
fn pi_favorites() -> Vec<String> {
    std::fs::read_to_string(home().join(".pi/agent/settings.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|s| {
            s.get("enabledModels").and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        })
        .unwrap_or_default()
}

/// Model maker: `openai/gpt-6` -> openai, `OpenAI: GPT-6` -> openai.
fn vendor(model: &Value) -> String {
    let id = model["id"].as_str().unwrap_or("");
    let id = id.trim_start_matches('~');
    if let Some((vendor, _)) = id.split_once('/') {
        return vendor.to_lowercase();
    }
    let name = model["name"].as_str().unwrap_or("");
    if let Some((vendor, _)) = name.split_once(':') {
        return vendor.trim().to_lowercase();
    }
    let lower = id.to_lowercase();
    for (prefix, vendor) in [
        ("gpt", "openai"),
        ("claude", "anthropic"),
        ("gemini", "google"),
        ("gemma", "google"),
        ("qwen", "qwen"),
        ("llama", "meta"),
        ("mistral", "mistral"),
        ("glm", "z-ai"),
        ("deepseek", "deepseek"),
    ] {
        if lower.starts_with(prefix) {
            return vendor.into();
        }
    }
    "другие".into()
}

fn model_key(model: &Value) -> String {
    format!("{}/{}", model["provider"].as_str().unwrap_or(""), model["id"].as_str().unwrap_or(""))
}

/// Sessions of this folder: search, switch, new, rename.
fn session_picker(ui: &egui::Ui, agent: &mut AgentTab, anchor: egui::Rect, actions: &mut Vec<Action>) {
    let sessions = agent.sessions.lock().unwrap().clone();
    let mut close = ui.ctx().input(|i| i.key_pressed(Key::Escape));
    let area = egui::Area::new(egui::Id::new(("session_picker", agent.id)))
        .order(egui::Order::Foreground)
        .pivot(egui::Align2::RIGHT_TOP)
        .fixed_pos(anchor.right_bottom() + vec2(0.0, 4.0))
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                ui.set_width(460.0);
                ui.horizontal(|ui| {
                    if ui.button("+ Новая сессия").clicked() {
                        actions.push(Action::NewSession);
                        close = true;
                    }
                    match &mut agent.renaming {
                        Some(name) => {
                            let field = ui.add(egui::TextEdit::singleline(name).hint_text("имя сессии").desired_width(200.0));
                            field.request_focus();
                            if field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                                actions.push(Action::RenameSession(name.trim().to_string()));
                                agent.renaming = None;
                            }
                        }
                        None => {
                            if ui.button("Переименовать текущую").clicked() {
                                agent.renaming = Some(agent.title.trim_start_matches("π ").replace("Агент", ""));
                            }
                        }
                    }
                });
                let search = ui
                    .add(egui::TextEdit::singleline(&mut agent.session_filter).hint_text("поиск по сессиям").desired_width(f32::INFINITY));
                if agent.renaming.is_none() {
                    search.request_focus();
                }
                let filter = agent.session_filter.to_lowercase();
                egui::ScrollArea::vertical().max_height(440.0).auto_shrink([false, true]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    let Some(list) = &sessions else {
                        ui.spinner();
                        return;
                    };
                    if list.is_empty() {
                        ui.label(RichText::new("Сессий в этой папке пока нет").color(theme::MUTED_TEXT));
                    }
                    for session in list {
                        let title = session.name.clone().or(session.first_message.clone()).unwrap_or_else(|| "(без сообщений)".into());
                        if !filter.is_empty()
                            && !format!("{} {}", title, session.first_message.as_deref().unwrap_or("")).to_lowercase().contains(&filter)
                        {
                            continue;
                        }
                        let current = agent.current_session.as_deref() == Some(session.path.as_str());
                        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
                        if current || response.hovered() {
                            ui.painter().rect_filled(rect, 4.0, if current { theme::ACCENT_DIM } else { theme::SURFACE_HOVER });
                        }
                        let painter = ui.painter_at(rect);
                        painter.text(
                            rect.left_top() + vec2(8.0, 5.0),
                            egui::Align2::LEFT_TOP,
                            util::truncate_middle(&title, 64),
                            egui::FontId::proportional(13.0),
                            if current { theme::ACCENT } else { theme::TEXT },
                        );
                        let meta = format!("{} · сообщений: {}", util::time_ago(session.modified), session.messages);
                        painter.text(
                            rect.left_bottom() + vec2(8.0, -5.0),
                            egui::Align2::LEFT_BOTTOM,
                            meta,
                            egui::FontId::proportional(11.0),
                            theme::FAINT_TEXT,
                        );
                        if response.clicked() && !current {
                            actions.push(Action::SwitchSession(session.path.clone()));
                            close = true;
                        }
                        if let Some(first) = &session.first_message {
                            response.on_hover_text(util::truncate_middle(first, 400));
                        }
                    }
                });
            });
        });
    let outside = ui.ctx().input(|i| {
        i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !area.response.rect.contains(p) && !anchor.contains(p))
    });
    if close || outside {
        agent.sessions_open = false;
        agent.renaming = None;
    }
}

/// Searchable model list grouped by provider, then vendor.
fn model_picker(ui: &egui::Ui, agent: &mut AgentTab, anchor: egui::Rect, actions: &mut Vec<Action>) {
    let filter = agent.model_filter.to_lowercase();
    let matches = |model: &Value| {
        let haystack = format!(
            "{} {} {} {}",
            AgentTab::model_label(model),
            model["id"].as_str().unwrap_or(""),
            model["provider"].as_str().unwrap_or(""),
            vendor(model)
        )
        .to_lowercase();
        filter.split_whitespace().all(|word| haystack.contains(word))
    };
    // provider -> vendor -> models, all sorted.
    let mut groups: BTreeMap<String, BTreeMap<String, Vec<&Value>>> = BTreeMap::new();
    for model in agent.models.iter().filter(|m| matches(m)) {
        groups.entry(model["provider"].as_str().unwrap_or("?").to_string()).or_default().entry(vendor(model)).or_default().push(model);
    }
    for vendors in groups.values_mut() {
        for models in vendors.values_mut() {
            models.sort_by_key(|m| AgentTab::model_label(m).to_lowercase());
        }
    }
    let favorites: Vec<&Value> =
        agent.favorites.iter().filter_map(|key| agent.models.iter().find(|m| model_key(m) == *key)).filter(|m| matches(m)).collect();
    // Flat order for keyboard navigation.
    let searching = !filter.is_empty();
    let mut order: Vec<&Value> = favorites.clone();
    for (provider, vendors) in &groups {
        if !searching && agent.collapsed.contains(provider) {
            continue;
        }
        for (vendor, models) in vendors {
            if !searching && agent.collapsed.contains(&format!("{provider}/{vendor}")) {
                continue;
            }
            order.extend(models.iter().copied());
        }
    }
    let mut chosen: Option<(String, String)> = None;
    let mut close = false;
    ui.ctx().input_mut(|i| {
        if i.consume_key(egui::Modifiers::NONE, Key::ArrowDown) {
            agent.picker_selected = (agent.picker_selected + 1).min(order.len().saturating_sub(1));
            agent.picker_scroll = true;
        }
        if i.consume_key(egui::Modifiers::NONE, Key::ArrowUp) {
            agent.picker_selected = agent.picker_selected.saturating_sub(1);
            agent.picker_scroll = true;
        }
        if i.consume_key(egui::Modifiers::NONE, Key::Escape) {
            close = true;
        }
        if i.consume_key(egui::Modifiers::NONE, Key::Enter) {
            chosen = order
                .get(agent.picker_selected)
                .map(|m| (m["provider"].as_str().unwrap_or("").to_string(), m["id"].as_str().unwrap_or("").to_string()));
        }
    });
    let current = agent.model.as_ref().map(model_key);
    let selected_key = order.get(agent.picker_selected).map(|m| model_key(m));
    let scroll_to_selected = std::mem::take(&mut agent.picker_scroll);
    let mut toggle: Option<String> = None;
    let area = egui::Area::new(egui::Id::new(("model_picker", agent.id)))
        .order(egui::Order::Foreground)
        .fixed_pos(anchor.left_bottom() + vec2(0.0, 4.0))
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                ui.set_width(480.0);
                let search = ui.add(
                    egui::TextEdit::singleline(&mut agent.model_filter)
                        .hint_text("поиск: название, вендор, провайдер")
                        .desired_width(f32::INFINITY),
                );
                search.request_focus();
                if search.changed() {
                    agent.picker_selected = 0;
                }
                let total: usize = groups.values().flat_map(|v| v.values()).map(Vec::len).sum();
                ui.label(RichText::new(format!("{total} моделей")).small().color(theme::FAINT_TEXT));
                egui::ScrollArea::vertical().max_height(460.0).auto_shrink([false, true]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    let mut row = |ui: &mut egui::Ui, model: &Value, indent: f32| {
                        let key = model_key(model);
                        let is_current = current.as_deref() == Some(key.as_str());
                        let is_selected = selected_key.as_deref() == Some(key.as_str());
                        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
                        if is_selected || response.hovered() {
                            ui.painter().rect_filled(rect, 3.0, theme::ACCENT_DIM);
                        }
                        if is_selected && scroll_to_selected {
                            response.scroll_to_me(None);
                        }
                        let name = AgentTab::model_label(model);
                        let name = name.split_once(": ").map(|(_, n)| n.to_string()).unwrap_or(name);
                        let color = if is_current { theme::ACCENT } else { theme::TEXT };
                        ui.painter().text(
                            rect.left_center() + vec2(indent, 0.0),
                            egui::Align2::LEFT_CENTER,
                            if is_current { format!("● {name}") } else { name },
                            egui::FontId::proportional(13.0),
                            color,
                        );
                        let context = model["contextWindow"]
                            .as_u64()
                            .map(|c| if c >= 1_000_000 { format!("{:.1}M", c as f64 / 1e6) } else { format!("{}K", c / 1000) })
                            .unwrap_or_default();
                        let free = model["cost"]["input"].as_f64() == Some(0.0) && model["cost"]["output"].as_f64() == Some(0.0);
                        let price = if free {
                            "бесплатно".to_string()
                        } else {
                            format!(
                                "${}/${}",
                                model["cost"]["input"].as_f64().unwrap_or(0.0),
                                model["cost"]["output"].as_f64().unwrap_or(0.0)
                            )
                        };
                        ui.painter().text(
                            rect.right_center() - vec2(8.0, 0.0),
                            egui::Align2::RIGHT_CENTER,
                            format!("{context}  {price}"),
                            egui::FontId::proportional(11.0),
                            theme::FAINT_TEXT,
                        );
                        if response.clicked() {
                            chosen = Some((
                                model["provider"].as_str().unwrap_or("").to_string(),
                                model["id"].as_str().unwrap_or("").to_string(),
                            ));
                        }
                        response.on_hover_text(key);
                    };
                    if !favorites.is_empty() {
                        ui.label(RichText::new("ИЗБРАННЫЕ").small().strong().color(theme::WARNING));
                        for model in &favorites {
                            row(ui, model, 12.0);
                        }
                        ui.add_space(6.0);
                    }
                    for (provider, vendors) in &groups {
                        let count: usize = vendors.values().map(Vec::len).sum();
                        let open = searching || !agent.collapsed.contains(provider);
                        let header = ui.add(
                            egui::Button::new(
                                RichText::new(format!("{} {}  ({count})", if open { "⏷" } else { "⏵" }, provider.to_uppercase()))
                                    .small()
                                    .strong()
                                    .color(theme::MUTED_TEXT),
                            )
                            .frame(false),
                        );
                        if header.clicked() {
                            toggle = Some(provider.clone());
                        }
                        if !open {
                            continue;
                        }
                        for (vendor, models) in vendors {
                            let key = format!("{provider}/{vendor}");
                            let open = searching || !agent.collapsed.contains(&key);
                            let header = ui.horizontal(|ui| {
                                ui.add_space(10.0);
                                ui.add(
                                    egui::Button::new(
                                        RichText::new(format!("{} {vendor}  ({})", if open { "⏷" } else { "⏵" }, models.len()))
                                            .small()
                                            .color(theme::TEXT),
                                    )
                                    .frame(false),
                                )
                            });
                            if header.inner.clicked() {
                                toggle = Some(key);
                            }
                            if open {
                                for model in models {
                                    row(ui, model, 26.0);
                                }
                            }
                        }
                        ui.add_space(4.0);
                    }
                });
            });
        });
    if let Some(key) = toggle
        && !agent.collapsed.remove(&key)
    {
        agent.collapsed.insert(key);
    }
    if let Some((provider, id)) = chosen {
        actions.push(Action::SetModel(provider, id));
        close = true;
    }
    let clicked_outside = ui.ctx().input(|i| {
        i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !area.response.rect.contains(p) && !anchor.contains(p))
    });
    if close || clicked_outside {
        agent.picker_open = false;
    }
}

/// Modes defined by the pi-modes extension (~/.pi/agent/modes.config.json).
fn pi_modes() -> Vec<String> {
    std::fs::read_to_string(home().join(".pi/agent/modes.config.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|config| config.get("modes").and_then(Value::as_object).map(|m| m.keys().cloned().collect()))
        .unwrap_or_default()
}

impl AgentTab {
    /// Starts pi for the project. Remote projects get a local scratch folder
    /// as working directory and work on the project through lumen tools.
    pub fn spawn(id: u64, host: &Host, root: &str, socket: Option<&Path>, options: &LaunchOptions, bus: Bus) -> Self {
        let mut tab = AgentTab {
            id,
            title: "π Агент".into(),
            child: None,
            stdin: None,
            items: Vec::new(),
            input: String::new(),
            streaming: false,
            started_at: None,
            model: None,
            models: Vec::new(),
            thinking: String::new(),
            levels: Vec::new(),
            commands: Vec::new(),
            statuses: BTreeMap::new(),
            widgets: BTreeMap::new(),
            requests: Vec::new(),
            usage: None,
            sandboxed: false,
            cwd: root.to_string(),
            exited: None,
            next_request: 1,
            focus_input: true,
            stderr_tail: Arc::default(),
            modes: pi_modes(),
            model_filter: String::new(),
            picker_open: false,
            picker_selected: 0,
            picker_scroll: false,
            collapsed: std::collections::HashSet::new(),
            favorites: pi_favorites(),
            current_session: None,
            sessions_open: false,
            sessions: Arc::default(),
            session_filter: String::new(),
            renaming: None,
        };

        let (cwd, context) = match host {
            Host::Local => (
                PathBuf::from(root),
                format!(
                    "You are running inside the Lumen Edit editor with the project at {root}. Besides your own tools you can drive the editor: \
                     its MCP server `lumen` (find its tools with tool_search, e.g. \"lumen diagnostics\", \"lumen open file\") and the `lumen-edit` CLI \
                     (`lumen-edit methods`, `lumen-edit help <method>`). Use them to open files for the user, read LSP diagnostics, hover and definitions, \
                     run tasks in visible terminals and inspect git."
                ),
            ),
            Host::Remote(remote) => {
                let scratch = dirs::cache_dir()
                    .unwrap_or_else(std::env::temp_dir)
                    .join("lumen-edit/agent")
                    .join(remote.target.replace(['/', ':'], "_"));
                let _ = std::fs::create_dir_all(&scratch);
                (
                    scratch,
                    format!(
                        "The project lives on the SSH host `{}` at {root}. Your built-in file and shell tools are disabled because they would act on \
                         this machine, not on the project. Work on the project through the editor's MCP server `lumen`: first load its tools with \
                         tool_search (e.g. query \"lumen exec read_file edit write_file list_files search diagnostics git\"), then use exec to run shell \
                         commands on the remote host (cwd = project root), read_file/edit/write_file/create_file for files, search and list_files to \
                         explore, diagnostics/hover/definition for code intelligence, git_* for git and terminal_new to run long jobs visibly. \
                         Paths are relative to the project root.",
                        remote.target
                    ),
                )
            }
        };
        tab.cwd = cwd.to_string_lossy().into_owned();

        match ensure_pi_mcp_entry() {
            Ok(true) => tab
                .items
                .push(Item::Notice {
                    text: "В ~/.pi/agent/mcp.json добавлен сервер lumen (инструменты по запросу)".into(), error: false
                }),
            Ok(false) => {}
            Err(error) => {
                tab.items.push(Item::Notice { text: format!("Не удалось настроить MCP для pi: {error}"), error: true })
            }
        }

        let mut pi_args: Vec<String> = vec!["--mode".into(), "rpc".into(), "--append-system-prompt".into(), context];
        if host.is_remote() {
            // Local read/bash/edit would act on the scratch folder, not on the
            // project; the lumen tools work on the remote host instead.
            pi_args.push("--no-builtin-tools".into());
        }
        if options.resume {
            pi_args.push("--continue".into());
        }
        if !options.model.trim().is_empty() && !options.resume {
            pi_args.extend(["--model".into(), options.model.trim().to_string()]);
        }
        pi_args.extend(options.extra_args.iter().cloned());

        // Absolute path: inside bwrap there is no shell to search PATH.
        let pi = if options.pi_command.contains('/') { Some(PathBuf::from(&options.pi_command)) } else { which(&options.pi_command) };
        let Some(pi) = pi else {
            tab.exited = Some(format!(
                "{} не найден. Укажите полный путь в настройках (Агент pi: команда), PATH: {}",
                options.pi_command,
                crate::host::local_path()
            ));
            return tab;
        };
        let bwrap = which("bwrap").filter(|_| options.sandbox);
        let mut command = if let Some(bwrap) = bwrap {
            tab.sandboxed = true;
            let mut command = Command::new(bwrap);
            command.args(sandbox_args(&cwd));
            command.arg("--").arg(&pi).args(&pi_args);
            command
        } else {
            let mut command = Command::new(&pi);
            command.args(&pi_args);
            command
        };
        command
            .current_dir(&cwd)
            .env("PATH", crate::host::local_path())
            .env("LUMEN_PROJECT_ROOT", root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(socket) = socket {
            command.env("LUMEN_SOCKET", socket);
        }

        match command.spawn() {
            Ok(mut child) => {
                let stdout = child.stdout.take().unwrap();
                let stderr = child.stderr.take().unwrap();
                tab.stdin = Some(Arc::new(Mutex::new(child.stdin.take().unwrap())));
                tab.child = Some(child);
                let reader_bus = bus.clone();
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stdout);
                    let mut line = Vec::new();
                    // Split strictly on LF; JSON strings may contain U+2028.
                    while reader.read_until(b'\n', &mut line).map(|n| n > 0).unwrap_or(false) {
                        if let Ok(value) = serde_json::from_slice::<Value>(&line) {
                            reader_bus.send(Msg::Agent { id, event: value });
                        }
                        line.clear();
                    }
                    reader_bus.send(Msg::Agent { id, event: json!({ "type": "__exit" }) });
                });
                let tail = tab.stderr_tail.clone();
                std::thread::spawn(move || {
                    for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                        let mut tail = tail.lock().unwrap();
                        tail.push(line);
                        if tail.len() > 30 {
                            tail.remove(0);
                        }
                    }
                });
                for command in ["get_state", "get_available_models", "get_commands", "get_available_thinking_levels"] {
                    tab.send(json!({ "type": command }));
                }
                if options.resume {
                    tab.send(json!({ "type": "get_messages" }));
                }
                let mode = options.mode.trim();
                if !mode.is_empty() && tab.modes.iter().any(|m| m == mode) {
                    tab.send(json!({ "type": "prompt", "message": format!("/mode {mode}") }));
                }
            }
            Err(error) => {
                tab.exited = Some(format!("не удалось запустить {}: {error}", options.pi_command));
            }
        }
        tab
    }

    fn send(&mut self, mut command: Value) {
        if command.get("id").is_none() {
            command["id"] = json!(format!("c{}", self.next_request));
            self.next_request += 1;
        }
        if let Some(stdin) = &self.stdin {
            let mut stdin = stdin.lock().unwrap();
            let _ = writeln!(stdin, "{command}");
            let _ = stdin.flush();
        }
    }

    pub fn sessions_json(&self) -> Value {
        let list = list_sessions(&self.cwd);
        json!(
            list.iter()
                .map(|s| json!({
                    "path": s.path,
                    "name": s.name,
                    "first_message": s.first_message.as_deref().map(|m| util::truncate_middle(m, 120)),
                    "messages": s.messages,
                    "updated": util::time_ago(s.modified),
                    "current": self.current_session.as_deref() == Some(s.path.as_str()),
                }))
                .collect::<Vec<_>>()
        )
    }

    /// Switches to a session file, or starts a new one for an empty path.
    pub fn switch_session(&mut self, path: String) {
        if path.is_empty() {
            self.send(json!({ "type": "new_session" }));
        } else {
            self.send(json!({ "type": "switch_session", "sessionPath": path }));
        }
    }

    pub fn abort(&mut self) {
        self.send(json!({ "type": "clear_queue" }));
        self.send(json!({ "type": "abort" }));
    }

    /// Plain-text transcript for the control API.
    pub fn transcript(&self, last: usize) -> Vec<String> {
        let start = self.items.len().saturating_sub(last);
        self.items[start..]
            .iter()
            .map(|item| match item {
                Item::User(text) => format!("user: {text}"),
                Item::Assistant { blocks, error } => {
                    let text: Vec<String> =
                        blocks.values().filter_map(|b| if let Block::Text(t) = b { Some(t.clone()) } else { None }).collect();
                    format!("assistant: {}{}", text.join("\n"), error.as_ref().map(|e| format!(" [ошибка: {e}]")).unwrap_or_default())
                }
                Item::Tool(tool) => {
                    let status = if !tool.done {
                        "…"
                    } else if tool.is_error {
                        "✗"
                    } else {
                        "✓"
                    };
                    format!("tool {status} {} {}: {}", tool.name, tool_summary(tool), tool.output.chars().take(300).collect::<String>())
                }
                Item::Notice { text, .. } => format!("note: {text}"),
            })
            .collect()
    }

    pub fn is_running(&self) -> bool {
        self.exited.is_none() && self.child.is_some()
    }

    pub fn prompt(&mut self, text: String) {
        let text = text.trim().to_string();
        if text.is_empty() {
            return;
        }
        let mut command = json!({ "type": "prompt", "message": text });
        if self.streaming {
            command["streamingBehavior"] = json!("steer");
        }
        self.send(command);
    }

    /// Current pi-modes mode, as shown by its widget.
    fn current_mode(&self) -> Option<String> {
        self.widgets.get("pi-modes").and_then(|lines| lines.first()).map(|l| l.trim().to_string()).filter(|l| !l.is_empty())
    }

    fn model_label(model: &Value) -> String {
        let name = model.get("name").and_then(Value::as_str).or(model.get("id").and_then(Value::as_str)).unwrap_or("?");
        name.to_string()
    }

    /// Applies one record from pi's stdout. Returns true when a tool that
    /// may have changed files finished.
    pub fn handle(&mut self, event: Value) -> bool {
        let kind = event.get("type").and_then(Value::as_str).unwrap_or("");
        match kind {
            "response" => {
                let command = event.get("command").and_then(Value::as_str).unwrap_or("");
                let success = event.get("success").and_then(Value::as_bool).unwrap_or(false);
                let data = event.get("data").cloned().unwrap_or(Value::Null);
                if !success {
                    let error = event.get("error").and_then(Value::as_str).unwrap_or("ошибка");
                    self.items.push(Item::Notice { text: format!("{command}: {error}"), error: true });
                    return false;
                }
                match command {
                    "get_state" => {
                        self.model = data.get("model").cloned();
                        self.thinking = data.get("thinkingLevel").and_then(Value::as_str).unwrap_or("").to_string();
                        self.streaming = data.get("isStreaming").and_then(Value::as_bool).unwrap_or(false);
                        if let Some(name) = data.get("sessionName").and_then(Value::as_str) {
                            self.title = format!("π {name}");
                        } else {
                            self.title = "π Агент".into();
                        }
                        self.current_session = data.get("sessionFile").and_then(Value::as_str).map(str::to_string);
                    }
                    "get_available_models" => {
                        self.models = data.get("models").and_then(Value::as_array).cloned().unwrap_or_default();
                        // Large catalogs (e.g. openrouter) start collapsed.
                        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
                        for model in &self.models {
                            *counts.entry(model["provider"].as_str().unwrap_or("?").to_string()).or_default() += 1;
                        }
                        for (provider, count) in counts {
                            if count > 40 {
                                self.collapsed.insert(provider);
                            }
                        }
                    }
                    "get_available_thinking_levels" => {
                        self.levels = data
                            .get("levels")
                            .and_then(Value::as_array)
                            .map(|l| l.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                            .unwrap_or_default()
                    }
                    "get_commands" => {
                        self.commands = data
                            .get("commands")
                            .and_then(Value::as_array)
                            .map(|list| {
                                list.iter()
                                    .map(|c| {
                                        (c["name"].as_str().unwrap_or("").to_string(), c["description"].as_str().unwrap_or("").to_string())
                                    })
                                    .collect()
                            })
                            .unwrap_or_default()
                    }
                    "set_model" | "cycle_model" => {
                        self.model = data.get("model").cloned().or(Some(data.clone()));
                        self.send(json!({ "type": "get_available_thinking_levels" }));
                        self.send(json!({ "type": "get_state" }));
                    }
                    "get_messages" => {
                        if let Some(messages) = data.get("messages").and_then(Value::as_array) {
                            for message in messages {
                                self.add_message(message);
                            }
                        }
                    }
                    "new_session" | "switch_session" => {
                        if data.get("cancelled").and_then(Value::as_bool) != Some(true) {
                            self.items.clear();
                            self.usage = None;
                            if command == "switch_session" {
                                self.send(json!({ "type": "get_messages" }));
                            }
                            self.send(json!({ "type": "get_state" }));
                        }
                    }
                    "set_session_name" => self.send(json!({ "type": "get_state" })),
                    "get_session_stats" => {}
                    _ => {}
                }
            }
            "agent_start" => {
                self.streaming = true;
                self.started_at.get_or_insert_with(Instant::now);
            }
            "agent_settled" => {
                self.streaming = false;
                self.started_at = None;
                if event.get("aborted").and_then(Value::as_bool) == Some(true) {
                    self.items.push(Item::Notice { text: "Остановлено".into(), error: false });
                }
            }
            "message_start" => {
                let message = &event["message"];
                match message.get("role").and_then(Value::as_str) {
                    Some("user") => self.items.push(Item::User(content_text(&message["content"]))),
                    Some("assistant") => self.items.push(Item::Assistant { blocks: BTreeMap::new(), error: None }),
                    _ => {}
                }
            }
            "message_update" => {
                if let Some(usage) = event.get("usage") {
                    self.usage = Some(usage.clone());
                }
                let update = &event["assistantMessageEvent"];
                let index = update.get("contentIndex").and_then(Value::as_u64).unwrap_or(0) as usize;
                let Some(Item::Assistant { blocks, .. }) = self.items.iter_mut().rev().find(|i| matches!(i, Item::Assistant { .. })) else {
                    return false;
                };
                match update.get("type").and_then(Value::as_str) {
                    Some("text_start") => {
                        blocks.insert(index, Block::Text(String::new()));
                    }
                    Some("thinking_start") => {
                        blocks.insert(index, Block::Thinking(String::new()));
                    }
                    Some("text_delta") => {
                        if let Some(Block::Text(text)) = blocks.entry(index).or_insert(Block::Text(String::new())).into() {
                            text.push_str(update["delta"].as_str().unwrap_or(""));
                        }
                    }
                    Some("thinking_delta") => {
                        if let Some(Block::Thinking(text)) = blocks.entry(index).or_insert(Block::Thinking(String::new())).into() {
                            text.push_str(update["delta"].as_str().unwrap_or(""));
                        }
                    }
                    _ => {}
                }
            }
            "message_end" => {
                let message = &event["message"];
                if message.get("role").and_then(Value::as_str) == Some("assistant") {
                    let (blocks_new, error) = assistant_blocks(message);
                    if let Some(Item::Assistant { blocks, error: slot }) =
                        self.items.iter_mut().rev().find(|i| matches!(i, Item::Assistant { .. }))
                    {
                        *blocks = blocks_new;
                        *slot = error;
                    }
                    if let Some(usage) = message.get("usage") {
                        self.usage = Some(usage.clone());
                    }
                }
            }
            "tool_execution_start" => {
                self.items.push(Item::Tool(ToolCall {
                    id: event["toolCallId"].as_str().unwrap_or("").to_string(),
                    name: event["toolName"].as_str().unwrap_or("tool").to_string(),
                    args: event["args"].clone(),
                    output: String::new(),
                    done: false,
                    is_error: false,
                    duration_ms: None,
                    expanded: false,
                }));
            }
            "tool_execution_update" | "tool_execution_end" => {
                let id = event["toolCallId"].as_str().unwrap_or("");
                let end = kind == "tool_execution_end";
                let result = if end { &event["result"] } else { &event["partialResult"] };
                if let Some(Item::Tool(tool)) = self.items.iter_mut().rev().find(|i| matches!(i, Item::Tool(t) if t.id == id)) {
                    let text = content_text(&result["content"]);
                    if !text.is_empty() || end {
                        tool.output = text;
                    }
                    if end {
                        tool.done = true;
                        tool.is_error = event["isError"].as_bool().unwrap_or(false);
                        tool.duration_ms = event["durationMs"].as_u64();
                        let name = tool.name.to_lowercase();
                        return !(name.contains("read")
                            || name.contains("search")
                            || name.contains("grep")
                            || name.contains("find")
                            || name.contains("ls"));
                    }
                }
            }
            "auto_retry_start" => {
                let error = event["errorMessage"].as_str().unwrap_or("");
                self.items
                    .push(Item::Notice {
                        text: format!("Повтор {}/{}: {error}", event["attempt"], event["maxAttempts"]), error: true
                    });
            }
            "compaction_start" => self.items.push(Item::Notice { text: "Сжатие контекста…".into(), error: false }),
            "compaction_end" => {
                let text = if let Some(error) = event["errorMessage"].as_str() {
                    format!("Сжатие не удалось: {error}")
                } else {
                    "Контекст сжат".into()
                };
                self.items.push(Item::Notice { text, error: event["errorMessage"].is_string() });
            }
            "thinking_level_changed" => self.thinking = event["level"].as_str().unwrap_or("").to_string(),
            "session_info_changed" => {
                if let Some(name) = event["name"].as_str() {
                    self.title = format!("π {name}");
                }
            }
            "extension_error" => {
                self.items
                    .push(Item::Notice { text: format!("Расширение: {}", event["error"].as_str().unwrap_or("")), error: true });
            }
            "extension_ui_request" => self.ui_request(&event),
            "__exit" => {
                self.streaming = false;
                let tail = self.stderr_tail.lock().unwrap().join("\n");
                let reason = if tail.trim().is_empty() {
                    "pi завершился".to_string()
                } else {
                    format!("pi завершился:\n{tail}")
                };
                self.exited = Some(reason);
                self.stdin = None;
            }
            _ => {}
        }
        false
    }

    fn add_message(&mut self, message: &Value) {
        match message.get("role").and_then(Value::as_str) {
            Some("user") => self.items.push(Item::User(content_text(&message["content"]))),
            Some("assistant") => {
                let (blocks, error) = assistant_blocks(message);
                self.items.push(Item::Assistant { blocks, error });
                // Show tool calls of history as finished cards.
                if let Some(content) = message["content"].as_array() {
                    for block in content.iter().filter(|b| b["type"] == "toolCall") {
                        self.items.push(Item::Tool(ToolCall {
                            id: block["id"].as_str().unwrap_or("").to_string(),
                            name: block["name"].as_str().unwrap_or("tool").to_string(),
                            args: block["arguments"].clone(),
                            output: String::new(),
                            done: true,
                            is_error: false,
                            duration_ms: None,
                            expanded: false,
                        }));
                    }
                }
            }
            Some("toolResult") => {
                let id = message["toolCallId"].as_str().unwrap_or("");
                if let Some(Item::Tool(tool)) = self.items.iter_mut().rev().find(|i| matches!(i, Item::Tool(t) if t.id == id)) {
                    tool.output = content_text(&message["content"]);
                    tool.is_error = message["isError"].as_bool().unwrap_or(false);
                }
            }
            _ => {}
        }
    }

    fn ui_request(&mut self, event: &Value) {
        let method = event["method"].as_str().unwrap_or("");
        let text = |key: &str| event.get(key).and_then(Value::as_str).unwrap_or("").to_string();
        match method {
            "setStatus" => {
                let key = text("statusKey");
                match event.get("statusText").and_then(Value::as_str) {
                    Some(status) if !strip_ansi(status).trim().is_empty() => {
                        self.statuses.insert(key, strip_ansi(status).trim().to_string());
                    }
                    _ => {
                        self.statuses.remove(&key);
                    }
                }
            }
            "setWidget" => {
                let key = text("widgetKey");
                match event.get("widgetLines").and_then(Value::as_array) {
                    Some(lines) => {
                        let lines: Vec<String> = lines.iter().filter_map(|l| l.as_str().map(strip_ansi)).collect();
                        self.widgets.insert(key, lines);
                    }
                    None => {
                        self.widgets.remove(&key);
                    }
                }
            }
            "notify" => {
                let error = event["notifyType"] == "error";
                self.items.push(Item::Notice { text: strip_ansi(&text("message")), error });
            }
            "setTitle" => {}
            "set_editor_text" => self.input = text("text"),
            "select" | "confirm" | "input" | "editor" => {
                let options = event["options"]
                    .as_array()
                    .map(|o| o.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                    .unwrap_or_default();
                self.requests.push(UiRequest {
                    id: text("id"),
                    method: method.to_string(),
                    title: strip_ansi(&text("title")),
                    message: strip_ansi(&text("message")),
                    options,
                    value: text("prefill"),
                });
            }
            _ => {}
        }
    }

    fn answer(&mut self, index: usize, response: Value) {
        let request = self.requests.remove(index);
        let mut message = response;
        message["type"] = json!("extension_ui_response");
        message["id"] = json!(request.id);
        if let Some(stdin) = &self.stdin {
            let mut stdin = stdin.lock().unwrap();
            let _ = writeln!(stdin, "{message}");
            let _ = stdin.flush();
        }
    }
}

fn which(program: &str) -> Option<PathBuf> {
    crate::host::local_path().split(':').map(|dir| Path::new(dir).join(program)).find(|p| p.is_file())
}

/// bubblewrap: read-only system, writable project and the agent's own state.
fn sandbox_args(project: &Path) -> Vec<String> {
    let home = home();
    let mut args: Vec<String> =
        ["--die-with-parent", "--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc", "--bind", "/tmp", "/tmp", "--share-net"]
            .iter()
            .map(|s| s.to_string())
            .collect();
    let mut writable = vec![project.to_path_buf(), home.join(".pi")];
    for dir in [".cache", ".npm", ".bun", ".cargo/registry", ".cargo/git", "go/pkg", ".local/share/pnpm", ".config/pi"] {
        writable.push(home.join(dir));
    }
    writable.push(crate::control::socket_dir());
    for dir in writable {
        if dir.exists() {
            let dir = dir.to_string_lossy().into_owned();
            args.extend(["--bind".into(), dir.clone(), dir]);
        }
    }
    args.extend(["--chdir".into(), project.to_string_lossy().into_owned()]);
    args
}

fn content_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|b| match b["type"].as_str() {
                Some("text") => b["text"].as_str().map(str::to_string),
                Some("image") => Some("[изображение]".into()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn assistant_blocks(message: &Value) -> (BTreeMap<usize, Block>, Option<String>) {
    let mut blocks = BTreeMap::new();
    if let Some(content) = message["content"].as_array() {
        for (i, block) in content.iter().enumerate() {
            match block["type"].as_str() {
                Some("text") => {
                    blocks.insert(i, Block::Text(block["text"].as_str().unwrap_or("").to_string()));
                }
                Some("thinking") => {
                    blocks.insert(i, Block::Thinking(block["thinking"].as_str().unwrap_or("").to_string()));
                }
                _ => {}
            }
        }
    }
    let error = match message["stopReason"].as_str() {
        Some("error") => Some(message["errorMessage"].as_str().unwrap_or("ошибка модели").to_string()),
        Some("length") => Some("ответ обрезан по лимиту токенов".into()),
        _ => None,
    };
    (blocks, error)
}

/// One-line summary of tool arguments: the command, path or pattern.
fn tool_summary(tool: &ToolCall) -> String {
    for key in ["command", "path", "file_path", "pattern", "query", "url", "method", "action"] {
        if let Some(value) = tool.args.get(key).and_then(Value::as_str) {
            return value.lines().next().unwrap_or("").to_string();
        }
    }
    let text = tool.args.to_string();
    if text == "{}" || text == "null" { String::new() } else { text }
}

fn tool_path(tool: &ToolCall) -> Option<String> {
    ["path", "file_path", "file"].iter().find_map(|k| tool.args.get(*k).and_then(Value::as_str)).map(str::to_string)
}

enum Action {
    Send(String),
    SwitchSession(String),
    RenameSession(String),
    FollowUp(String),
    Abort,
    NewSession,
    Compact,
    Restart,
    SetModel(String, String),
    SetThinking(String),
    OpenFile(String),
    OpenLink(String),
    RunInTerminal(String),
    ToggleDock,
    Hide,
}

/// Where an agent lives: an editor tab or the dock beside the code.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Loc {
    Tab(usize),
    Dock,
}

fn at(app: &mut App, loc: Loc) -> Option<&mut AgentTab> {
    match loc {
        Loc::Tab(index) => match app.tabs.get_mut(index) {
            Some(super::Tab::Agent(agent)) => Some(agent),
            _ => None,
        },
        Loc::Dock => app.agent_dock.as_deref_mut(),
    }
}

/// The agent, wherever it is (dock first).
pub fn find(app: &App) -> Option<Loc> {
    if app.agent_dock.is_some() {
        return Some(Loc::Dock);
    }
    app.tabs.iter().position(|t| matches!(t, super::Tab::Agent(_))).map(Loc::Tab)
}

pub fn agent_mut(app: &mut App) -> Option<&mut AgentTab> {
    let loc = find(app)?;
    at(app, loc)
}

pub fn show(ui: &mut egui::Ui, app: &mut App, loc: Loc) {
    let font_size = app.config.editor_font_size;
    let Some(agent) = at(app, loc) else { return };
    let mut actions = Vec::new();

    // ---- header
    egui::Frame::new().inner_margin(egui::Margin::symmetric(12, 6)).fill(theme::TAB_BACKGROUND).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("π").size(18.0).color(theme::ACCENT).strong());
            let current = agent.model.as_ref().map(AgentTab::model_label).unwrap_or_else(|| "модель…".into());
            let button = ui.add(egui::Button::new(format!("{}  ▾", util::truncate_middle(&current, 28))).min_size(vec2(150.0, 0.0)));
            if button.clicked() {
                agent.picker_open = !agent.picker_open;
                agent.model_filter.clear();
                agent.picker_selected = 0;
            }
            if agent.picker_open {
                model_picker(ui, agent, button.rect, &mut actions);
            }
            if !agent.modes.is_empty() {
                let current = agent.current_mode().unwrap_or_else(|| "режим".into());
                egui::ComboBox::from_id_salt(("agent_mode", agent.id)).selected_text(format!("режим: {current}")).show_ui(ui, |ui| {
                    for mode in &agent.modes {
                        if ui.selectable_label(*mode == current, mode).clicked() {
                            actions.push(Action::Send(format!("/mode {mode}")));
                        }
                    }
                });
            }
            if !agent.levels.is_empty() && agent.levels != ["off"] {
                egui::ComboBox::from_id_salt(("agent_thinking", agent.id)).selected_text(format!("мышление: {}", agent.thinking)).show_ui(
                    ui,
                    |ui| {
                        for level in &agent.levels {
                            if ui.selectable_label(*level == agent.thinking, level).clicked() {
                                actions.push(Action::SetThinking(level.clone()));
                            }
                        }
                    },
                );
            }
        });
        // Second row: sandbox, usage, extension statuses and session buttons.
        ui.horizontal(|ui| {
            let (badge, color, hint) = if agent.sandboxed {
                ("⚿ песочница", theme::SUCCESS, "pi может писать только в папку проекта, ~/.pi и кэши")
            } else {
                ("без песочницы", theme::WARNING, "bubblewrap не найден или песочница отключена в настройках")
            };
            ui.label(RichText::new(badge).small().color(color)).on_hover_text(hint);
            if let Some(usage) = &agent.usage {
                let total = usage["totalTokens"].as_u64().unwrap_or(0);
                let cost = usage["cost"]["total"].as_f64().unwrap_or(0.0);
                let text = if cost > 0.0 { format!("{total} ток · ${cost:.3}") } else { format!("{total} ток") };
                ui.label(RichText::new(text).small().color(theme::FAINT_TEXT));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if loc == Loc::Dock && ui.small_button("×").on_hover_text("Скрыть панель (агент продолжит работу)").clicked()
                {
                    actions.push(Action::Hide);
                }
                let dock_label = if loc == Loc::Dock { "⇱" } else { "⇥" };
                let dock_hint = if loc == Loc::Dock { "Открыть во вкладке" } else { "Справа от кода" };
                if ui.small_button(dock_label).on_hover_text(dock_hint).clicked() {
                    actions.push(Action::ToggleDock);
                }
                if ui.small_button("⟳").on_hover_text("Перезапустить pi").clicked() {
                    actions.push(Action::Restart);
                }
                if ui.small_button("⊟").on_hover_text("Сжать контекст").clicked() {
                    actions.push(Action::Compact);
                }
                if ui.small_button("+").on_hover_text("Новая сессия").clicked() {
                    actions.push(Action::NewSession);
                }
                let sessions_button = ui.small_button("☰ Сессии").on_hover_text("Список сессий этой папки");
                if sessions_button.clicked() {
                    agent.sessions_open = !agent.sessions_open;
                    if agent.sessions_open {
                        agent.session_filter.clear();
                        let slot = agent.sessions.clone();
                        *slot.lock().unwrap() = None;
                        let cwd = agent.cwd.clone();
                        let ctx = ui.ctx().clone();
                        std::thread::spawn(move || {
                            let list = list_sessions(&cwd);
                            *slot.lock().unwrap() = Some(list);
                            ctx.request_repaint();
                        });
                    }
                }
                if agent.sessions_open {
                    session_picker(ui, agent, sessions_button.rect, &mut actions);
                }
                if !agent.statuses.is_empty() {
                    let statuses: Vec<&str> = agent.statuses.values().map(String::as_str).collect();
                    let text = statuses.join("  ·  ");
                    ui.add(egui::Label::new(RichText::new(text).small().color(theme::MUTED_TEXT)).truncate());
                }
            });
        });
    });

    // ---- input area at the bottom (laid out first so the transcript fills the rest)
    let input_height = 150.0;
    let available = ui.available_rect_before_wrap();
    let transcript_rect = egui::Rect::from_min_max(
        available.min,
        egui::pos2(available.right(), (available.bottom() - input_height).max(available.top() + 60.0)),
    );
    let input_rect = egui::Rect::from_min_max(egui::pos2(available.left(), transcript_rect.bottom()), available.max);
    ui.allocate_rect(available, Sense::hover());

    let mut transcript = ui.new_child(egui::UiBuilder::new().max_rect(transcript_rect).layout(egui::Layout::top_down(egui::Align::Min)));
    egui::ScrollArea::vertical().id_salt(("agent_scroll", agent.id)).stick_to_bottom(true).auto_shrink([false, false]).show(
        &mut transcript,
        |ui| {
            ui.set_width(ui.available_width() - 8.0);
            ui.add_space(8.0);
            if agent.items.is_empty() && agent.exited.is_none() {
                ui.vertical_centered(|ui| {
                    ui.add_space(40.0);
                    ui.label(RichText::new("π").size(48.0).color(theme::SURFACE_HOVER));
                    ui.add(
                        egui::Label::new(RichText::new("Агент pi с вашими настройками, расширениями и навыками").color(theme::MUTED_TEXT))
                            .wrap(),
                    );
                    ui.add(egui::Label::new(RichText::new(format!("Папка: {}", agent.cwd)).small().color(theme::FAINT_TEXT)).truncate());
                });
            }
            for (i, item) in agent.items.iter_mut().enumerate() {
                match item {
                    Item::User(text) => {
                        ui.add_space(6.0);
                        egui::Frame::new()
                            .fill(theme::SURFACE)
                            .corner_radius(CornerRadius::same(8))
                            .inner_margin(egui::Margin::symmetric(12, 8))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.label(RichText::new(text.as_str()).color(theme::TEXT));
                            });
                    }
                    Item::Assistant { blocks, error } => {
                        egui::Frame::new().inner_margin(egui::Margin::symmetric(12, 6)).show(ui, |ui| {
                            for block in blocks.values() {
                                match block {
                                    Block::Text(text) if !text.trim().is_empty() => {
                                        if let Some(target) = render_markdown(ui, text, font_size) {
                                            actions.push(Action::OpenLink(target));
                                        }
                                    }
                                    Block::Thinking(text) if !text.trim().is_empty() => {
                                        egui::CollapsingHeader::new(RichText::new("размышления").small().color(theme::FAINT_TEXT))
                                            .id_salt(("thinking", agent.id, i))
                                            .show(ui, |ui| {
                                                ui.label(RichText::new(text.as_str()).small().italics().color(theme::MUTED_TEXT));
                                            });
                                    }
                                    _ => {}
                                }
                            }
                            if let Some(error) = error {
                                ui.label(RichText::new(error.as_str()).color(theme::ERROR));
                            }
                        });
                    }
                    Item::Tool(tool) => {
                        let stroke_color = if tool.is_error { theme::ERROR.gamma_multiply(0.5) } else { theme::BORDER };
                        egui::Frame::new()
                            .stroke(Stroke::new(1.0, stroke_color))
                            .corner_radius(CornerRadius::same(6))
                            .inner_margin(egui::Margin::symmetric(10, 5))
                            .outer_margin(egui::Margin::symmetric(12, 2))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                let header = ui.horizontal(|ui| {
                                    if !tool.done {
                                        ui.spinner();
                                    } else if tool.is_error {
                                        ui.label(RichText::new("✗").color(theme::ERROR));
                                    } else {
                                        ui.label(RichText::new("✓").color(theme::SUCCESS));
                                    }
                                    ui.label(RichText::new(&tool.name).strong().monospace().size(font_size - 1.5));
                                    let summary = tool_summary(tool);
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        if let Some(ms) = tool.duration_ms {
                                            ui.label(
                                                RichText::new(format!("{:.1} с", ms as f64 / 1000.0)).small().color(theme::FAINT_TEXT),
                                            );
                                        }
                                        if let Some(path) = tool_path(tool)
                                            && ui.small_button("открыть").clicked()
                                        {
                                            actions.push(Action::OpenFile(path));
                                        }
                                        if let Some(command) = tool.args.get("command").and_then(Value::as_str)
                                            && ui
                                                .small_button("в терминале")
                                                .on_hover_text("Запустить эту команду во встроенном терминале")
                                                .clicked()
                                        {
                                            actions.push(Action::RunInTerminal(command.to_string()));
                                        }
                                        // Last in right-to-left order: takes and truncates to what is left.
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(summary).monospace().size(font_size - 2.0).color(theme::MUTED_TEXT),
                                            )
                                            .truncate(),
                                        );
                                    });
                                });
                                if header.response.interact(Sense::click()).clicked() {
                                    tool.expanded = !tool.expanded;
                                }
                                let output = tool.output.trim_end();
                                if !output.is_empty() {
                                    let lines: Vec<&str> = output.lines().collect();
                                    let shown = if tool.expanded {
                                        lines.len()
                                    } else {
                                        lines.len().min(if tool.done && !tool.is_error { 3 } else { 8 })
                                    };
                                    let start = if tool.done { 0 } else { lines.len() - shown };
                                    let text = lines[start..start + shown].join("\n");
                                    ui.label(RichText::new(text).monospace().size(font_size - 2.5).color(if tool.is_error {
                                        theme::ERROR
                                    } else {
                                        theme::MUTED_TEXT
                                    }));
                                    if lines.len() > shown {
                                        let label = format!("… ещё {} строк", lines.len() - shown);
                                        if ui
                                            .add(egui::Button::new(RichText::new(label).small().color(theme::ACCENT)).frame(false))
                                            .clicked()
                                        {
                                            tool.expanded = true;
                                        }
                                    } else if tool.expanded
                                        && lines.len() > 3
                                        && ui
                                            .add(egui::Button::new(RichText::new("свернуть").small().color(theme::ACCENT)).frame(false))
                                            .clicked()
                                    {
                                        tool.expanded = false;
                                    }
                                }
                            });
                    }
                    Item::Notice { text, error } => {
                        ui.horizontal(|ui| {
                            ui.add_space(12.0);
                            ui.label(RichText::new(text.as_str()).small().color(if *error { theme::ERROR } else { theme::MUTED_TEXT }));
                        });
                    }
                }
            }
            if agent.streaming {
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.spinner();
                    let elapsed = agent.started_at.map(|t| t.elapsed().as_secs()).unwrap_or(0);
                    ui.label(RichText::new(format!("работает… {elapsed} с")).small().color(theme::MUTED_TEXT));
                });
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
            }
            if let Some(reason) = &agent.exited {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.label(RichText::new(reason.as_str()).color(theme::ERROR));
                });
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    if ui.button("Перезапустить").clicked() {
                        actions.push(Action::Restart);
                    }
                });
            }
            ui.add_space(12.0);
        },
    );

    // ---- input
    let mut input_ui =
        ui.new_child(egui::UiBuilder::new().max_rect(input_rect.shrink2(vec2(12.0, 6.0))).layout(egui::Layout::top_down(egui::Align::Min)));
    let ui = &mut input_ui;
    for (key, lines) in &agent.widgets {
        if key == "pi-modes" {
            continue;
        }
        let text = lines.join("\n");
        if !text.trim().is_empty() {
            ui.label(RichText::new(text).small().monospace().color(theme::MUTED_TEXT));
        }
    }
    // Dialogs from pi extensions.
    let mut answer = None;
    if let Some((index, request)) = agent.requests.iter_mut().enumerate().next() {
        egui::Frame::new()
            .fill(theme::SURFACE)
            .stroke(Stroke::new(1.0, theme::ACCENT_DIM))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(&request.title).strong());
                if !request.message.is_empty() {
                    ui.label(&request.message);
                }
                ui.horizontal_wrapped(|ui| match request.method.as_str() {
                    "confirm" => {
                        if ui.button("Да").clicked() {
                            answer = Some((index, json!({ "confirmed": true })));
                        }
                        if ui.button("Нет").clicked() {
                            answer = Some((index, json!({ "confirmed": false })));
                        }
                    }
                    "select" => {
                        for option in &request.options {
                            if ui.button(option).clicked() {
                                answer = Some((index, json!({ "value": option })));
                            }
                        }
                        if ui.button("Отмена").clicked() {
                            answer = Some((index, json!({ "cancelled": true })));
                        }
                    }
                    _ => {
                        let editor = if request.method == "editor" {
                            egui::TextEdit::multiline(&mut request.value).desired_rows(4)
                        } else {
                            egui::TextEdit::singleline(&mut request.value)
                        };
                        ui.add(editor.desired_width(ui.available_width() - 160.0));
                        if ui.button("OK").clicked() {
                            answer = Some((index, json!({ "value": request.value })));
                        }
                        if ui.button("Отмена").clicked() {
                            answer = Some((index, json!({ "cancelled": true })));
                        }
                    }
                });
            });
        ui.add_space(4.0);
    }
    if let Some((index, response)) = answer {
        agent.answer(index, response);
    }

    // Slash command suggestions.
    let mut insert_command = None;
    if agent.input.starts_with('/') && !agent.input.contains(' ') && !agent.commands.is_empty() {
        let filter = agent.input[1..].to_lowercase();
        let matches: Vec<&(String, String)> = agent.commands.iter().filter(|(n, _)| n.to_lowercase().contains(&filter)).take(6).collect();
        for (name, description) in matches {
            let response = ui
                .add(egui::Button::new(RichText::new(format!("/{name}  {}", util::truncate_middle(description, 70))).small()).frame(false));
            if response.clicked() {
                insert_command = Some(format!("/{name} "));
            }
        }
    }
    if let Some(command) = insert_command {
        agent.input = command;
        agent.focus_input = true;
    }

    let running = agent.is_running();
    let hint = if agent.streaming {
        "Уточнение по ходу работы (Enter), Alt+Enter — после завершения"
    } else {
        "Задача для агента (Enter — отправить, Shift+Enter — новая строка, / — команды)"
    };
    let editor = ui.add_enabled(
        running,
        egui::TextEdit::multiline(&mut agent.input)
            .hint_text(hint)
            .desired_rows(3)
            .desired_width(f32::INFINITY)
            .id(egui::Id::new(("agent_input", agent.id))),
    );
    if agent.focus_input {
        editor.request_focus();
        agent.focus_input = false;
    }
    if editor.has_focus() {
        let (enter, shift, alt) = ui.input(|i| (i.key_pressed(Key::Enter), i.modifiers.shift, i.modifiers.alt));
        if enter && !shift {
            // The TextEdit already inserted a newline; drop it.
            let text = agent.input.trim_end_matches('\n').to_string();
            agent.input.clear();
            if alt && agent.streaming {
                actions.push(Action::FollowUp(text));
            } else {
                actions.push(Action::Send(text));
            }
        }
        if ui.input(|i| i.key_pressed(Key::Escape)) && agent.streaming {
            actions.push(Action::Abort);
        }
    }
    ui.horizontal(|ui| {
        if agent.streaming {
            if ui.button(RichText::new("■ Стоп").color(theme::ERROR)).on_hover_text("Esc").clicked() {
                actions.push(Action::Abort);
            }
        } else if ui.add_enabled(running && !agent.input.trim().is_empty(), egui::Button::new("Отправить ⏎")).clicked() {
            let text = std::mem::take(&mut agent.input);
            actions.push(Action::Send(text));
        }
        if ui.small_button("+ выделение").on_hover_text("Вставить выделенный в редакторе код (Ctrl+Alt+A)").clicked()
        {
            actions.push(Action::Send("\u{0}selection".into()));
        }
        ui.add(egui::Label::new(RichText::new(&agent.cwd).small().color(theme::FAINT_TEXT)).truncate());
    });

    // ---- apply
    for action in actions {
        match action {
            Action::Send(text) if text == "\u{0}selection" => insert_selection(app),
            other => apply(app, loc, other),
        }
    }
}

fn apply(app: &mut App, loc: Loc, action: Action) {
    match action {
        Action::Restart => return restart(app, loc),
        Action::OpenFile(path) | Action::OpenLink(path) => return open_link(app, &path),
        Action::RunInTerminal(command) => {
            let task = crate::tasks::Task {
                label: command.chars().take(30).collect(),
                command,
                cwd: None,
                group: crate::tasks::Group::Other,
                default: false,
                source: "pi".into(),
            };
            app.new_terminal(Some(&task));
            return;
        }
        Action::ToggleDock => return toggle_dock(app, loc),
        Action::Hide => {
            app.agent_dock_open = false;
            return;
        }
        _ => {}
    }
    let Some(agent) = at(app, loc) else { return };
    match action {
        Action::Send(text) => agent.prompt(text),
        Action::FollowUp(text) => agent.send(json!({ "type": "follow_up", "message": text })),
        Action::Abort => agent.abort(),
        Action::NewSession => agent.send(json!({ "type": "new_session" })),
        Action::SwitchSession(path) => agent.send(json!({ "type": "switch_session", "sessionPath": path })),
        Action::RenameSession(name) => agent.send(json!({ "type": "set_session_name", "name": name })),
        Action::Compact => agent.send(json!({ "type": "compact" })),
        Action::SetModel(provider, model) => agent.send(json!({ "type": "set_model", "provider": provider, "modelId": model })),
        Action::SetThinking(level) => {
            agent.send(json!({ "type": "set_thinking_level", "level": level }));
            agent.thinking = level;
        }
        _ => {}
    }
}

/// Opens a link from the transcript: URLs in the browser, `path[:line[-line]]`
/// in the editor (paths relative to the project, or a unique file name).
pub fn open_link(app: &mut App, target: &str) {
    if target.starts_with("http://") || target.starts_with("https://") {
        app.ctx.open_url(eframe::egui::OpenUrl::new_tab(target));
        return;
    }
    let target = target.trim_start_matches("file://");
    let (path, start, end) = split_location(target);
    let Some(workspace) = &app.workspace else { return };
    let root = workspace.root.clone();
    let relative = path.trim_start_matches("./");
    let full = if path.starts_with('/') {
        path.clone()
    } else if let Some(rest) = path.strip_prefix("~/") {
        crate::host::join(&workspace.host.home(), rest)
    } else {
        // Agents often name files without their folder; find them in the index.
        let found = workspace.file_index.as_ref().and_then(|files| {
            files.iter().find(|f| *f == relative).or_else(|| {
                let suffix = format!("/{relative}");
                files.iter().find(|f| f.ends_with(&suffix))
            })
        });
        match found {
            Some(file) => crate::host::join(&root, file),
            None => {
                if workspace.file_index.is_none() {
                    app.load_file_index();
                }
                crate::host::join(&root, relative)
            }
        }
    };
    let Some(start) = start else {
        app.open_file(full, None);
        return;
    };
    let end = end.unwrap_or(start);
    app.with_doc(
        full,
        Box::new(move |app, result| match result {
            Ok(index) => {
                app.activate_tab(index);
                if let Some(super::Tab::Doc(doc)) = app.tabs.get_mut(index) {
                    let anchor = doc.buffer.char_at(start.saturating_sub(1), 0);
                    let head = doc.buffer.char_at(end.saturating_sub(1), usize::MAX);
                    doc.buffer.set_selections(vec![crate::text::Selection { anchor, head }]);
                    doc.view.reveal = true;
                    doc.view.reveal_center = true;
                    doc.view.focus_request = true;
                }
            }
            Err(error) => app.notify(format!("Не удалось открыть: {error}"), true),
        }),
    );
}

/// `file.rs:10-20` -> ("file.rs", Some(10), Some(20)); `file.rs:3:7` -> line 3.
fn split_location(target: &str) -> (String, Option<usize>, Option<usize>) {
    let mut parts = target.splitn(3, ':');
    let path = parts.next().unwrap_or("").to_string();
    let lines = parts.next().unwrap_or("");
    let (start, end) = match lines.split_once('-') {
        Some((a, b)) => (a.parse().ok(), b.parse().ok()),
        None => (lines.parse().ok(), None),
    };
    (path, start, end)
}

/// Moves the agent between its tab and the dock beside the code.
fn toggle_dock(app: &mut App, loc: Loc) {
    match loc {
        Loc::Dock => {
            if let Some(agent) = app.agent_dock.take() {
                app.tabs.push(super::Tab::Agent(agent));
                app.active_tab = Some(app.tabs.len() - 1);
            }
        }
        Loc::Tab(index) => {
            if index < app.tabs.len() && matches!(app.tabs[index], super::Tab::Agent(_)) {
                if let super::Tab::Agent(agent) = app.tabs.remove(index) {
                    app.agent_dock = Some(agent);
                    app.agent_dock_open = true;
                }
                app.active_tab = if app.tabs.is_empty() { None } else { Some(index.min(app.tabs.len() - 1)) };
            }
        }
    }
    // Moving out of a tab means the user wants it beside the code.
    app.config.agent_docked = matches!(loc, Loc::Tab(_));
    app.config.save();
}

/// Puts the editor selection (or current line) into the agent input.
pub fn insert_selection(app: &mut App) {
    let Some(snippet) = snippet(app, app.last_doc_id) else { return };
    open(app);
    if let Some(agent) = agent_mut(app) {
        if !agent.input.is_empty() && !agent.input.ends_with('\n') {
            agent.input.push('\n');
        }
        agent.input.push_str(&snippet);
        agent.focus_input = true;
    }
}

/// Quoted selection (or current line) of a document as Markdown.
fn snippet(app: &App, doc_id: Option<u64>) -> Option<String> {
    let root = app.workspace.as_ref().map(|w| w.root.clone()).unwrap_or_default();
    app.tabs.iter().find_map(|t| match t {
        super::Tab::Doc(doc) if Some(doc.id) == doc_id => {
            let primary = doc.buffer.primary();
            let (start_line, _) = doc.buffer.line_col(primary.start());
            let (end_line, _) = doc.buffer.line_col(primary.end());
            let text = if primary.is_empty() { doc.buffer.line_text(start_line) } else { doc.buffer.slice(primary.range()) };
            let relative = doc.path.strip_prefix(&format!("{root}/")).unwrap_or(&doc.path).to_string();
            Some(format!("`{relative}:{}-{}`\n```{}\n{}\n```\n", start_line + 1, end_line + 1, doc.lang.lsp_id(), text.trim_end()))
        }
        _ => None,
    })
}

/// "Ask π" from the editor: quote the selection and either put it into the
/// input or send it right away with an instruction.
pub fn ask(app: &mut App, doc_id: u64, instruction: Option<&str>) {
    let Some(snippet) = snippet(app, Some(doc_id)) else { return };
    open(app);
    let Some(agent) = agent_mut(app) else { return };
    match instruction {
        Some(instruction) => agent.prompt(format!("{instruction}\n\n{snippet}")),
        None => {
            if !agent.input.is_empty() && !agent.input.ends_with('\n') {
                agent.input.push('\n');
            }
            agent.input.push_str(&snippet);
            agent.focus_input = true;
        }
    }
}

/// Mentions a file in the agent input (explorer / tab "Send to π").
pub fn attach_file(app: &mut App, path: &str) {
    let root = app.workspace.as_ref().map(|w| w.root.clone()).unwrap_or_default();
    let relative = path.strip_prefix(&format!("{root}/")).unwrap_or(path).to_string();
    open(app);
    if let Some(agent) = agent_mut(app) {
        if !agent.input.is_empty() && !agent.input.ends_with(' ') && !agent.input.ends_with('\n') {
            agent.input.push(' ');
        }
        agent.input.push_str(&format!("`{relative}` "));
        agent.focus_input = true;
    }
}

/// Opens (or focuses) the agent for the current project, beside the code by
/// default.
pub fn open(app: &mut App) {
    if let Some(loc) = find(app) {
        match loc {
            Loc::Dock => app.agent_dock_open = true,
            Loc::Tab(index) => app.activate_tab(index),
        }
        if let Some(agent) = at(app, loc) {
            agent.focus_input = true;
        }
        return;
    }
    let Some(workspace) = &app.workspace else {
        app.notify("Сначала откройте проект", true);
        return;
    };
    let host = workspace.host.clone();
    let root = workspace.root.clone();
    let id = app.next_id();
    let options = LaunchOptions::from_config(&app.config, false);
    let tab = Box::new(AgentTab::spawn(id, &host, &root, app.control_socket.as_deref(), &options, app.bus.clone()));
    if app.config.agent_docked {
        app.agent_dock = Some(tab);
        app.agent_dock_open = true;
    } else {
        app.tabs.push(super::Tab::Agent(tab));
        app.active_tab = Some(app.tabs.len() - 1);
    }
}

/// Restarts pi, continuing the previous session of this folder.
fn restart(app: &mut App, loc: Loc) {
    let Some(workspace) = &app.workspace else { return };
    let host = workspace.host.clone();
    let root = workspace.root.clone();
    let id = app.next_id();
    let options = LaunchOptions::from_config(&app.config, true);
    let tab = Box::new(AgentTab::spawn(id, &host, &root, app.control_socket.as_deref(), &options, app.bus.clone()));
    match loc {
        Loc::Dock => app.agent_dock = Some(tab),
        Loc::Tab(index) => {
            if index < app.tabs.len() {
                app.tabs[index] = super::Tab::Agent(tab);
            }
        }
    }
}

/// Routes a record from pi to its tab.
pub fn on_event(app: &mut App, id: u64, event: Value) {
    let agent = match app.agent_dock.as_deref_mut() {
        Some(agent) if agent.id == id => Some(agent),
        _ => app.tabs.iter_mut().find_map(|t| match t {
            super::Tab::Agent(a) if a.id == id => Some(a.as_mut()),
            _ => None,
        }),
    };
    let Some(agent) = agent else { return };
    let changed_files = agent.handle(event);
    if changed_files {
        app.reload_unmodified_docs();
        app.git_refresh();
        if let Some(workspace) = &mut app.workspace {
            workspace.file_index = None;
        }
    }
}
