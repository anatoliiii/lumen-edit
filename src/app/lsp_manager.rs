//! Starts language servers per project, keeps documents in sync and turns
//! responses into editor state.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

use super::{App, Bus, Msg, Tab, Workspace, document::Document, editor::EditorAction};
use crate::{
    host::Host,
    lsp::{self, ServerSpec, client::LspClient, client::ServerEvent},
    text::Selection,
};

#[derive(Clone)]
pub enum Request {
    Completion { anchor: usize },
    Hover { at: usize },
    Signature,
    Definition,
    References,
    Rename,
    CodeActions { at: usize },
    ResolveCodeAction,
    Format { then_save: bool },
}

enum ServerState {
    Starting,
    Ready(Arc<LspClient>),
    Unavailable,
    Failed(String),
}

pub struct LspManager {
    specs: Vec<ServerSpec>,
    servers: HashMap<String, ServerState>,
    bus: Bus,
    events: crossbeam_channel::Sender<ServerEvent>,
    pub progress: HashMap<String, String>,
    host: Option<Host>,
    root: String,
    pending_code_actions: Option<(u64, Arc<LspClient>, Vec<Value>)>,
    shellcheck: bool,
    /// Raw diagnostics for files that are not open (for the Problems panel).
    pub closed_diagnostics: HashMap<String, Vec<(String, lsp_types::Diagnostic)>>,
}

impl LspManager {
    pub fn new(specs: Vec<ServerSpec>, bus: Bus) -> Self {
        let (events, rx) = crossbeam_channel::unbounded::<ServerEvent>();
        let forward = bus.clone();
        std::thread::spawn(move || {
            while let Ok(event) = rx.recv() {
                forward.send(Msg::Lsp(event));
            }
        });
        Self {
            specs,
            servers: HashMap::new(),
            bus,
            events,
            progress: HashMap::new(),
            host: None,
            root: String::new(),
            pending_code_actions: None,
            shellcheck: true,
            closed_diagnostics: HashMap::new(),
        }
    }

    pub fn reset(&mut self, specs: Vec<ServerSpec>) {
        self.shutdown_all();
        self.specs = specs;
    }

    pub fn shutdown_all(&mut self) {
        for (_, state) in self.servers.drain() {
            if let ServerState::Ready(client) = state {
                std::thread::spawn(move || client.shutdown());
            }
        }
        self.progress.clear();
        self.closed_diagnostics.clear();
        self.host = None;
    }

    pub fn shellcheck_enabled(&self) -> bool {
        self.shellcheck
    }

    /// (name, state text) for the status bar.
    pub fn status(&self) -> Vec<(String, &'static str, Option<String>)> {
        let mut list: Vec<_> = self
            .servers
            .iter()
            .filter_map(|(name, state)| match state {
                ServerState::Starting => Some((name.clone(), "starting", None)),
                ServerState::Ready(c) if c.is_alive() => Some((name.clone(), "ready", None)),
                ServerState::Ready(_) => Some((name.clone(), "dead", None)),
                ServerState::Failed(e) => Some((name.clone(), "failed", Some(e.clone()))),
                ServerState::Unavailable => None,
            })
            .collect();
        list.sort();
        list
    }

    pub fn unavailable(&self) -> Vec<String> {
        let mut list: Vec<String> =
            self.servers.iter().filter(|(_, s)| matches!(s, ServerState::Unavailable)).map(|(n, _)| n.clone()).collect();
        list.sort();
        list
    }

    fn clients_for(&self, doc: &Document) -> Vec<Arc<LspClient>> {
        lsp::servers_for(doc.lang, &self.specs)
            .iter()
            .filter_map(|spec| match self.servers.get(&spec.name) {
                Some(ServerState::Ready(client)) if client.is_alive() => Some(client.clone()),
                _ => None,
            })
            .collect()
    }

