//! Application state and the main loop. Workers report back through `Msg`.

mod about;
mod agent;
mod commands;
mod control;
mod diffview;
mod document;
pub mod editor;
mod explorer;
mod icons;
mod lsp_manager;
mod markdown;
mod media;
mod palette;
mod panel;
mod pulse;
mod run;
mod scm;
mod search;
mod status;
mod teleport;
mod util;
mod welcome;

use std::{
    collections::BTreeSet,
    sync::Arc,
    time::{Duration, Instant},
};

use crossbeam_channel::{Receiver, Sender, unbounded};
use eframe::egui::{self, Key, RichText};
use lumen_shared::fsops::{DirEntry, SearchMatch};

use crate::{
    config::Config,
    git,
    host::{self, Host, Remote},
    lsp::client::ServerEvent,
    syntax,
    tasks::Task,
    terminal::Terminal,
    theme,
};

pub use commands::Command;

/// Version line for `lumen-edit --version`.
pub fn about_line() -> String {
    t!("{n1} — {n2}\nAuthor: {n3}\n{n4}", n1 = about::version_line(), n2 = about::tagline(), n3 = about::author(), n4 = about::REPOSITORY)
}
use diffview::{DiffTab, LogTab};
use document::Document;

pub enum Msg {
    Connected {
        target: String,
        result: Result<Arc<Remote>, String>,
    },
    DirLoaded {
        path: String,
        result: Result<Vec<DirEntry>, String>,
    },
    FileLoaded {
        path: String,
        result: Result<Vec<u8>, String>,
        goto: Option<(usize, usize)>,
        mtime: u64,
    },
    FileSaved {
        doc_id: u64,
        revision: u64,
        result: Result<u64, String>,
    },
    FileReloaded {
        path: String,
        data: Vec<u8>,
        mtime: u64,
    },
    FsChanged(Vec<String>),
    Highlight(syntax::Done),
    Lsp(ServerEvent),
    LspStarted {
        name: String,
        result: Result<Arc<crate::lsp::client::LspClient>, Option<String>>,
    },
    LspResult {
        doc_id: u64,
        version: u64,
        request: lsp_manager::Request,
        client: Arc<crate::lsp::client::LspClient>,
        result: Result<serde_json::Value, String>,
    },
    Shellcheck {
        doc_id: u64,
        version: u64,
        result: Result<Vec<(usize, usize, usize, usize, crate::lsp::Severity, String)>, String>,
    },
    GitRepo(Option<String>),
    GitStatus(Result<git::Status, String>),
    GitRefs {
        branches: Vec<git::Branch>,
        stashes: Vec<git::StashEntry>,
        remotes: Vec<git::Remote>,
    },
    GitOpDone {
        label: String,
        result: Result<String, String>,
    },
    GitDiff {
        key: String,
        result: Result<String, String>,
    },
    GitLog(Result<Vec<git::LogEntry>, String>),
    GitHead {
        path: String,
        content: Option<String>,
    },
    GitBlame {
        doc_id: u64,
        version: u64,
        result: Result<Vec<git::BlameLine>, String>,
    },
    Tasks(Vec<Task>, Option<String>),
    FileIndex(Result<Vec<String>, String>),
    SearchResults {
        generation: u64,
        result: Result<(Vec<SearchMatch>, bool), String>,
    },
    WorkspaceEditDone(Result<usize, String>),
    FsOpDone {
        reload: Vec<String>,
        open: Option<String>,
        renamed: Option<(String, String)>,
        error: Option<String>,
    },
    RemoteDir {
        path: String,
        result: Result<Vec<DirEntry>, String>,
    },
    Notice(String, bool),
    /// Request from the CLI / MCP control socket.
    Control(crate::control::ControlRequest),
    /// A record from a pi agent process.
    Agent {
        id: u64,
        event: serde_json::Value,
    },
    /// Continuation from a worker thread.
    Run(Box<dyn FnOnce(&mut App) + Send>),
}

/// Wraps the channel so workers can wake the UI.
#[derive(Clone)]
pub struct Bus {
    tx: Sender<Msg>,
    ctx: egui::Context,
}

impl Bus {
    pub fn send(&self, msg: Msg) {
        let _ = self.tx.send(msg);
        self.ctx.request_repaint();
    }

    /// Runs `job` on a worker thread and sends its message.
    pub fn spawn(&self, job: impl FnOnce() -> Msg + Send + 'static) {
        let bus = self.clone();
        std::thread::spawn(move || bus.send(job()));
    }
}

pub struct Workspace {
    pub host: Host,
    pub root: String,
    pub explorer: explorer::ExplorerState,
    pub git: scm::GitState,
    pub tasks: Vec<Task>,
    pub tasks_error: Option<String>,
    pub file_index: Option<Vec<String>>,
    pub file_index_loading: bool,
    pub search: search::SearchState,
    _watcher: Option<lumen_shared::watch::TreeWatcher>,
}

impl Workspace {
    pub fn name(&self) -> String {
        host::file_name(&self.root).to_string()
    }

    pub fn relative<'a>(&self, path: &'a str) -> &'a str {
        path.strip_prefix(&self.root).map(|p| p.trim_start_matches('/')).filter(|p| !p.is_empty()).unwrap_or(path)
    }

    pub fn session_key(&self) -> String {
        format!("{}|{}", self.host.key(), self.root)
    }
}

pub enum Tab {
    Doc(Box<Document>),
    Diff(DiffTab),
    Log(LogTab),
    Agent(Box<agent::AgentTab>),
    Teleport(Box<teleport::TeleportTab>),
    Image(Box<media::ImageTab>),
}

