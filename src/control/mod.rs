//! Remote control of a running editor: a Unix socket speaking JSON lines,
//! used by the CLI (`codium-rs call ...`) and the MCP server (`codium-rs mcp`).
//!
//! Request:  `{"id": 1, "method": "open_file", "params": {"path": "src/main.rs"}}`
//! Response: `{"id": 1, "result": ...}` or `{"id": 1, "error": "..."}`

pub mod mcp;
pub mod methods;

use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
    time::Duration,
};

use crossbeam_channel::{Sender, bounded};
use serde_json::{Value, json};

pub type Reply = Sender<Result<Value, String>>;

pub struct ControlRequest {
    pub method: String,
    pub params: Value,
    pub reply: Reply,
}

pub fn socket_dir() -> PathBuf {
    dirs::runtime_dir().unwrap_or_else(std::env::temp_dir).join("codium-rs")
}

fn current_link() -> PathBuf {
    socket_dir().join("current.sock")
}

/// Starts the control server. `dispatch` hands requests to the UI thread.
pub fn serve(dispatch: impl Fn(ControlRequest) + Send + Sync + 'static) -> Option<PathBuf> {
    let dir = socket_dir();
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("ctl-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).ok()?;
    // The most recently started editor is the default target.
    let link = current_link();
    let _ = std::fs::remove_file(&link);
    let _ = std::os::unix::fs::symlink(&path, &link);

    let dispatch = std::sync::Arc::new(dispatch);
    std::thread::Builder::new()
        .name("control".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let dispatch = dispatch.clone();
                std::thread::spawn(move || handle_connection(stream, &*dispatch));
            }
        })
        .ok()?;
    Some(path)
}

fn handle_connection(stream: UnixStream, dispatch: &(dyn Fn(ControlRequest) + Send + Sync)) {
    let Ok(mut writer) = stream.try_clone() else { return };
    let reader = BufReader::new(stream);
    for line in reader.lines().map_while(Result::ok) {
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Value>(&line) {
            Ok(request) => {
                let id = request.get("id").cloned().unwrap_or(Value::Null);
                let method = request.get("method").and_then(Value::as_str).unwrap_or("").to_string();
                let params = request.get("params").cloned().unwrap_or(json!({}));
                let (tx, rx) = bounded(1);
                dispatch(ControlRequest { method, params, reply: tx });
                match rx.recv_timeout(Duration::from_secs(600)) {
                    Ok(Ok(result)) => json!({ "id": id, "result": result }),
                    Ok(Err(error)) => json!({ "id": id, "error": error }),
                    Err(_) => json!({ "id": id, "error": "редактор не ответил" }),
                }
            }
            Err(error) => json!({ "id": null, "error": format!("bad request: {error}") }),
        };
        if writeln!(writer, "{response}").is_err() {
            break;
        }
    }
}

/// Removes the socket of this process (called on exit).
pub fn cleanup() {
    let path = socket_dir().join(format!("ctl-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    if std::fs::read_link(current_link()).ok().as_ref() == Some(&path) {
        let _ = std::fs::remove_file(current_link());
    }
}

/// Finds a running editor: `$CODIUM_SOCKET`, the current one, or any live one.
pub fn find_socket() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("CODIUM_SOCKET").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(path));
    }
    let alive = |path: &PathBuf| UnixStream::connect(path).is_ok();
    let link = current_link();
    if alive(&link) {
        return Some(link);
    }
    instances().into_iter().find(alive)
}

pub fn instances() -> Vec<PathBuf> {
    let mut list: Vec<PathBuf> = std::fs::read_dir(socket_dir())
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("ctl-")))
        .filter(|p| {
            let live = UnixStream::connect(p).is_ok();
            if !live {
                let _ = std::fs::remove_file(p);
            }
            live
        })
        .collect();
    list.sort();
    list
}

/// A connection to a running editor.
pub struct Client {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
    next_id: u64,
}

impl Client {
    pub fn connect(path: &PathBuf) -> Result<Self, String> {
        let stream = UnixStream::connect(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
        Ok(Self { writer: stream, reader, next_id: 1 })
    }

    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        writeln!(self.writer, "{}", json!({ "id": id, "method": method, "params": params })).map_err(|e| e.to_string())?;
        let mut line = String::new();
        self.reader.read_line(&mut line).map_err(|e| e.to_string())?;
        if line.is_empty() {
            return Err("соединение с редактором закрыто".into());
        }
        let response: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
        match response.get("error") {
            Some(error) if !error.is_null() => Err(error.as_str().map(str::to_string).unwrap_or_else(|| error.to_string())),
            _ => Ok(response.get("result").cloned().unwrap_or(Value::Null)),
        }
    }
}

/// Connects to a running editor, starting one if needed.
pub fn connect_or_launch(launch: bool) -> Result<Client, String> {
    if let Some(path) = find_socket() {
        return Client::connect(&path);
    }
    if !launch {
        return Err("редактор не запущен (запустите codium-rs или используйте --launch)".into());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    std::process::Command::new(exe)
        .env_remove("CODIUM_SOCKET")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    for _ in 0..100 {
        std::thread::sleep(Duration::from_millis(100));
        if let Some(path) = find_socket() {
            return Client::connect(&path);
        }
    }
    Err("редактор не запустился".into())
}
