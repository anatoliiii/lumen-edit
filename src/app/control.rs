//! Executes control requests (CLI / MCP) against the live editor state.

use std::{
    io::Read,
    process::Stdio,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

use super::{App, Command, Msg, Tab, commands, search};
use crate::{
    control::{ControlRequest, Reply},
    git,
    host::{self, Host},
    lsp,
    text::Selection,
};

/// Runs once a file is loaded into a tab (`Ok(tab index)`), or failed to load.
pub type DocAction = Box<dyn FnOnce(&mut App, Result<usize, String>) + Send>;

fn str_param<'a>(params: &'a Value, name: &str) -> Option<&'a str> {
    params.get(name).and_then(Value::as_str)
}

fn req_str(params: &Value, name: &str) -> Result<String, String> {
    str_param(params, name).map(str::to_string).ok_or_else(|| t!("missing parameter `{name}`", name = name))
}

fn int_param(params: &Value, name: &str) -> Option<i64> {
    params.get(name).and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
}

fn bool_param(params: &Value, name: &str, default: bool) -> bool {
    params.get(name).and_then(Value::as_bool).unwrap_or(default)
}

/// 1-based (line, column) -> 0-based.
fn position(params: &Value, line: &str, column: &str) -> (usize, usize) {
    let line = int_param(params, line).unwrap_or(1).max(1) as usize - 1;
    let column = int_param(params, column).unwrap_or(1).max(1) as usize - 1;
    (line, column)
}

impl App {
    fn resolve(&self, path: &str) -> String {
        let root = self.workspace.as_ref().map(|w| w.root.clone());
        if host::is_absolute(path) {
            return path.to_string();
        }
        if let Some(rest) = path.strip_prefix("~/") {
            let home = self.host().map(|h| h.home()).unwrap_or_else(|| Host::Local.home());
            return host::join(&home, rest);
        }
        match root {
            Some(root) => host::join(&root, path.trim_start_matches("./")),
            None => std::env::current_dir().map(|d| crate::platform::host_path(&d.join(path))).unwrap_or_else(|_| path.to_string()),
        }
    }

    fn path_or_active(&mut self, params: &Value) -> Result<String, String> {
        match str_param(params, "path") {
            Some(path) => Ok(self.resolve(path)),
            None => self.active_doc().map(|d| d.path.clone()).ok_or_else(|| t!("no active file, pass `path`").into()),
        }
    }

    fn relative_path(&self, path: &str) -> String {
        match &self.workspace {
            Some(workspace) => workspace.relative(path).to_string(),
            None => path.to_string(),
        }
    }

    /// Runs `action` with the tab index of `path`, opening the file first.
    pub(super) fn with_doc(&mut self, path: String, action: DocAction) {
        if let Some(index) = self.tabs.iter().position(|t| matches!(t, Tab::Doc(d) if d.path == path)) {
            action(self, Ok(index));
            return;
        }
        if self.workspace.is_none() {
            action(self, Err(t!("no project is open").into()));
            return;
        }
        self.doc_waiters.push((path.clone(), action));
        self.open_file(path, None);
    }

    pub(super) fn run_doc_waiters(&mut self, path: &str, result: Result<usize, String>) {
        let mut ready = Vec::new();
        let mut index = 0;
        while index < self.doc_waiters.len() {
            if self.doc_waiters[index].0 == path {
                ready.push(self.doc_waiters.remove(index).1);
            } else {
                index += 1;
            }
        }
        for action in ready {
            // Tab indices may shift between actions; look the path up again.
            let current = match &result {
                Ok(_) => {
                    self.tabs.iter().position(|t| matches!(t, Tab::Doc(d) if d.path == path)).ok_or_else(|| t!("file closed").to_string())
                }
                Err(e) => Err(e.clone()),
            };
            action(self, current);
        }
    }

    fn save_blocking(&mut self, index: usize) -> Result<(), String> {
        let host = self.host().ok_or(t!("no project is open"))?;
        let Some(Tab::Doc(doc)) = self.tabs.get_mut(index) else { return Err(t!("not a file").into()) };
        let revision = doc.buffer.revision();
        host.write_file(&doc.path, doc.buffer.text().as_bytes())?;
        doc.buffer.mark_saved_at(revision);
        let doc_id = doc.id;
        self.lsp.document_saved(doc);
        let _ = doc_id;
        self.git_refresh();
        Ok(())
    }

    fn require_workspace(&self) -> Result<(Host, String), String> {
        self.workspace.as_ref().map(|w| (w.host.clone(), w.root.clone())).ok_or_else(|| t!("no project is open").into())
    }

    fn repo(&self) -> Result<(Host, String), String> {
        let workspace = self.workspace.as_ref().ok_or(t!("no project is open"))?;
        let repo = workspace.git.repo_root.clone().ok_or(t!("the project is not in a git repository"))?;
        Ok((workspace.host.clone(), repo))
    }

    /// Runs `job` on a worker thread and replies with its result.
    fn reply_async(&self, reply: Reply, job: impl FnOnce() -> Result<Value, String> + Send + 'static) {
        std::thread::spawn(move || {
            let _ = reply.send(job());
        });
    }

    pub(super) fn handle_control(&mut self, request: ControlRequest) {
        let ControlRequest { method, params, reply } = request;
        crate::debug_log!("control: {method} {params}");
        let result = self.dispatch(&method, &params, reply.clone());
        // `None` means the reply is sent later.
        if let Some(result) = result {
            let _ = reply.send(result);
        }
    }

