//! Command line: start the GUI, control a running editor, or serve MCP.

use std::process::ExitCode;

use serde_json::{Value, json};

use crate::control::{self, methods};

fn help() -> &'static str {
    t_file!("cli-help.txt")
}

/// Returns `None` when the GUI should start.
pub fn run(args: &[String]) -> Option<ExitCode> {
    let first = args.first()?;
    let rest = &args[1..];
    match first.as_str() {
        "-h" | "--help" => {
            print!("{}", help());
            Some(ExitCode::SUCCESS)
        }
        "-V" | "--version" => {
            println!("{}", crate::app::about_line());
            Some(ExitCode::SUCCESS)
        }
        "--new-window" | "gui" => None,
        "mcp" => Some(match control::mcp::run(rest.iter().any(|a| a == "--granular")) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        }),
        "methods" => {
            for method in methods::METHODS {
                println!("{:<16} {}", method.name, first_sentence(method.description));
            }
            Some(ExitCode::SUCCESS)
        }
        "help" => match rest.first().and_then(|m| methods::find(m)) {
            Some(method) => {
                println!("{}\n\n{}", method.name, method.description);
                for param in method.params {
                    let required = if param.required { t!(" (required)") } else { "" };
                    println!("  {:<14}{required} {}", param.name, param.description);
                }
                Some(ExitCode::SUCCESS)
            }
            None => {
                print!("{}", help());
                Some(ExitCode::SUCCESS)
            }
        },
        "instances" => {
            for path in control::instances() {
                println!("{}", path.display());
            }
            Some(ExitCode::SUCCESS)
        }
        "commands" => Some(call_and_print("commands", json!({}), rest)),
        "cmd" | "command" => {
            let Some(id) = rest.first() else {
                eprintln!("{}", t!("name a command: lumen-edit commands"));
                return Some(ExitCode::FAILURE);
            };
            Some(call_and_print("run_command", json!({ "command": id }), &rest[1..]))
        }
        "open" => {
            let Some(target) = rest.first() else {
                eprintln!("{}", t!("name a file"));
                return Some(ExitCode::FAILURE);
            };
            Some(call_and_print("open_file", open_params(target), &rest[1..]))
        }
        "call" => {
            let Some(name) = rest.first() else {
                eprintln!("{}", t!("name a method: lumen-edit methods"));
                return Some(ExitCode::FAILURE);
            };
            let Some(method) = methods::find(name) else {
                eprintln!("{}", t!("unknown method {name}", name = name));
                return Some(ExitCode::FAILURE);
            };
            Some(match parse_args(method, &rest[1..]) {
                Ok(params) => call_and_print(name, params, rest),
                Err(error) => fail(error),
            })
        }
        name if methods::find(name).is_some() => {
            let method = methods::find(name).unwrap();
            Some(match parse_args(method, rest) {
                Ok(params) => call_and_print(name, params, rest),
                Err(error) => fail(error),
            })
        }
        // A path: hand it to a running editor if there is one.
        path => {
            if path.starts_with('-') {
                eprintln!("{}", t!("unknown option {path}\n\n{HELP}", path = path, HELP = help()));
                return Some(ExitCode::FAILURE);
            }
            // A bare word that is not a file is a mistyped command, not a
            // reason to open a window.
            if !path.contains(['/', '.']) && !std::path::Path::new(path).exists() {
                eprintln!("{}", t!("unknown command {path}; see: lumen-edit methods", path = path));
                return Some(ExitCode::FAILURE);
            }
            if path.starts_with("ssh://") {
                let socket = control::find_socket()?;
                let rest = path.trim_start_matches("ssh://");
                let (target, dir) = rest.split_once('/').map(|(t, d)| (t, format!("/{d}"))).unwrap_or((rest, String::new()));
                let mut params = json!({ "target": target });
                if !dir.is_empty() {
                    params["path"] = json!(dir);
                }
                return Some(call_on(&socket, "connect_ssh", params, &[]));
            }
            let socket = control::find_socket()?;
            let (file, _, _) = split_location(path);
            let absolute = std::fs::canonicalize(&file).ok()?;
            if absolute.is_dir() {
                Some(call_on(&socket, "open_folder", json!({ "path": absolute.to_string_lossy(), "host": "local" }), &[]))
            } else {
                let mut params = open_params(path);
                params["path"] = json!(absolute.to_string_lossy());
                Some(call_on(&socket, "open_file", params, &[]))
            }
        }
    }
}

fn first_sentence(text: &str) -> &str {
    text.split_once(". ").map(|(s, _)| s).unwrap_or(text)
}

fn fail(error: String) -> ExitCode {
    eprintln!("{}", t!("error: {error}", error = error));
    ExitCode::FAILURE
}