    /// A ready server for the document that supports `capability`, or
    /// `Err(true)` while servers are still starting.
    /// Why a command the user asked for (go to definition, rename, ...)
    /// cannot run on `doc`, or `None` when a server can do it.
    pub fn missing_reason(&self, doc: &Document, capability: &str) -> Option<String> {
        let starting = match self.client_with(doc, capability) {
            Ok(_) => return None,
            Err(starting) => starting,
        };
        if starting {
            return Some(t!("The language server is still starting, try again in a moment").into());
        }
        let specs = lsp::servers_for(doc.lang, &self.specs);
        if specs.is_empty() {
            return Some(t!("No language server for {lang}", lang = doc.lang.display_name()));
        }
        let reasons: Vec<String> = specs
            .iter()
            .map(|spec| match self.servers.get(&spec.name) {
                Some(ServerState::Failed(error)) => {
                    let reason = format!("{}: {}", spec.name, error.lines().last().unwrap_or(error));
                    with_install_hint(reason, &spec.name)
                }
                Some(ServerState::Unavailable) => with_install_hint(t!("{name}: not installed", name = spec.command), &spec.name),
                Some(ServerState::Ready(client)) if !client.is_alive() => t!("{name}: stopped", name = spec.name),
                _ => t!("{name}: does not support this", name = spec.name),
            })
            .collect();
        Some(t!("Language server cannot do this. {reasons}", reasons = reasons.join("; ")))
    }

    pub fn client_with(&self, doc: &Document, capability: &str) -> Result<Arc<LspClient>, bool> {
        if let Some(client) = self.clients_for(doc).into_iter().find(|c| c.has_capability(capability)) {
            return Ok(client);
        }
        let starting =
            lsp::servers_for(doc.lang, &self.specs).iter().any(|s| matches!(self.servers.get(&s.name), Some(ServerState::Starting) | None));
        Err(starting)
    }

    pub fn has_clients(&self, doc: &Document) -> bool {
        !self.clients_for(doc).is_empty()
    }

    pub fn triggers(&self, doc: &Document) -> (Vec<String>, Vec<String>) {
        let mut completion = Vec::new();
        let mut signature = Vec::new();
        for client in self.clients_for(doc) {
            completion.extend(client.completion_triggers());
            if let Some(chars) = client.capabilities.pointer("/signatureHelpProvider/triggerCharacters").and_then(Value::as_array) {
                signature.extend(chars.iter().filter_map(|c| c.as_str().map(str::to_string)));
            }
        }
        (completion, signature)
    }

    pub fn can_format(&self, doc: &Document) -> bool {
        self.clients_for(doc).iter().any(|c| c.has_capability("documentFormattingProvider"))
    }

    pub fn document_opened(&mut self, doc: &mut Document, workspace: Option<&Workspace>) {
        let Some(workspace) = workspace else { return };
        if doc.large {
            return;
        }
        if self.host.as_ref().is_none_or(|h| !h.same(&workspace.host)) || self.root != workspace.root {
            self.host = Some(workspace.host.clone());
            self.root = workspace.root.clone();
        }
        for spec in lsp::servers_for(doc.lang, &self.specs) {
            if self.servers.contains_key(&spec.name) {
                continue;
            }
            if let Some(primary) = &spec.fallback_for {
                match self.servers.get(primary) {
                    Some(ServerState::Unavailable) | Some(ServerState::Failed(_)) => {}
                    // Decide once the primary server's fate is known.
                    _ => continue,
                }
            }
            self.start(spec);
        }
        for client in self.clients_for(doc) {
            open_in(&client, doc);
        }
    }

    fn start(&mut self, spec: ServerSpec) {
        let Some(host) = self.host.clone() else { return };
        self.servers.insert(spec.name.clone(), ServerState::Starting);
        let root = self.root.clone();
        let events = self.events.clone();
        self.bus.spawn(move || {
            if !host.which(&spec.command) {
                return Msg::LspStarted { name: spec.name, result: Err(None) };
            }
            let result = LspClient::start(&host, &root, &spec.name, &spec.command, &spec.args, events).map_err(Some);
            Msg::LspStarted { name: spec.name, result }
        });
    }