    fn dispatch(&mut self, method: &str, params: &Value, reply: Reply) -> Option<Result<Value, String>> {
        let done = |value: Value| Some(Ok(value));
        let fail = |error: String| Some(Err(error));
        match method {
            "state" => done(self.state_json()),
            "open_files" => done(json!(self.open_files().iter().map(|f| json!({
                "path": f.path,
                "name": f.name,
                "dirty": f.dirty,
                "lines": f.lines,
            })).collect::<Vec<_>>())),
            "methods" => done(json!(crate::control::methods::METHODS.iter().map(|m| m.name).collect::<Vec<_>>())),
            "commands" => done(json!(
                commands::ALL.iter().map(|c| json!({ "id": c.id(), "label": c.label(), "shortcut": c.shortcut_text() })).collect::<Vec<_>>()
            )),
            "run_command" => {
                let id = str_param(params, "command").unwrap_or("");
                match Command::from_id(id) {
                    Some(command) => {
                        self.run_command(command);
                        done(json!(t!("done: {n1}", n1 = command.label())))
                    }
                    None => fail(t!("unknown command `{id}`; see the commands method", id = id)),
                }
            }
            "open_folder" => {
                let path = match req_str(params, "path") {
                    Ok(p) => p,
                    Err(e) => return fail(e),
                };
                let host_param = str_param(params, "host").map(str::to_string).unwrap_or_else(|| self.host().map(|h| h.key()).unwrap_or_else(|| "local".into()));
                if host_param == "local" {
                    let path = std::fs::canonicalize(&path).map(|p| crate::platform::host_path(&p)).unwrap_or(path);
                    self.open_workspace(Host::Local, path.clone());
                    return done(json!(t!("opened {path}", path = path)));
                }
                let target = host_param.trim_start_matches("ssh:").to_string();
                super::welcome::open_recent(self, crate::config::RecentProject { host: format!("ssh:{target}"), path: path.clone() });
                done(json!(t!("opening {target}:{path}", target = target, path = path)))
            }
            "connect_ssh" => {
                let target = match req_str(params, "target") {
                    Ok(t) => t,
                    Err(e) => return fail(e),
                };
                self.start_connect(target.clone(), str_param(params, "path").map(str::to_string));
                done(json!(t!("connecting to {target}; if a password is needed, enter it in the editor window. Check `state`.", target = target)))
            }
            "close_workspace" => {
                self.close_workspace();
                done(json!("ok"))
            }
            "open_file" => {
                let path = match req_str(params, "path") {
                    Ok(p) => self.resolve(&p),
                    Err(e) => return fail(e),
                };
                // Images open in a viewer tab, not as text.
                if super::media::kind(&path) == Some(super::media::Media::Raster) {
                    self.open_file(path.clone(), None);
                    return done(json!({ "path": path, "viewer": "image" }));
                }
                let goto = params.get("line").map(|_| position(params, "line", "column"));
                self.with_doc(
                    path,
                    Box::new(move |app, result| {
                        let _ = reply.send(result.map(|index| {
                            app.activate_tab(index);
                            if let (Some(goto), Some(Tab::Doc(doc))) = (goto, app.tabs.get_mut(index)) {
                                super::goto_position(doc, goto);
                            }
                            let Some(Tab::Doc(doc)) = app.tabs.get(index) else { return Value::Null };
                            json!({ "path": doc.path, "lines": doc.buffer.line_count(), "language": doc.lang.display_name() })
                        }));
                    }),
                );
                None
            }
            "close_file" => {
                let path = match self.path_or_active(params) {
                    Ok(p) => p,
                    Err(e) => return fail(e),
                };
                let Some(index) = self.tabs.iter().position(|t| matches!(t, Tab::Doc(d) if d.path == path)) else {
                    return fail(t!("file is not open").into());
                };
                if self.tabs[index].dirty() && !bool_param(params, "force", false) {
                    return fail(t!("there are unsaved changes: save first or pass force=true").into());
                }
                self.close_tab(index);
                done(json!(t!("closed")))
            }
            "save" => {
                if bool_param(params, "all", false) {
                    let mut saved = Vec::new();
                    for index in 0..self.tabs.len() {
                        if self.tabs[index].dirty() {
                            if let Err(e) = self.save_blocking(index) {
                                return fail(e);
                            }
                            saved.push(self.tabs[index].title());
                        }
                    }
                    return done(json!({ "saved": saved }));
                }
                let path = match self.path_or_active(params) {
                    Ok(p) => p,
                    Err(e) => return fail(e),
                };
                match self.tabs.iter().position(|t| matches!(t, Tab::Doc(d) if d.path == path)) {
                    Some(index) => Some(self.save_blocking(index).map(|_| json!(t!("saved {n1}", n1 = self.relative_path(&path))))),
                    None => fail(t!("file is not open").into()),
                }
            }
            "read_file" => {
                let path = match req_str(params, "path") {
                    Ok(p) => self.resolve(&p),
                    Err(e) => return fail(e),
                };
                let start = int_param(params, "start_line").map(|l| l.max(1) as usize);
                let end = int_param(params, "end_line").map(|l| l.max(1) as usize);
                let numbers = bool_param(params, "line_numbers", false);
                let format = move |text: &str| -> Value {
                    let total = text.lines().count();
                    let from = start.unwrap_or(1);
                    let to = end.unwrap_or(total).min(total);
                    let mut out = String::new();
                    for (i, line) in text.lines().enumerate().skip(from - 1).take(to.saturating_sub(from - 1)) {
                        if numbers {
                            out.push_str(&format!("{:>5}\t", i + 1));
                        }
                        out.push_str(line);
                        out.push('\n');
                    }
                    if start.is_none() && end.is_none() && !numbers {
                        return json!(text);
                    }
                    json!(out)
                };
                if let Some(doc) = self.doc_by_path(&path) {
                    return done(format(&doc.buffer.text()));
                }
                let host = match self.host() {
                    Some(h) => h,
                    None => Host::Local,
                };
                self.reply_async(reply, move || {
                    let data = host.read_file(&path)?;
                    Ok(format(&String::from_utf8_lossy(&data)))
                });
                None
            }
            "write_file" | "create_file" => {
                let path = match req_str(params, "path") {
                    Ok(p) => self.resolve(&p),
                    Err(e) => return fail(e),
                };
                let text = str_param(params, "text").unwrap_or("").to_string();
                let save = bool_param(params, "save", method == "create_file");
                let Ok((host, _)) = self.require_workspace() else { return fail(t!("no project is open").into()) };
                let bus = self.bus.clone();
                let open_now = self.doc_by_path(&path).is_some();
                let apply: DocAction = Box::new(move |app, result| {
                    let outcome = result.and_then(|index| {
                        if let Some(Tab::Doc(doc)) = app.tabs.get_mut(index) {
                            doc.buffer.replace_all(&text, false);
                        }
                        app.activate_tab(index);
                        if save { app.save_blocking(index) } else { Ok(()) }
                    });
                    let _ = reply.send(outcome.map(|_| json!(if save { t!("written and saved") } else { t!("written to the buffer (not saved)") })));
                });
                if open_now {
                    self.with_doc(path, apply);
                } else {
                    // Make sure the file exists so it can be opened.
                    std::thread::spawn(move || {
                        let missing = host.stat(&path).is_err();
                        let created = if missing { host.create_dir(&host::parent(&path)).and_then(|_| host.write_file(&path, b"")) } else { Ok(()) };
                        bus.send(Msg::Run(Box::new(move |app: &mut App| match created {
                            Ok(()) => {
                                if missing && let Some(workspace) = &app.workspace {
                                    let dir = host::parent(&path);
                                    if workspace.explorer.is_loaded(&dir) {
                                        app.load_dir(dir);
                                    }
                                }
                                app.with_doc(path, apply);
                            }
                            Err(e) => apply(app, Err(e)),
                        })));
                    });
                }
                None
            }
            "create_folder" | "delete_path" => {
                let path = match req_str(params, "path") {
                    Ok(p) => self.resolve(&p),
                    Err(e) => return fail(e),
                };
                let Ok((host, _)) = self.require_workspace() else { return fail(t!("no project is open").into()) };
                let delete = method == "delete_path";
                if delete {
                    let prefix = format!("{path}/");
                    let closing: Vec<usize> =
                        self.tabs.iter().enumerate().filter(|(_, t)| matches!(t, Tab::Doc(d) if d.path == path || d.path.starts_with(&prefix))).map(|(i, _)| i).rev().collect();
                    for index in closing {
                        self.close_tab(index);
                    }
                }
                let bus = self.bus.clone();
                self.reply_async(reply, move || {
                    if delete { host.remove(&path)? } else { host.create_dir(&path)? }
                    bus.send(Msg::FsOpDone { reload: vec![host::parent(&path)], open: None, renamed: None, error: None });
                    Ok(json!("ok"))
                });
                None
            }
            "rename_path" => {
                let (Ok(from), Ok(to)) = (req_str(params, "from"), req_str(params, "to")) else { return fail(t!("`from` and `to` are required").into()) };
                let (from, to) = (self.resolve(&from), self.resolve(&to));
                let Ok((host, _)) = self.require_workspace() else { return fail(t!("no project is open").into()) };
                let bus = self.bus.clone();
                self.reply_async(reply, move || {
                    host.rename(&from, &to)?;
                    bus.send(Msg::FsOpDone { reload: vec![host::parent(&from), host::parent(&to)], open: None, renamed: Some((from, to)), error: None });
                    Ok(json!("ok"))
                });
                None
            }
            "edit" | "replace_text" => {
                let path = match req_str(params, "path") {
                    Ok(p) => self.resolve(&p),
                    Err(e) => return fail(e),
                };
                let params = params.clone();
                let is_edit = method == "edit";
                self.with_doc(
                    path,
                    Box::new(move |app, result| {
                        let outcome = result.and_then(|index| {
                            let Some(Tab::Doc(doc)) = app.tabs.get_mut(index) else { return Err(t!("not a file").to_string()) };
                            let count = if is_edit { apply_edits(doc, &params)? } else { apply_replace(doc, &params)? };
                            doc.view.reveal = true;
                            app.activate_tab(index);
                            if bool_param(&params, "save", false) {
                                app.save_blocking(index)?;
                            }
                            Ok(json!({ "changes": count, "saved": bool_param(&params, "save", false) }))
                        });
                        let _ = reply.send(outcome);
                    }),
                );
                None
            }
            "insert_text" => {
                let text = str_param(params, "text").unwrap_or("").to_string();
                match self.active_doc() {
                    Some(doc) => {
                        doc.buffer.paste(&text);
                        doc.view.reveal = true;
                        done(json!("ok"))
                    }
                    None => fail(t!("no active file").into()),
                }
            }
            "set_cursor" => {
                let path = match self.path_or_active(params) {
                    Ok(p) => p,
                    Err(e) => return fail(e),
                };
                let params = params.clone();
                self.with_doc(
                    path,
                    Box::new(move |app, result| {
                        let outcome = result.map(|index| {
                            app.activate_tab(index);
                            if let Some(Tab::Doc(doc)) = app.tabs.get_mut(index) {
                                let (line, col) = position(&params, "line", "column");
                                let anchor = doc.buffer.char_at(line, col);
                                let head = if params.get("end_line").is_some() {
                                    let (end_line, end_col) = position(&params, "end_line", "end_column");
                                    doc.buffer.char_at(end_line, end_col)
                                } else {
                                    anchor
                                };
                                doc.buffer.set_selections(vec![Selection { anchor, head }]);
                                doc.view.reveal = true;
                                doc.view.reveal_center = true;
                                doc.view.focus_request = true;
                            }
                            json!("ok")
                        });
                        let _ = reply.send(outcome);
                    }),
                );
                None
            }
            "list_dir" => {
                let Ok((host, root)) = self.require_workspace() else { return fail(t!("no project is open").into()) };
                let path = str_param(params, "path").map(|p| self.resolve(p)).unwrap_or(root);
                self.reply_async(reply, move || {
                    let entries = host.read_dir(&path)?;
                    Ok(json!(entries.iter().map(|e| if e.is_dir { format!("{}/", e.name) } else { e.name.clone() }).collect::<Vec<_>>()))
                });
                None
            }
            "list_files" => {
                let Ok((host, root)) = self.require_workspace() else { return fail(t!("no project is open").into()) };
                let query = str_param(params, "query").unwrap_or("").to_string();
                let limit = int_param(params, "limit").unwrap_or(200).max(1) as usize;
                self.reply_async(reply, move || {
                    let files = host.list_files(&root, 200_000)?;
                    let mut fuzzy = super::util::Fuzzy::default();
                    let mut scored: Vec<(u32, String)> = files.into_iter().filter_map(|f| fuzzy.score(&f, &query).map(|(s, _)| (s, f))).collect();
                    if !query.is_empty() {
                        scored.sort_by_key(|s| std::cmp::Reverse(s.0));
                    }
                    let total = scored.len();
                    let files: Vec<String> = scored.into_iter().take(limit).map(|(_, f)| f).collect();
                    Ok(json!({ "total": total, "files": files }))
                });
                None
            }
            "search" => {
                let Ok((host, root)) = self.require_workspace() else { return fail(t!("no project is open").into()) };
                let query = lumen_shared::fsops::SearchQuery {
                    pattern: str_param(params, "query").unwrap_or("").to_string(),
                    regex: bool_param(params, "regex", false),
                    case_sensitive: bool_param(params, "case_sensitive", false),
                    whole_word: bool_param(params, "whole_word", false),
                    max_results: int_param(params, "limit").unwrap_or(500).max(1) as usize,
                };
                if query.pattern.is_empty() {
                    return fail(t!("empty query").into());
                }
                let include: Vec<String> = str_param(params, "include").unwrap_or("").split(',').map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect();
                if bool_param(params, "show", false)
                    && let Some(workspace) = &mut self.workspace
                {
                    workspace.search.query = query.pattern.clone();
                    workspace.search.regex = query.regex;
                    workspace.search.case_sensitive = query.case_sensitive;
                    workspace.search.whole_word = query.whole_word;
                    workspace.search.include = include.join(", ");
                    self.side_view = super::SideView::Search;
                    self.sidebar_open = true;
                    search::start(self);
                }
                self.reply_async(reply, move || {
                    let (matches, truncated) = host.search(&root, &query)?;
                    let matches: Vec<Value> = matches
                        .into_iter()
                        .filter(|m| include.is_empty() || include.iter().any(|p| m.path.to_lowercase().contains(p.trim_start_matches('*'))))
                        .map(|m| json!({ "path": m.path, "line": m.line + 1, "column": m.text[..(m.start as usize).min(m.text.len())].chars().count() + 1, "text": m.text }))
                        .collect();
                    Ok(json!({ "count": matches.len(), "truncated": truncated, "matches": matches }))
                });
                None
            }
            "diagnostics" => {
                let only = str_param(params, "path").map(|p| self.resolve(p));
                done(self.diagnostics_json(only.as_deref()))
            }
            "hover" | "definition" | "references" | "completion" | "lsp" | "format" => {
                self.control_lsp(method, params.clone(), reply);
                None
            }
            "exec" => {
                let Ok((host, root)) = self.require_workspace() else { return fail(t!("no project is open").into()) };
                let command = match req_str(params, "command") {
                    Ok(c) => c,
                    Err(e) => return fail(e),
                };
                let cwd = str_param(params, "cwd").map(|c| self.resolve(c)).unwrap_or(root);
                let timeout = Duration::from_millis(int_param(params, "timeout_ms").unwrap_or(120_000).max(100) as u64);
                self.reply_async(reply, move || exec(&host, &cwd, &command, timeout));
                None
            }
            "terminal_list" => done(json!(
                self.terminals
                    .iter()
                    .enumerate()
                    .map(|(i, t)| json!({ "terminal": i, "title": t.title, "alive": t.is_alive(), "exit_code": *t.exit_code.lock().unwrap(), "active": i == self.active_terminal }))
                    .collect::<Vec<_>>()
            )),
            "terminal_new" => {
                let Ok((host, root)) = self.require_workspace() else { return fail(t!("no project is open").into()) };
                let cwd = str_param(params, "cwd").map(|c| self.resolve(c)).unwrap_or(root);
                let title = str_param(params, "title").map(str::to_string);
                match str_param(params, "command") {
                    Some(command) => {
                        let task = crate::tasks::Task {
                            label: title.unwrap_or_else(|| command.chars().take(30).collect()),
                            command: command.to_string(),
                            cwd: Some(cwd),
                            group: crate::tasks::Group::Other,
                            default: false,
                            source: "cli".into(),
                        };
                        self.new_terminal(Some(&task));
                    }
                    None => {
                        let title = title.unwrap_or_else(|| host::file_name(&cwd).to_string());
                        self.spawn_terminal_in(host, cwd, title);
                    }
                }
                done(json!({ "terminal": self.terminals.len().saturating_sub(1) }))
            }
            "terminal_write" | "terminal_read" | "terminal_kill" => {
                let index = int_param(params, "terminal").map(|i| i as usize).unwrap_or(self.active_terminal);
                if index >= self.terminals.len() {
                    return fail(t!("no terminal {index}", index = index));
                }
                match method {
                    "terminal_write" => {
                        let mut text = str_param(params, "text").unwrap_or("").replace('\n', "\r");
                        if bool_param(params, "enter", true) {
                            text.push('\r');
                        }
                        self.terminals[index].write(text.as_bytes());
                        done(json!("ok"))
                    }
                    "terminal_kill" => {
                        self.terminals[index].kill();
                        if bool_param(params, "close", false) {
                            self.terminals.remove(index);
                            self.active_terminal = self.active_terminal.min(self.terminals.len().saturating_sub(1));
                        }
                        done(json!("ok"))
                    }
                    _ => {
                        let wait = int_param(params, "wait_ms").unwrap_or(0).max(0) as u64;
                        if wait == 0 {
                            return done(json!(self.terminals[index].screen_text()));
                        }
                        let bus = self.bus.clone();
                        std::thread::spawn(move || {
                            std::thread::sleep(Duration::from_millis(wait.min(60_000)));
                            bus.send(Msg::Run(Box::new(move |app: &mut App| {
                                let text = app.terminals.get(index).map(|t| t.screen_text()).ok_or_else(|| t!("terminal closed").to_string());
                                let _ = reply.send(text.map(Value::String));
                            })));
                        });
                        None
                    }
                }
            }
            "tasks" => {
                let tasks = self.workspace.as_ref().map(|w| w.tasks.clone()).unwrap_or_default();
                done(json!(tasks.iter().map(|t| json!({ "label": t.label, "command": t.command, "group": format!("{:?}", t.group).to_lowercase(), "source": t.source })).collect::<Vec<_>>()))
            }
            "run_task" => {
                let label = str_param(params, "label").unwrap_or("");
                let task = self.workspace.as_ref().and_then(|w| w.tasks.iter().find(|t| t.label == label).cloned());
                match task {
                    Some(task) => {
                        self.run_task(task);
                        done(json!({ "terminal": self.terminals.len().saturating_sub(1), "hint": t!("output — terminal_read") }))
                    }
                    None => fail(t!("no task `{label}`; see the tasks method", label = label)),
                }
            }
            "git_status" => {
                let (host, repo) = match self.repo() {
                    Ok(r) => r,
                    Err(e) => return fail(e),
                };
                self.reply_async(reply, move || {
                    let status = git::status(&host, &repo)?;
                    Ok(json!({
                        "root": status.root,
                        "branch": status.branch,
                        "upstream": status.upstream,
                        "ahead": status.ahead,
                        "behind": status.behind,
                        "state": format!("{:?}", status.state).to_lowercase(),
                        "files": status.files.iter().map(|f| json!({
                            "path": f.path,
                            "index": f.index.to_string(),
                            "worktree": f.worktree.to_string(),
                            "staged": f.staged(),
                            "conflict": f.conflict,
                            "untracked": f.untracked,
                        })).collect::<Vec<_>>(),
                    }))
                });
                None
            }
            "git" | "git_commit" | "git_diff" | "git_log" => {
                let (host, repo) = match self.repo() {
                    Ok(r) => r,
                    Err(e) => return fail(e),
                };
                let bus = self.bus.clone();
                let params = params.clone();
                let method = method.to_string();
                let relative = |p: &str| p.strip_prefix(&format!("{repo}/")).unwrap_or(p).to_string();
                let path = str_param(&params, "path").map(|p| relative(&self.resolve(p)));
                self.reply_async(reply, move || {
                    let result = match method.as_str() {
                        "git" => {
                            let args: Vec<String> = params.get("args").and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
                            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
                            git::git_verbose(&host, &repo, &refs).map(Value::String)
                        }
                        "git_commit" => {
                            let message = str_param(&params, "message").unwrap_or("").to_string();
                            if message.trim().is_empty() {
                                return Err(t!("empty message").into());
                            }
                            let status = git::status(&host, &repo)?;
                            if bool_param(&params, "all", false) || !status.files.iter().any(|f| f.staged()) {
                                git::stage_all(&host, &repo)?;
                            }
                            git::commit(&host, &repo, &message, bool_param(&params, "amend", false), false).map(Value::String)
                        }
                        "git_diff" => {
                            let mut args = vec!["diff"];
                            if bool_param(&params, "staged", false) {
                                args.push("--cached");
                            }
                            if let Some(path) = &path {
                                args.extend(["--", path.as_str()]);
                            }
                            git::git(&host, &repo, &args).map(Value::String)
                        }
                        _ => {
                            let limit = int_param(&params, "limit").unwrap_or(30).max(1) as usize;
                            git::log(&host, &repo, false, limit, path.as_deref()).map(|entries| {
                                json!(entries
                                    .iter()
                                    .filter(|e| !e.hash.is_empty())
                                    .map(|e| json!({ "hash": e.short, "subject": e.subject, "author": e.author, "date": e.date, "refs": e.refs }))
                                    .collect::<Vec<_>>())
                            })
                        }
                    };
                    bus.send(Msg::Run(Box::new(|app: &mut App| {
                        app.git_refresh();
                        app.reload_unmodified_docs();
                    })));
                    result
                });
                None
            }
            "screenshot" => {
                let path = str_param(params, "path")
                    .map(|p| self.resolve(p))
                    .unwrap_or_else(|| crate::control::socket_dir().join(format!("screenshot-{}.png", std::process::id())).to_string_lossy().into_owned());
                self.ctx.request_screenshot(move |image| {
                    let _ = reply.send(save_screenshot(&path, &image));
                });
                self.ctx.request_repaint();
                None
            }
            "agent" => match super::agent::manager::control(self, params) {
                Ok(value) => done(value), Err(error) => fail(error),
            },
            "pulse" => done(self.pulse.json()),
            "transfer" => {
                let hosts = super::teleport::known_hosts(self);
                let pick = |key: Option<&str>, app: &App| -> Result<Host, String> {
                    match key {
                        None => app.host().ok_or_else(|| t!("no project is open").to_string()),
                        Some("local") => Ok(Host::Local),
                        Some(key) => {
                            let key = if key.starts_with("ssh:") { key.to_string() } else { format!("ssh:{key}") };
                            hosts.iter().find(|h| h.key() == key).cloned().ok_or_else(|| t!("{key} is not connected (connect_ssh)", key = key))
                        }
                    }
                };
                let (src_host, dst_host) = match (pick(str_param(params, "src_host"), self), pick(str_param(params, "dst_host"), self)) {
                    (Ok(a), Ok(b)) => (a, b),
                    (Err(e), _) | (_, Err(e)) => return fail(e),
                };
                let project = self.host();
                // Relative paths resolve against the project root on the project host.
                let resolve = |path: &str, host: &Host, app: &App| -> String {
                    if host::is_absolute(path) || project.as_ref().is_none_or(|p| !p.same(host)) { path.to_string() } else { app.resolve(path) }
                };
                let src = resolve(str_param(params, "src").unwrap_or("").trim_end_matches('/'), &src_host, self);
                let dst_dir = resolve(str_param(params, "dst_dir").unwrap_or(""), &dst_host, self);
                if src.is_empty() || dst_dir.is_empty() {
                    return fail(t!("`src` and `dst_dir` are required").into());
                }
                if !self.tabs.iter().any(|t| matches!(t, Tab::Teleport(_))) {
                    let active = self.active_tab;
                    super::teleport::open(self);
                    self.active_tab = active.or(self.active_tab);
                }
                let tab_id = self.tabs.iter().find_map(|t| if let Tab::Teleport(t) = t { Some(t.id) } else { None }).unwrap_or(0);
                let job = super::teleport::Job {
                    src: src_host,
                    src_dir: host::parent(&src),
                    names: vec![host::file_name(&src).to_string()],
                    dst: dst_host,
                    dst_dir: dst_dir.clone(),
                    move_after: bool_param(params, "move", false),
                };
                super::teleport::start_transfer(self, tab_id, job);
                done(json!(t!("transfer started: {src} → {dst_dir}; progress is in the Teleport tab", src = src, dst_dir = dst_dir)))
            }
            "notify" => {
                self.notify(str_param(params, "text").unwrap_or(""), bool_param(params, "error", false));
                done(json!("ok"))
            }
            "quit" => {
                self.run_command(Command::Quit);
                done(json!("ok"))
            }
            _ => fail(t!("unknown method `{method}`; see lumen-edit methods", method = method)),
        }
    }