impl Tab {
    fn title(&self) -> String {
        match self {
            Tab::Doc(doc) => doc.name().to_string(),
            Tab::Diff(diff) => diff.title.clone(),
            Tab::Log(_) => t!("Git history").into(),
            Tab::Agent(agent) => agent.title.clone(),
            Tab::Teleport(_) => t!("⇄ Teleport").into(),
            Tab::Image(image) => host::file_name(&image.path).to_string(),
        }
    }
    /// The file behind the tab, if any.
    pub fn path(&self) -> Option<&str> {
        match self {
            Tab::Doc(doc) => Some(&doc.path),
            Tab::Image(image) => Some(&image.path),
            _ => None,
        }
    }
    fn dirty(&self) -> bool {
        matches!(self, Tab::Doc(doc) if doc.is_dirty())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SideView {
    Explorer,
    Search,
    Git,
    Run,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PanelTab {
    Terminal,
    Problems,
    Output,
    References,
    Pulse,
}

pub enum Dialog {
    CloseTab { index: usize },
    CloseWorkspace { then: Box<dyn FnOnce(&mut App) + Send> },
    Quit,
    Confirm { title: String, message: String, action: Box<dyn FnOnce(&mut App) + Send> },
    Input { title: String, value: String, action: Box<dyn FnOnce(&mut App, String) + Send>, focus: bool },
}

pub struct Toast {
    text: String,
    error: bool,
    at: Instant,
}

pub struct App {
    ctx: egui::Context,
    pub config: Config,
    bus: Bus,
    rx: Receiver<Msg>,
    highlight_tx: Sender<syntax::Job>,
    pub workspace: Option<Workspace>,
    pub tabs: Vec<Tab>,
    pub active_tab: Option<usize>,
    next_id: u64,
    pub side_view: SideView,
    pub sidebar_open: bool,
    pub panel_open: bool,
    pub panel_tab: PanelTab,
    pub terminals: Vec<Terminal>,
    pub active_terminal: usize,
    pub palette: Option<palette::Palette>,
    pub lsp: lsp_manager::LspManager,
    pub output: Vec<(String, bool)>,
    toasts: Vec<Toast>,
    pub dialog: Option<Dialog>,
    pub connect: Option<welcome::ConnectState>,
    pub references: Vec<(String, usize, usize, String)>,
    loading_files: BTreeSet<String>,
    fs_pending: BTreeSet<String>,
    fs_pending_since: Option<Instant>,
    allow_close: bool,
    last_title: String,
    last_session_save: Instant,
    pub last_task: Option<Task>,
    terminal_focus: bool,
    /// Actions waiting for a file to finish loading (control requests).
    doc_waiters: Vec<(String, control::DocAction)>,
    pub settings_open: bool,
    pub about_open: bool,
    pub pulse: pulse::Pulse,
    /// SSH hosts connected in this session (for Teleport), besides the project host.
    pub known_hosts: Vec<Host>,
    /// The agent docked beside the code (when not in a tab).
    pub agent_dock: Option<Box<agent::AgentTab>>,
    pub agent_dock_open: bool,
    /// Images referenced from Markdown previews, by absolute path.
    pub image_cache: std::collections::HashMap<String, media::ImageEntry>,
    /// This editor's control socket, passed to agents.
    pub control_socket: Option<std::path::PathBuf>,
    /// Last focused document (for "send selection" from the agent tab).
    pub last_doc_id: Option<u64>,
    debug_typing: Option<String>,
    debug_goto: Option<(usize, usize)>,
    debug_clicks: Vec<(egui::Pos2, egui::PointerButton)>,
    debug_key: Option<egui::Key>,
    /// SSH master connections (kept alive while the editor runs).
    pub ssh_masters: Vec<Terminal>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext) -> Self {
        let ctx = cc.egui_ctx.clone();
        let config = Config::load();
        install_fonts(&ctx, &config);
        egui_extras::install_image_loaders(&ctx);
        theme::apply(&ctx, config.ui_font_size);
        let (tx, rx) = unbounded();
        let bus = Bus { tx, ctx: ctx.clone() };
        let highlight_bus = bus.clone();
        let highlight_tx = syntax::spawn_worker(move |done| highlight_bus.send(Msg::Highlight(done)));
        let lsp = lsp_manager::LspManager::new(config.servers(), bus.clone());

        let mut app = Self {
            ctx,
            config,
            bus,
            rx,
            highlight_tx,
            workspace: None,
            tabs: Vec::new(),
            active_tab: None,
            next_id: 1,
            side_view: SideView::Explorer,
            sidebar_open: true,
            panel_open: false,
            panel_tab: PanelTab::Terminal,
            terminals: Vec::new(),
            active_terminal: 0,
            palette: None,
            lsp,
            output: Vec::new(),
            toasts: Vec::new(),
            dialog: None,
            connect: None,
            references: Vec::new(),
            loading_files: BTreeSet::new(),
            fs_pending: BTreeSet::new(),
            fs_pending_since: None,
            allow_close: false,
            last_title: String::new(),
            last_session_save: Instant::now(),
            last_task: None,
            terminal_focus: false,
            doc_waiters: Vec::new(),
            settings_open: false,
            about_open: false,
            pulse: pulse::Pulse::default(),
            known_hosts: Vec::new(),
            agent_dock: None,
            agent_dock_open: false,
            image_cache: std::collections::HashMap::new(),
            control_socket: None,
            last_doc_id: None,
            debug_typing: None,
            debug_goto: None,
            debug_clicks: Vec::new(),
            debug_key: None,
            ssh_masters: Vec::new(),
        };

        let control_bus = app.bus.clone();
        app.control_socket = crate::control::serve(move |request| control_bus.send(Msg::Control(request)));
        if app.control_socket.is_none() {
            eprintln!("{}", t!("lumen-edit: could not open the control socket"));
        }

        // `lumen-edit [path]` or `lumen-edit ssh://host/path`.
        if let Some(arg) = crate::gui_args().first().cloned() {
            if let Some(rest) = arg.strip_prefix("ssh://") {
                let (target, path) = match rest.split_once('/') {
                    Some((target, path)) => (target.to_string(), Some(format!("/{path}"))),
                    None => (rest.to_string(), None),
                };
                app.start_connect(target, path);
            } else {
                let path = std::fs::canonicalize(&arg).unwrap_or_else(|_| arg.clone().into());
                if path.is_dir() {
                    app.open_workspace(Host::Local, crate::platform::host_path(&path));
                    if let Some(file) = crate::gui_args().get(1)
                        && let Ok(file) = std::fs::canonicalize(file)
                    {
                        app.open_file(crate::platform::host_path(&file), None);
                    }
                } else if path.is_file() {
                    let parent = path.parent().map(crate::platform::host_path).unwrap_or_else(|| "/".into());
                    app.open_workspace(Host::Local, parent);
                    app.open_file(crate::platform::host_path(&path), None);
                }
            }
        }
        // Debug aid for screenshots: open a view at start-up.
        if let Ok(view) = std::env::var("LUMEN_DEBUG_VIEW") {
            for part in view.split(',') {
                match part.split_once('=').unwrap_or((part, "")) {
                    ("git", _) => app.run_command(Command::ShowGit),
                    ("run", _) => app.run_command(Command::ShowRun),
                    ("terminal", _) => app.run_command(Command::ToggleTerminal),
                    ("palette", _) => app.run_command(Command::QuickOpen),
                    ("commands", _) => app.run_command(Command::CommandPalette),
                    ("log", _) => app.open_log(true),
                    ("settings", _) => app.settings_open = true,
                    ("teleport", _) => app.run_command(Command::Teleport),
                    ("agent", _) => app.run_command(Command::Agent),
                    ("ssh", _) => app.run_command(Command::ConnectSsh),
                    ("search", query) => {
                        app.run_command(Command::ShowSearch);
                        if let Some(workspace) = &mut app.workspace {
                            workspace.search.query = query.to_string();
                            workspace.search.pending_run = true;
                        }
                    }
                    ("type", text) => app.debug_typing = Some(text.to_string()),
                    ("key", name) => app.debug_key = egui::Key::from_name(name),
                    (kind @ ("click" | "rclick"), at) => {
                        let mut parts = at.split(':').map(|n| n.parse::<f32>().unwrap_or(0.0));
                        let button = if kind == "rclick" { egui::PointerButton::Secondary } else { egui::PointerButton::Primary };
                        app.debug_clicks.push((egui::pos2(parts.next().unwrap_or(0.0), parts.next().unwrap_or(0.0)), button));
                    }
                    ("goto", at) => {
                        let mut parts = at.split(':').map(|n| n.parse::<usize>().unwrap_or(1));
                        app.debug_goto = Some((parts.next().unwrap_or(1) - 1, parts.next().unwrap_or(1) - 1));
                    }
                    _ => {}
                }
            }
        }
        app
    }

    pub fn next_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    pub fn notify(&mut self, text: impl Into<String>, error: bool) {
        let text = text.into();
        self.output.push((text.clone(), error));
        if self.output.len() > 500 {
            self.output.remove(0);
        }
        self.toasts.push(Toast { text, error, at: Instant::now() });
    }

    pub fn host(&self) -> Option<Host> {
        self.workspace.as_ref().map(|w| w.host.clone())
    }

    // ------------------------------------------------------------ workspace

    pub fn open_workspace(&mut self, host: Host, root: String) {
        if self.tabs.iter().any(Tab::dirty) {
            self.dialog = Some(Dialog::CloseWorkspace { then: Box::new(move |app: &mut App| app.open_workspace_now(host, root)) });
            return;
        }
        self.open_workspace_now(host, root);
    }

    fn open_workspace_now(&mut self, host: Host, root: String) {
        // The agent is bound to its project (cwd, local or remote tools), so a
        // new project gets a new agent.
        let reopen_agent = self.agent_dock.is_some() && self.agent_dock_open;
        self.close_workspace_now();
        let root = if root.len() > 1 { root.trim_end_matches('/').to_string() } else { root };
        self.config.add_recent(&host.key(), &root);
        self.config.save();

        let watcher = match &host {
            Host::Local => local_watcher(&root, self.bus.clone()),
            Host::Remote(remote) => {
                let (tx, rx) = unbounded::<Vec<String>>();
                let bus = self.bus.clone();
                std::thread::spawn(move || {
                    while let Ok(paths) = rx.recv() {
                        bus.send(Msg::FsChanged(paths));
                    }
                });
                let remote = remote.clone();
                let watch_root = root.clone();
                std::thread::spawn(move || {
                    remote.watch(&watch_root, tx);
                });
                None
            }
        };

        self.workspace = Some(Workspace {
            explorer: explorer::ExplorerState::new(&root),
            git: scm::GitState::default(),
            tasks: Vec::new(),
            tasks_error: None,
            file_index: None,
            file_index_loading: false,
            search: search::SearchState::default(),
            host,
            root: root.clone(),
            _watcher: watcher,
        });
        self.load_dir(root.clone());
        self.git_detect();
        self.refresh_tasks();
        self.lsp.reset(self.config.servers());

        let session = self.config.sessions.get(&self.workspace.as_ref().unwrap().session_key()).cloned().unwrap_or_default();
        for path in session {
            self.open_file(path, None);
        }
        if reopen_agent {
            agent::open(self);
        }
    }

    pub fn close_workspace(&mut self) {
        if self.tabs.iter().any(Tab::dirty) {
            self.dialog = Some(Dialog::CloseWorkspace { then: Box::new(|app: &mut App| app.close_workspace_now()) });
        } else {
            self.close_workspace_now();
        }
    }

    fn close_workspace_now(&mut self) {
        self.save_session();
        self.lsp.shutdown_all();
        self.tabs.clear();
        self.active_tab = None;
        self.agent_dock = None;
        self.agent_dock_open = false;
        self.terminals.clear();
        self.references.clear();
        self.loading_files.clear();
        if let Some(workspace) = self.workspace.take()
            && let Host::Remote(remote) = workspace.host
        {
            // The connection stays up for other projects on the same host only
            // while something still holds it.
            if Arc::strong_count(&remote) <= 2 {
                std::thread::spawn(move || remote.disconnect());
            }
        }
    }

    fn save_session(&mut self) {
        let Some(workspace) = &self.workspace else { return };
        let files: Vec<String> = self
            .tabs
            .iter()
            .filter_map(|t| match t {
                Tab::Doc(doc) => Some(doc.path.clone()),
                _ => None,
            })
            .collect();
        self.config.sessions.insert(workspace.session_key(), files);
        self.config.save();
    }

    pub fn start_connect(&mut self, target: String, open_path: Option<String>) {
        self.connect = Some(welcome::ConnectState::connecting(target, open_path, self.config.use_agent, &self.bus, &self.ctx));
    }

    // -------------------------------------------------------------- files

    pub fn load_dir(&mut self, path: String) {
        let Some(workspace) = &mut self.workspace else { return };
        workspace.explorer.mark_loading(&path);
        let host = workspace.host.clone();
        self.bus.spawn(move || {
            let result = host.read_dir(&path);
            Msg::DirLoaded { path, result }
        });
    }

    pub fn open_file(&mut self, path: String, goto: Option<(usize, usize)>) {
        if let Some(index) = self.tabs.iter().position(|t| matches!(t, Tab::Image(i) if i.path == path)) {
            self.activate_tab(index);
            return;
        }
        if let Some(index) = self.tabs.iter().position(|t| matches!(t, Tab::Doc(d) if d.path == path)) {
            self.activate_tab(index);
            if let (Some(goto), Tab::Doc(doc)) = (goto, &mut self.tabs[index]) {
                goto_position(doc, goto);
            }
            return;
        }
        let Some(host) = self.host() else { return };
        if !self.loading_files.insert(path.clone()) {
            return;
        }
        self.bus.spawn(move || {
            let mtime = host.stat(&path).map(|s| s.mtime).unwrap_or(0);
            let result = host.read_file(&path);
            Msg::FileLoaded { path, result, goto, mtime }
        });
    }

    fn on_file_loaded(&mut self, path: String, result: Result<Vec<u8>, String>, goto: Option<(usize, usize)>, mtime: u64) {
        self.loading_files.remove(&path);
        let data = match result {
            Ok(data) => data,
            Err(error) => {
                self.notify(t!("Could not open {n1}: {error}", n1 = host::file_name(&path), error = error), true);
                self.run_doc_waiters(&path, Err(error));
                return;
            }
        };
        if media::kind(&path) == Some(media::Media::Raster) {
            media::open_image_tab(self, path.clone(), data);
            self.run_doc_waiters(&path, Err(t!("this is an image").into()));
            return;
        }
        if data.iter().take(8000).any(|b| *b == 0) {
            self.notify(t!("{n1} is a binary file, not opening it", n1 = host::file_name(&path)), true);
            return;
        }
        let text = String::from_utf8(data).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());
        let id = self.next_id();
        let mut doc = Document::new(id, path.clone(), &text, self.config.tab_size, self.config.insert_spaces);
        doc.disk_mtime = mtime;
        doc.view.focus_request = true;
        if let Some(goto) = goto {
            goto_position(&mut doc, goto);
        }
        let insert_at = self.active_tab.map(|i| i + 1).unwrap_or(self.tabs.len()).min(self.tabs.len());
        self.tabs.insert(insert_at, Tab::Doc(Box::new(doc)));
        self.active_tab = Some(insert_at);
        self.request_highlight(id);
        self.load_git_base(&path);
        if let Some(Tab::Doc(doc)) = self.tabs.get_mut(insert_at) {
            self.lsp.document_opened(doc, self.workspace.as_ref());
        }
        self.run_doc_waiters(&path, Ok(insert_at));
        self.run_shellcheck(id);
        self.request_blame(id);
        let missing = match &mut self.workspace {
            Some(workspace) => {
                let root = workspace.root.clone();
                workspace.explorer.reveal(&path, &root)
            }
            None => Vec::new(),
        };
        for dir in missing {
            self.load_dir(dir);
        }
    }

    pub fn doc_index(&self, doc_id: u64) -> Option<usize> {
        self.tabs.iter().position(|t| matches!(t, Tab::Doc(d) if d.id == doc_id))
    }

    pub fn doc_mut(&mut self, doc_id: u64) -> Option<&mut Document> {
        self.tabs.iter_mut().find_map(|t| match t {
            Tab::Doc(d) if d.id == doc_id => Some(d.as_mut()),
            _ => None,
        })
    }

    pub fn doc_by_path(&mut self, path: &str) -> Option<&mut Document> {
        self.tabs.iter_mut().find_map(|t| match t {
            Tab::Doc(d) if d.path == path => Some(d.as_mut()),
            _ => None,
        })
    }

    pub fn active_doc(&mut self) -> Option<&mut Document> {
        let index = self.active_tab?;
        match self.tabs.get_mut(index)? {
            Tab::Doc(doc) => Some(doc.as_mut()),
            _ => None,
        }
    }

    pub fn activate_tab(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.active_tab = Some(index);
            if let Tab::Doc(doc) = &mut self.tabs[index] {
                doc.view.focus_request = true;
            }
        }
    }

