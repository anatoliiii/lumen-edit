//! The single list of control methods. The CLI help and the MCP tool list are
//! generated from it, so both always match what the editor implements.

use serde_json::{Value, json};

#[derive(Clone, Copy)]
pub enum Kind {
    Str,
    Int,
    Bool,
    StrArray,
    Edits,
    Object,
}

pub struct Param {
    pub name: &'static str,
    pub kind: Kind,
    pub required: bool,
    pub description: &'static str,
}

pub struct Method {
    pub name: &'static str,
    pub description: &'static str,
    pub params: &'static [Param],
}

const fn p(name: &'static str, kind: Kind, required: bool, description: &'static str) -> Param {
    Param { name, kind, required, description }
}

use Kind::*;

const PATH_OPT: Param = p("path", Str, false, "File path (absolute or relative to the project root). Defaults to the active editor.");
const PATH_REQ: Param = p("path", Str, true, "File path (absolute or relative to the project root).");
const LINE: Param = p("line", Int, true, "1-based line number.");
const COLUMN: Param = p("column", Int, false, "1-based column (characters). Default 1.");
const SAVE: Param = p("save", Bool, false, "Save the file after the change.");
const TERMINAL: Param = p("terminal", Int, false, "Terminal index from terminal_list. Defaults to the active terminal.");