    fn state_json(&mut self) -> Value {
        let active_index = self.active_tab;
        let tabs: Vec<Value> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(i, t)| match t {
                Tab::Doc(doc) => json!({ "path": self.relative_path(&doc.path), "dirty": doc.is_dirty(), "language": doc.lang.display_name(), "active": Some(i) == active_index }),
                other => json!({ "title": other.title(), "active": Some(i) == active_index }),
            })
            .collect();
        let active = self.active_tab.and_then(|i| match self.tabs.get(i) {
            Some(Tab::Doc(doc)) => {
                let primary = doc.buffer.primary();
                let (line, col) = doc.buffer.line_col(primary.head);
                let selected = doc.buffer.slice(primary.range());
                Some(json!({
                    "path": doc.path,
                    "line": line + 1,
                    "column": col + 1,
                    "selection": if selected.is_empty() { Value::Null } else { json!(selected.chars().take(2000).collect::<String>()) },
                    "cursors": doc.buffer.selections.len(),
                    "inline_completion_visible": doc.view.ghost.as_ref().is_some_and(|g| g.valid(doc.id, &doc.buffer)),
                    "lines": doc.buffer.line_count(),
                }))
            }
            _ => None,
        });
        let (errors, warnings) = super::panel::status_counts(self);
        let workspace = self.workspace.as_ref().map(|w| {
            let git = w.git.status.as_ref().map(|s| {
                json!({ "branch": s.branch, "upstream": s.upstream, "ahead": s.ahead, "behind": s.behind, "changes": s.files.len(), "state": format!("{:?}", s.state).to_lowercase() })
            });
            json!({ "host": w.host.key(), "root": w.root, "name": w.name(), "git": git, "tasks": w.tasks.len() })
        });
        let m = &self.inline_completion.metrics;
        let (latency_p50_ms, latency_p95_ms) = m.latency_percentiles();
        let inline_completion = json!({"requests":m.requests,"success":m.success,"errors":m.errors,"accepted":m.accepted,"rejected":m.rejected,
            "cache_hits":m.cache_hits,"latency_ms":m.latency_ms,"debounce_ms":m.debounce_ms,"context_prepare_ms":m.context_prepare_ms,
            "queue_ms":m.queue_ms,"http_request_ms":m.http_request_ms,"total_latency_ms":m.total_latency_ms,"display_ms":m.display_ms,"latency_p50_ms":latency_p50_ms,"latency_p95_ms":latency_p95_ms,
            "input_tokens":m.input_tokens,"output_tokens":m.output_tokens,"cached_tokens":m.cached_tokens,"rejection_reason":m.rejection_reason,
            "session_requests":self.inline_completion.ledger.requests,"cost_usd":self.inline_completion.ledger.cost_usd,
            "reserved_usd":self.inline_completion.ledger.reserved_usd,"busy":self.inline_completion.busy(),"status":self.inline_completion.status,
            "enabled":!self.config.inline_completion.paused && self.workspace.as_ref().is_some_and(|w|!w.host.is_remote() && self.config.inline_completion.projects.contains(&w.session_key()))});
        let connecting = self.connect.as_ref().map(|c| json!({ "target": c.target, "error": c.error }));
        json!({
            "workspace": workspace,
            "agent_workspace": self.agent_manager.open,
            "inline_completion": inline_completion,
            "connecting": connecting,
            "tabs": tabs,
            "active": active,
            "problems": { "errors": errors, "warnings": warnings },
            "terminals": self.terminals.iter().enumerate().map(|(i, t)| json!({ "terminal": i, "title": t.title, "alive": t.is_alive() })).collect::<Vec<_>>(),
            "language_servers": self.lsp.status().into_iter().map(|(name, state, error)| json!({ "name": name, "state": state, "error": error })).collect::<Vec<_>>(),
            "sidebar": if self.sidebar_open { json!(format!("{:?}", self.side_view).to_lowercase()) } else { Value::Null },
            "panel": if self.panel_open { json!(format!("{:?}", self.panel_tab).to_lowercase()) } else { Value::Null },
        })
    }

    fn diagnostics_json(&self, only: Option<&str>) -> Value {
        let mut out = Vec::new();
        let severity = |s: lsp::Severity| format!("{s:?}").to_lowercase();
        for tab in &self.tabs {
            if let Tab::Doc(doc) = tab {
                if only.is_some_and(|p| p != doc.path) {
                    continue;
                }
                for d in doc.all_diagnostics() {
                    let (line, col) = doc.buffer.line_col(d.range.start);
                    out.push(json!({ "path": self.relative_path(&doc.path), "line": line + 1, "column": col + 1, "severity": severity(d.severity), "source": d.source, "message": d.message }));
                }
            }
        }
        for (path, list) in &self.lsp.closed_diagnostics {
            if only.is_some_and(|p| p != path) {
                continue;
            }
            for (source, d) in list {
                let s = match d.severity {
                    Some(lsp_types::DiagnosticSeverity::WARNING) => "warning",
                    Some(lsp_types::DiagnosticSeverity::INFORMATION) => "info",
                    Some(lsp_types::DiagnosticSeverity::HINT) => "hint",
                    _ => "error",
                };
                out.push(json!({ "path": self.relative_path(path), "line": d.range.start.line + 1, "column": d.range.start.character + 1, "severity": s, "source": d.source.clone().unwrap_or(source.clone()), "message": d.message }));
            }
        }
        json!(out)
    }

    fn control_lsp(&mut self, method: &str, params: Value, reply: Reply) {
        let path = match str_param(&params, "path") {
            Some(p) => self.resolve(p),
            None => match self.active_doc() {
                Some(doc) => doc.path.clone(),
                None => {
                    let _ = reply.send(Err(t!("`path` is required").into()));
                    return;
                }
            },
        };
        let method = method.to_string();
        self.with_doc(
            path,
            Box::new(move |app, result| match result {
                Ok(index) => lsp_request(app, index, method, params, reply, Instant::now()),
                Err(e) => {
                    let _ = reply.send(Err(e));
                }
            }),
        );
    }
}

