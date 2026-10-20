//! A JSON-RPC connection to one language server process.

use std::{
    collections::HashMap,
    io::{BufRead, BufReader, BufWriter, Write},
    process::{Child, ChildStdin, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicI64, Ordering},
    },
    time::Duration,
};

use crossbeam_channel::{Sender, bounded};
use serde_json::{Value, json};

use crate::host::Host;

#[derive(Debug)]
pub enum ServerEvent {
    Diagnostics { uri: String, diagnostics: Vec<lsp_types::Diagnostic> },
    Progress { server: String, message: Option<String> },
    ApplyEdit(lsp_types::WorkspaceEdit),
    Message { server: String, text: String, error: bool },
    Exited { server: String },
}

type Pending = Arc<Mutex<HashMap<i64, Sender<Result<Value, String>>>>>;

pub struct LspClient {
    pub name: String,
    writer: Arc<Mutex<BufWriter<ChildStdin>>>,
    pending: Pending,
    next_id: AtomicI64,
    alive: Arc<AtomicBool>,
    child: Mutex<Child>,
    pub capabilities: Value,
    pub utf8: bool,
}

fn send(writer: &Mutex<BufWriter<ChildStdin>>, message: &Value) -> std::io::Result<()> {
    let body = serde_json::to_vec(message)?;
    let mut writer = writer.lock().unwrap();
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(&body)?;
    writer.flush()
}

fn read_message(reader: &mut impl BufRead) -> std::io::Result<Value> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        let line = line.trim_end();
        if line.is_empty() {
            if length.is_some() {
                break;
            }
            continue;
        }
        if let Some(value) = line.strip_prefix("Content-Length:") {
            length = value.trim().parse::<usize>().ok();
        }
    }
    let mut body = vec![0; length.unwrap()];
    reader.read_exact(&mut body)?;
    serde_json::from_slice(&body).map_err(std::io::Error::other)
}

impl LspClient {
    /// Spawns the server and performs the `initialize` handshake.
    pub fn start(host: &Host, root: &str, name: &str, command: &str, args: &[String], events: Sender<ServerEvent>) -> Result<Arc<Self>, String> {
        let mut child = host
            .command(root, command, args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("{command}: {e}"))?;
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let writer = Arc::new(Mutex::new(BufWriter::new(stdin)));
        let pending: Pending = Arc::default();
        let alive = Arc::new(AtomicBool::new(true));

        // stderr is mostly logging; keep the last lines for error reports.
        let stderr_tail = Arc::new(Mutex::new(Vec::<String>::new()));
        {
            let tail = stderr_tail.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    let mut tail = tail.lock().unwrap();
                    tail.push(line);
                    if tail.len() > 20 {
                        tail.remove(0);
                    }
                }
            });
        }

        {
            let pending = pending.clone();
            let alive = alive.clone();
            let writer = writer.clone();
            let name = name.to_string();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stdout);
                while let Ok(message) = read_message(&mut reader) {
                    handle_incoming(message, &pending, &writer, &events, &name);
                }
                alive.store(false, Ordering::Relaxed);
                for (_, tx) in pending.lock().unwrap().drain() {
                    let _ = tx.send(Err("language server exited".into()));
                }
                let _ = events.send(ServerEvent::Exited { server: name });
            });
        }

        let mut client = LspClient {
            name: name.to_string(),
            writer,
            pending,
            next_id: AtomicI64::new(1),
            alive,
            child: Mutex::new(child),
            capabilities: Value::Null,
            utf8: false,
        };

        let root_uri = super::path_to_uri(root);
        let params = json!({
            "processId": std::process::id(),
            "clientInfo": { "name": "codium-rs", "version": env!("CARGO_PKG_VERSION") },
            "rootUri": root_uri,
            "rootPath": root,
            "workspaceFolders": [{ "uri": root_uri, "name": crate::host::file_name(root) }],
            "capabilities": client_capabilities(),
            "initializationOptions": initialization_options(name),
        });
        let result = client.request_timeout("initialize", params, Duration::from_secs(60)).map_err(|error| {
            let tail = stderr_tail.lock().unwrap().join("\n");
            if tail.is_empty() { error } else { format!("{error}\n{tail}") }
        })?;
        client.capabilities = result.get("capabilities").cloned().unwrap_or(Value::Null);
        client.utf8 = client.capabilities.get("positionEncoding").and_then(Value::as_str) == Some("utf-8");
        client.notify("initialized", json!({}));
        Ok(Arc::new(client))
    }

    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }

    pub fn notify(&self, method: &str, params: Value) {
        crate::debug_log!("-> {}: notify {method}", self.name);
        if send(&self.writer, &json!({ "jsonrpc": "2.0", "method": method, "params": params })).is_err() {
            self.alive.store(false, Ordering::Relaxed);
        }
    }

    pub fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        self.request_timeout(method, params, Duration::from_secs(30))
    }

    pub fn request_timeout(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, String> {
        if !self.is_alive() {
            return Err(format!("{} не запущен", self.name));
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = bounded(1);
        self.pending.lock().unwrap().insert(id, tx);
        crate::debug_log!("-> {}: id={id} {method}", self.name);
        if let Err(error) = send(&self.writer, &json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })) {
            self.pending.lock().unwrap().remove(&id);
            return Err(error.to_string());
        }
        match rx.recv_timeout(timeout) {
            Ok(result) => result,
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                self.notify("$/cancelRequest", json!({ "id": id }));
                Err(format!("{}: нет ответа на {method}", self.name))
            }
        }
    }

    pub fn has_capability(&self, name: &str) -> bool {
        match self.capabilities.get(name) {
            None | Some(Value::Null) | Some(Value::Bool(false)) => false,
            Some(_) => true,
        }
    }

    pub fn completion_triggers(&self) -> Vec<String> {
        self.capabilities
            .pointer("/completionProvider/triggerCharacters")
            .and_then(Value::as_array)
            .map(|chars| chars.iter().filter_map(|c| c.as_str().map(str::to_string)).collect())
            .unwrap_or_default()
    }

    pub fn shutdown(&self) {
        if self.is_alive() {
            let _ = self.request_timeout("shutdown", Value::Null, Duration::from_millis(500));
            self.notify("exit", Value::Null);
        }
        self.alive.store(false, Ordering::Relaxed);
        let _ = self.child.lock().unwrap().kill();
    }
}