/// `file:12:3` -> (file, Some(12), Some(3)).
fn split_location(target: &str) -> (String, Option<i64>, Option<i64>) {
    let mut parts: Vec<&str> = target.rsplitn(3, ':').collect();
    parts.reverse();
    let numbers: Vec<Option<i64>> = parts.iter().map(|p| p.parse().ok()).collect();
    match (parts.len(), numbers.as_slice()) {
        (3, [_, Some(line), Some(col)]) => (parts[0].to_string(), Some(*line), Some(*col)),
        (3, [_, _, Some(line)]) => (format!("{}:{}", parts[0], parts[1]), Some(*line), None),
        (2, [_, Some(line)]) => (parts[0].to_string(), Some(*line), None),
        _ => (target.to_string(), None, None),
    }
}

fn open_params(target: &str) -> Value {
    let (file, line, column) = split_location(target);
    let file = std::fs::canonicalize(&file).map(|p| p.to_string_lossy().into_owned()).unwrap_or(file);
    let mut params = json!({ "path": file });
    if let Some(line) = line {
        params["line"] = json!(line);
    }
    if let Some(column) = column {
        params["column"] = json!(column);
    }
    params
}

/// Accepts `key=value`, `--key value`, `--key=value`, `--flag`, JSON objects
/// and bare values for required params.
fn parse_args(method: &methods::Method, args: &[String]) -> Result<Value, String> {
    let mut normalized = Vec::new();
    let mut iter = args.iter().peekable();
    while let Some(arg) = iter.next() {
        if arg == "--json" {
            continue;
        }
        if let Some(name) = arg.strip_prefix("--") {
            if let Some((name, value)) = name.split_once('=') {
                normalized.push(format!("{name}={value}"));
                continue;
            }
            let param = method
                .params
                .iter()
                .find(|p| p.name == name)
                .ok_or_else(|| t!("{n1} has no parameter --{name}", n1 = method.name, name = name))?;
            let is_bool = matches!(param.kind, methods::Kind::Bool);
            match iter.peek() {
                Some(next) if !is_bool && !next.starts_with("--") => normalized.push(format!("{name}={}", iter.next().unwrap())),
                Some(next) if is_bool && matches!(next.as_str(), "true" | "false") => {
                    normalized.push(format!("{name}={}", iter.next().unwrap()))
                }
                _ if is_bool => normalized.push(format!("{name}=true")),
                _ => return Err(t!("--{name}: value required", name = name)),
            }
            continue;
        }
        normalized.push(arg.clone());
    }
    methods::parse_cli_params(method, &normalized)
}

fn call_and_print(method: &str, params: Value, args: &[String]) -> ExitCode {
    let launch = matches!(method, "open_file" | "open_folder" | "connect_ssh");
    match control::connect_or_launch(launch) {
        Ok(mut client) => print_result(method, client.call(method, params), args),
        Err(error) => fail(error),
    }
}

fn call_on(socket: &std::path::PathBuf, method: &str, params: Value, args: &[String]) -> ExitCode {
    match control::Client::connect(socket) {
        Ok(mut client) => print_result(method, client.call(method, params), args),
        Err(error) => fail(error),
    }
}

fn print_result(method: &str, result: Result<Value, String>, args: &[String]) -> ExitCode {
    let json_output = args.iter().any(|a| a == "--json");
    match result {
        Ok(value) if json_output => {
            println!("{}", serde_json::to_string_pretty(&value).unwrap_or_default());
            ExitCode::SUCCESS
        }
        // `exec` behaves like the command itself.
        Ok(value) if method == "exec" => {
            print!("{}", value["stdout"].as_str().unwrap_or(""));
            eprint!("{}", value["stderr"].as_str().unwrap_or(""));
            let code = value["exit_code"].as_i64().unwrap_or(1);
            ExitCode::from(code.clamp(0, 255) as u8)
        }
        Ok(value) => {
            let text = control::mcp::render(&value);
            if text.ends_with('\n') {
                print!("{text}")
            } else {
                println!("{text}")
            }
            ExitCode::SUCCESS
        }
        Err(error) => fail(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locations() {
        assert_eq!(split_location("src/main.rs:12:3"), ("src/main.rs".into(), Some(12), Some(3)));
        assert_eq!(split_location("src/main.rs:12"), ("src/main.rs".into(), Some(12), None));
        assert_eq!(split_location("src/main.rs"), ("src/main.rs".into(), None, None));
    }

    #[test]
    fn arguments() {
        let method = methods::find("search").unwrap();
        let params = parse_args(method, &["fn main".into(), "--regex".into(), "--include".into(), ".rs".into(), "limit=5".into()]).unwrap();
        assert_eq!(params, json!({ "query": "fn main", "regex": true, "include": ".rs", "limit": 5 }));
        let git = methods::find("git").unwrap();
        assert_eq!(parse_args(git, &["log --oneline -3".into()]).unwrap(), json!({ "args": ["log", "--oneline", "-3"] }));
    }
}