/// Sends an LSP request for the tab at `index`, waiting for the server to
/// start if necessary.
fn lsp_request(app: &mut App, index: usize, method: String, params: Value, reply: Reply, started: Instant) {
    let Some(Tab::Doc(doc)) = app.tabs.get_mut(index) else {
        let _ = reply.send(Err(t!("file closed").into()));
        return;
    };
    let (lsp_method, capability) = match method.as_str() {
        "hover" => ("textDocument/hover", "hoverProvider"),
        "definition" => ("textDocument/definition", "definitionProvider"),
        "references" => ("textDocument/references", "referencesProvider"),
        "completion" => ("textDocument/completion", "completionProvider"),
        "format" => ("textDocument/formatting", "documentFormattingProvider"),
        _ => (str_param(&params, "method").unwrap_or(""), ""),
    };
    let lsp_method = lsp_method.to_string();
    let client = if capability.is_empty() {
        app.lsp
            .client_with(doc, "textDocumentSync")
            .or_else(|starting| app.lsp.client_with(doc, "positionEncoding").map_err(|s| s || starting))
    } else {
        app.lsp.client_with(doc, capability)
    };
    let client = match client {
        Ok(client) => client,
        Err(true) if started.elapsed() < Duration::from_secs(60) => {
            // Server still starting: retry shortly.
            let path = doc.path.clone();
            let bus = app.bus.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(300));
                bus.send(Msg::Run(Box::new(move |app: &mut App| {
                    match app.tabs.iter().position(|t| matches!(t, Tab::Doc(d) if d.path == path)) {
                        Some(index) => lsp_request(app, index, method, params, reply, started),
                        None => {
                            let _ = reply.send(Err(t!("file closed").into()));
                        }
                    }
                })));
            });
            return;
        }
        Err(_) => {
            let _ = reply.send(Err(t!("no suitable language server for {n1}", n1 = doc.lang.display_name())));
            return;
        }
    };
    app.lsp.sync(doc);
    let uri = lsp::path_to_uri(&doc.path);
    let (line, col) = position(&params, "line", "column");
    let at = doc.buffer.char_at(line, col);
    let position = lsp::to_position(&doc.buffer, at, client.utf8);
    let mut request = match method.as_str() {
        "format" => json!({
            "textDocument": { "uri": uri },
            "options": { "tabSize": doc.buffer.tab_size, "insertSpaces": doc.buffer.insert_spaces, "trimTrailingWhitespace": true, "insertFinalNewline": true },
        }),
        "references" => json!({ "textDocument": { "uri": uri }, "position": position, "context": { "includeDeclaration": true } }),
        "lsp" if params.get("line").is_none() => json!({ "textDocument": { "uri": uri } }),
        _ => json!({ "textDocument": { "uri": uri }, "position": position }),
    };
    if let Some(Value::Object(extra)) = params.get("params")
        && let Value::Object(map) = &mut request
    {
        map.extend(extra.clone());
    }
    let doc_id = doc.id;
    let root = app.workspace.as_ref().map(|w| w.root.clone()).unwrap_or_default();
    let bus = app.bus.clone();
    let limit = int_param(&params, "limit").unwrap_or(50).max(1) as usize;
    let save = bool_param(&params, "save", false);
    std::thread::spawn(move || {
        let result = client.request(&lsp_method, request);
        let value = match result {
            Ok(value) => value,
            Err(e) => {
                let _ = reply.send(Err(e));
                return;
            }
        };
        let relative = |path: &str| path.strip_prefix(&format!("{root}/")).unwrap_or(path).to_string();
        let output = match method.as_str() {
            "hover" => Ok(json!(lsp::hover_text(&value).unwrap_or_default())),
            "definition" | "references" => Ok(json!(
                lsp::parse_locations(&value)
                    .into_iter()
                    .map(|(path, range)| json!({ "path": relative(&path), "line": range.start.line + 1, "column": range.start.character + 1 }))
                    .collect::<Vec<_>>()
            )),
            "completion" => Ok(json!({
                let mut items = lsp::parse_completions(value, "");
                items.sort_by(|a, b| a.sort_text.cmp(&b.sort_text));
                items
            }
                    .into_iter()
                    .take(limit)
                    .map(|item| json!({ "label": item.label, "detail": item.detail, "insert": item.insert_text }))
                    .collect::<Vec<_>>()
            )),
            "format" => {
                let edits: Vec<lsp_types::TextEdit> = serde_json::from_value(value).unwrap_or_default();
                let utf8 = client.utf8;
                bus.send(Msg::Run(Box::new(move |app: &mut App| {
                    let Some(index) = app.doc_index(doc_id) else {
                        let _ = reply.send(Err(t!("file closed").into()));
                        return;
                    };
                    if let Some(Tab::Doc(doc)) = app.tabs.get_mut(index) {
                        let ranges = lsp::text_edits_to_ranges(&doc.buffer, &edits, utf8);
                        doc.buffer.apply_edits(ranges);
                    }
                    let saved = if save { app.save_blocking(index).map(|_| true) } else { Ok(false) };
                    let _ = reply.send(saved.map(|saved| json!({ "edits": edits.len(), "saved": saved })));
                })));
                return;
            }
            _ => Ok(value),
        };
        let _ = reply.send(output);
    });
}