impl Drop for LspClient {
    fn drop(&mut self) {
        let _ = self.child.lock().unwrap().kill();
    }
}

fn handle_incoming(message: Value, pending: &Pending, writer: &Mutex<BufWriter<ChildStdin>>, events: &Sender<ServerEvent>, name: &str) {
    let id = message.get("id").cloned();
    let method = message.get("method").and_then(Value::as_str).map(str::to_string);
    crate::debug_log!("<- {name}: id={id:?} method={method:?}");
    match (id, method) {
        // Response to one of our requests.
        (Some(id), None) => {
            let Some(id) = id.as_i64() else { return };
            if let Some(tx) = pending.lock().unwrap().remove(&id) {
                let result = match message.get("error") {
                    Some(error) => Err(error.get("message").and_then(Value::as_str).unwrap_or("error").to_string()),
                    None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
                };
                let _ = tx.send(result);
            }
        }
        // Request from the server.
        (Some(id), Some(method)) => {
            let params = message.get("params").cloned().unwrap_or(Value::Null);
            let result = match method.as_str() {
                "workspace/configuration" => {
                    let count = params.get("items").and_then(Value::as_array).map(|a| a.len()).unwrap_or(0);
                    Value::Array(vec![Value::Null; count])
                }
                "workspace/applyEdit" => {
                    if let Some(edit) = params.get("edit").and_then(|e| serde_json::from_value(e.clone()).ok()) {
                        let _ = events.send(ServerEvent::ApplyEdit(edit));
                    }
                    json!({ "applied": true })
                }
                "workspace/workspaceFolders" => Value::Null,
                _ => Value::Null,
            };
            let _ = send(writer, &json!({ "jsonrpc": "2.0", "id": id, "result": result }));
        }
        (None, Some(method)) => {
            let params = message.get("params").cloned().unwrap_or(Value::Null);
            match method.as_str() {
                "textDocument/publishDiagnostics" => {
                    let uri = params.get("uri").and_then(Value::as_str).unwrap_or("").to_string();
                    let diagnostics = params
                        .get("diagnostics")
                        .and_then(|d| serde_json::from_value(d.clone()).ok())
                        .unwrap_or_default();
                    let _ = events.send(ServerEvent::Diagnostics { uri, diagnostics });
                }
                "$/progress" => {
                    let value = params.get("value");
                    let kind = value.and_then(|v| v.get("kind")).and_then(Value::as_str);
                    let message = match kind {
                        Some("end") => None,
                        _ => {
                            let title = value.and_then(|v| v.get("title")).and_then(Value::as_str);
                            let detail = value.and_then(|v| v.get("message")).and_then(Value::as_str);
                            let percent = value.and_then(|v| v.get("percentage")).and_then(Value::as_u64);
                            let mut text = [title, detail].into_iter().flatten().collect::<Vec<_>>().join(": ");
                            if let Some(p) = percent {
                                text.push_str(&format!(" {p}%"));
                            }
                            Some(text)
                        }
                    };
                    let _ = events.send(ServerEvent::Progress { server: name.to_string(), message });
                }
                "window/showMessage" => {
                    let text = params.get("message").and_then(Value::as_str).unwrap_or("").to_string();
                    let error = params.get("type").and_then(Value::as_u64) == Some(1);
                    let _ = events.send(ServerEvent::Message { server: name.to_string(), text, error });
                }
                _ => {}
            }
        }
        _ => {}
    }
}

