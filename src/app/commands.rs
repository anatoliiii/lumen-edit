//! All user-facing commands: palette entries, menu items and shortcuts.

use super::{App, PanelTab, SideView, Tab, palette::Palette};
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
    TerminalCopy,
    TerminalPaste,
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
    FindNext,
    FindPrevious,
    MarkdownPreview,
    GotoDefinition,
    References,
    Rename,
    CodeActions,
    Format,
    TriggerSuggest,
    ShowHover,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    SelectLine,
    AddNextOccurrence,
    AddCursorAbove,
    AddCursorBelow,
    MoveLinesUp,
    MoveLinesDown,
    CopyLinesUp,
    CopyLinesDown,
    DeleteLines,
    InsertLineBelow,
    InsertLineAbove,
    ToggleComment,
    Indent,
    Outdent,
    ScrollLineUp,
    ScrollLineDown,
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
    AgentSessions,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    Settings,
    Keybindings,
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
    Command::TerminalCopy,
    Command::TerminalPaste,
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
    Command::FindNext,
    Command::FindPrevious,
    Command::MarkdownPreview,
    Command::GotoDefinition,
    Command::References,
    Command::Rename,
    Command::CodeActions,
    Command::Format,
    Command::TriggerSuggest,
    Command::ShowHover,
    Command::Undo,
    Command::Redo,
    Command::Cut,
    Command::Copy,
    Command::Paste,
    Command::SelectAll,
    Command::SelectLine,
    Command::AddNextOccurrence,
    Command::AddCursorAbove,
    Command::AddCursorBelow,
    Command::MoveLinesUp,
    Command::MoveLinesDown,
    Command::CopyLinesUp,
    Command::CopyLinesDown,
    Command::DeleteLines,
    Command::InsertLineBelow,
    Command::InsertLineAbove,
    Command::ToggleComment,
    Command::Indent,
    Command::Outdent,
    Command::ScrollLineUp,
    Command::ScrollLineDown,
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
    Command::AgentSessions,
    Command::ZoomIn,
    Command::ZoomOut,
    Command::ZoomReset,
    Command::Settings,
    Command::Keybindings,
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
        crate::i18n::tr(self.label_key())
    }

    /// The English name, which is also the translation key.
    pub fn label_key(self) -> &'static str {
        match self {
            Command::QuickOpen => t_key!("Go to File…"),
            Command::CommandPalette => t_key!("Command Palette"),
            Command::GotoLine => t_key!("Go to Line…"),
            Command::OpenFolder => t_key!("Open Folder…"),
            Command::ConnectSsh => t_key!("Connect via SSH…"),
            Command::OpenRecent => t_key!("Open Recent…"),
            Command::CloseWorkspace => t_key!("Close Project"),
            Command::NewFile => t_key!("New File…"),
            Command::NewFolder => t_key!("New Folder…"),
            Command::Teleport => t_key!("File Teleport"),
            Command::Save => t_key!("Save"),
            Command::SaveAll => t_key!("Save All"),
            Command::CloseTab => t_key!("Close Tab"),
            Command::NextTab => t_key!("Next Tab"),
            Command::PrevTab => t_key!("Previous Tab"),
            Command::ToggleSidebar => t_key!("Toggle Sidebar"),
            Command::ShowExplorer => t_key!("Explorer"),
            Command::ShowSearch => t_key!("Search in Project"),
            Command::ShowGit => t_key!("Git: Changes"),
            Command::ShowRun => t_key!("Run and Tasks"),
            Command::TogglePanel => t_key!("Toggle Bottom Panel"),
            Command::ToggleTerminal => t_key!("Terminal: Toggle"),
            Command::NewTerminal => t_key!("Terminal: New"),
            Command::KillTerminal => t_key!("Terminal: Close Current"),
            Command::TerminalCopy => t_key!("Terminal: Copy Selection"),
            Command::TerminalPaste => t_key!("Terminal: Paste"),
            Command::ShowProblems => t_key!("Problems"),
            Command::ShowOutput => t_key!("Output"),
            Command::ShowPulse => t_key!("Server Pulse"),
            Command::RunDefault => t_key!("Run: Default Task"),
            Command::RunTask => t_key!("Run: Choose Task…"),
            Command::Build => t_key!("Run: Build"),
            Command::Test => t_key!("Run: Test"),
            Command::RerunLast => t_key!("Run: Rerun Last Task"),
            Command::EditTasks => t_key!("Run: Configure Tasks (tasks.toml)"),
            Command::Find => t_key!("Find"),
            Command::MarkdownPreview => t_key!("Markdown: Preview / Source"),
            Command::Replace => t_key!("Replace"),
            Command::FindNext => t_key!("Find Next"),
            Command::FindPrevious => t_key!("Find Previous"),
            Command::GotoDefinition => t_key!("Go to Definition"),
            Command::References => t_key!("Find All References"),
            Command::Rename => t_key!("Rename Symbol"),
            Command::CodeActions => t_key!("Quick Fixes…"),
            Command::Format => t_key!("Format Document"),
            Command::TriggerSuggest => t_key!("Trigger Suggestions"),
            Command::ShowHover => t_key!("Show Hover"),
            Command::Undo => t_key!("Undo"),
            Command::Redo => t_key!("Redo"),
            Command::Cut => t_key!("Cut"),
            Command::Copy => t_key!("Copy"),
            Command::Paste => t_key!("Paste"),
            Command::SelectAll => t_key!("Select All"),
            Command::SelectLine => t_key!("Select Line"),
            Command::AddNextOccurrence => t_key!("Add Selection to Next Match"),
            Command::AddCursorAbove => t_key!("Add Cursor Above"),
            Command::AddCursorBelow => t_key!("Add Cursor Below"),
            Command::MoveLinesUp => t_key!("Move Line Up"),
            Command::MoveLinesDown => t_key!("Move Line Down"),
            Command::CopyLinesUp => t_key!("Copy Line Up"),
            Command::CopyLinesDown => t_key!("Copy Line Down"),
            Command::DeleteLines => t_key!("Delete Line"),
            Command::InsertLineBelow => t_key!("Insert Line Below"),
            Command::InsertLineAbove => t_key!("Insert Line Above"),
            Command::ToggleComment => t_key!("Toggle Line Comment"),
            Command::Indent => t_key!("Indent Line"),
            Command::Outdent => t_key!("Outdent Line"),
            Command::ScrollLineUp => t_key!("Scroll Up"),
            Command::ScrollLineDown => t_key!("Scroll Down"),
            Command::RestartLsp => t_key!("Restart Language Servers"),
            Command::GitCommit => t_key!("Git: Commit"),
            Command::GitCommitAmend => t_key!("Git: Amend Last Commit"),
            Command::GitPush => "Git: push",
            Command::GitPull => "Git: pull",
            Command::GitFetch => "Git: fetch",
            Command::GitSync => t_key!("Git: Sync (pull + push)"),
            Command::GitCheckout => t_key!("Git: Switch Branch…"),
            Command::GitNewBranch => t_key!("Git: Create Branch…"),
            Command::GitMerge => t_key!("Git: Merge Branch into Current…"),
            Command::GitStash => t_key!("Git: Stash Changes"),
            Command::GitStashPop => t_key!("Git: Pop Stash"),
            Command::GitLog => t_key!("Git: History (graph)"),
            Command::GitStageFile => t_key!("Git: Stage Current File"),
            Command::GitFileHistory => t_key!("Git: Current File History"),
            Command::GitInit => t_key!("Git: Initialize Repository"),
            Command::GitAddRemote => t_key!("Git: Add Remote…"),
            Command::GitToggleBlame => t_key!("Git: Toggle Line Blame"),
            Command::Agent => t_key!("pi Agent"),
            Command::AgentSendSelection => t_key!("Agent: Send Selection"),
            Command::AgentSessions => t_key!("Agent: Manage Sessions"),
            Command::ZoomIn => t_key!("Zoom In"),
            Command::ZoomOut => t_key!("Zoom Out"),
            Command::ZoomReset => t_key!("Reset Zoom"),
            Command::Settings => t_key!("Settings"),
            Command::Keybindings => t_key!("Keyboard Shortcuts"),
            Command::About => t_key!("About"),
            Command::Quit => t_key!("Quit"),
        }
    }

    /// Where the command's shortcuts work.
    pub fn scope(self) -> Scope {
        match self {
            Command::Find
            | Command::Replace
            | Command::FindNext
            | Command::FindPrevious
            | Command::GotoDefinition
            | Command::References
            | Command::Rename
            | Command::CodeActions
            | Command::Format
            | Command::TriggerSuggest
            | Command::ShowHover
            | Command::Undo
            | Command::Redo
            | Command::Cut
            | Command::Copy
            | Command::Paste
            | Command::SelectAll
            | Command::SelectLine
            | Command::AddNextOccurrence
            | Command::AddCursorAbove
            | Command::AddCursorBelow
            | Command::MoveLinesUp
            | Command::MoveLinesDown
            | Command::CopyLinesUp
            | Command::CopyLinesDown
            | Command::DeleteLines
            | Command::InsertLineBelow
            | Command::InsertLineAbove
            | Command::ToggleComment
            | Command::Indent
            | Command::Outdent
            | Command::ScrollLineUp
            | Command::ScrollLineDown => Scope::Editor,
            Command::GitCommit => Scope::CommitMessage,
            Command::TerminalCopy | Command::TerminalPaste => Scope::Terminal,
            _ => Scope::Global,
        }
    }

    /// The first shortcut of the command, as shown in menus.
    pub fn shortcut_text(self) -> Option<String> {
        super::keymap::current().label(self)
    }
}

