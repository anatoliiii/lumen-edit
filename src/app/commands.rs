//! All user-facing commands: palette entries, menu items and shortcuts.

use eframe::egui::{Key, KeyboardShortcut, Modifiers};

use super::{App, PanelTab, SideView, Tab, editor::EditorAction, palette::Palette};
use crate::tasks::Group;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    QuickOpen,
    CommandPalette,
    GotoLine,
    OpenFolder,
    ConnectSsh,
    OpenRecent,
    CloseWorkspace,
    NewFile,
    NewFolder,
    Teleport,
    Save,
    SaveAll,
    CloseTab,
    NextTab,
    PrevTab,
    ToggleSidebar,
    ShowExplorer,
    ShowSearch,
    ShowGit,
    ShowRun,
    TogglePanel,
    ToggleTerminal,
    NewTerminal,
    KillTerminal,
    ShowProblems,
    ShowOutput,
    ShowPulse,
    RunDefault,
    RunTask,
    Build,
    Test,
    RerunLast,
    EditTasks,
    Find,
    Replace,
    MarkdownPreview,
    GotoDefinition,
    References,
    Rename,
    CodeActions,
    Format,
    RestartLsp,
    GitCommit,
    GitCommitAmend,
    GitPush,
    GitPull,
    GitFetch,
    GitSync,
    GitCheckout,
    GitNewBranch,
    GitMerge,
    GitStash,
    GitStashPop,
    GitLog,
    GitStageFile,
    GitFileHistory,
    GitInit,
    GitAddRemote,
    GitToggleBlame,
    Agent,
    AgentSendSelection,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    Settings,
    About,
    Quit,
}

pub const ALL: &[Command] = &[
    Command::QuickOpen,
    Command::CommandPalette,
    Command::GotoLine,
    Command::OpenFolder,
    Command::ConnectSsh,
    Command::OpenRecent,
    Command::CloseWorkspace,
    Command::NewFile,
    Command::NewFolder,
    Command::Teleport,
    Command::Save,
    Command::SaveAll,
    Command::CloseTab,
    Command::NextTab,
    Command::PrevTab,
    Command::ToggleSidebar,
    Command::ShowExplorer,
    Command::ShowSearch,
    Command::ShowGit,
    Command::ShowRun,
    Command::TogglePanel,
    Command::ToggleTerminal,
    Command::NewTerminal,
    Command::KillTerminal,
    Command::ShowProblems,
    Command::ShowOutput,
    Command::ShowPulse,
    Command::RunDefault,
    Command::RunTask,
    Command::Build,
    Command::Test,
    Command::RerunLast,
    Command::EditTasks,
    Command::Find,
    Command::Replace,
    Command::MarkdownPreview,
    Command::GotoDefinition,
    Command::References,
    Command::Rename,
    Command::CodeActions,
    Command::Format,
    Command::RestartLsp,
    Command::GitCommit,
    Command::GitCommitAmend,
    Command::GitPush,
    Command::GitPull,
    Command::GitFetch,
    Command::GitSync,
    Command::GitCheckout,
    Command::GitNewBranch,
    Command::GitMerge,
    Command::GitStash,
    Command::GitStashPop,
    Command::GitLog,
    Command::GitStageFile,
    Command::GitFileHistory,
    Command::GitInit,
    Command::GitAddRemote,
    Command::GitToggleBlame,
    Command::Agent,
    Command::AgentSendSelection,
    Command::ZoomIn,
    Command::ZoomOut,
    Command::ZoomReset,
    Command::Settings,
    Command::About,
    Command::Quit,
];

impl Command {
    /// Stable snake_case id for the CLI and MCP, e.g. `GitPush` -> `git_push`.
    pub fn id(self) -> String {
        let name = format!("{self:?}");
        let mut id = String::new();
        for (i, c) in name.chars().enumerate() {
            if c.is_uppercase() && i > 0 {
                id.push('_');
            }
            id.push(c.to_ascii_lowercase());
        }
        id
    }