    pub fn request_close_tab(&mut self, index: usize) {
        if self.tabs.get(index).is_some_and(Tab::dirty) {
            self.dialog = Some(Dialog::CloseTab { index });
        } else {
            self.close_tab(index);
        }
    }

    pub fn close_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(index);
        if let Tab::Doc(doc) = &tab {
            self.lsp.document_closed(doc);
        }
        self.active_tab = match self.active_tab {
            _ if self.tabs.is_empty() => None,
            Some(active) if active > index => Some(active - 1),
            Some(active) if active == index => Some(index.min(self.tabs.len() - 1)),
            other => other,
        };
        if let Some(active) = self.active_tab
            && let Tab::Doc(doc) = &mut self.tabs[active]
        {
            doc.view.focus_request = true;
        }
    }

    pub fn save_tab(&mut self, index: usize) {
        let format_first = self.config.format_on_save;
        let Some(Tab::Doc(doc)) = self.tabs.get_mut(index) else { return };
        if format_first && !doc.view.rename.is_some() && self.lsp.can_format(doc) {
            let doc_id = doc.id;
            self.lsp.request(doc, lsp_manager::Request::Format { then_save: true }, None);
            let _ = doc_id;
            return;
        }
        self.save_doc_now(index);
    }

    pub fn save_doc_now(&mut self, index: usize) {
        let Some(host) = self.host() else { return };
        let Some(Tab::Doc(doc)) = self.tabs.get_mut(index) else { return };
        if doc.saving {
            return;
        }
        doc.saving = true;
        let data = doc.buffer.text().into_bytes();
        let path = doc.path.clone();
        let doc_id = doc.id;
        let revision = doc.buffer.revision();
        self.bus.spawn(move || {
            let result = host.write_file(&path, &data).and_then(|_| host.stat(&path).map(|s| s.mtime));
            Msg::FileSaved { doc_id, revision, result }
        });
    }

    pub fn save_all(&mut self) {
        for index in 0..self.tabs.len() {
            if self.tabs[index].dirty() {
                self.save_doc_now(index);
            }
        }
    }

    fn on_file_saved(&mut self, doc_id: u64, revision: u64, result: Result<u64, String>) {
        let Some(index) = self.doc_index(doc_id) else { return };
        let Tab::Doc(doc) = &mut self.tabs[index] else { return };
        doc.saving = false;
        match result {
            Ok(mtime) => {
                doc.buffer.mark_saved_at(revision);
                doc.disk_mtime = mtime;
                let path = doc.path.clone();
                self.lsp.document_saved(doc);
                self.run_shellcheck(doc_id);
                self.request_blame(doc_id);
                self.git_refresh();
                if path.ends_with(crate::tasks::TASKS_FILE)
                    || matches!(host::file_name(&path), "Cargo.toml" | "Makefile" | "package.json" | "go.mod")
                {
                    self.refresh_tasks();
                }
            }
            Err(error) => self.notify(t!("Could not save: {error}", error = error), true),
        }
    }

    // ----------------------------------------------------------- derived data

    fn request_highlight(&mut self, doc_id: u64) {
        let Some(doc) = self.doc_mut(doc_id) else { return };
        if doc.large || doc.highlight_requested == doc.buffer.version {
            return;
        }
        doc.highlight_requested = doc.buffer.version;
        let job =
            syntax::Job { doc_id, version: doc.buffer.version, lang: doc.lang, text: doc.buffer.rope.to_string(), path: doc.path.clone() };
        let _ = self.highlight_tx.send(job);
    }

    fn load_git_base(&mut self, path: &str) {
        let Some(workspace) = &self.workspace else { return };
        let Some(repo) = workspace.git.repo_root.clone() else { return };
        let Some(relative) = path.strip_prefix(&repo).map(|p| p.trim_start_matches('/').to_string()) else { return };
        let host = workspace.host.clone();
        let path = path.to_string();
        self.bus.spawn(move || {
            let content = git::head_content(&host, &repo, &relative);
            Msg::GitHead { path, content }
        });
    }

    fn request_blame(&mut self, doc_id: u64) {
        if !self.config.inline_blame {
            return;
        }
        let Some(workspace) = &self.workspace else { return };
        let Some(repo) = workspace.git.repo_root.clone() else { return };
        let host = workspace.host.clone();
        let Some(doc) = self.doc_mut(doc_id) else { return };
        let Some(relative) = doc.path.strip_prefix(&repo).map(|p| p.trim_start_matches('/').to_string()) else { return };
        if doc.large {
            return;
        }
        let text = doc.buffer.text();
        let version = doc.buffer.version;
        self.bus.spawn(move || {
            let result = git::blame(&host, &repo, &relative, &text);
            Msg::GitBlame { doc_id, version, result }
        });
    }

    fn run_shellcheck(&mut self, doc_id: u64) {
        let Some(host) = self.host() else { return };
        let root = self.workspace.as_ref().map(|w| w.root.clone()).unwrap_or_default();
        if !self.lsp.shellcheck_enabled() {
            return;
        }
        let Some(doc) = self.doc_mut(doc_id) else { return };
        if doc.lang != syntax::Lang::Bash || doc.shellcheck_version == doc.buffer.version {
            return;
        }
        doc.shellcheck_version = doc.buffer.version;
        let text = doc.buffer.rope.to_string();
        let version = doc.buffer.version;
        let cwd = host::parent(&doc.path);
        let _ = root;
        self.bus.spawn(move || {
            let result = if host.which("shellcheck") {
                crate::lsp::shellcheck::run(&host, &cwd, &text)
            } else {
                Err(t!("shellcheck not found").into())
            };
            Msg::Shellcheck { doc_id, version, result }
        });
    }

    pub fn refresh_tasks(&mut self) {
        let Some(workspace) = &self.workspace else { return };
        let host = workspace.host.clone();
        let root = workspace.root.clone();
        self.bus.spawn(move || {
            let (tasks, error) = crate::tasks::detect(&host, &root);
            Msg::Tasks(tasks, error)
        });
    }

    pub fn load_file_index(&mut self) {
        let Some(workspace) = &mut self.workspace else { return };
        if workspace.file_index_loading {
            return;
        }
        workspace.file_index_loading = true;
        let host = workspace.host.clone();
        let root = workspace.root.clone();
        self.bus.spawn(move || Msg::FileIndex(host.list_files(&root, 200_000)));
    }

    // ---------------------------------------------------------------- git

    pub fn git_detect(&mut self) {
        let Some(workspace) = &self.workspace else { return };
        let host = workspace.host.clone();
        let root = workspace.root.clone();
        self.bus.spawn(move || Msg::GitRepo(git::repo_root(&host, &root)));
    }

    pub fn git_refresh(&mut self) {
        let Some(workspace) = &mut self.workspace else { return };
        let Some(repo) = workspace.git.repo_root.clone() else { return };
        if workspace.git.loading {
            workspace.git.refresh_again = true;
            return;
        }
        workspace.git.loading = true;
        let host = workspace.host.clone();
        let refs_host = host.clone();
        let refs_repo = repo.clone();
        self.bus.spawn(move || Msg::GitStatus(git::status(&host, &repo)));
        self.bus.spawn(move || Msg::GitRefs {
            branches: git::branches(&refs_host, &refs_repo).unwrap_or_default(),
            stashes: git::stashes(&refs_host, &refs_repo).unwrap_or_default(),
            remotes: git::remotes(&refs_host, &refs_repo).unwrap_or_default(),
        });
    }

    /// Runs a git operation in the background and refreshes afterwards.
    pub fn git_op(&mut self, label: &str, op: impl FnOnce(&Host, &str) -> Result<String, String> + Send + 'static) {
        let Some(workspace) = &mut self.workspace else { return };
        let Some(repo) = workspace.git.repo_root.clone() else { return };
        workspace.git.busy = Some(label.to_string());
        let host = workspace.host.clone();
        let label = label.to_string();
        self.bus.spawn(move || {
            let result = op(&host, &repo);
            Msg::GitOpDone { label, result }
        });
    }

    pub fn open_diff(&mut self, path: String, mode: diffview::DiffMode) {
        let title = format!("{} ({})", host::file_name(&path), mode.label());
        let key = format!("{}:{path}", mode.label());
        if let Some(index) = self.tabs.iter().position(|t| matches!(t, Tab::Diff(d) if d.key == key)) {
            self.activate_tab(index);
        } else {
            let tab = DiffTab::new(title, key.clone(), Some(path.clone()), mode);
            self.tabs.push(Tab::Diff(tab));
            self.active_tab = Some(self.tabs.len() - 1);
        }
        self.reload_diff(&key);
    }

    pub fn reload_diff(&mut self, key: &str) {
        let Some(Tab::Diff(tab)) = self.tabs.iter_mut().find(|t| matches!(t, Tab::Diff(d) if d.key == key)) else { return };
        let Some(workspace) = &self.workspace else { return };
        let Some(repo) = workspace.git.repo_root.clone() else { return };
        let host = workspace.host.clone();
        tab.loading = true;
        let path = tab.path.clone().unwrap_or_default();
        let mode = tab.mode.clone();
        let key = key.to_string();
        self.bus.spawn(move || {
            let result = match mode {
                diffview::DiffMode::Unstaged => git::diff_file(&host, &repo, &path, git::DiffTarget::Unstaged),
                diffview::DiffMode::Staged => git::diff_file(&host, &repo, &path, git::DiffTarget::Staged),
                diffview::DiffMode::Untracked => git::diff_file(&host, &repo, &path, git::DiffTarget::Untracked),
                diffview::DiffMode::Commit(hash) => git::show_commit(&host, &repo, &hash),
            };
            Msg::GitDiff { key, result }
        });
    }

    pub fn open_commit(&mut self, hash: String, subject: String) {
        let key = format!("commit:{hash}");
        if let Some(index) = self.tabs.iter().position(|t| matches!(t, Tab::Diff(d) if d.key == key)) {
            self.activate_tab(index);
            return;
        }
        let short: String = hash.chars().take(8).collect();
        let tab =
            DiffTab::new(format!("{short} {}", util::truncate_middle(&subject, 30)), key.clone(), None, diffview::DiffMode::Commit(hash));
        self.tabs.push(Tab::Diff(tab));
        self.active_tab = Some(self.tabs.len() - 1);
        self.reload_diff(&key);
    }

    pub fn open_log(&mut self, all: bool) {
        if let Some(index) = self.tabs.iter().position(|t| matches!(t, Tab::Log(_))) {
            self.activate_tab(index);
        } else {
            self.tabs.push(Tab::Log(LogTab::new(all)));
            self.active_tab = Some(self.tabs.len() - 1);
        }
        self.reload_log();
    }

    pub fn reload_log(&mut self) {
        let Some(workspace) = &self.workspace else { return };
        let Some(repo) = workspace.git.repo_root.clone() else { return };
        let host = workspace.host.clone();
        let all = self.tabs.iter().find_map(|t| if let Tab::Log(l) = t { Some(l.all) } else { None }).unwrap_or(true);
        if let Some(Tab::Log(log)) = self.tabs.iter_mut().find(|t| matches!(t, Tab::Log(_))) {
            log.loading = true;
        }
        self.bus.spawn(move || Msg::GitLog(git::log(&host, &repo, all, 1500, None)));
    }

    // ------------------------------------------------------------ terminal

    pub fn new_terminal(&mut self, task: Option<&Task>) {
        let Some(host) = self.host() else {
            // Without a project, open a local shell in the home directory.
            let home = Host::Local.home();
            self.spawn_terminal(Host::Local, home, None, "terminal".into());
            return;
        };
        let root = self.workspace.as_ref().map(|w| w.root.clone()).unwrap_or_default();
        match task {
            Some(task) => {
                let cwd = match &task.cwd {
                    Some(dir) if host::is_absolute(dir) => dir.clone(),
                    Some(dir) => host::join(&root, dir),
                    None => root.clone(),
                };
                let title = task.label.clone();
                // Reuse a finished terminal of the same task.
                if let Some(index) = self.terminals.iter().position(|t| t.title == title && !t.is_alive()) {
                    self.terminals.remove(index);
                }
                let script = format!("printf '\\033[1;34m> %s\\033[0m\\n' {}\n{}", host::shell_quote(&task.command), task.command);
                self.spawn_terminal(host, cwd, Some(script), title);
                self.last_task = Some(task.clone());
            }
            None => {
                let title = if host.is_remote() { format!("ssh {}", host.label()) } else { "zsh".into() };
                let shell_name = std::env::var("SHELL").ok().map(|s| host::file_name(&s).to_string());
                let title = if host.is_remote() { title } else { shell_name.unwrap_or(title) };
                self.spawn_terminal(host, root, None, title);
            }
        }
    }

    fn spawn_terminal(&mut self, host: Host, cwd: String, script: Option<String>, title: String) {
        let command = host.pty_command(&cwd, script.as_deref());
        match Terminal::spawn(title, cwd, command, self.ctx.clone(), None) {
            Ok(terminal) => {
                self.terminals.push(terminal);
                self.active_terminal = self.terminals.len() - 1;
                self.panel_open = true;
                self.panel_tab = PanelTab::Terminal;
                self.terminal_focus = true;
            }
            Err(error) => self.notify(t!("Could not start the terminal: {error}", error = error), true),
        }
    }

    pub fn spawn_terminal_in(&mut self, host: Host, dir: String, title: String) {
        self.spawn_terminal(host, dir, None, title);
    }

    pub fn run_task(&mut self, task: Task) {
        // Run what is on screen, not what was last saved.
        self.save_all_blocking();
        self.new_terminal(Some(&task));
    }

    // ------------------------------------------------------------ messages

    fn process_messages(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            self.handle(msg);
        }
    }

    fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::Connected { target, result } => welcome::on_connected(self, target, result),
            Msg::DirLoaded { path, result } => {
                if let Some(workspace) = &mut self.workspace {
                    workspace.explorer.loaded(path, result);
                }
            }
            Msg::FileLoaded { path, result, goto, mtime } => self.on_file_loaded(path, result, goto, mtime),
            Msg::FileSaved { doc_id, revision, result } => self.on_file_saved(doc_id, revision, result),
            Msg::FileReloaded { path, data, mtime } => {
                let Some(doc) = self.doc_by_path(&path) else { return };
                if doc.is_dirty() {
                    return;
                }
                let text = String::from_utf8_lossy(&data).into_owned();
                if text != doc.buffer.text() {
                    doc.buffer.replace_all(&text, true);
                    doc.disk_mtime = mtime;
                    let id = doc.id;
                    self.request_highlight(id);
                }
            }
            Msg::FsChanged(paths) => {
                self.fs_pending.extend(paths);
                self.fs_pending_since.get_or_insert_with(Instant::now);
            }
            Msg::Highlight(done) => {
                if let Some(doc) = self.doc_mut(done.doc_id)
                    && done.version == doc.buffer.version
                {
                    doc.spans = done.spans;
                    doc.highlighted_version = done.version;
                }
            }
            Msg::Lsp(event) => lsp_manager::handle_event(self, event),
            Msg::LspStarted { name, result } => lsp_manager::on_started(self, name, result),
            Msg::LspResult { doc_id, version, request, client, result } => {
                lsp_manager::on_result(self, doc_id, version, request, client, result)
            }
            Msg::Shellcheck { doc_id, version, result } => {
                if let Some(doc) = self.doc_mut(doc_id)
                    && let Ok(raw) = result
                    && doc.buffer.version == version
                {
                    let diagnostics = crate::lsp::shellcheck::to_diagnostics(&doc.buffer, raw);
                    doc.diagnostics.insert("shellcheck".into(), diagnostics);
                }
            }
            Msg::GitRepo(root) => {
                if let Some(workspace) = &mut self.workspace {
                    workspace.git.repo_root = root;
                    workspace.git.checked = true;
                }
                self.git_refresh();
                let paths: Vec<String> =
                    self.tabs.iter().filter_map(|t| if let Tab::Doc(d) = t { Some(d.path.clone()) } else { None }).collect();
                for path in paths {
                    self.load_git_base(&path);
                }
                let ids: Vec<u64> = self.tabs.iter().filter_map(|t| if let Tab::Doc(d) = t { Some(d.id) } else { None }).collect();
                for id in ids {
                    self.request_blame(id);
                }
            }
            Msg::GitStatus(result) => {
                let mut again = false;
                if let Some(workspace) = &mut self.workspace {
                    workspace.git.loading = false;
                    again = std::mem::take(&mut workspace.git.refresh_again);
                    match result {
                        Ok(status) => {
                            workspace.explorer.set_git_status(&status);
                            workspace.git.status = Some(status);
                            workspace.git.error = None;
                        }
                        Err(error) => workspace.git.error = Some(error),
                    }
                }
                if again {
                    self.git_refresh();
                }
            }
            Msg::GitRefs { branches, stashes, remotes } => {
                if let Some(workspace) = &mut self.workspace {
                    workspace.git.branches = branches;
                    workspace.git.stashes = stashes;
                    workspace.git.remotes = remotes;
                }
            }
            Msg::GitOpDone { label, result } => {
                if let Some(workspace) = &mut self.workspace {
                    workspace.git.busy = None;
                }
                match result {
                    Ok(output) => {
                        let summary = output.lines().last().unwrap_or("").to_string();
                        self.output.push((format!("git {label}: {output}"), false));
                        self.toasts.push(Toast {
                            text: if summary.is_empty() {
                                t!("git {label}: done", label = label)
                            } else {
                                format!("git {label}: {summary}")
                            },
                            error: false,
                            at: Instant::now(),
                        });
                    }
                    Err(error) => self.notify(format!("git {label}: {error}"), true),
                }
                self.git_refresh();
                // The working tree may have changed (checkout, pull, stash).
                self.reload_unmodified_docs();
                let paths: Vec<String> =
                    self.tabs.iter().filter_map(|t| if let Tab::Doc(d) = t { Some(d.path.clone()) } else { None }).collect();
                for path in paths {
                    self.load_git_base(&path);
                }
                let keys: Vec<String> =
                    self.tabs.iter().filter_map(|t| if let Tab::Diff(d) = t { Some(d.key.clone()) } else { None }).collect();
                for key in keys {
                    self.reload_diff(&key);
                }
                if self.tabs.iter().any(|t| matches!(t, Tab::Log(_))) {
                    self.reload_log();
                }
            }
            Msg::GitDiff { key, result } => {
                if let Some(Tab::Diff(tab)) = self.tabs.iter_mut().find(|t| matches!(t, Tab::Diff(d) if d.key == key)) {
                    tab.set_content(result);
                }
            }
            Msg::GitLog(result) => {
                if let Some(Tab::Log(log)) = self.tabs.iter_mut().find(|t| matches!(t, Tab::Log(_))) {
                    log.loading = false;
                    match result {
                        Ok(entries) => log.entries = entries,
                        Err(error) => log.error = Some(error),
                    }
                }
            }
            Msg::GitHead { path, content } => {
                if let Some(doc) = self.doc_by_path(&path) {
                    doc.git_base = content;
                    doc.git_base_loaded = true;
                    doc.line_changes_version = u64::MAX;
                }
            }
            Msg::GitBlame { doc_id, version, result } => {
                if let Some(doc) = self.doc_mut(doc_id) {
                    match result {
                        Ok(blame) => {
                            doc.blame = Some(blame);
                            doc.blame_version = version;
                        }
                        Err(_) => doc.blame = None,
                    }
                }
            }
            Msg::Tasks(tasks, error) => {
                if let Some(workspace) = &mut self.workspace {
                    workspace.tasks = tasks;
                    workspace.tasks_error = error;
                }
            }
            Msg::FileIndex(result) => {
                if let Some(workspace) = &mut self.workspace {
                    workspace.file_index_loading = false;
                    match result {
                        Ok(files) => workspace.file_index = Some(files),
                        Err(error) => {
                            workspace.file_index = Some(Vec::new());
                            self.notify(t!("Could not list files: {error}", error = error), true);
                        }
                    }
                }
            }
            Msg::SearchResults { generation, result } => {
                if let Some(workspace) = &mut self.workspace {
                    workspace.search.on_results(generation, result);
                }
            }
            Msg::WorkspaceEditDone(result) => match result {
                Ok(count) if count > 0 => self.notify(t!("Files changed on disk: {count}", count = count), false),
                Ok(_) => {}
                Err(error) => self.notify(error, true),
            },
            Msg::FsOpDone { reload, open, renamed, error } => {
                if let Some(error) = error {
                    self.notify(error, true);
                }
                for dir in reload {
                    if self.workspace.as_ref().is_some_and(|w| w.explorer.is_loaded(&dir)) {
                        self.load_dir(dir);
                    }
                }
                if let Some((from, to)) = renamed {
                    let prefix = format!("{from}/");
                    for tab in &mut self.tabs {
                        if let Tab::Doc(doc) = tab {
                            if doc.path == from {
                                doc.path = to.clone();
                            } else if let Some(rest) = doc.path.strip_prefix(&prefix) {
                                doc.path = host::join(&to, rest);
                            }
                        }
                    }
                }
                if let Some(workspace) = &mut self.workspace {
                    workspace.file_index = None;
                }
                if let Some(path) = open {
                    self.open_file(path, None);
                }
                self.git_refresh();
            }
            Msg::RemoteDir { path, result } => welcome::on_remote_dir(self, path, result),
            Msg::Control(request) => self.handle_control(request),
            Msg::Run(job) => job(self),
            Msg::Agent { id, event } => agent::on_event(self, id, event),
            Msg::Notice(text, error) => {
                if !text.is_empty() {
                    self.notify(text, error);
                }
            }
        }
    }

    /// Loads image files for Markdown previews from the project host.
    pub fn request_images(&mut self, paths: Vec<String>) {
        let Some(host) = self.host() else { return };
        for path in paths {
            if self.image_cache.contains_key(&path) {
                continue;
            }
            self.image_cache.insert(path.clone(), media::ImageEntry::Loading);
            let host = host.clone();
            self.bus.spawn(move || {
                let entry = match host.read_file(&path) {
                    Ok(data) => {
                        let uri = media::bytes_uri(&path, &data);
                        media::ImageEntry::Ready(data.into(), uri)
                    }
                    Err(error) => media::ImageEntry::Failed(error),
                };
                Msg::Run(Box::new(move |app: &mut App| {
                    app.image_cache.insert(path, entry);
                }))
            });
        }
    }

    /// Images changed on disk: open viewers reload, previews load again.
    fn reload_changed_images(&mut self, paths: &BTreeSet<String>) {
        for path in paths {
            if let Some(media::ImageEntry::Ready(_, uri)) = self.image_cache.remove(path) {
                self.ctx.forget_image(&uri);
            }
        }
        let Some(host) = self.host() else { return };
        let viewers: Vec<String> = self
            .tabs
            .iter()
            .filter_map(|t| if let Tab::Image(i) = t { paths.contains(&i.path).then(|| i.path.clone()) } else { None })
            .collect();
        for path in viewers {
            let host = host.clone();
            self.bus.spawn(move || {
                let data = host.read_file(&path);
                Msg::Run(Box::new(move |app: &mut App| {
                    let Ok(data) = data else { return };
                    let ctx = app.ctx.clone();
                    for tab in &mut app.tabs {
                        if let Tab::Image(image) = tab
                            && image.path == path
                        {
                            image.replace(&ctx, data.clone());
                        }
                    }
                }))
            });
        }
    }

    /// Reloads every expanded directory of the file tree.
    pub fn reload_explorer(&mut self) {
        let Some(workspace) = &mut self.workspace else { return };
        let dirs = workspace.explorer.loaded_dirs();
        workspace.file_index = None;
        for dir in dirs {
            self.load_dir(dir);
        }
        self.git_refresh();
    }

    fn reload_unmodified_docs(&mut self) {
        let Some(host) = self.host() else { return };
        for tab in &self.tabs {
            if let Tab::Doc(doc) = tab
                && !doc.is_dirty()
            {
                let path = doc.path.clone();
                let host = host.clone();
                let known = doc.disk_mtime;
                self.bus.spawn(move || {
                    let mtime = host.stat(&path).map(|s| s.mtime).unwrap_or(0);
                    if mtime != 0 && mtime == known {
                        return Msg::Notice(String::new(), false);
                    }
                    match host.read_file(&path) {
                        Ok(data) => Msg::FileReloaded { path, data, mtime },
                        Err(_) => Msg::Notice(String::new(), false),
                    }
                });
            }
        }
    }

    fn process_fs_changes(&mut self) {
        let Some(since) = self.fs_pending_since else { return };
        if since.elapsed() < Duration::from_millis(250) {
            self.ctx.request_repaint_after(Duration::from_millis(260));
            return;
        }
        self.fs_pending_since = None;
        let paths = std::mem::take(&mut self.fs_pending);
        let Some(workspace) = &mut self.workspace else { return };
        let mut dirs = BTreeSet::new();
        let mut git_changed = false;
        let mut worktree_changed = false;
        for path in &paths {
            if path.contains("/.git/") || path.ends_with("/.git") {
                if path.ends_with("/index") || path.ends_with("HEAD") || path.contains("/refs/") {
                    git_changed = true;
                }
                continue;
            }
            worktree_changed = true;
            dirs.insert(host::parent(path));
        }
        let reload: Vec<String> = dirs.into_iter().filter(|d| workspace.explorer.is_loaded(d)).collect();
        if worktree_changed {
            workspace.file_index = None;
        }
        for dir in reload {
            self.load_dir(dir);
        }
        if git_changed || worktree_changed {
            self.git_refresh();
        }
        // Reload clean documents changed on disk.
        let open: Vec<String> = self.tabs.iter().filter_map(|t| if let Tab::Doc(d) = t { Some(d.path.clone()) } else { None }).collect();
        if open.iter().any(|p| paths.contains(p)) {
            self.reload_unmodified_docs();
        }
        self.reload_changed_images(&paths);
        if git_changed {
            let open_paths = open.clone();
            for path in open_paths {
                self.load_git_base(&path);
            }
        }
    }

    // ------------------------------------------------------------- frame

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let shortcuts = commands::shortcuts();
        let mut triggered = Vec::new();
        ctx.input_mut(|input| {
            for (shortcut, command) in &shortcuts {
                if input.consume_shortcut(shortcut) {
                    triggered.push(*command);
                }
            }
        });
        for command in triggered {
            self.run_command(command);
        }
    }

    fn per_frame_upkeep(&mut self) {
        if let Some(Tab::Doc(doc)) = self.active_tab.and_then(|i| self.tabs.get(i)) {
            self.last_doc_id = Some(doc.id);
        }
        let workspace_root = self.workspace.as_ref().map(|w| w.root.clone());
        let mut highlight = Vec::new();
        let mut shellcheck = Vec::new();
        for tab in &mut self.tabs {
            if let Tab::Doc(doc) = tab {
                doc.absorb_edits();
                if doc.highlight_requested != doc.buffer.version {
                    highlight.push(doc.id);
                }
                doc.refresh_line_changes();
                if doc.lang == syntax::Lang::Bash
                    && doc.shellcheck_version != doc.buffer.version
                    && doc.last_edit.elapsed() > Duration::from_millis(700)
                {
                    shellcheck.push(doc.id);
                }
            }
        }
        for id in highlight {
            self.request_highlight(id);
        }
        for id in shellcheck {
            self.run_shellcheck(id);
        }
        self.lsp.sync_documents(&mut self.tabs, Duration::from_millis(150));
        if self.tabs.iter().any(|t| matches!(t, Tab::Doc(d) if d.buffer.version != d.lsp_synced)) {
            self.ctx.request_repaint_after(Duration::from_millis(160));
        }
        if self.tabs.iter().any(|t| matches!(t, Tab::Doc(d) if d.lang == syntax::Lang::Bash && d.shellcheck_version != d.buffer.version)) {
            self.ctx.request_repaint_after(Duration::from_millis(720));
        }
        self.process_fs_changes();
        pulse::tick(self);

        // Auto save after a pause.
        if self.config.auto_save {
            let due: Vec<usize> = self
                .tabs
                .iter()
                .enumerate()
                .filter(
                    |(_, t)| matches!(t, Tab::Doc(d) if d.is_dirty() && !d.saving && d.last_edit.elapsed() > Duration::from_millis(1200)),
                )
                .map(|(i, _)| i)
                .collect();
            for index in due {
                self.save_doc_now(index);
            }
            if self.tabs.iter().any(Tab::dirty) {
                self.ctx.request_repaint_after(Duration::from_millis(1300));
            }
        }
        if self.last_session_save.elapsed() > Duration::from_secs(30) && workspace_root.is_some() {
            self.last_session_save = Instant::now();
            self.save_session();
        }
        self.toasts.retain(|t| t.at.elapsed() < Duration::from_secs(if t.error { 8 } else { 4 }));
        if !self.toasts.is_empty() {
            self.ctx.request_repaint_after(Duration::from_millis(500));
        }
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let title = match (&self.workspace, self.active_tab.and_then(|i| self.tabs.get(i))) {
            (Some(workspace), Some(tab)) => {
                let remote = if workspace.host.is_remote() { format!(" [{}]", workspace.host.label()) } else { String::new() };
                format!("{}{} — {}{remote} — Lumen Edit", if tab.dirty() { "● " } else { "" }, tab.title(), workspace.name())
            }
            (Some(workspace), None) => format!("{} — Lumen Edit", workspace.name()),
            _ => "Lumen Edit".into(),
        };
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
    }

    fn handle_close_request(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_close {
            if self.tabs.iter().any(Tab::dirty) {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.dialog = Some(Dialog::Quit);
            } else {
                self.save_session();
                self.lsp.shutdown_all();
            }
        }
    }

    fn show_dialogs(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.dialog.take() else { return };
        let mut keep = true;
        let mut after: Option<Box<dyn FnOnce(&mut App)>> = None;
        let modal = |ctx: &egui::Context, title: &str, body: &mut dyn FnMut(&mut egui::Ui)| {
            egui::Window::new(title)
                .id(egui::Id::unique("app_dialog"))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 120.0))
                .show(ctx, |ui| {
                    ui.set_min_width(380.0);
                    body(ui);
                });
        };
        match dialog {
            Dialog::CloseTab { index } => {
                let name = self.tabs.get(index).map(|t| t.title()).unwrap_or_default();
                let mut choice = None;
                modal(ctx, t!("Unsaved changes"), &mut |ui| {
                    ui.label(t!("Save changes to “{name}”?", name = name));
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.button(t!("Save")).clicked() {
                            choice = Some(0);
                        }
                        if ui.button(t!("Don't save")).clicked() {
                            choice = Some(1);
                        }
                        if ui.button(t!("Cancel")).clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                            choice = Some(2);
                        }
                    });
                });
                match choice {
                    Some(0) => {
                        self.save_doc_now(index);
                        after = Some(Box::new(move |app: &mut App| app.close_tab(index)));
                        keep = false;
                    }
                    Some(1) => {
                        self.close_tab(index);
                        keep = false;
                    }
                    Some(_) => keep = false,
                    None => self.dialog = Some(Dialog::CloseTab { index }),
                }
            }
            Dialog::CloseWorkspace { then } => {
                let mut choice = None;
                modal(ctx, t!("Unsaved changes"), &mut |ui| {
                    ui.label(t!("There are unsaved files. Save them?"));
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.button(t!("Save All")).clicked() {
                            choice = Some(0);
                        }
                        if ui.button(t!("Don't save")).clicked() {
                            choice = Some(1);
                        }
                        if ui.button(t!("Cancel")).clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                            choice = Some(2);
                        }
                    });
                });
                match choice {
                    Some(0) => {
                        // Write synchronously so nothing is lost when tabs close.
                        self.save_all_blocking();
                        then(self);
                    }
                    Some(1) => {
                        for tab in &mut self.tabs {
                            if let Tab::Doc(doc) = tab {
                                doc.buffer.mark_saved();
                            }
                        }
                        then(self);
                    }
                    Some(_) => {}
                    None => self.dialog = Some(Dialog::CloseWorkspace { then }),
                }
                return;
            }
            Dialog::Quit => {
                let mut choice = None;
                modal(ctx, t!("Quit"), &mut |ui| {
                    ui.label(t!("There are unsaved files."));
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.button(t!("Save and quit")).clicked() {
                            choice = Some(0);
                        }
                        if ui.button(t!("Quit without saving")).clicked() {
                            choice = Some(1);
                        }
                        if ui.button(t!("Cancel")).clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                            choice = Some(2);
                        }
                    });
                });
                match choice {
                    Some(0) => {
                        self.save_all_blocking();
                        self.quit(ctx);
                    }
                    Some(1) => self.quit(ctx),
                    Some(_) => {}
                    None => self.dialog = Some(Dialog::Quit),
                }
                return;
            }
            Dialog::Confirm { title, message, action } => {
                let mut choice = None;
                modal(ctx, &title, &mut |ui| {
                    ui.label(&message);
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new(t!("Yes")).color(theme::ERROR)).clicked() {
                            choice = Some(true);
                        }
                        if ui.button(t!("Cancel")).clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                            choice = Some(false);
                        }
                    });
                });
                match choice {
                    Some(true) => action(self),
                    Some(false) => {}
                    None => self.dialog = Some(Dialog::Confirm { title, message, action }),
                }
                return;
            }
            Dialog::Input { title, mut value, action, focus } => {
                let mut choice = None;
                modal(ctx, &title, &mut |ui| {
                    let input = ui.add(egui::TextEdit::singleline(&mut value).desired_width(f32::INFINITY));
                    if focus {
                        input.request_focus();
                    }
                    if input.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        choice = Some(true);
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("OK").clicked() {
                            choice = Some(true);
                        }
                        if ui.button(t!("Cancel")).clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                            choice = Some(false);
                        }
                    });
                });
                match choice {
                    Some(true) if !value.trim().is_empty() => action(self, value.trim().to_string()),
                    Some(_) => {}
                    None => self.dialog = Some(Dialog::Input { title, value, action, focus: false }),
                }
                return;
            }
        }
        if !keep {
            self.dialog = None;
        }
        if let Some(after) = after {
            after(self);
        }
    }

    fn save_all_blocking(&mut self) {
        let Some(host) = self.host() else { return };
        for tab in &mut self.tabs {
            if let Tab::Doc(doc) = tab
                && doc.is_dirty()
            {
                match host.write_file(&doc.path, doc.buffer.text().as_bytes()) {
                    Ok(()) => doc.buffer.mark_saved(),
                    Err(error) => eprintln!("lumen-edit: save {}: {error}", doc.path),
                }
            }
        }
    }

    fn quit(&mut self, ctx: &egui::Context) {
        self.save_session();
        self.lsp.shutdown_all();
        self.allow_close = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn show_toasts(&mut self, ctx: &egui::Context) {
        if self.toasts.is_empty() {
            return;
        }
        let screen = ctx.content_rect();
        egui::Area::new(egui::Id::unique("toasts"))
            .order(egui::Order::Tooltip)
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-16.0, -36.0))
            .interactable(false)
            .show(ctx, |ui| {
                ui.set_max_width((screen.width() * 0.4).max(320.0));
                for toast in self.toasts.iter().rev().take(4).rev() {
                    egui::Frame::popup(ui.style()).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
                        let color = if toast.error { theme::ERROR } else { theme::TEXT };
                        let icon = if toast.error { "▲ " } else { "ℹ " };
                        ui.label(RichText::new(format!("{icon}{}", util::truncate_middle(&toast.text, 300))).color(color));
                    });
                    ui.add_space(4.0);
                }
            });
    }
}