/// Where a shortcut is listened for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Anywhere in the window, before widgets see the keys.
    Global,
    /// In a focused code editor.
    Editor,
    /// In the commit message box.
    CommitMessage,
    /// In a focused terminal.
    Terminal,
}

impl Scope {
    pub fn label(self) -> &'static str {
        match self {
            Scope::Global => t!("everywhere"),
            Scope::Editor => t!("in the editor"),
            Scope::CommitMessage => t!("in the commit message"),
            Scope::Terminal => t!("in the terminal"),
        }
    }
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
                if self.agent_manager.open {
                    super::agent::manager::hide(self);
                    return;
                }
                if let Some(index) = self.active_tab {
                    self.request_close_tab(index);
                }
            }
            Command::NextTab | Command::PrevTab => {
                // The agent workspace is one visible tab, independent of process count.
                let pages: Vec<Option<usize>> = std::iter::once(None)
                    .chain(self.tabs.iter().enumerate().filter(|(_, tab)| !matches!(tab, Tab::Agent(_))).map(|(i, _)| Some(i)))
                    .collect();
                let current = if self.agent_manager.open { 0 } else { pages.iter().position(|page| *page == self.active_tab).unwrap_or(0) };
                let next =
                    if command == Command::NextTab { (current + 1) % pages.len() } else { (current + pages.len() - 1) % pages.len() };
                match pages[next] {
                    None => super::agent::manager::open(self),
                    Some(index) => self.activate_tab(index),
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
            Command::MarkdownPreview => {
                if let Some(doc) = self.active_doc()
                    && (doc.lang == crate::syntax::Lang::Markdown || super::media::kind(&doc.path).is_some())
                {
                    doc.preview = !doc.preview;
                    doc.view.focus_request = !doc.preview;
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
            Command::Agent => super::agent::manager::open(self),
            Command::AgentSessions => super::agent::manager::open(self),
            Command::AgentSendSelection => {
                if let Some(Tab::Doc(doc)) = self.active_tab.and_then(|i| self.tabs.get(i)) {
                    self.last_doc_id = Some(doc.id);
                }
                super::agent::insert_selection(self);
            }
            Command::Settings => self.settings_open = !self.settings_open,
            Command::Keybindings => super::keybindings::open(self),
            Command::TerminalCopy => {
                if let Some(terminal) = self.terminals.get_mut(self.active_terminal) {
                    terminal.copy_selection(&self.ctx);
                }
            }
            Command::TerminalPaste if !self.terminals.is_empty() => {
                // The paste arrives next frame in the focused terminal.
                self.panel_open = true;
                self.panel_tab = PanelTab::Terminal;
                self.terminal_focus = true;
                // E.g. the palette, on closing, gives the focus back to the editor.
                if let Some(doc) = self.active_doc() {
                    doc.view.focus_request = false;
                }
                super::keymap::request_paste(&self.ctx);
            }
            Command::About => self.about_open = true,
            Command::Quit => self.ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Close),
            _ if command.scope() == Scope::Editor => self.run_editor_command(command),
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
                    self.open_workspace(crate::host::Host::Local, crate::platform::host_path(&path));
                }
            }
        }
    }

    /// Runs an editor command (from the palette, a menu or the CLI) on the
    /// active document.
    fn run_editor_command(&mut self, command: Command) {
        let Some(index) = self.active_tab else { return };
        let Some(Tab::Doc(doc)) = self.tabs.get_mut(index) else { return };
        let has_lsp = self.lsp.has_clients(doc);
        let mut actions = Vec::new();
        super::editor::run_command(&self.ctx, doc, command, &mut actions, has_lsp);
        if !actions.is_empty() {
            super::lsp_manager::run_actions(self, index, actions);
        }
    }
}