    pub fn document_closed(&mut self, doc: &Document) {
        for client in self.clients_for(doc) {
            if doc.lsp_opened.contains(&client.name) {
                client.notify("textDocument/didClose", json!({ "textDocument": { "uri": lsp::path_to_uri(&doc.path) } }));
            }
        }
    }

    pub fn document_saved(&mut self, doc: &mut Document) {
        self.sync(doc);
        for client in self.clients_for(doc) {
            client.notify(
                "textDocument/didSave",
                json!({ "textDocument": { "uri": lsp::path_to_uri(&doc.path) }, "text": doc.buffer.rope.to_string() }),
            );
        }
    }

    /// Sends pending changes once typing pauses.
    pub fn sync_documents(&mut self, tabs: &mut [Tab], debounce: Duration) {
        for tab in tabs {
            if let Tab::Doc(doc) = tab
                && doc.lsp_synced != doc.buffer.version
                && doc.last_edit.elapsed() >= debounce
            {
                self.sync(doc);
            }
        }
    }

    pub fn sync(&mut self, doc: &mut Document) {
        if doc.lsp_synced == doc.buffer.version {
            return;
        }
        let clients = self.clients_for(doc);
        if clients.is_empty() {
            doc.lsp_synced = doc.buffer.version;
            return;
        }
        doc.lsp_version += 1;
        let text = doc.buffer.rope.to_string();
        for client in clients {
            if !doc.lsp_opened.contains(&client.name) {
                open_in(&client, doc);
                continue;
            }
            client.notify(
                "textDocument/didChange",
                json!({
                    "textDocument": { "uri": lsp::path_to_uri(&doc.path), "version": doc.lsp_version },
                    "contentChanges": [{ "text": text }],
                }),
            );
        }
        doc.lsp_synced = doc.buffer.version;
    }