fn goto_position(doc: &mut Document, (line, col): (usize, usize)) {
    let at = doc.buffer.char_at(line, col);
    doc.buffer.set_cursor(at, false);
    doc.view.reveal = true;
    doc.view.reveal_center = true;
    doc.view.focus_request = true;
}

fn local_watcher(root: &str, bus: Bus) -> Option<lumen_shared::watch::TreeWatcher> {
    lumen_shared::watch::watch_tree(root, move |paths| bus.send(Msg::FsChanged(paths))).ok()
}

fn install_fonts(ctx: &egui::Context, config: &Config) {
    let mut fonts = egui::FontDefinitions::default();
    let candidates = crate::platform::FONTS;
    if let Some(data) = candidates.ui.iter().find_map(|p| std::fs::read(p).ok()) {
        fonts.font_data.insert("ui".into(), egui::FontData::from_owned(data).into());
        fonts.families.entry(egui::FontFamily::Proportional).or_default().insert(0, "ui".into());
    }
    let mono_candidates: Vec<String> = config.editor_font.iter().cloned().chain(candidates.mono.iter().map(|s| s.to_string())).collect();
    if let Some(data) = mono_candidates.iter().find_map(|p| std::fs::read(p).ok()) {
        fonts.font_data.insert("code".into(), egui::FontData::from_owned(data).into());
        fonts.families.entry(egui::FontFamily::Monospace).or_default().insert(0, "code".into());
    }
    // Powerline / Nerd Font glyphs used by shell prompts.
    if let Some(data) = candidates.nerd.iter().find_map(|p| std::fs::read(p).ok()) {
        fonts.font_data.insert("nerd".into(), egui::FontData::from_owned(data).into());
        fonts.families.entry(egui::FontFamily::Monospace).or_default().push("nerd".into());
    }
    // Symbols for icons that the UI font may lack.
    for (i, path) in candidates.symbols.iter().enumerate() {
        if let Ok(data) = std::fs::read(path) {
            let name = format!("symbols{i}");
            fonts.font_data.insert(name.clone(), egui::FontData::from_owned(data).into());
            fonts.families.entry(egui::FontFamily::Proportional).or_default().push(name.clone());
            fonts.families.entry(egui::FontFamily::Monospace).or_default().push(name);
        }
    }
    // Bundled last, so icons exist even where the system has no symbol fonts.
    for (name, data) in BUNDLED_SYMBOLS {
        fonts.font_data.insert(name.to_string(), egui::FontData::from_static(data).into());
        fonts.families.entry(egui::FontFamily::Proportional).or_default().push(name.to_string());
        fonts.families.entry(egui::FontFamily::Monospace).or_default().push(name.to_string());
    }
    ctx.set_fonts(fonts);
}

