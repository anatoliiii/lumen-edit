//! Shell script diagnostics from `shellcheck` (a native binary), used in place
//! of the Node-based bash language server.

use serde::Deserialize;

use super::{Diagnostic, Severity};
use crate::{host::Host, text::Buffer};

#[derive(Deserialize)]
struct Output {
    comments: Vec<Comment>,
}

#[derive(Deserialize)]
struct Comment {
    line: usize,
    column: usize,
    #[serde(rename = "endLine")]
    end_line: usize,
    #[serde(rename = "endColumn")]
    end_column: usize,
    level: String,
    code: u32,
    message: String,
}

/// Runs shellcheck on `text`; returns raw (line, col) based results which
/// are converted against the buffer by the caller.
pub fn run(host: &Host, cwd: &str, text: &str) -> Result<Vec<(usize, usize, usize, usize, Severity, String)>, String> {
    let args: Vec<String> = ["-f", "json1", "-x", "-"].iter().map(|s| s.to_string()).collect();
    let mut command = host.command(cwd, "shellcheck", &args);
    // shellcheck exits with 1 when it found issues; read stdout regardless.
    use std::io::Write;
    use std::process::Stdio;
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let mut stdin = child.stdin.take().unwrap();
    let data = text.as_bytes().to_vec();
    std::thread::spawn(move || {
        let _ = stdin.write_all(&data);
    });
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    let parsed: Output = serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())?;
    Ok(parsed
        .comments
        .into_iter()
        .map(|c| {
            let severity = match c.level.as_str() {
                "error" => Severity::Error,
                "warning" => Severity::Warning,
                "info" => Severity::Info,
                _ => Severity::Hint,
            };
            (c.line, c.column, c.end_line, c.end_column, severity, format!("{} [SC{}]", c.message, c.code))
        })
        .collect())
}

pub fn to_diagnostics(buffer: &Buffer, raw: Vec<(usize, usize, usize, usize, Severity, String)>) -> Vec<Diagnostic> {
    raw.into_iter()
        .map(|(line, col, end_line, end_col, severity, message)| {
            let start = buffer.char_at(line.saturating_sub(1), col.saturating_sub(1));
            let end = buffer.char_at(end_line.saturating_sub(1), end_col.saturating_sub(1)).max(start + 1);
            Diagnostic { range: start..end.min(buffer.len_chars()), severity, message, source: "shellcheck".into(), raw: None }
        })
        .collect()
}