    pub fn from_id(id: &str) -> Option<Command> {
        let id = id.trim().replace('-', "_").to_lowercase();
        ALL.iter().copied().find(|c| c.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            Command::QuickOpen => t!("Go to File…"),
            Command::CommandPalette => t!("Command Palette"),
            Command::GotoLine => t!("Go to Line…"),
            Command::OpenFolder => t!("Open Folder…"),
            Command::ConnectSsh => t!("Connect via SSH…"),
            Command::OpenRecent => t!("Open Recent…"),
            Command::CloseWorkspace => t!("Close Project"),
            Command::NewFile => t!("New File…"),
            Command::NewFolder => t!("New Folder…"),
            Command::Teleport => t!("File Teleport"),
            Command::Save => t!("Save"),
            Command::SaveAll => t!("Save All"),
            Command::CloseTab => t!("Close Tab"),
            Command::NextTab => t!("Next Tab"),
            Command::PrevTab => t!("Previous Tab"),
            Command::ToggleSidebar => t!("Toggle Sidebar"),
            Command::ShowExplorer => t!("Explorer"),
            Command::ShowSearch => t!("Search in Project"),
            Command::ShowGit => t!("Git: Changes"),
            Command::ShowRun => t!("Run and Tasks"),
            Command::TogglePanel => t!("Toggle Bottom Panel"),
            Command::ToggleTerminal => t!("Terminal: Toggle"),
            Command::NewTerminal => t!("Terminal: New"),
            Command::KillTerminal => t!("Terminal: Close Current"),
            Command::ShowProblems => t!("Problems"),
            Command::ShowOutput => t!("Output"),
            Command::ShowPulse => t!("Server Pulse"),
            Command::RunDefault => t!("Run: Default Task"),
            Command::RunTask => t!("Run: Choose Task…"),
            Command::Build => t!("Run: Build"),
            Command::Test => t!("Run: Test"),
            Command::RerunLast => t!("Run: Rerun Last Task"),
            Command::EditTasks => t!("Run: Configure Tasks (tasks.toml)"),
            Command::Find => t!("Find"),
            Command::MarkdownPreview => t!("Markdown: Preview / Source"),
            Command::Replace => t!("Replace"),
            Command::GotoDefinition => t!("Go to Definition"),
            Command::References => t!("Find All References"),
            Command::Rename => t!("Rename Symbol"),
            Command::CodeActions => t!("Quick Fixes…"),
            Command::Format => t!("Format Document"),
            Command::RestartLsp => t!("Restart Language Servers"),
            Command::GitCommit => t!("Git: Commit"),
            Command::GitCommitAmend => t!("Git: Amend Last Commit"),
            Command::GitPush => "Git: push",
            Command::GitPull => "Git: pull",
            Command::GitFetch => "Git: fetch",
            Command::GitSync => t!("Git: Sync (pull + push)"),
            Command::GitCheckout => t!("Git: Switch Branch…"),
            Command::GitNewBranch => t!("Git: Create Branch…"),
            Command::GitMerge => t!("Git: Merge Branch into Current…"),
            Command::GitStash => t!("Git: Stash Changes"),
            Command::GitStashPop => t!("Git: Pop Stash"),
            Command::GitLog => t!("Git: History (graph)"),
            Command::GitStageFile => t!("Git: Stage Current File"),
            Command::GitFileHistory => t!("Git: Current File History"),
            Command::GitInit => t!("Git: Initialize Repository"),
            Command::GitAddRemote => t!("Git: Add Remote…"),
            Command::GitToggleBlame => t!("Git: Toggle Line Blame"),
            Command::Agent => t!("pi Agent"),
            Command::AgentSendSelection => t!("Agent: Send Selection"),
            Command::ZoomIn => t!("Zoom In"),
            Command::ZoomOut => t!("Zoom Out"),
            Command::ZoomReset => t!("Reset Zoom"),
            Command::Settings => t!("Settings"),
            Command::About => t!("About"),
            Command::Quit => t!("Quit"),
        }
    }

