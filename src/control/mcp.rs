//! MCP server on stdio (`lumen-edit mcp`). Every control method is exposed as
//! a tool; calls are forwarded to the running editor, which is started on
//! demand.

use std::io::{BufRead, Write};

use serde_json::{Value, json};

use super::{Client, methods};

const PROTOCOL_VERSION: &str = "2025-06-18";

/// `granular`: one tool per method, for clients that discover tools on
/// demand (pi's `tool_search`); otherwise the compact grouped catalog.
pub fn run(granular: bool) -> Result<(), String> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let mut client: Option<Client> = None;

    for line in stdin.lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            write_message(&mut stdout, &json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": "parse error" } }));
            continue;
        };
        let id = message.get("id").cloned();
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        // Notifications get no response.
        let Some(id) = id else { continue };

        let response = match method {
            "initialize" => {
                let version = params.get("protocolVersion").and_then(Value::as_str).unwrap_or(PROTOCOL_VERSION);
                Ok(json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": "lumen-edit", "version": env!("CARGO_PKG_VERSION") },
                    "instructions": "Controls the running Lumen Edit editor (local or SSH projects). Start with `state`. Paths may be relative to the project root. Lines and columns are 1-based. File edits go through the editor buffers and are undoable; call `save` to write them.",
                }))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": if granular { granular_tools() } else { tools() } })),
            "tools/call" => Ok(call_tool(&mut client, &params, granular)),
            _ => Err((-32601, format!("method not found: {method}"))),
        };
        let message = match response {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err((code, text)) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": text } }),
        };
        write_message(&mut stdout, &message);
    }
    Ok(())
}

fn write_message(stdout: &mut std::io::Stdout, message: &Value) {
    let mut out = stdout.lock();
    let _ = writeln!(out, "{message}");
    let _ = out.flush();
}

/// MCP tools group control methods by entity, selected with `action`, so
/// the catalog stays small for the agent's context.
pub const GROUPS: &[(&str, &str, &[(&str, &str)])] = &[
    (
        "state",
        "Editor state: project/host, open files, cursor, git summary, terminals, language servers, problem counts. Call first.",
        &[("", "state")],
    ),
    (
        "workspace",
        "Project and window.",
        &[
            ("open_folder", "open_folder"),
            ("connect_ssh", "connect_ssh"),
            ("close", "close_workspace"),
            ("notify", "notify"),
            ("screenshot", "screenshot"),
            ("quit", "quit"),
            ("pulse", "pulse"),
        ],
    ),
    (
        "file",
        "Files through the editor (buffers include unsaved changes; edits are undoable, call save to write). Paths may be relative to the project root; lines/columns are 1-based.",
        &[
            ("read", "read_file"),
            ("open", "open_file"),
            ("edit", "edit"),
            ("replace", "replace_text"),
            ("write", "write_file"),
            ("insert", "insert_text"),
            ("cursor", "set_cursor"),
            ("save", "save"),
            ("close", "close_file"),
            ("create", "create_file"),
            ("mkdir", "create_folder"),
            ("rename", "rename_path"),
            ("delete", "delete_path"),
            ("list_dir", "list_dir"),
            ("list_files", "list_files"),
            ("transfer", "transfer"),
        ],
    ),
    ("search", "Text search across the project.", &[("", "search")]),
    (
        "code",
        "Language intelligence (LSP) and diagnostics.",
        &[
            ("diagnostics", "diagnostics"),
            ("hover", "hover"),
            ("definition", "definition"),
            ("references", "references"),
            ("completion", "completion"),
            ("format", "format"),
            ("lsp", "lsp"),
        ],
    ),
    ("exec", "Run a shell command on the project host (local or SSH) and get stdout/stderr/exit code.", &[("", "exec")]),
    (
        "terminal",
        "Integrated terminals (visible in the GUI).",
        &[
            ("list", "terminal_list"),
            ("new", "terminal_new"),
            ("write", "terminal_write"),
            ("read", "terminal_read"),
            ("kill", "terminal_kill"),
        ],
    ),
    ("task", "Project tasks (run/build/test).", &[("list", "tasks"), ("run", "run_task")]),
    (
        "git",
        "Git in the project repository.",
        &[("status", "git_status"), ("diff", "git_diff"), ("log", "git_log"), ("commit", "git_commit"), ("run", "git")],
    ),
    ("command", "Any GUI menu/shortcut command by id.", &[("list", "commands"), ("run", "run_command")]),
    ("agent", "The pi coding agent tab in the editor.", &[("", "agent")]),
];

fn first_sentence(text: &str) -> &str {
    text.split_once(". ").map(|(s, _)| s).unwrap_or(text).trim_end_matches('.')
}