fn client_capabilities() -> Value {
    json!({
        "general": { "positionEncodings": ["utf-8", "utf-16"] },
        "window": { "workDoneProgress": true, "showMessage": {} },
        "workspace": {
            "applyEdit": true,
            "workspaceEdit": { "documentChanges": true, "resourceOperations": ["create", "rename", "delete"] },
            "configuration": true,
            "workspaceFolders": true,
            "didChangeWatchedFiles": { "dynamicRegistration": false },
            "symbol": {},
        },
        "textDocument": {
            "synchronization": { "didSave": true, "willSave": false, "dynamicRegistration": false },
            "completion": {
                "completionItem": {
                    "snippetSupport": true,
                    "insertReplaceSupport": false,
                    "documentationFormat": ["plaintext", "markdown"],
                    "resolveSupport": { "properties": ["documentation", "detail", "additionalTextEdits"] },
                    "labelDetailsSupport": true,
                },
                "contextSupport": true,
            },
            "hover": { "contentFormat": ["plaintext", "markdown"] },
            "signatureHelp": { "signatureInformation": { "documentationFormat": ["plaintext", "markdown"], "parameterInformation": { "labelOffsetSupport": true } } },
            "definition": { "linkSupport": true },
            "typeDefinition": { "linkSupport": true },
            "implementation": { "linkSupport": true },
            "references": {},
            "documentSymbol": { "hierarchicalDocumentSymbolSupport": true },
            "formatting": {},
            "rangeFormatting": {},
            "rename": { "prepareSupport": false },
            "codeAction": {
                "codeActionLiteralSupport": { "codeActionKind": { "valueSet": ["", "quickfix", "refactor", "refactor.extract", "refactor.inline", "refactor.rewrite", "source", "source.organizeImports", "source.fixAll"] } },
                "resolveSupport": { "properties": ["edit"] },
                "dataSupport": true,
            },
            "publishDiagnostics": { "relatedInformation": false, "versionSupport": true },
            "inlayHint": {},
        },
    })
}

fn initialization_options(server: &str) -> Value {
    match server {
        "rust-analyzer" => json!({ "checkOnSave": true }),
        "clangd" => json!({ "clangdFileStatus": true }),
        _ => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs rust-analyzer in PATH: `cargo test -- --ignored lsp_completion`.
    #[test]
    #[ignore]
    fn lsp_completion_with_rust_analyzer() {
        let dir = std::env::temp_dir().join(format!("codium-lsp-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
        let text = "fn main() {\n    let v: Vec<i32> = Vec::new();\n    v.\n}\n";
        let file = dir.join("src/main.rs");
        std::fs::write(&file, text).unwrap();
        let root = dir.to_string_lossy().into_owned();
        let (tx, rx) = crossbeam_channel::unbounded();
        let client = LspClient::start(&Host::Local, &root, "rust-analyzer", "rust-analyzer", &[], tx).unwrap();
        let uri = crate::lsp::path_to_uri(&file.to_string_lossy());
        client.notify("textDocument/didOpen", json!({ "textDocument": { "uri": uri, "languageId": "rust", "version": 1, "text": text } }));
        let mut labels = Vec::new();
        for _ in 0..60 {
            let result = client.request("textDocument/completion", json!({ "textDocument": { "uri": uri }, "position": { "line": 2, "character": 6 } }));
            labels = crate::lsp::parse_completions(result.unwrap_or(Value::Null), "ra").into_iter().map(|i| i.label).collect();
            if labels.iter().any(|l| l.starts_with("push")) {
                break;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        let events: Vec<_> = rx.try_iter().map(|e| format!("{e:?}").chars().take(80).collect::<String>()).collect();
        client.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(labels.iter().any(|l| l.starts_with("push")), "labels: {:?}\nevents: {events:?}", labels.iter().take(10).collect::<Vec<_>>());
    }
}
