//! Language server support: which servers to run, position conversion and
//! helpers that turn LSP payloads into editor data.

pub mod client;
pub mod shellcheck;

use std::ops::Range;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{syntax::Lang, text::Buffer};

/// A language server the editor knows how to start. All defaults are native
/// binaries — no Node.js servers.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServerSpec {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// LSP language ids handled by this server.
    pub languages: Vec<String>,
    /// Only start when the named server is unavailable.
    #[serde(default)]
    pub fallback_for: Option<String>,
}

/// A command that installs one of the default servers, shown when it is
/// missing or fails to start.
pub fn install_hint(name: &str) -> Option<&'static str> {
    Some(match name {
        "rust-analyzer" => "rustup component add rust-analyzer",
        "clangd" if cfg!(target_os = "macos") => "xcode-select --install",
        "clangd" if cfg!(windows) => "winget install LLVM.LLVM",
        "clangd" => "sudo apt install clangd",
        "gopls" => "go install golang.org/x/tools/gopls@latest",
        "ty" => "uv tool install ty",
        "ruff" => "uv tool install ruff",
        "pylsp" => "pipx install python-lsp-server",
        "csharp-ls" => "dotnet tool install -g csharp-ls",
        "tsgo" => "npm install -g @typescript/native-preview",
        "biome" => "npm install -g @biomejs/biome",
        "taplo" => "cargo install taplo-cli --locked",
        "sourcekit-lsp" if cfg!(target_os = "macos") => "xcode-select --install",
        _ => return None,
    })
}

pub fn default_servers() -> Vec<ServerSpec> {
    let spec = |name: &str, command: &str, args: &[&str], languages: &[&str], fallback_for: Option<&str>| ServerSpec {
        name: name.into(),
        command: command.into(),
        args: args.iter().map(|s| s.to_string()).collect(),
        languages: languages.iter().map(|s| s.to_string()).collect(),
        fallback_for: fallback_for.map(str::to_string),
    };
    vec![
        spec("rust-analyzer", "rust-analyzer", &[], &["rust"], None),
        spec("clangd", "clangd", &["--background-index", "--header-insertion=never"], &["c", "cpp"], None),
        spec("gopls", "gopls", &[], &["go"], None),
        spec("ty", "ty", &["server"], &["python"], None),
        spec("ruff", "ruff", &["server"], &["python"], None),
        spec("pylsp", "pylsp", &[], &["python"], Some("ty")),
        spec("csharp-ls", "csharp-ls", &[], &["csharp"], None),
        spec("omnisharp", "OmniSharp", &["-lsp"], &["csharp"], Some("csharp-ls")),
        spec("tsgo", "tsgo", &["--lsp", "--stdio"], &["typescript", "typescriptreact", "javascript"], None),
        spec("biome", "biome", &["lsp-proxy"], &["typescript", "typescriptreact", "javascript", "css", "json"], None),
        spec("superhtml", "superhtml", &["lsp"], &["html"], None),
        spec("taplo", "taplo", &["lsp", "stdio"], &["toml"], None),
    ]
}

pub fn path_to_uri(path: &str) -> String {
    crate::platform::path_to_uri(path)
}

pub fn uri_to_path(uri: &str) -> Option<String> {
    crate::platform::uri_to_path(uri)
}

/// Char index -> LSP position (UTF-8 bytes or UTF-16 units per server).
pub fn to_position(buffer: &Buffer, char_index: usize, utf8: bool) -> lsp_types::Position {
    let (line, col) = buffer.line_col(char_index);
    let line_start = buffer.rope.line_to_char(line);
    let prefix = buffer.rope.slice(line_start..line_start + col);
    let character = if utf8 { prefix.len_bytes() } else { prefix.chars().map(char::len_utf16).sum() };
    lsp_types::Position { line: line as u32, character: character as u32 }
}

pub fn from_position(buffer: &Buffer, position: &lsp_types::Position, utf8: bool) -> usize {
    let line = position.line as usize;
    if line >= buffer.line_count() {
        return buffer.len_chars();
    }
    let line_start = buffer.rope.line_to_char(line);
    let mut units = 0usize;
    let target = position.character as usize;
    for (offset, c) in buffer.rope.line(line).chars().enumerate() {
        if units >= target || c == '\n' {
            return line_start + offset;
        }
        units += if utf8 { c.len_utf8() } else { c.len_utf16() };
    }
    line_start + buffer.line_len(line)
}