pub const METHODS: &[Method] = &[
    Method { name: "state", description: "Editor state: project and host, open files, active file and cursor, git branch and changes, terminals, language servers, problem counts.", params: &[] },
    Method { name: "commands", description: "List every GUI command id usable with run_command.", params: &[] },
    Method {
        name: "run_command",
        description: "Run any GUI command by id (see `commands`), exactly as from the menu or a shortcut: e.g. git_push, show_git, build, toggle_terminal, format.",
        params: &[p("command", Str, true, "Command id, e.g. git_sync.")],
    },
    Method {
        name: "open_folder",
        description: "Open a project folder. On an SSH host when `host` is `ssh:<target>` (reuses the connection) or local.",
        params: &[p("path", Str, true, "Folder path."), p("host", Str, false, "`local` (default when no remote project is open) or `ssh:<target>`. Defaults to the current project's host.")],
    },
    Method {
        name: "connect_ssh",
        description: "Connect to an SSH host (system OpenSSH, ~/.ssh/config) and optionally open a folder there. Password/2FA prompts appear in the GUI.",
        params: &[p("target", Str, true, "Host alias or user@host."), p("path", Str, false, "Remote folder to open after connecting.")],
    },
    Method { name: "close_workspace", description: "Close the current project.", params: &[] },
    Method { name: "open_file", description: "Open a file in an editor tab and optionally move the cursor.", params: &[PATH_REQ, p("line", Int, false, "1-based line."), COLUMN] },
    Method { name: "close_file", description: "Close a file tab (unsaved changes are kept unless force).", params: &[PATH_OPT, p("force", Bool, false, "Discard unsaved changes.")] },
    Method { name: "save", description: "Save a file, or all files.", params: &[PATH_OPT, p("all", Bool, false, "Save every modified file.")] },
    Method {
        name: "read_file",
        description: "Read a file. Returns the editor buffer (including unsaved changes) when the file is open, otherwise the file from the project host.",
        params: &[PATH_REQ, p("start_line", Int, false, "First line (1-based)."), p("end_line", Int, false, "Last line (inclusive)."), p("line_numbers", Bool, false, "Prefix lines with numbers.")],
    },
    Method { name: "write_file", description: "Replace the whole content of a file in the editor (opens it; undoable). Creates the file if missing.", params: &[PATH_REQ, p("text", Str, true, "New content."), SAVE] },
    Method {
        name: "edit",
        description: "Apply text edits to a file in the editor (opens it; undoable as one step). Positions are 1-based; end is exclusive.",
        params: &[PATH_REQ, p("edits", Edits, true, "List of {start_line, start_column, end_line, end_column, text}."), SAVE],
    },
    Method {
        name: "replace_text",
        description: "Find and replace in a file in the editor.",
        params: &[PATH_REQ, p("find", Str, true, "Text or regex to find."), p("replace", Str, true, "Replacement ($1 groups with regex)."), p("regex", Bool, false, "Treat `find` as a regex."), p("all", Bool, false, "Replace all occurrences (default true)."), SAVE],
    },
    Method { name: "insert_text", description: "Type text at the cursor(s) of the active editor.", params: &[p("text", Str, true, "Text to insert.")] },
    Method {
        name: "set_cursor",
        description: "Move the cursor or select a range in a file (opens it).",
        params: &[PATH_OPT, LINE, COLUMN, p("end_line", Int, false, "Selection end line."), p("end_column", Int, false, "Selection end column.")],
    },
    Method { name: "create_file", description: "Create a file (and parent folders) on the project host and open it.", params: &[PATH_REQ, p("text", Str, false, "Initial content.")] },
    Method { name: "create_folder", description: "Create a folder on the project host.", params: &[PATH_REQ] },
    Method { name: "delete_path", description: "Delete a file or folder on the project host.", params: &[PATH_REQ] },
    Method { name: "rename_path", description: "Rename or move a file or folder on the project host.", params: &[p("from", Str, true, "Existing path."), p("to", Str, true, "New path.")] },
    Method { name: "list_dir", description: "List a directory on the project host.", params: &[p("path", Str, false, "Directory (default: project root).")] },
    Method { name: "list_files", description: "List project files (respects .gitignore), optionally fuzzy-filtered.", params: &[p("query", Str, false, "Fuzzy filter."), p("limit", Int, false, "Max results (default 200).")] },
    Method {
        name: "search",
        description: "Search text in the project (agent, ripgrep or grep on the project host).",
        params: &[
            p("query", Str, true, "Text or regex."),
            p("regex", Bool, false, "Regex search."),
            p("case_sensitive", Bool, false, "Match case."),
            p("whole_word", Bool, false, "Whole words only."),
            p("include", Str, false, "Comma-separated path fragments to keep, e.g. `.rs,src/`."),
            p("limit", Int, false, "Max matches (default 500)."),
            p("show", Bool, false, "Also show the results in the GUI search panel."),
        ],
    },
    Method { name: "diagnostics", description: "Errors and warnings from language servers and linters.", params: &[p("path", Str, false, "Only this file.")] },
    Method { name: "hover", description: "Language server hover (type, docs) at a position.", params: &[PATH_REQ, LINE, COLUMN] },
    Method { name: "definition", description: "Go-to-definition locations for the symbol at a position.", params: &[PATH_REQ, LINE, COLUMN] },
    Method { name: "references", description: "All references to the symbol at a position.", params: &[PATH_REQ, LINE, COLUMN] },
    Method { name: "completion", description: "Completion items at a position.", params: &[PATH_REQ, LINE, COLUMN, p("limit", Int, false, "Max items (default 50).")] },
    Method {
        name: "lsp",
        description: "Send any textDocument/* request to the file's language server. `params` is merged over {textDocument, position}.",
        params: &[PATH_REQ, p("method", Str, true, "e.g. textDocument/documentSymbol."), p("line", Int, false, "1-based line."), COLUMN, p("params", Object, false, "Extra request params.")],
    },
    Method { name: "format", description: "Format a file with its language server.", params: &[PATH_OPT, SAVE] },
    Method {
        name: "exec",
        description: "Run a shell command on the project host (local or SSH) and return stdout, stderr and the exit code. Not shown in the GUI; use terminal_new for that.",
        params: &[p("command", Str, true, "Shell command (sh -c)."), p("cwd", Str, false, "Working directory (default: project root)."), p("timeout_ms", Int, false, "Kill after this many ms (default 120000).")],
    },
    Method { name: "terminal_list", description: "List integrated terminals.", params: &[] },
    Method {
        name: "terminal_new",
        description: "Open an integrated terminal on the project host, optionally running a command (the shell stays open afterwards).",
        params: &[p("command", Str, false, "Command to run."), p("cwd", Str, false, "Working directory."), p("title", Str, false, "Tab title.")],
    },
    Method { name: "terminal_write", description: "Type into a terminal.", params: &[TERMINAL, p("text", Str, true, "Text to send."), p("enter", Bool, false, "Press Enter afterwards (default true).")] },
    Method { name: "terminal_read", description: "Read the visible screen of a terminal.", params: &[TERMINAL, p("wait_ms", Int, false, "Wait this long before reading.")] },
    Method { name: "terminal_kill", description: "Stop the process in a terminal.", params: &[TERMINAL, p("close", Bool, false, "Also close the tab.")] },
    Method { name: "tasks", description: "List detected and configured tasks (run/build/test).", params: &[] },
    Method { name: "run_task", description: "Run a task in a terminal (unsaved files are saved first).", params: &[p("label", Str, true, "Task label from `tasks`.")] },
    Method { name: "git_status", description: "Branch, upstream, ahead/behind, merge state and changed files.", params: &[] },
    Method { name: "git", description: "Run git with arguments in the repository on the project host; the GUI refreshes afterwards.", params: &[p("args", StrArray, true, "Arguments, e.g. [\"log\", \"--oneline\", \"-5\"].")] },
    Method {
        name: "git_commit",
        description: "Commit staged changes (or everything when nothing is staged).",
        params: &[p("message", Str, true, "Commit message."), p("amend", Bool, false, "Amend the last commit."), p("all", Bool, false, "Stage all changes first.")],
    },
    Method { name: "git_diff", description: "Diff of the working tree or index.", params: &[p("path", Str, false, "Only this path."), p("staged", Bool, false, "Diff the index instead of the working tree.")] },
    Method { name: "git_log", description: "Recent commits.", params: &[p("limit", Int, false, "Number of commits (default 30)."), p("path", Str, false, "Only commits touching this path.")] },
    Method {
        name: "agent",
        description: "The pi agent tab: open it, send a task, read the transcript, stop it, list or switch sessions.",
        params: &[p("action", Str, true, "open | prompt | transcript | abort | sessions | switch (text = session path) | new_session"), p("text", Str, false, "Task text for prompt."), p("last", Int, false, "Transcript: only the last N items (default 30).")],
    },
    Method { name: "screenshot", description: "Capture the editor window as PNG.", params: &[p("path", Str, false, "Where to save (default: runtime dir).")] },
    Method { name: "notify", description: "Show a notification in the editor.", params: &[p("text", Str, true, "Message."), p("error", Bool, false, "Show as error.")] },
    Method { name: "quit", description: "Quit the editor (asks about unsaved files in the GUI).", params: &[] },
];

