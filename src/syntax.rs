//! Language detection and tree-sitter highlighting on a background thread.

use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, OnceLock},
};

use crossbeam_channel::{Receiver, Sender, unbounded};
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Lang {
    Rust,
    Bash,
    Python,
    C,
    Cpp,
    CSharp,
    Go,
    Html,
    Css,
    JavaScript,
    TypeScript,
    Tsx,
    Json,
    Toml,
    Markdown,
    Yaml,
    Makefile,
    Groovy,
    Hcl,
    Dockerfile,
    Plain,
}

impl Lang {
    pub fn from_path(path: &str) -> Lang {
        let name = crate::host::file_name(path);
        let lower = name.to_ascii_lowercase();
        match lower.as_str() {
            "makefile" | "gnumakefile" => return Lang::Makefile,
            "jenkinsfile" => return Lang::Groovy,
            "dockerfile" | "containerfile" => return Lang::Dockerfile,
            _ if lower.starts_with("dockerfile.") || lower.ends_with(".dockerfile") => return Lang::Dockerfile,
            "cmakelists.txt" => return Lang::Plain,
            ".bashrc" | ".bash_profile" | ".profile" | ".zshrc" | ".zprofile" | "pkgbuild" => return Lang::Bash,
            "cargo.lock" => return Lang::Toml,
            _ => {}
        }
        let ext = Path::new(&lower).extension().and_then(|e| e.to_str()).unwrap_or("");
        match ext {
            "rs" => Lang::Rust,
            "sh" | "bash" | "zsh" | "ksh" => Lang::Bash,
            "py" | "pyi" | "pyw" => Lang::Python,
            "c" | "h" => Lang::C,
            "cc" | "cpp" | "cxx" | "c++" | "hpp" | "hh" | "hxx" | "h++" | "ipp" | "inl" => Lang::Cpp,
            "cs" | "csx" => Lang::CSharp,
            "go" => Lang::Go,
            "html" | "htm" | "xhtml" => Lang::Html,
            "css" => Lang::Css,
            "js" | "mjs" | "cjs" | "jsx" => Lang::JavaScript,
            "ts" | "mts" | "cts" => Lang::TypeScript,
            "tsx" => Lang::Tsx,
            "json" | "jsonc" | "json5" => Lang::Json,
            "toml" => Lang::Toml,
            "md" | "markdown" => Lang::Markdown,
            "yml" | "yaml" => Lang::Yaml,
            "mk" => Lang::Makefile,
            "groovy" | "gradle" | "jenkinsfile" => Lang::Groovy,
            "tf" | "tfvars" | "hcl" | "nomad" => Lang::Hcl,
            _ => Lang::Plain,
        }
    }