pub fn from_range(buffer: &Buffer, range: &lsp_types::Range, utf8: bool) -> Range<usize> {
    from_position(buffer, &range.start, utf8)..from_position(buffer, &range.end, utf8)
}

pub fn to_range(buffer: &Buffer, range: Range<usize>, utf8: bool) -> lsp_types::Range {
    lsp_types::Range { start: to_position(buffer, range.start, utf8), end: to_position(buffer, range.end, utf8) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
    Info,
    Hint,
}

#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub range: Range<usize>,
    pub severity: Severity,
    pub message: String,
    pub source: String,
    /// Original payload, needed to request code actions.
    pub raw: Option<lsp_types::Diagnostic>,
}

pub fn convert_diagnostics(buffer: &Buffer, diagnostics: &[lsp_types::Diagnostic], utf8: bool, default_source: &str) -> Vec<Diagnostic> {
    diagnostics
        .iter()
        .map(|d| {
            let severity = match d.severity {
                Some(lsp_types::DiagnosticSeverity::ERROR) | None => Severity::Error,
                Some(lsp_types::DiagnosticSeverity::WARNING) => Severity::Warning,
                Some(lsp_types::DiagnosticSeverity::INFORMATION) => Severity::Info,
                _ => Severity::Hint,
            };
            let code = match &d.code {
                Some(lsp_types::NumberOrString::String(code)) => format!(" [{code}]"),
                Some(lsp_types::NumberOrString::Number(code)) => format!(" [{code}]"),
                None => String::new(),
            };
            Diagnostic {
                range: from_range(buffer, &d.range, utf8),
                severity,
                message: format!("{}{code}", d.message),
                source: d.source.clone().unwrap_or_else(|| default_source.to_string()),
                raw: Some(d.clone()),
            }
        })
        .collect()
}

#[derive(Clone, Debug)]
pub struct CompletionItem {
    pub label: String,
    pub detail: String,
    pub kind: Option<lsp_types::CompletionItemKind>,
    pub filter_text: String,
    pub sort_text: String,
    /// Replacement range and text; `None` range = word before the cursor.
    pub edit_range: Option<lsp_types::Range>,
    pub insert_text: String,
    pub snippet: bool,
    pub additional_edits: Vec<lsp_types::TextEdit>,
    pub documentation: Option<String>,
    pub utf8: bool,
}

pub fn parse_completions(value: Value, _server: &str) -> Vec<CompletionItem> {
    let items: Vec<Value> = match value {
        Value::Array(items) => items,
        Value::Object(ref object) => object.get("items").and_then(Value::as_array).cloned().unwrap_or_default(),
        _ => Vec::new(),
    };
    items
        .into_iter()
        .filter_map(|raw| {
            let item: lsp_types::CompletionItem = serde_json::from_value(raw.clone()).ok()?;
            let snippet = item.insert_text_format == Some(lsp_types::InsertTextFormat::SNIPPET);
            let (edit_range, insert_text) = match &item.text_edit {
                Some(lsp_types::CompletionTextEdit::Edit(edit)) => (Some(edit.range), edit.new_text.clone()),
                Some(lsp_types::CompletionTextEdit::InsertAndReplace(edit)) => (Some(edit.replace), edit.new_text.clone()),
                None => (None, item.insert_text.clone().unwrap_or_else(|| item.label.clone())),
            };
            let mut detail = item.detail.clone().unwrap_or_default();
            if let Some(details) = &item.label_details
                && let Some(d) = &details.detail
            {
                detail = format!("{d} {detail}");
            }
            Some(CompletionItem {
                filter_text: item.filter_text.clone().unwrap_or_else(|| item.label.clone()),
                sort_text: item.sort_text.clone().unwrap_or_else(|| item.label.clone()),
                label: item.label.clone(),
                detail: detail.trim().to_string(),
                kind: item.kind,
                edit_range,
                insert_text,
                snippet,
                additional_edits: item.additional_text_edits.clone().unwrap_or_default(),
                documentation: item.documentation.as_ref().map(documentation_text),
                utf8: false,
            })
        })
        .collect()
}

pub fn documentation_text(doc: &lsp_types::Documentation) -> String {
    match doc {
        lsp_types::Documentation::String(s) => s.clone(),
        lsp_types::Documentation::MarkupContent(m) => m.value.clone(),
    }
}