pub fn find(name: &str) -> Option<&'static Method> {
    METHODS.iter().find(|m| m.name == name)
}

fn schema(kind: Kind) -> Value {
    match kind {
        Str => json!({ "type": "string" }),
        Int => json!({ "type": "integer" }),
        Bool => json!({ "type": "boolean" }),
        StrArray => json!({ "type": "array", "items": { "type": "string" } }),
        Object => json!({ "type": "object" }),
        Edits => json!({
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "start_line": { "type": "integer" },
                    "start_column": { "type": "integer" },
                    "end_line": { "type": "integer" },
                    "end_column": { "type": "integer" },
                    "text": { "type": "string" }
                },
                "required": ["start_line", "start_column", "end_line", "end_column", "text"]
            }
        }),
    }
}

pub fn input_schema(method: &Method) -> Value {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();
    for param in method.params {
        let mut s = schema(param.kind);
        s["description"] = json!(param.description);
        properties.insert(param.name.into(), s);
        if param.required {
            required.push(param.name);
        }
    }
    json!({ "type": "object", "properties": properties, "required": required })
}

/// Turns `key=value` CLI arguments into params, typed by the method spec.
pub fn parse_cli_params(method: &Method, args: &[String]) -> Result<Value, String> {
    let mut params = serde_json::Map::new();
    // Bare values fill required params first, then optional ones in order.
    let mut positional = method.params.iter().filter(|p| p.required).chain(method.params.iter().filter(|p| !p.required));
    for arg in args {
        if arg.starts_with('{') {
            let value: Value = serde_json::from_str(arg).map_err(|e| format!("JSON: {e}"))?;
            if let Value::Object(map) = value {
                params.extend(map);
            }
            continue;
        }
        let (name, raw) = match arg.split_once('=') {
            Some((name, raw)) if method.params.iter().any(|p| p.name == name) => (name.to_string(), raw.to_string()),
            _ => {
                let param = positional.find(|p| !params.contains_key(p.name)).ok_or_else(|| format!("лишний аргумент: {arg}"))?;
                (param.name.to_string(), arg.clone())
            }
        };
        let param = method.params.iter().find(|p| p.name == name).unwrap();
        let value = match param.kind {
            Str => json!(raw),
            Int => json!(raw.parse::<i64>().map_err(|_| format!("{name}: ожидалось число"))?),
            Bool => json!(matches!(raw.as_str(), "1" | "true" | "yes" | "on" | "")),
            StrArray => match serde_json::from_str::<Value>(&raw) {
                Ok(v @ Value::Array(_)) => v,
                _ => json!(shell_words::split(&raw).map_err(|e| e.to_string())?),
            },
            Edits | Object => serde_json::from_str(&raw).map_err(|e| format!("{name}: JSON: {e}"))?,
        };
        params.insert(name, value);
    }
    for param in method.params.iter().filter(|p| p.required) {
        if !params.contains_key(param.name) {
            return Err(format!("не хватает параметра {}", param.name));
        }
    }
    Ok(Value::Object(params))
}