fn apply_edits(doc: &mut super::document::Document, params: &Value) -> Result<usize, String> {
    let edits = params.get("edits").and_then(Value::as_array).ok_or(t!("`edits` array is required"))?;
    let mut ranges = Vec::new();
    for edit in edits {
        let (start_line, start_col) = position(edit, "start_line", "start_column");
        let (end_line, end_col) = position(edit, "end_line", "end_column");
        let start = doc.buffer.char_at(start_line, start_col);
        let end = doc.buffer.char_at(end_line, end_col).max(start);
        let text = edit.get("text").and_then(Value::as_str).unwrap_or("").replace("\r\n", "\n");
        ranges.push((start..end, text));
    }
    ranges.sort_by_key(|(r, _)| r.start);
    if ranges.windows(2).any(|w| w[0].0.end > w[1].0.start) {
        return Err(t!("edits overlap").into());
    }
    let count = ranges.len();
    doc.buffer.apply_edits(ranges);
    Ok(count)
}

fn apply_replace(doc: &mut super::document::Document, params: &Value) -> Result<usize, String> {
    let query = lumen_shared::fsops::SearchQuery {
        pattern: req_str(params, "find")?,
        regex: bool_param(params, "regex", false),
        case_sensitive: true,
        whole_word: false,
        max_results: 0,
    };
    let replacement = str_param(params, "replace").unwrap_or("").to_string();
    let regex = lumen_shared::fsops::build_regex(&query)?;
    let text = doc.buffer.rope.to_string();
    let all = bool_param(params, "all", true);
    let mut edits = Vec::new();
    for found in regex.find_iter(&text) {
        let start = doc.buffer.rope.byte_to_char(found.start());
        let end = doc.buffer.rope.byte_to_char(found.end());
        let new = if query.regex { regex.replace(found.as_str(), replacement.as_str()).into_owned() } else { replacement.clone() };
        edits.push((start..end, new));
        if !all {
            break;
        }
    }
    if edits.is_empty() {
        return Err(t!("no matches").into());
    }
    let count = edits.len();
    doc.buffer.apply_edits(edits);
    Ok(count)
}