    /// Uses a shebang for extension-less scripts.
    pub fn from_path_and_text(path: &str, text: &str) -> Lang {
        let lang = Lang::from_path(path);
        if lang != Lang::Plain {
            return lang;
        }
        let first = text.lines().next().unwrap_or("");
        if first.starts_with("#!") {
            if first.contains("python") {
                return Lang::Python;
            }
            if first.contains("sh") {
                return Lang::Bash;
            }
        }
        Lang::Plain
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Lang::Rust => "Rust",
            Lang::Bash => "Shell",
            Lang::Python => "Python",
            Lang::C => "C",
            Lang::Cpp => "C++",
            Lang::CSharp => "C#",
            Lang::Go => "Go",
            Lang::Html => "HTML",
            Lang::Css => "CSS",
            Lang::JavaScript => "JavaScript",
            Lang::TypeScript => "TypeScript",
            Lang::Tsx => "TSX",
            Lang::Json => "JSON",
            Lang::Toml => "TOML",
            Lang::Markdown => "Markdown",
            Lang::Yaml => "YAML",
            Lang::Makefile => "Makefile",
            Lang::Groovy => "Groovy",
            Lang::Hcl => "HCL",
            Lang::Dockerfile => "Dockerfile",
            Lang::Plain => t!("Text"),
        }
    }

    /// LSP `languageId`.
    pub fn lsp_id(self) -> &'static str {
        match self {
            Lang::Rust => "rust",
            Lang::Bash => "shellscript",
            Lang::Python => "python",
            Lang::C => "c",
            Lang::Cpp => "cpp",
            Lang::CSharp => "csharp",
            Lang::Go => "go",
            Lang::Html => "html",
            Lang::Css => "css",
            Lang::JavaScript => "javascript",
            Lang::TypeScript => "typescript",
            Lang::Tsx => "typescriptreact",
            Lang::Json => "json",
            Lang::Toml => "toml",
            Lang::Markdown => "markdown",
            Lang::Yaml => "yaml",
            Lang::Makefile => "makefile",
            Lang::Groovy => "groovy",
            Lang::Hcl => "terraform",
            Lang::Dockerfile => "dockerfile",
            Lang::Plain => "plaintext",
        }
    }

    pub fn line_comment(self) -> Option<&'static str> {
        match self {
            Lang::Rust | Lang::C | Lang::Cpp | Lang::CSharp | Lang::Go | Lang::JavaScript | Lang::TypeScript | Lang::Tsx | Lang::Json => {
                Some("//")
            }
            Lang::Bash | Lang::Python | Lang::Toml | Lang::Yaml | Lang::Makefile | Lang::Hcl | Lang::Dockerfile => Some("#"),
            Lang::Groovy => Some("//"),
            _ => None,
        }
    }

    fn from_injection_name(name: &str) -> Option<Lang> {
        Some(match name {
            "javascript" | "js" => Lang::JavaScript,
            "typescript" | "ts" => Lang::TypeScript,
            "css" => Lang::Css,
            "html" => Lang::Html,
            "rust" => Lang::Rust,
            "bash" | "sh" => Lang::Bash,
            "python" => Lang::Python,
            "json" => Lang::Json,
            "c" => Lang::C,
            "cpp" => Lang::Cpp,
            "yaml" => Lang::Yaml,
            _ => return None,
        })
    }
}

/// Capture names we color. Tree-sitter picks the longest matching prefix.
pub const HIGHLIGHT_NAMES: &[&str] = &[
    "attribute",
    "boolean",
    "comment",
    "constant",
    "constant.builtin",
    "constructor",
    "embedded",
    "escape",
    "function",
    "function.builtin",
    "function.macro",
    "function.method",
    "keyword",
    "label",
    "module",
    "number",
    "operator",
    "property",
    "punctuation",
    "punctuation.bracket",
    "punctuation.delimiter",
    "punctuation.special",
    "string",
    "string.special",
    "string.escape",
    "tag",
    "type",
    "type.builtin",
    "variable",
    "variable.builtin",
    "variable.parameter",
    "namespace",
    "character",
    "conditional",
    "repeat",
    "include",
    "exception",
    "method",
    "field",
    "parameter",
    "storageclass",
    "delimiter",
    "keyword.function",
    "keyword.return",
    "keyword.operator",
    "property.definition",
    "text.title",
    "text.literal",
    "text.uri",
    "markup.heading",
    "markup.link",
];

#[derive(Clone, Copy, Debug)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub highlight: u8,
}