    /// Sends a request for the document to the first capable server.
    pub fn request(&mut self, doc: &mut Document, request: Request, client: Option<Arc<LspClient>>) {
        self.sync(doc);
        let capability = match request {
            Request::Completion { .. } => "completionProvider",
            Request::Hover { .. } => "hoverProvider",
            Request::Signature => "signatureHelpProvider",
            Request::Definition => "definitionProvider",
            Request::References => "referencesProvider",
            Request::Rename => "renameProvider",
            Request::CodeActions { .. } | Request::ResolveCodeAction => "codeActionProvider",
            Request::Format { .. } => "documentFormattingProvider",
        };
        let Some(client) = client.or_else(|| self.clients_for(doc).into_iter().find(|c| c.has_capability(capability))) else {
            if let Request::Format { then_save: true } = request {
                // No formatter: just save.
                self.bus.send(Msg::LspResult {
                    doc_id: doc.id,
                    version: doc.buffer.version,
                    request,
                    client: match self.clients_for(doc).into_iter().next() {
                        Some(c) => c,
                        None => return,
                    },
                    result: Ok(Value::Null),
                });
            }
            return;
        };
        let uri = lsp::path_to_uri(&doc.path);
        let cursor = doc.buffer.primary().head;
        let position = lsp::to_position(&doc.buffer, cursor, client.utf8);
        let (method, params) = match &request {
            Request::Completion { .. } => (
                "textDocument/completion",
                json!({ "textDocument": { "uri": uri }, "position": position, "context": { "triggerKind": 1 } }),
            ),
            Request::Hover { at } => (
                "textDocument/hover",
                json!({ "textDocument": { "uri": uri }, "position": lsp::to_position(&doc.buffer, *at, client.utf8) }),
            ),
            Request::Signature => ("textDocument/signatureHelp", json!({ "textDocument": { "uri": uri }, "position": position })),
            Request::Definition => ("textDocument/definition", json!({ "textDocument": { "uri": uri }, "position": position })),
            Request::References => (
                "textDocument/references",
                json!({ "textDocument": { "uri": uri }, "position": position, "context": { "includeDeclaration": true } }),
            ),
            Request::Rename => {
                let new_name = doc.view.rename.as_ref().map(|r| r.text.clone()).unwrap_or_default();
                ("textDocument/rename", json!({ "textDocument": { "uri": uri }, "position": position, "newName": new_name }))
            }
            Request::CodeActions { at } => {
                let selection = doc.buffer.primary();
                let range = if selection.is_empty() { *at..*at } else { selection.range() };
                let diagnostics: Vec<Value> = doc
                    .diagnostics
                    .get(&client.name)
                    .map(|list| {
                        list.iter()
                            .filter(|d| d.range.start <= range.end && d.range.end >= range.start)
                            .filter_map(|d| d.raw.as_ref().and_then(|raw| serde_json::to_value(raw).ok()))
                            .collect()
                    })
                    .unwrap_or_default();
                (
                    "textDocument/codeAction",
                    json!({
                        "textDocument": { "uri": uri },
                        "range": lsp::to_range(&doc.buffer, range, client.utf8),
                        "context": { "diagnostics": diagnostics },
                    }),
                )
            }
            Request::ResolveCodeAction => return,
            Request::Format { .. } => (
                "textDocument/formatting",
                json!({
                    "textDocument": { "uri": uri },
                    "options": { "tabSize": doc.buffer.tab_size, "insertSpaces": doc.buffer.insert_spaces, "trimTrailingWhitespace": true, "insertFinalNewline": true },
                }),
            ),
        };
        let doc_id = doc.id;
        let version = doc.buffer.version;
        let method = method.to_string();
        crate::debug_log!("lsp request {method} -> {} at {:?}", client.name, position);
        self.bus.spawn(move || {
            let result = client.request(&method, params);
            Msg::LspResult { doc_id, version, request, client, result }
        });
    }

    pub fn ready_client(&self, name: &str) -> Option<Arc<LspClient>> {
        match self.servers.get(name) {
            Some(ServerState::Ready(c)) => Some(c.clone()),
            _ => None,
        }
    }
}

fn open_in(client: &LspClient, doc: &mut Document) {
    if doc.lsp_opened.contains(&client.name) {
        return;
    }
    doc.lsp_version += 1;
    client.notify(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": lsp::path_to_uri(&doc.path),
                "languageId": doc.lang.lsp_id(),
                "version": doc.lsp_version,
                "text": doc.buffer.rope.to_string(),
            }
        }),
    );
    doc.lsp_opened.push(client.name.clone());
    doc.lsp_synced = doc.buffer.version;
}

pub fn on_started(app: &mut App, name: String, result: Result<Arc<LspClient>, Option<String>>) {
    let state = match result {
        Ok(client) => ServerState::Ready(client),
        Err(None) => ServerState::Unavailable,
        Err(Some(error)) => {
            app.output.push((format!("{name}: {error}"), true));
            ServerState::Failed(error)
        }
    };
    let ready = matches!(state, ServerState::Ready(_));
    app.lsp.servers.insert(name.clone(), state);
    // Open documents in the new server, or start fallbacks for the failed one.
    let workspace = app.workspace.as_ref();
    for tab in &mut app.tabs {
        if let Tab::Doc(doc) = tab {
            if ready {
                if let Some(client) = app.lsp.ready_client(&name)
                    && lsp::servers_for(doc.lang, &app.lsp.specs).iter().any(|s| s.name == name)
                {
                    open_in(&client, doc);
                }
            } else {
                app.lsp.document_opened(doc, workspace);
            }
        }
    }
}