pub fn hover_text(value: &Value) -> Option<String> {
    let hover: lsp_types::Hover = serde_json::from_value(value.clone()).ok()?;
    let text = match hover.contents {
        lsp_types::HoverContents::Scalar(s) => marked_string(&s),
        lsp_types::HoverContents::Array(list) => list.iter().map(marked_string).collect::<Vec<_>>().join("\n\n"),
        lsp_types::HoverContents::Markup(m) => m.value,
    };
    let text = text.trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn marked_string(s: &lsp_types::MarkedString) -> String {
    match s {
        lsp_types::MarkedString::String(s) => s.clone(),
        lsp_types::MarkedString::LanguageString(ls) => format!("```{}\n{}\n```", ls.language, ls.value),
    }
}

/// Strips snippet placeholders: `foo(${1:x})$0` -> (`foo(x)`, cursor offset).
pub fn expand_snippet(snippet: &str) -> (String, Option<usize>) {
    let mut out = String::new();
    let mut cursor = None;
    let mut chars = snippet.chars().peekable();
    let mut depth_stack: Vec<bool> = Vec::new();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(next) = chars.next() {
                    out.push(next);
                }
            }
            '$' => {
                if chars.peek() == Some(&'{') {
                    chars.next();
                    let mut number = String::new();
                    while let Some(&d) = chars.peek() {
                        if d.is_ascii_digit() {
                            number.push(d);
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    if chars.peek() == Some(&':') {
                        chars.next();
                    } else if chars.peek() == Some(&'|') {
                        // Choice: take the first option.
                        chars.next();
                        let mut first = String::new();
                        for d in chars.by_ref() {
                            if d == ',' || d == '|' {
                                break;
                            }
                            first.push(d);
                        }
                        for d in chars.by_ref() {
                            if d == '}' {
                                break;
                            }
                        }
                        if (number == "1" || number == "0") && cursor.is_none() {
                            cursor = Some(out.chars().count());
                        }
                        out.push_str(&first);
                        continue;
                    }
                    if (number == "1" || number == "0") && cursor.is_none() {
                        cursor = Some(out.chars().count());
                    }
                    depth_stack.push(true);
                } else {
                    let mut number = String::new();
                    while let Some(&d) = chars.peek() {
                        if d.is_ascii_digit() {
                            number.push(d);
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    if number.is_empty() {
                        out.push('$');
                    } else if (number == "1" || number == "0") && cursor.is_none() {
                        cursor = Some(out.chars().count());
                    }
                }
            }
            '}' if !depth_stack.is_empty() => {
                depth_stack.pop();
            }
            _ => out.push(c),
        }
    }
    (out, cursor)
}

/// Locations from definition/references responses.
pub fn parse_locations(value: &Value) -> Vec<(String, lsp_types::Range)> {
    let mut out = Vec::new();
    let mut push = |v: &Value| {
        if let Ok(location) = serde_json::from_value::<lsp_types::Location>(v.clone()) {
            if let Some(path) = uri_to_path(location.uri.as_str()) {
                out.push((path, location.range));
            }
        } else if let Ok(link) = serde_json::from_value::<lsp_types::LocationLink>(v.clone())
            && let Some(path) = uri_to_path(link.target_uri.as_str())
        {
            out.push((path, link.target_selection_range));
        }
    };
    match value {
        Value::Array(items) => items.iter().for_each(&mut push),
        Value::Null => {}
        other => push(other),
    }
    out
}

/// File path -> edits, from a `WorkspaceEdit`.
pub fn workspace_edit_files(edit: &lsp_types::WorkspaceEdit) -> Vec<(String, Vec<lsp_types::TextEdit>)> {
    let mut files = Vec::new();
    if let Some(changes) = &edit.changes {
        for (uri, edits) in changes {
            if let Some(path) = uri_to_path(uri.as_str()) {
                files.push((path, edits.clone()));
            }
        }
    }
    if let Some(changes) = &edit.document_changes {
        let mut push = |edit: &lsp_types::TextDocumentEdit| {
            if let Some(path) = uri_to_path(edit.text_document.uri.as_str()) {
                let edits = edit
                    .edits
                    .iter()
                    .map(|e| match e {
                        lsp_types::OneOf::Left(edit) => edit.clone(),
                        lsp_types::OneOf::Right(annotated) => annotated.text_edit.clone(),
                    })
                    .collect();
                files.push((path, edits));
            }
        };
        match changes {
            lsp_types::DocumentChanges::Edits(edits) => edits.iter().for_each(&mut push),
            lsp_types::DocumentChanges::Operations(ops) => {
                for op in ops {
                    if let lsp_types::DocumentChangeOperation::Edit(edit) = op {
                        push(edit);
                    }
                }
            }
        }
    }
    files
}

pub fn text_edits_to_ranges(buffer: &Buffer, edits: &[lsp_types::TextEdit], utf8: bool) -> Vec<(Range<usize>, String)> {
    let mut ranges: Vec<(Range<usize>, String)> =
        edits.iter().map(|e| (from_range(buffer, &e.range, utf8), e.new_text.replace("\r\n", "\n"))).collect();
    ranges.sort_by_key(|(r, _)| r.start);
    // Drop overlaps defensively; the rope cannot apply them.
    let mut last_end = 0;
    ranges.retain(|(r, _)| {
        let ok = r.start >= last_end;
        if ok {
            last_end = r.end;
        }
        ok
    });
    ranges
}

pub fn kind_icon(kind: Option<lsp_types::CompletionItemKind>) -> (&'static str, eframe::egui::Color32) {
    use eframe::egui::Color32;
    use lsp_types::CompletionItemKind as K;
    match kind {
        Some(K::FUNCTION) | Some(K::METHOD) | Some(K::CONSTRUCTOR) => ("ƒ", Color32::from_rgb(198, 120, 221)),
        Some(K::FIELD) | Some(K::PROPERTY) => ("◆", Color32::from_rgb(97, 175, 239)),
        Some(K::VARIABLE) => ("x", Color32::from_rgb(97, 175, 239)),
        Some(K::CLASS) | Some(K::STRUCT) | Some(K::INTERFACE) => ("◇", Color32::from_rgb(229, 192, 123)),
        Some(K::ENUM) | Some(K::ENUM_MEMBER) => ("∈", Color32::from_rgb(229, 192, 123)),
        Some(K::MODULE) => ("{}", Color32::from_rgb(200, 200, 200)),
        Some(K::KEYWORD) => ("⚑", Color32::from_rgb(160, 160, 170)),
        Some(K::SNIPPET) => ("✄", Color32::from_rgb(160, 160, 170)),
        Some(K::CONSTANT) => ("π", Color32::from_rgb(209, 154, 102)),
        Some(K::TYPE_PARAMETER) => ("T", Color32::from_rgb(229, 192, 123)),
        Some(K::FILE) | Some(K::FOLDER) => ("▤", Color32::from_rgb(160, 160, 170)),
        _ => ("•", Color32::from_rgb(140, 140, 150)),
    }
}

/// Which servers apply to a language.
pub fn servers_for(lang: Lang, specs: &[ServerSpec]) -> Vec<ServerSpec> {
    let id = lang.lsp_id();
    specs.iter().filter(|s| s.languages.iter().any(|l| l == id)).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_hints_cover_the_default_servers() {
        assert_eq!(install_hint("rust-analyzer"), Some("rustup component add rust-analyzer"));
        for spec in default_servers() {
            let known = install_hint(&spec.name).is_some() || matches!(spec.name.as_str(), "omnisharp" | "superhtml" | "sourcekit-lsp");
            assert!(known, "no install hint for {}", spec.name);
        }
    }

    #[test]
    fn snippet_expansion() {
        assert_eq!(expand_snippet("foo(${1:x}, ${2:y})$0"), ("foo(x, y)".to_string(), Some(4)));
        assert_eq!(expand_snippet("println!(\"$1\")"), ("println!(\"\")".to_string(), Some(10)));
        assert_eq!(expand_snippet("plain"), ("plain".to_string(), None));
    }

    #[test]
    fn utf16_positions() {
        let buffer = Buffer::new("aé😀b\nx");
        let index = 3; // before 'b'
        let pos = to_position(&buffer, index, false);
        assert_eq!(pos.character, 4);
        assert_eq!(from_position(&buffer, &pos, false), index);
        let pos8 = to_position(&buffer, index, true);
        assert_eq!(pos8.character, 7);
        assert_eq!(from_position(&buffer, &pos8, true), index);
    }
}