/// Noto Sans Symbols 1 and 2 (SIL OFL 1.1, see assets/fonts/OFL.txt).
const BUNDLED_SYMBOLS: [(&str, &[u8]); 2] = [
    ("noto_symbols", include_bytes!("../../assets/fonts/NotoSansSymbols-Regular.ttf")),
    ("noto_symbols2", include_bytes!("../../assets/fonts/NotoSansSymbols2-Regular.ttf")),
];

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.process_messages();
        self.handle_close_request(&ctx);
        if self.palette.is_none() && self.dialog.is_none() {
            self.handle_shortcuts(&ctx);
        }
        if ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::WindowFocused(true)))) {
            // Catch changes made elsewhere when there is no watcher.
            if self.workspace.as_ref().is_some_and(|w| w.host.is_remote()) {
                self.git_refresh();
                self.reload_unmodified_docs();
            }
        }

        if let Some(at) = self.debug_goto
            && let Some(doc) = self.active_doc()
        {
            goto_position(doc, at);
            self.debug_goto = None;
        }
        let delay: u64 = std::env::var("LUMEN_DEBUG_TYPE_FRAME").ok().and_then(|s| s.parse().ok()).unwrap_or(120);
        if let Some(text) = self.debug_typing.clone()
            && ctx.cumulative_frame_nr() > delay
            && self.active_doc().is_some()
        {
            self.debug_typing = None;
            if let Some(id) = self.active_doc().map(|d| d.id) {
                ctx.memory_mut(|m| m.request_focus(egui::Id::unique(("editor", id))));
            }
            ctx.input_mut(|i| {
                for c in text.split('|') {
                    i.events.push(egui::Event::Text(c.replace("\\n", "\n")));
                }
            });
        }
        // Files dropped from the desktop go to the selected project folder.
        let dropped: Vec<std::path::PathBuf> =
            ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
        if !dropped.is_empty()
            && let Some(workspace) = &self.workspace
        {
            let dir = workspace.explorer.target_dir(&workspace.root);
            teleport::upload_dropped(self, dropped, dir);
        }
        status::menu_bar(ui, self);
        status::status_bar(ui, self);
        if self.workspace.is_some() {
            status::activity_bar(ui, self);
            if self.sidebar_open {
                egui::Panel::left("sidebar")
                    .resizable(true)
                    .default_size(280.0)
                    .size_range(180.0..=640.0)
                    .frame(egui::Frame::new().fill(theme::SIDEBAR_BACKGROUND).stroke(egui::Stroke::new(1.0, theme::BORDER)))
                    .show(ui, |ui| match self.side_view {
                        SideView::Explorer => explorer::show(ui, self),
                        SideView::Search => search::show(ui, self),
                        SideView::Git => scm::show(ui, self),
                        SideView::Run => run::show(ui, self),
                    });
            }
            if self.agent_dock.is_some() && self.agent_dock_open {
                let width = (ctx.content_rect().width() * 0.38).clamp(340.0, 620.0);
                egui::Panel::right("agent_dock")
                    .resizable(true)
                    .default_size(width)
                    .size_range(320.0..=1000.0)
                    .frame(egui::Frame::new().fill(theme::EDITOR_BACKGROUND).stroke(egui::Stroke::new(1.0, theme::BORDER)))
                    .show(ui, |ui| agent::show(ui, self, agent::Loc::Dock));
            }
            if self.panel_open {
                egui::Panel::bottom("panel")
                    .resizable(true)
                    .default_size(260.0)
                    .size_range(100.0..=900.0)
                    .frame(egui::Frame::new().fill(theme::PANEL_BACKGROUND).stroke(egui::Stroke::new(1.0, theme::BORDER)))
                    .show(ui, |ui| panel::show(ui, self));
            }
        }

        egui::CentralPanel::default().frame(egui::Frame::new().fill(theme::EDITOR_BACKGROUND)).show(ui, |ui| {
            if self.workspace.is_none() {
                welcome::show(ui, self);
            } else {
                panel::editor_area(ui, self);
            }
        });

        if self.connect.is_some() {
            welcome::connect_window(&ctx, self);
        }
        if self.palette.is_some() {
            palette::show(&ctx, self);
        }
        if self.settings_open {
            status::settings_window(&ctx, self);
        }
        if self.about_open {
            about::window(&ctx, self);
        }
        self.show_dialogs(&ctx);
        self.show_toasts(&ctx);
        self.per_frame_upkeep();
        self.update_title(&ctx);
        if let Ok(path) = std::env::var("LUMEN_SCREENSHOT") {
            self.allow_close = true;
            screenshot_hook(&ctx, &path);
        }
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        // Keep frames coming while synthetic input waits for its turn.
        if self.debug_key.is_some() || !self.debug_clicks.is_empty() {
            ctx.request_repaint();
        }
        // Debug aid: one synthetic key press once the delay passed.
        if let Some(key) = self.debug_key
            && ctx.cumulative_frame_nr() > std::env::var("LUMEN_DEBUG_TYPE_FRAME").ok().and_then(|s| s.parse().ok()).unwrap_or(120)
        {
            self.debug_key = None;
            for pressed in [true, false] {
                raw_input.events.push(egui::Event::Key {
                    key,
                    physical_key: Some(key),
                    pressed,
                    repeat: false,
                    modifiers: Default::default(),
                });
            }
        }
        // Debug aid: synthetic clicks (LUMEN_DEBUG_VIEW=click=x:y), one per 60 frames.
        if self.debug_clicks.is_empty() {
            return;
        }
        let delay: u64 = std::env::var("LUMEN_DEBUG_TYPE_FRAME").ok().and_then(|s| s.parse().ok()).unwrap_or(120);
        let frame = ctx.cumulative_frame_nr();
        if frame <= delay {
            return;
        }
        let (pos, button) = self.debug_clicks[0];
        let pressed = |pressed| egui::Event::PointerButton { pos, button, pressed, modifiers: Default::default() };
        match (frame - delay) % 60 {
            0 => raw_input.events.push(egui::Event::PointerMoved(pos)),
            2 => raw_input.events.push(pressed(true)),
            3 => {
                raw_input.events.push(pressed(false));
                self.debug_clicks.remove(0);
            }
            _ => {}
        }
        ctx.request_repaint();
    }

    fn on_exit(&mut self) {
        crate::control::cleanup();
        self.save_session();
        self.lsp.shutdown_all();
    }
}