pub fn handle_event(app: &mut App, event: ServerEvent) {
    match event {
        ServerEvent::Diagnostics { uri, diagnostics } => {
            let Some(path) = lsp::uri_to_path(&uri) else { return };
            // Which server sent it is not part of the notification; attribute
            // by language of the file and the server that has it open.
            let mut handled = false;
            for tab in &mut app.tabs {
                if let Tab::Doc(doc) = tab
                    && doc.path == path
                {
                    let source = guess_source(&app.lsp, doc, &diagnostics);
                    let utf8 = app.lsp.ready_client(&source).map(|c| c.utf8).unwrap_or(false);
                    let converted = lsp::convert_diagnostics(&doc.buffer, &diagnostics, utf8, &source);
                    doc.diagnostics.insert(source, converted);
                    handled = true;
                }
            }
            if !handled {
                if diagnostics.is_empty() {
                    app.lsp.closed_diagnostics.remove(&path);
                } else {
                    let source = diagnostics.first().and_then(|d| d.source.clone()).unwrap_or_default();
                    app.lsp.closed_diagnostics.insert(path, diagnostics.into_iter().map(|d| (source.clone(), d)).collect());
                }
            }
        }
        ServerEvent::Progress { server, message } => match message {
            Some(message) => {
                app.lsp.progress.insert(server, message);
            }
            None => {
                app.lsp.progress.remove(&server);
            }
        },
        ServerEvent::ApplyEdit(edit) => apply_workspace_edit(app, &edit, false),
        ServerEvent::Message { server, text, error } => {
            if error {
                app.notify(format!("{server}: {text}"), true);
            } else {
                app.output.push((format!("{server}: {text}"), false));
            }
        }
        ServerEvent::Exited { server, .. } => {
            if let Some(ServerState::Ready(_)) = app.lsp.servers.get(&server) {
                app.lsp.servers.insert(server.clone(), ServerState::Failed(t!("the process exited").into()));
                app.lsp.progress.remove(&server);
                app.notify(t!("Language server {server} stopped. Restart it from the command palette.", server = server), true);
            }
        }
    }
}

/// Diagnostics notifications don't name the server. Use the `source`
/// field when it matches a running server, otherwise the first server
/// that has the document open.
fn guess_source(manager: &LspManager, doc: &Document, diagnostics: &[lsp_types::Diagnostic]) -> String {
    let opened = &doc.lsp_opened;
    if opened.len() == 1 {
        return opened[0].clone();
    }
    if let Some(source) = diagnostics.iter().find_map(|d| d.source.clone()) {
        let source_lower = source.to_lowercase();
        if let Some(name) = opened.iter().find(|n| source_lower.contains(&n.to_lowercase()) || n.to_lowercase().contains(&source_lower)) {
            return name.clone();
        }
        // e.g. rust-analyzer reports "rustc" / "clippy".
        if let Some(name) = opened.first() {
            let _ = manager;
            if diagnostics.is_empty() {
                return name.clone();
            }
        }
        return opened.iter().find(|n| *n != "ruff" && *n != "biome").cloned().unwrap_or(source);
    }
    opened.first().cloned().unwrap_or_else(|| "lsp".into())
}

/// `reason` followed by the command that installs the server, if known.
fn with_install_hint(reason: String, server: &str) -> String {
    match lsp::install_hint(server) {
        Some(command) => t!("{reason}. Install: {command}", reason = reason.trim_end_matches('.'), command = command),
        None => reason,
    }
}