fn exec(host: &Host, cwd: &str, command: &str, timeout: Duration) -> Result<Value, String> {
    let mut child =
        host.shell(cwd, command).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| e.to_string())?;
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let out_thread = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = stdout.read_to_end(&mut buffer);
        buffer
    });
    let err_thread = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = stderr.read_to_end(&mut buffer);
        buffer
    });
    let started = Instant::now();
    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break Some(status);
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            timed_out = true;
            break child.wait().ok();
        }
        std::thread::sleep(Duration::from_millis(15));
    };
    let clip = |data: Vec<u8>| -> String {
        let text = String::from_utf8_lossy(&data).into_owned();
        const LIMIT: usize = 200_000;
        if text.len() > LIMIT {
            let mut cut = text.len() - LIMIT;
            while !text.is_char_boundary(cut) {
                cut += 1;
            }
            t!("…(truncated)\n{n1}", n1 = &text[cut..])
        } else {
            text
        }
    };
    Ok(json!({
        "exit_code": status.and_then(|s| s.code()),
        "timed_out": timed_out,
        "stdout": clip(out_thread.join().unwrap_or_default()),
        "stderr": clip(err_thread.join().unwrap_or_default()),
    }))
}

/// Saves a `screenshot` request's image as PNG.
fn save_screenshot(path: &str, image: &eframe::egui::ColorImage) -> Result<Value, String> {
    let [width, height] = image.size;
    let file = std::fs::File::create(path).map_err(|e| format!("{path}: {e}"))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width as u32, height as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    let data: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
    writer.write_image_data(&data).map_err(|e| e.to_string())?;
    Ok(json!({ "path": path, "width": width, "height": height }))
}