fn build(lang: Lang) -> Option<HighlightConfiguration> {
    let js_highlights = format!("{}\n{}", tree_sitter_javascript::JSX_HIGHLIGHT_QUERY, tree_sitter_javascript::HIGHLIGHT_QUERY);
    let (language, highlights, injections, locals): (tree_sitter::Language, String, &str, &str) = match lang {
        Lang::Rust => {
            (tree_sitter_rust::LANGUAGE.into(), tree_sitter_rust::HIGHLIGHTS_QUERY.into(), tree_sitter_rust::INJECTIONS_QUERY, "")
        }
        Lang::Bash => (tree_sitter_bash::LANGUAGE.into(), tree_sitter_bash::HIGHLIGHT_QUERY.into(), "", ""),
        Lang::Python => (tree_sitter_python::LANGUAGE.into(), tree_sitter_python::HIGHLIGHTS_QUERY.into(), "", ""),
        Lang::C => (tree_sitter_c::LANGUAGE.into(), tree_sitter_c::HIGHLIGHT_QUERY.into(), "", ""),
        Lang::Cpp => {
            (tree_sitter_cpp::LANGUAGE.into(), format!("{}\n{}", tree_sitter_cpp::HIGHLIGHT_QUERY, tree_sitter_c::HIGHLIGHT_QUERY), "", "")
        }
        Lang::CSharp => (tree_sitter_c_sharp::LANGUAGE.into(), tree_sitter_c_sharp::HIGHLIGHTS_QUERY.into(), "", ""),
        Lang::Go => (tree_sitter_go::LANGUAGE.into(), tree_sitter_go::HIGHLIGHTS_QUERY.into(), "", ""),
        Lang::Html => {
            (tree_sitter_html::LANGUAGE.into(), tree_sitter_html::HIGHLIGHTS_QUERY.into(), tree_sitter_html::INJECTIONS_QUERY, "")
        }
        Lang::Css => (tree_sitter_css::LANGUAGE.into(), tree_sitter_css::HIGHLIGHTS_QUERY.into(), "", ""),
        Lang::JavaScript => (
            tree_sitter_javascript::LANGUAGE.into(),
            js_highlights,
            tree_sitter_javascript::INJECTIONS_QUERY,
            tree_sitter_javascript::LOCALS_QUERY,
        ),
        Lang::TypeScript => (
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            format!("{}\n{}", tree_sitter_typescript::HIGHLIGHTS_QUERY, tree_sitter_javascript::HIGHLIGHT_QUERY),
            "",
            tree_sitter_typescript::LOCALS_QUERY,
        ),
        Lang::Tsx => (
            tree_sitter_typescript::LANGUAGE_TSX.into(),
            format!("{}\n{}", tree_sitter_typescript::HIGHLIGHTS_QUERY, js_highlights),
            "",
            tree_sitter_typescript::LOCALS_QUERY,
        ),
        Lang::Json => (tree_sitter_json::LANGUAGE.into(), tree_sitter_json::HIGHLIGHTS_QUERY.into(), "", ""),
        Lang::Toml => (tree_sitter_toml_ng::LANGUAGE.into(), tree_sitter_toml_ng::HIGHLIGHTS_QUERY.into(), "", ""),
        Lang::Yaml => (tree_sitter_yaml::LANGUAGE.into(), tree_sitter_yaml::HIGHLIGHTS_QUERY.into(), "", ""),
        Lang::Makefile => (tree_sitter_make::LANGUAGE.into(), tree_sitter_make::HIGHLIGHTS_QUERY.into(), "", ""),
        Lang::Groovy => (tree_sitter_groovy::LANGUAGE.into(), include_str!("queries/groovy.scm").into(), "", ""),
        Lang::Hcl => (tree_sitter_hcl::LANGUAGE.into(), include_str!("queries/hcl.scm").into(), "", ""),
        _ => return None,
    };
    let name = lang.lsp_id();
    let mut config = HighlightConfiguration::new(language.clone(), name, &highlights, injections, locals)
        .or_else(|_| HighlightConfiguration::new(language.clone(), name, &highlights, "", ""))
        .map_err(|error| eprintln!("lumen-edit: highlight query for {name}: {error}"))
        .ok()?;
    config.configure(HIGHLIGHT_NAMES);
    Some(config)
}

fn configs() -> &'static HashMap<Lang, Arc<HighlightConfiguration>> {
    static CONFIGS: OnceLock<HashMap<Lang, Arc<HighlightConfiguration>>> = OnceLock::new();
    CONFIGS.get_or_init(|| {
        [
            Lang::Rust,
            Lang::Bash,
            Lang::Python,
            Lang::C,
            Lang::Cpp,
            Lang::CSharp,
            Lang::Go,
            Lang::Html,
            Lang::Css,
            Lang::JavaScript,
            Lang::TypeScript,
            Lang::Tsx,
            Lang::Json,
            Lang::Toml,
            Lang::Yaml,
            Lang::Makefile,
            Lang::Groovy,
            Lang::Hcl,
        ]
        .into_iter()
        .filter_map(|lang| build(lang).map(|config| (lang, Arc::new(config))))
        .collect()
    })
}