    pub fn shortcut_text(self) -> Option<&'static str> {
        Some(match self {
            Command::QuickOpen => "Ctrl+P",
            Command::CommandPalette => "Ctrl+Shift+P",
            Command::GotoLine => "Ctrl+G",
            Command::OpenFolder => "Ctrl+O",
            Command::ConnectSsh => "Ctrl+Alt+O",
            Command::NewFile => "Ctrl+N",
            Command::Teleport => "Ctrl+Alt+T",
            Command::Save => "Ctrl+S",
            Command::SaveAll => "Ctrl+Alt+S",
            Command::CloseTab => "Ctrl+W",
            Command::NextTab => "Ctrl+Tab",
            Command::PrevTab => "Ctrl+Shift+Tab",
            Command::ToggleSidebar => "Ctrl+B",
            Command::ShowExplorer => "Ctrl+Shift+E",
            Command::ShowSearch => "Ctrl+Shift+F",
            Command::ShowGit => "Ctrl+Shift+G",
            Command::ShowRun => "Ctrl+Shift+R",
            Command::TogglePanel => "Ctrl+J",
            Command::ToggleTerminal => "Ctrl+`",
            Command::NewTerminal => "Ctrl+Shift+`",
            Command::ShowProblems => "Ctrl+Shift+M",
            Command::RunDefault => "F5",
            Command::RunTask => "Ctrl+Alt+R",
            Command::Build => "Ctrl+Shift+B",
            Command::RerunLast => "Ctrl+F5",
            Command::KillTerminal => "Shift+F5",
            Command::Find => "Ctrl+F",
            Command::MarkdownPreview => "Ctrl+Alt+P",
            Command::Replace => "Ctrl+H",
            Command::GotoDefinition => "F12",
            Command::References => "Shift+F12",
            Command::Rename => "F2",
            Command::CodeActions => "Ctrl+.",
            Command::Format => "Ctrl+Shift+I",
            Command::GitCommit => "Ctrl+Enter",
            Command::Agent => "Ctrl+Shift+A",
            Command::AgentSendSelection => "Ctrl+Alt+A",
            Command::ZoomIn => "Ctrl+=",
            Command::ZoomOut => "Ctrl+-",
            Command::ZoomReset => "Ctrl+0",
            Command::Settings => "Ctrl+,",
            Command::Quit => "Ctrl+Q",
            _ => return None,
        })
    }
}

/// Global shortcuts, checked before widgets see the input. Editor-local
/// keys (F12, Ctrl+F, ...) are handled by the editor itself.
pub fn shortcuts() -> Vec<(KeyboardShortcut, Command)> {
    let ctrl = Modifiers::COMMAND;
    let ctrl_shift = Modifiers::COMMAND | Modifiers::SHIFT;
    let ctrl_alt = Modifiers::COMMAND | Modifiers::ALT;
    let none = Modifiers::NONE;
    let s = |m: Modifiers, k: Key| KeyboardShortcut::new(m, k);
    vec![
        // Shift variants first: consume_shortcut ignores extra modifiers.
        (s(ctrl_shift, Key::P), Command::CommandPalette),
        (s(ctrl, Key::P), Command::QuickOpen),
        (s(ctrl, Key::G), Command::GotoLine),
        (s(ctrl_alt, Key::O), Command::ConnectSsh),
        (s(ctrl, Key::O), Command::OpenFolder),
        (s(ctrl, Key::N), Command::NewFile),
        (s(ctrl_alt, Key::T), Command::Teleport),
        (s(ctrl_alt, Key::S), Command::SaveAll),
        (s(ctrl, Key::S), Command::Save),
        (s(ctrl, Key::W), Command::CloseTab),
        (s(ctrl_shift, Key::Tab), Command::PrevTab),
        (s(ctrl, Key::Tab), Command::NextTab),
        (s(ctrl, Key::PageDown), Command::NextTab),
        (s(ctrl, Key::PageUp), Command::PrevTab),
        (s(ctrl, Key::B), Command::ToggleSidebar),
        (s(ctrl_alt, Key::P), Command::MarkdownPreview),
        (s(ctrl_shift, Key::E), Command::ShowExplorer),
        (s(ctrl_shift, Key::F), Command::ShowSearch),
        (s(ctrl_shift, Key::G), Command::ShowGit),
        (s(ctrl_shift, Key::R), Command::ShowRun),
        (s(ctrl_shift, Key::M), Command::ShowProblems),
        (s(ctrl, Key::J), Command::TogglePanel),
        (s(ctrl_shift, Key::Backtick), Command::NewTerminal),
        (s(ctrl, Key::Backtick), Command::ToggleTerminal),
        (s(ctrl, Key::F5), Command::RerunLast),
        (s(Modifiers::SHIFT, Key::F5), Command::KillTerminal),
        (s(none, Key::F5), Command::RunDefault),
        (s(ctrl_alt, Key::R), Command::RunTask),
        (s(ctrl_shift, Key::B), Command::Build),
        (s(ctrl_shift, Key::A), Command::Agent),
        (s(ctrl_alt, Key::A), Command::AgentSendSelection),
        (s(ctrl, Key::Equals), Command::ZoomIn),
        (s(ctrl, Key::Plus), Command::ZoomIn),
        (s(ctrl, Key::Minus), Command::ZoomOut),
        (s(ctrl, Key::Num0), Command::ZoomReset),
        (s(ctrl, Key::Comma), Command::Settings),
        (s(ctrl, Key::Q), Command::Quit),
    ]
}