pub fn run_actions(app: &mut App, tab_index: usize, actions: Vec<EditorAction>) {
    for action in actions {
        crate::debug_log!(
            "editor action {}",
            match &action {
                EditorAction::Completion { trigger } => format!("completion {trigger:?}"),
                _ => "other".into(),
            }
        );
        let Some(Tab::Doc(doc)) = app.tabs.get_mut(tab_index) else { return };
        // Commands the user asked for say why nothing happens; hover and
        // completion while typing stay quiet.
        let asked = match &action {
            EditorAction::GotoDefinition => Some("definitionProvider"),
            EditorAction::References => Some("referencesProvider"),
            EditorAction::Rename(_) => Some("renameProvider"),
            EditorAction::CodeActions => Some("codeActionProvider"),
            EditorAction::Format => Some("documentFormattingProvider"),
            _ => None,
        };
        if let Some(reason) = asked.and_then(|capability| app.lsp.missing_reason(doc, capability)) {
            app.notify(reason, true);
            continue;
        }
        let Some(Tab::Doc(doc)) = app.tabs.get_mut(tab_index) else { return };
        match action {
            EditorAction::Completion { trigger } => {
                let head = doc.buffer.primary().head;
                let word = doc.buffer.word_at(head);
                let anchor = if trigger.is_some() { head } else { word.start.min(head) };
                app.lsp.request(doc, Request::Completion { anchor }, None);
            }
            EditorAction::Hover(at) => app.lsp.request(doc, Request::Hover { at }, None),
            EditorAction::SignatureHelp => app.lsp.request(doc, Request::Signature, None),
            EditorAction::GotoDefinition => app.lsp.request(doc, Request::Definition, None),
            EditorAction::References => app.lsp.request(doc, Request::References, None),
            EditorAction::Rename(name) => {
                doc.view.rename = Some(super::editor::RenamePopup { text: name, focus: false });
                app.lsp.request(doc, Request::Rename, None);
                if let Some(Tab::Doc(doc)) = app.tabs.get_mut(tab_index) {
                    doc.view.rename = None;
                }
            }
            EditorAction::CodeActions => {
                let at = doc.buffer.primary().head;
                app.lsp.request(doc, Request::CodeActions { at }, None);
            }
            EditorAction::ApplyCodeAction(index) => {
                let Some((doc_id, client, items)) = app.lsp.pending_code_actions.take() else { continue };
                if doc_id != doc.id {
                    continue;
                }
                let Some(item) = items.get(index).cloned() else { continue };
                apply_code_action(app, client, item);
            }
            EditorAction::Format => app.lsp.request(doc, Request::Format { then_save: false }, None),
            EditorAction::AskAgent(instruction) => {
                let doc_id = doc.id;
                super::agent::ask(app, doc_id, instruction.as_deref());
            }
        }
    }
}

fn apply_code_action(app: &mut App, client: Arc<LspClient>, item: Value) {
    // A bare Command, or a CodeAction with edit and/or command.
    let is_command = item.get("command").is_some_and(Value::is_string);
    if is_command {
        execute_command(app, client, item);
        return;
    }
    if let Some(edit) = item.get("edit").and_then(|e| serde_json::from_value::<lsp_types::WorkspaceEdit>(e.clone()).ok()) {
        apply_workspace_edit(app, &edit, client.utf8);
        if let Some(command) = item.get("command").cloned() {
            execute_command(app, client, command);
        }
        return;
    }
    if client.capabilities.pointer("/codeActionProvider/resolveProvider").and_then(Value::as_bool) == Some(true) {
        let bus = app.bus.clone();
        let doc_id = app.active_doc().map(|d| d.id).unwrap_or(0);
        bus.spawn(move || {
            let result = client.request("codeAction/resolve", item);
            Msg::LspResult { doc_id, version: 0, request: Request::ResolveCodeAction, client, result }
        });
    } else if let Some(command) = item.get("command").cloned() {
        execute_command(app, client, command);
    }
}

fn execute_command(app: &mut App, client: Arc<LspClient>, command: Value) {
    let name = command.get("command").and_then(Value::as_str).unwrap_or("").to_string();
    let arguments = command.get("arguments").cloned().unwrap_or(json!([]));
    let bus = app.bus.clone();
    std::thread::spawn(move || {
        if let Err(error) = client.request("workspace/executeCommand", json!({ "command": name, "arguments": arguments })) {
            bus.send(Msg::Notice(format!("{name}: {error}"), true));
        }
    });
}