pub fn highlight(highlighter: &mut Highlighter, lang: Lang, text: &str) -> Vec<Span> {
    let configs = configs();
    let Some(config) = configs.get(&lang) else { return Vec::new() };
    let Ok(events) = highlighter.highlight(config, text.as_bytes(), None, None, |name| {
        Lang::from_injection_name(name).and_then(|lang| configs.get(&lang)).map(|c| c.as_ref())
    }) else {
        return Vec::new();
    };
    let mut spans: Vec<Span> = Vec::new();
    let mut stack: Vec<u8> = Vec::new();
    for event in events {
        match event {
            Ok(HighlightEvent::HighlightStart(h)) => stack.push(h.0 as u8),
            Ok(HighlightEvent::HighlightEnd) => {
                stack.pop();
            }
            Ok(HighlightEvent::Source { start, end }) => {
                if let Some(&highlight) = stack.last() {
                    if let Some(last) = spans.last_mut()
                        && last.end == start
                        && last.highlight == highlight
                    {
                        last.end = end;
                    } else {
                        spans.push(Span { start, end, highlight });
                    }
                }
            }
            Err(_) => break,
        }
    }
    spans
}

pub struct Job {
    pub doc_id: u64,
    pub version: u64,
    pub lang: Lang,
    pub text: String,
    /// File path, used to recognise CI/CD files.
    pub path: String,
}

/// Grammar highlighting plus the CI/CD overlay for pipeline files.
pub fn highlight_file(highlighter: &mut Highlighter, lang: Lang, path: &str, text: &str) -> Vec<Span> {
    if lang == Lang::Dockerfile {
        return crate::ci::dockerfile(text);
    }
    let base = highlight(highlighter, lang, text);
    match crate::ci::Ci::detect(path) {
        Some(ci) if lang == Lang::Yaml => crate::ci::merge(base, crate::ci::overlay(ci, text)),
        _ => base,
    }
}

pub struct Done {
    pub doc_id: u64,
    pub version: u64,
    pub spans: Vec<Span>,
}

/// Starts the highlighter thread. Older jobs for the same document are
/// dropped when newer ones are queued.
pub fn spawn_worker(done: impl Fn(Done) + Send + 'static) -> Sender<Job> {
    let (tx, rx): (Sender<Job>, Receiver<Job>) = unbounded();
    // Without the thread the editor still works, only unhighlighted.
    let spawned = std::thread::Builder::new().name("highlighter".into()).spawn(move || {
        let mut highlighter = Highlighter::new();
        // Warm up the queries off the UI thread.
        configs();
        while let Ok(first) = rx.recv() {
            let mut latest: HashMap<u64, Job> = HashMap::new();
            latest.insert(first.doc_id, first);
            while let Ok(job) = rx.try_recv() {
                latest.insert(job.doc_id, job);
            }
            for (_, job) in latest {
                let spans = if job.text.len() > 8 * 1024 * 1024 {
                    Vec::new()
                } else {
                    highlight_file(&mut highlighter, job.lang, &job.path, &job.text)
                };
                done(Done { doc_id: job.doc_id, version: job.version, spans });
            }
        }
    });
    if let Err(e) = spawned {
        eprintln!("lumen-edit: failed to start highlighter thread: {e}");
    }
    tx
}

/// The same grammar used for highlighting; parsing happens on the completion worker.
pub fn grammar(lang: Lang) -> Option<tree_sitter::Language> {
    configs().get(&lang).map(|config| config.language.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_grammars_load() {
        for lang in [
            Lang::Rust,
            Lang::Bash,
            Lang::Python,
            Lang::C,
            Lang::Cpp,
            Lang::CSharp,
            Lang::Go,
            Lang::Html,
            Lang::Css,
            Lang::JavaScript,
            Lang::TypeScript,
            Lang::Tsx,
            Lang::Json,
            Lang::Toml,
            Lang::Yaml,
            Lang::Makefile,
            Lang::Groovy,
            Lang::Hcl,
        ] {
            assert!(configs().contains_key(&lang), "{lang:?} failed to load");
        }
    }

    #[test]
    fn rust_highlights_keyword() {
        let mut highlighter = Highlighter::new();
        let spans = highlight(&mut highlighter, Lang::Rust, "fn main() { let x = 1; }");
        let keyword = HIGHLIGHT_NAMES.iter().position(|n| *n == "keyword").unwrap() as u8;
        assert!(spans.iter().any(|s| s.start == 0 && s.end == 2 && s.highlight == keyword));
    }
}