pub fn tools() -> Vec<Value> {
    GROUPS
        .iter()
        .map(|(name, description, actions)| {
            let single = actions.len() == 1 && actions[0].0.is_empty();
            let mut properties = serde_json::Map::new();
            let mut required: Vec<&str> = Vec::new();
            let mut lines = Vec::new();
            for (action, method_name) in actions.iter() {
                let method = methods::find(method_name).expect("grouped method exists");
                let schema = methods::input_schema(method);
                if let Some(props) = schema["properties"].as_object() {
                    for (key, value) in props {
                        properties.entry(key.clone()).or_insert_with(|| value.clone());
                    }
                }
                if single {
                    required.extend(method.params.iter().filter(|p| p.required).map(|p| p.name));
                } else {
                    let args: Vec<String> =
                        method.params.iter().map(|p| if p.required { p.name.to_string() } else { format!("{}?", p.name) }).collect();
                    lines.push(format!("{action}({}): {}", args.join(", "), first_sentence(method.description)));
                }
            }
            let description = if single {
                let method = methods::find(actions[0].1).unwrap();
                format!("{description} {}", method.description)
            } else {
                let names: Vec<&str> = actions.iter().map(|(a, _)| *a).collect();
                properties.insert("action".into(), json!({ "type": "string", "enum": names }));
                required.push("action");
                format!("{description}\n{}", lines.join("\n"))
            };
            let mut tool = json!({
                "name": name,
                "description": description,
                "inputSchema": { "type": "object", "properties": properties, "required": required },
            });
            if matches!(*name, "state" | "search") {
                tool["annotations"] = json!({ "readOnlyHint": true });
            }
            tool
        })
        .collect()
}

/// Maps a grouped tool call to the control method.
pub fn resolve(tool: &str, arguments: &Value) -> Result<&'static str, String> {
    let (_, _, actions) = GROUPS.iter().find(|(name, _, _)| *name == tool).ok_or_else(|| format!("unknown tool: {tool}"))?;
    if actions.len() == 1 && actions[0].0.is_empty() {
        return Ok(actions[0].1);
    }
    let action = arguments.get("action").and_then(Value::as_str).ok_or("missing `action`")?;
    actions.iter().find(|(a, _)| *a == action).map(|(_, m)| *m).ok_or_else(|| {
        let names: Vec<&str> = actions.iter().map(|(a, _)| *a).collect();
        format!("unknown action `{action}` for {tool}; expected one of: {}", names.join(", "))
    })
}

pub fn granular_tools() -> Vec<Value> {
    methods::METHODS
        .iter()
        .map(|m| json!({ "name": m.name, "description": m.description, "inputSchema": methods::input_schema(m) }))
        .collect()
}

fn call_tool(client: &mut Option<Client>, params: &Value, granular: bool) -> Value {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let mut arguments = params.get("arguments").cloned().unwrap_or(json!({}));
    let resolved = if granular {
        methods::find(name).map(|m| m.name).ok_or_else(|| format!("unknown tool: {name}"))
    } else {
        resolve(name, &arguments)
    };
    let method = match resolved {
        Ok(method) => method,
        Err(error) => return tool_error(error),
    };
    if let Value::Object(map) = &mut arguments {
        map.remove("action");
    }
    let mut result = None;
    // Reconnect once if the editor was restarted.
    for _ in 0..2 {
        if client.is_none() {
            match super::connect_or_launch(std::env::var_os("LUMEN_MCP_NO_LAUNCH").is_none()) {
                Ok(c) => *client = Some(c),
                Err(error) => return tool_error(error),
            }
        }
        match client.as_mut().unwrap().call(method, arguments.clone()) {
            Err(error) if error.contains("закрыто") || error.contains("Broken pipe") => *client = None,
            other => {
                result = Some(other);
                break;
            }
        }
    }
    match result {
        Some(Ok(value)) => {
            let mut content = vec![json!({ "type": "text", "text": render(&value) })];
            if method == "screenshot"
                && let Some(path) = value.get("path").and_then(Value::as_str)
                && let Ok(data) = std::fs::read(path)
            {
                content.push(json!({ "type": "image", "data": base64(&data), "mimeType": "image/png" }));
            }
            json!({ "content": content })
        }
        Some(Err(error)) => tool_error(error),
        None => tool_error("редактор недоступен".into()),
    }
}

fn tool_error(text: String) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": true })
}

/// Plain strings stay readable; everything else is pretty JSON.
pub fn render(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    }
}

pub fn base64(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[n as usize & 63] as char } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64_matches_rfc() {
        assert_eq!(super::base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(super::base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(super::base64(b"foob"), "Zm9vYg==");
    }

    #[test]
    fn grouped_tools_cover_every_method() {
        let tools = super::tools();
        assert!(tools.len() <= 12, "catalog too large: {}", tools.len());
        for method in crate::control::methods::METHODS {
            if method.name == "methods" {
                continue;
            }
            assert!(
                super::GROUPS.iter().any(|(_, _, actions)| actions.iter().any(|(_, m)| *m == method.name)),
                "{} is not exposed over MCP",
                method.name
            );
        }
        let json = serde_json::to_string(&tools).unwrap();
        // Keep the tool catalog well under ~10k tokens.
        assert!(json.len() < 24_000, "catalog is {} bytes", json.len());
    }

    #[test]
    fn resolves_actions() {
        use serde_json::json;
        assert_eq!(super::resolve("file", &json!({ "action": "read" })).unwrap(), "read_file");
        assert_eq!(super::resolve("exec", &json!({})).unwrap(), "exec");
        assert!(super::resolve("git", &json!({ "action": "nope" })).is_err());
    }
}