/// Debug aid: `LUMEN_SCREENSHOT=out.png` saves a frame after start-up and exits.
fn screenshot_hook(ctx: &egui::Context, path: &str) {
    let frame = ctx.cumulative_frame_nr();
    let delay: u64 = std::env::var("LUMEN_SCREENSHOT_FRAME").ok().and_then(|s| s.parse().ok()).unwrap_or(40);
    if frame == delay {
        let (ctx, path) = (ctx.clone(), path.to_string());
        ctx.clone().request_screenshot(move |image| {
            let [w, h] = image.size;
            let mut out = Vec::with_capacity(w * h * 3);
            for pixel in &image.pixels {
                out.extend_from_slice(&[pixel.r(), pixel.g(), pixel.b()]);
            }
            let header = format!("P6\n{w} {h}\n255\n");
            let mut data = header.into_bytes();
            data.extend(out);
            let _ = std::fs::write(&path, data);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            ctx.request_repaint();
        });
    }
    if frame < delay + 5 {
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod glyph_tests {
    use eframe::egui;

    #[test]
    fn icon_glyphs_exist() {
        let ctx = egui::Context::default();
        super::install_fonts(&ctx, &crate::config::Config::default());
        // Every non-Latin/Cyrillic symbol used in string literals must render.
        let mut chars = std::collections::BTreeSet::new();
        let mut stack = vec![std::path::PathBuf::from("src")];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let text = std::fs::read_to_string(&path).unwrap();
                    for literal in text.split('"').skip(1).step_by(2) {
                        chars.extend(literal.chars().filter(|c| *c as u32 > 0x2000 && !('\u{400}'..='\u{4ff}').contains(c) && *c != '😀'));
                    }
                }
            }
        }
        let mut missing = String::new();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            ui.fonts_mut(|f| {
                for &c in &chars {
                    if !f.has_glyph(&egui::FontId::proportional(13.0), c) {
                        missing.push(c);
                    }
                }
            });
        });
        output.textures_delta.clear();
        // egui reports a glyph as missing when it lives in the font that also
        // serves the replacement glyph; ask the font files themselves.
        let candidates = crate::platform::FONTS;
        let mut files: Vec<Vec<u8>> =
            candidates.ui.iter().chain(candidates.symbols).chain(candidates.nerd).filter_map(|p| std::fs::read(p).ok()).collect();
        files.extend(super::BUNDLED_SYMBOLS.iter().map(|(_, data)| data.to_vec()));
        missing.retain(|c| {
            !files.iter().any(|data| {
                use skrifa::MetadataProvider;
                skrifa::FontRef::from_index(data, 0).is_ok_and(|font| font.charmap().map(c).is_some())
            })
        });
        assert!(missing.is_empty(), "glyphs missing from fonts: {missing}");
    }
}