/// Applies edits to open documents directly and to other files on disk.
pub fn apply_workspace_edit(app: &mut App, edit: &lsp_types::WorkspaceEdit, utf8: bool) {
    let files = lsp::workspace_edit_files(edit);
    let mut on_disk = Vec::new();
    for (path, edits) in files {
        if let Some(doc) = app.doc_by_path(&path) {
            let ranges = lsp::text_edits_to_ranges(&doc.buffer, &edits, utf8);
            doc.buffer.apply_edits(ranges);
            doc.view.reveal = true;
        } else {
            on_disk.push((path, edits));
        }
    }
    if on_disk.is_empty() {
        return;
    }
    let Some(host) = app.host() else { return };
    app.bus.spawn(move || {
        let mut count = 0;
        for (path, edits) in on_disk {
            let data = match host.read_file(&path) {
                Ok(d) => d,
                Err(e) => return Msg::WorkspaceEditDone(Err(format!("{path}: {e}"))),
            };
            let mut buffer = crate::text::Buffer::new(&String::from_utf8_lossy(&data));
            let ranges = lsp::text_edits_to_ranges(&buffer, &edits, utf8);
            buffer.apply_edits(ranges);
            if let Err(e) = host.write_file(&path, buffer.text().as_bytes()) {
                return Msg::WorkspaceEditDone(Err(format!("{path}: {e}")));
            }
            count += 1;
        }
        Msg::WorkspaceEditDone(Ok(count))
    });
}