impl App {
    pub fn run_command(&mut self, command: Command) {
        let has_workspace = self.workspace.is_some();
        match command {
            Command::QuickOpen if has_workspace => {
                self.load_file_index_if_needed();
                self.palette = Some(Palette::files());
            }
            Command::CommandPalette => self.palette = Some(Palette::commands()),
            Command::GotoLine => self.palette = Some(Palette::goto_line()),
            Command::OpenFolder => self.open_folder_dialog(),
            Command::ConnectSsh => self.connect = Some(super::welcome::ConnectState::input(&self.config)),
            Command::OpenRecent => self.palette = Some(Palette::recent()),
            Command::CloseWorkspace => self.close_workspace(),
            Command::NewFile | Command::NewFolder if has_workspace => {
                let dir = self.workspace.as_ref().map(|w| w.explorer.target_dir(&w.root)).unwrap_or_default();
                super::explorer::prompt_create(self, dir, command == Command::NewFolder);
            }
            Command::Teleport => super::teleport::open(self),
            Command::Save => {
                if let Some(index) = self.active_tab {
                    self.save_tab(index);
                }
            }
            Command::SaveAll => self.save_all(),
            Command::CloseTab => {
                if let Some(index) = self.active_tab {
                    self.request_close_tab(index);
                }
            }
            Command::NextTab | Command::PrevTab => {
                if let Some(active) = self.active_tab
                    && !self.tabs.is_empty()
                {
                    let len = self.tabs.len();
                    let next = if command == Command::NextTab { (active + 1) % len } else { (active + len - 1) % len };
                    self.activate_tab(next);
                }
            }
            Command::ToggleSidebar => self.sidebar_open = !self.sidebar_open,
            Command::ShowExplorer => self.show_side(SideView::Explorer),
            Command::ShowSearch => {
                self.show_side(SideView::Search);
                let selection = self.active_doc().map(|d| d.buffer.selected_text()).filter(|s| !s.is_empty() && !s.contains('\n'));
                if let Some(workspace) = &mut self.workspace {
                    if let Some(selection) = selection {
                        workspace.search.query = selection;
                    }
                    workspace.search.focus = true;
                }
            }
            Command::ShowGit => {
                self.show_side(SideView::Git);
                self.git_refresh();
            }
            Command::ShowRun => self.show_side(SideView::Run),
            Command::TogglePanel => self.panel_open = !self.panel_open,
            Command::ToggleTerminal => {
                if self.panel_open && self.panel_tab == PanelTab::Terminal {
                    self.panel_open = false;
                    if let Some(doc) = self.active_doc() {
                        doc.view.focus_request = true;
                    }
                } else {
                    self.panel_open = true;
                    self.panel_tab = PanelTab::Terminal;
                    if self.terminals.is_empty() {
                        self.new_terminal(None);
                    }
                    self.terminal_focus = true;
                }
            }
            Command::NewTerminal => self.new_terminal(None),
            Command::KillTerminal => {
                if let Some(terminal) = self.terminals.get_mut(self.active_terminal)
                    && terminal.is_alive()
                {
                    terminal.write(b"\x03");
                    terminal.kill();
                }
            }
            Command::ShowProblems => {
                self.panel_open = true;
                self.panel_tab = PanelTab::Problems;
            }
            Command::ShowPulse => {
                self.panel_open = true;
                self.panel_tab = PanelTab::Pulse;
            }
            Command::ShowOutput => {
                self.panel_open = true;
                self.panel_tab = PanelTab::Output;
            }
            Command::RunDefault | Command::Build | Command::Test if has_workspace => {
                let group = match command {
                    Command::RunDefault => Group::Run,
                    Command::Build => Group::Build,
                    _ => Group::Test,
                };
                let task = self.workspace.as_ref().and_then(|w| crate::tasks::default_task(&w.tasks, group).cloned());
                match task {
                    Some(task) => self.run_task(task),
                    None => self.palette = Some(Palette::tasks()),
                }
            }
            Command::RunTask if has_workspace => self.palette = Some(Palette::tasks()),
            Command::RerunLast => match self.last_task.clone() {
                Some(task) => self.run_task(task),
                None => self.run_command(Command::RunDefault),
            },
            Command::EditTasks if has_workspace => super::run::edit_tasks_file(self),
            Command::Find | Command::Replace => {
                if let Some(doc) = self.active_doc() {
                    let selection = doc.buffer.selected_text();
                    doc.view.open_find(Some(selection), command == Command::Replace);
                }
            }
            Command::MarkdownPreview => {
                if let Some(doc) = self.active_doc()
                    && (doc.lang == crate::syntax::Lang::Markdown || super::media::kind(&doc.path).is_some())
                {
                    doc.preview = !doc.preview;
                    doc.view.focus_request = !doc.preview;
                }
            }
            Command::GotoDefinition => self.editor_action(EditorAction::GotoDefinition),
            Command::References => self.editor_action(EditorAction::References),
            Command::CodeActions => self.editor_action(EditorAction::CodeActions),
            Command::Format => self.editor_action(EditorAction::Format),
            Command::Rename => {
                if let Some(doc) = self.active_doc() {
                    let head = doc.buffer.primary().head;
                    let word = doc.buffer.word_at(head);
                    doc.view.rename = Some(super::editor::RenamePopup { text: doc.buffer.slice(word), focus: true });
                }
            }
            Command::RestartLsp => {
                self.lsp.reset(self.config.servers());
                let ids: Vec<usize> = (0..self.tabs.len()).collect();
                let workspace = self.workspace.as_ref();
                for index in ids {
                    if let Tab::Doc(doc) = &mut self.tabs[index] {
                        doc.diagnostics.clear();
                        doc.lsp_opened.clear();
                        self.lsp.document_opened(doc, workspace);
                    }
                }
                self.notify(t!("Restarting language servers"), false);
            }
            Command::GitCommit => super::scm::commit(self, false),
            Command::GitCommitAmend => super::scm::commit(self, true),
            Command::GitPush => super::scm::push(self),
            Command::GitPull => {
                let rebase = self.config.pull_rebase;
                self.git_op("pull", move |h, r| crate::git::pull(h, r, rebase));
            }
            Command::GitFetch => self.git_op("fetch", crate::git::fetch),
            Command::GitSync => {
                let rebase = self.config.pull_rebase;
                self.git_op("sync", move |h, r| {
                    let pulled = crate::git::pull(h, r, rebase)?;
                    let pushed = crate::git::push(h, r, None, false)?;
                    Ok(format!("{pulled}\n{pushed}"))
                });
            }
            Command::GitCheckout => {
                self.git_refresh();
                self.palette = Some(Palette::branches(false));
            }
            Command::GitMerge => self.palette = Some(Palette::branches(true)),
            Command::GitNewBranch => super::scm::prompt_new_branch(self),
            Command::GitStash => self.git_op("stash", |h, r| crate::git::stash_push(h, r, "", true)),
            Command::GitStashPop => self.git_op("stash pop", |h, r| crate::git::stash_action(h, r, "pop", "stash@{0}")),
            Command::GitLog => self.open_log(true),
            Command::GitStageFile => {
                let path = self.active_doc().map(|d| d.path.clone());
                if let Some(path) = path {
                    self.git_op("add", move |h, r| crate::git::stage(h, r, &[path]).map(|_| String::new()));
                }
            }
            Command::GitFileHistory => {
                let path = self.active_doc().map(|d| d.path.clone());
                if let (Some(path), Some(workspace)) = (path, &self.workspace)
                    && let Some(repo) = workspace.git.repo_root.clone()
                {
                    let relative = path.strip_prefix(&repo).unwrap_or(&path).trim_start_matches('/').to_string();
                    let host = workspace.host.clone();
                    self.tabs.push(Tab::Log(super::diffview::LogTab::new(false)));
                    self.active_tab = Some(self.tabs.len() - 1);
                    self.bus.spawn(move || super::Msg::GitLog(crate::git::log(&host, &repo, false, 500, Some(&relative))));
                }
            }
            Command::GitInit => {
                if let Some(workspace) = &self.workspace {
                    let host = workspace.host.clone();
                    let root = workspace.root.clone();
                    let bus = self.bus.clone();
                    std::thread::spawn(move || {
                        let result = crate::git::init(&host, &root);
                        bus.send(super::Msg::GitOpDone { label: "init".into(), result });
                        bus.send(super::Msg::GitRepo(crate::git::repo_root(&host, &root)));
                    });
                }
            }
            Command::GitAddRemote => super::scm::prompt_add_remote(self, None),
            Command::GitToggleBlame => {
                self.config.inline_blame = !self.config.inline_blame;
                self.config.save();
                let ids: Vec<u64> = self.tabs.iter().filter_map(|t| if let Tab::Doc(d) = t { Some(d.id) } else { None }).collect();
                for id in ids {
                    self.request_blame(id);
                }
            }
            Command::ZoomIn | Command::ZoomOut | Command::ZoomReset => {
                let delta = match command {
                    Command::ZoomIn => 1.0,
                    Command::ZoomOut => -1.0,
                    _ => 0.0,
                };
                if delta == 0.0 {
                    let defaults = crate::config::Config::default();
                    self.config.editor_font_size = defaults.editor_font_size;
                    self.config.terminal_font_size = defaults.terminal_font_size;
                } else {
                    self.config.editor_font_size = (self.config.editor_font_size + delta).clamp(8.0, 32.0);
                    self.config.terminal_font_size = (self.config.terminal_font_size + delta).clamp(8.0, 32.0);
                }
                self.config.save();
            }
            Command::Agent => super::agent::open(self),
            Command::AgentSendSelection => {
                if let Some(Tab::Doc(doc)) = self.active_tab.and_then(|i| self.tabs.get(i)) {
                    self.last_doc_id = Some(doc.id);
                }
                super::agent::insert_selection(self);
            }
            Command::Settings => self.settings_open = !self.settings_open,
            Command::About => self.about_open = true,
            Command::Quit => self.ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Close),
            _ => {}
        }
    }

    fn show_side(&mut self, view: SideView) {
        if self.sidebar_open && self.side_view == view && view == SideView::Explorer {
            return;
        }
        self.side_view = view;
        self.sidebar_open = true;
    }

    fn load_file_index_if_needed(&mut self) {
        if self.workspace.as_ref().is_some_and(|w| w.file_index.is_none()) {
            self.load_file_index();
        }
    }

    pub fn open_folder_dialog(&mut self) {
        match self.host() {
            Some(host) if host.is_remote() => {
                let root = self.workspace.as_ref().map(|w| w.root.clone()).unwrap_or_else(|| host.home());
                self.connect = Some(super::welcome::ConnectState::pick_folder(host, root, &self.bus));
            }
            _ => {
                let start = self.workspace.as_ref().map(|w| w.root.clone()).unwrap_or_else(|| crate::host::Host::Local.home());
                if let Some(path) = rfd::FileDialog::new().set_directory(&start).pick_folder() {
                    let path = path.canonicalize().unwrap_or(path);
                    self.open_workspace(crate::host::Host::Local, path.to_string_lossy().into_owned());
                }
            }
        }
    }

    /// Runs an LSP-backed action for the active document.
    pub fn editor_action(&mut self, action: EditorAction) {
        if let Some(index) = self.active_tab
            && let Some(Tab::Doc(_)) = self.tabs.get(index)
        {
            super::lsp_manager::run_actions(self, index, vec![action]);
        }
    }
}