pub fn on_result(app: &mut App, doc_id: u64, version: u64, request: Request, client: Arc<LspClient>, result: Result<Value, String>) {
    let value = match result {
        Ok(value) => value,
        Err(error) => {
            crate::debug_log!("lsp error from {}: {error}", client.name);
            if matches!(request, Request::Definition | Request::References | Request::Rename | Request::Format { .. }) {
                app.notify(error, true);
            }
            if let Request::Format { then_save: true } = request
                && let Some(index) = app.doc_index(doc_id)
            {
                app.save_doc_now(index);
            }
            return;
        }
    };
    match request {
        Request::Completion { anchor } => {
            crate::debug_log!("completion raw: {}", value.to_string().chars().take(200).collect::<String>());
            let Some(doc) = app.doc_mut(doc_id) else { return };
            let mut items = lsp::parse_completions(value, &client.name);
            for item in &mut items {
                item.utf8 = client.utf8;
            }
            if items.is_empty() {
                return;
            }
            let head = doc.buffer.primary().head;
            if head < anchor {
                return;
            }
            let prefix = doc.buffer.slice(anchor..head);
            crate::debug_log!("completion: {} items, prefix {prefix:?}", items.len());
            let mut completion = super::editor::Completion::new(items, anchor);
            completion_refilter(&mut completion, &prefix);
            if !completion.filtered.is_empty() {
                doc.view.completion = Some(completion);
            }
        }
        Request::Hover { at } => {
            let Some(doc) = app.doc_mut(doc_id) else { return };
            if let Some(text) = lsp::hover_text(&value)
                && let Some(hover) = &mut doc.view.hover
                && hover.anchor == at
            {
                if hover.text.is_empty() {
                    hover.text = text;
                } else {
                    hover.text = format!("{}\n\n---\n{text}", hover.text);
                }
            } else if let Some(text) = lsp::hover_text(&value) {
                // Keyboard-triggered hover (Ctrl+K): show at the cursor.
                let pos = doc.view.hover.as_ref().map(|h| h.pos);
                if pos.is_none() {
                    doc.view.signature = Some(super::editor::strip_markdown(&text).lines().take(12).collect::<Vec<_>>().join("\n"));
                }
            }
        }
        Request::Signature => {
            let Some(doc) = app.doc_mut(doc_id) else { return };
            let help: Option<lsp_types::SignatureHelp> = serde_json::from_value(value).ok();
            doc.view.signature = help.and_then(|h| {
                let active = h.active_signature.unwrap_or(0) as usize;
                h.signatures.get(active).or(h.signatures.first()).map(|s| s.label.clone())
            });
        }
        Request::Definition => {
            let locations = lsp::parse_locations(&value);
            match locations.first() {
                Some((path, range)) => {
                    let path = path.clone();
                    let range = *range;
                    let utf8 = client.utf8;
                    open_location(app, path, range, utf8);
                }
                None => app.notify(t!("Definition not found"), false),
            }
        }
        Request::References => {
            let locations = lsp::parse_locations(&value);
            let root = app.workspace.as_ref().map(|w| w.root.clone()).unwrap_or_default();
            app.references = locations
                .into_iter()
                .map(|(path, range)| {
                    let label = path.strip_prefix(&root).unwrap_or(&path).trim_start_matches('/').to_string();
                    (path, range.start.line as usize, range.start.character as usize, label)
                })
                .collect();
            app.panel_open = true;
            app.panel_tab = super::PanelTab::References;
            if app.references.is_empty() {
                app.notify(t!("No references found"), false);
            }
        }
        Request::Rename => {
            if let Ok(edit) = serde_json::from_value::<lsp_types::WorkspaceEdit>(value) {
                apply_workspace_edit(app, &edit, client.utf8);
            }
        }
        Request::CodeActions { .. } => {
            let Some(doc) = app.doc_mut(doc_id) else { return };
            let items: Vec<Value> = value.as_array().cloned().unwrap_or_default();
            let titles = items.iter().map(|i| i.get("title").and_then(Value::as_str).unwrap_or("?").to_string()).collect();
            doc.view.code_actions = Some(super::editor::CodeActionsPopup { titles, selected: 0 });
            app.lsp.pending_code_actions = Some((doc_id, client, items));
        }
        Request::ResolveCodeAction => {
            if let Some(edit) = value.get("edit").and_then(|e| serde_json::from_value::<lsp_types::WorkspaceEdit>(e.clone()).ok()) {
                apply_workspace_edit(app, &edit, client.utf8);
            }
            if let Some(command) = value.get("command").cloned() {
                execute_command(app, client, command);
            }
        }
        Request::Format { then_save } => {
            if let Some(doc) = app.doc_mut(doc_id)
                && doc.buffer.version == version
                && let Ok(edits) = serde_json::from_value::<Vec<lsp_types::TextEdit>>(value)
            {
                let ranges = lsp::text_edits_to_ranges(&doc.buffer, &edits, client.utf8);
                let head = doc.buffer.primary().head;
                doc.buffer.apply_edits(ranges);
                let len = doc.buffer.len_chars();
                doc.buffer.set_selections(vec![Selection::cursor(head.min(len))]);
            }
            if then_save && let Some(index) = app.doc_index(doc_id) {
                app.save_doc_now(index);
            }
        }
    }
}

fn completion_refilter(completion: &mut super::editor::Completion, prefix: &str) {
    // Reuse the editor's filter through a tiny shim.
    super::editor::refilter_completion(completion, prefix);
}

pub fn open_location(app: &mut App, path: String, range: lsp_types::Range, utf8: bool) {
    if let Some(doc) = app.doc_by_path(&path) {
        let at = lsp::from_position(&doc.buffer, &range.start, utf8);
        doc.buffer.set_cursor(at, false);
        doc.view.reveal = true;
        doc.view.reveal_center = true;
        doc.view.focus_request = true;
        if let Some(index) = app.tabs.iter().position(|t| matches!(t, Tab::Doc(d) if d.path == path)) {
            app.activate_tab(index);
        }
    } else {
        // Character offsets are close enough for positioning before load.
        app.open_file(path, Some((range.start.line as usize, range.start.character as usize)));
    }
    let _ = Instant::now();
}
