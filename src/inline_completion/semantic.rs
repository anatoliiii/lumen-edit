//! Bounded, capability-aware context. Never builds a project index.
use super::Context;
use crate::{
    lsp::{self, client::LspClient},
    syntax::{self, Lang},
    text::Buffer,
};
use serde_json::{Value, json};
use std::{
    io::Read,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

pub fn clip(text: &str, bytes: usize) -> String {
    let mut end = bytes.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}
pub fn safe_path(root: &Path, path: &Path) -> bool {
    let Ok(root) = root.canonicalize() else { return false };
    let Ok(path) = path.canonicalize() else { return false };
    if !path.starts_with(root) {
        return false;
    }
    !path.components().any(|part| {
        let name = part.as_os_str().to_string_lossy().to_lowercase();
        name.starts_with(".env")
            || matches!(
                name.as_str(),
                ".git"
                    | ".ssh"
                    | ".aws"
                    | ".gnupg"
                    | ".kube"
                    | ".npmrc"
                    | ".netrc"
                    | ".pypirc"
                    | ".git-credentials"
                    | "credentials.json"
                    | "credentials"
                    | "secrets"
                    | "secrets.json"
                    | "secrets.yaml"
                    | "id_rsa"
                    | "id_ed25519"
            )
            || name.ends_with(".pem")
            || name.ends_with(".key")
            || name.ends_with(".p12")
            || name.ends_with(".pfx")
    })
}
/// A conservative extra guard, not a claim that arbitrary secrets are detectable.
pub fn sensitive(text: &str) -> bool {
    static PATTERN: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    PATTERN.get_or_init(|| regex::Regex::new(r#"(?im)(-----BEGIN [A-Z ]*PRIVATE KEY|\b(?:[a-z0-9_]+_)?(?:api[_-]?key|access[_-]?token|auth[_-]?token|token|authorization|password|secret)\s*[:=]\s*["']?[^\s"']{8,}|\b(?:sk-or-v1-|sk-proj-|ghp_|github_pat_|AKIA)[A-Za-z0-9_-]{12,})"#).unwrap()).is_match(text)
}
pub fn window(buffer: &Buffer, cursor: usize, language: &str) -> Context {
    let cursor = cursor.min(buffer.len_chars());
    let mut prefix = buffer.slice(cursor.saturating_sub(6000)..cursor);
    if prefix.len() > 6000 {
        let mut start = prefix.len() - 6000;
        while !prefix.is_char_boundary(start) {
            start += 1;
        }
        prefix = prefix[start..].to_string();
    }
    Context {
        language: language.into(),
        prefix,
        suffix: clip(&buffer.slice(cursor..(cursor + 1800).min(buffer.len_chars())), 1800),
        semantic: String::new(),
    }
}
#[derive(Default)]
pub struct Ranked {
    pub entries: Vec<(u8, String)>,
}
impl Ranked {
    pub fn add(&mut self, rank: u8, label: &str, text: &str) {
        if !text.trim().is_empty() && !sensitive(text) {
            self.entries.push((rank, format!("{label}: {}", clip(text, 900))));
        }
    }
    pub fn finish(mut self, budget: usize) -> String {
        self.entries.sort_by_key(|(rank, _)| *rank);
        let mut out = String::new();
        let mut locals_bytes = 0;
        for (rank, entry) in self.entries {
            if rank == 2 {
                if locals_bytes + entry.len() > 800 {
                    continue;
                }
                locals_bytes += entry.len();
            }
            if out.contains(&entry) {
                continue;
            }
            let available = budget.saturating_sub(out.len() + 1);
            if available == 0 {
                break;
            }
            out.push_str(&clip(&entry, available));
            out.push('\n');
        }
        out
    }
}

pub fn syntax_context(lang: Lang, text: &str, cursor: usize, ranked: &mut Ranked) {
    let Some(grammar) = syntax::grammar(lang) else { return };
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&grammar).is_err() {
        return;
    }
    let Some(tree) = parser.parse(text, None) else { return };
    let at = cursor.min(text.len());
    if let Some(mut node) = tree.root_node().descendant_for_byte_range(at, at) {
        for _ in 0..12 {
            let kind = node.kind();
            if kind.contains("function") || kind.contains("method") || kind.contains("impl") || kind.contains("class") {
                let header = text[node.byte_range()].split('{').next().unwrap_or("").lines().take(3).collect::<Vec<_>>().join("\n");
                ranked.add(4, kind, &header);
            }
            let Some(parent) = node.parent() else { break };
            node = parent;
        }
    }
    // Walk only a bounded number of AST nodes. Local declarations must belong
    // to an ancestor scope of the cursor (not a previous, unrelated function).
    let mut locals = std::collections::VecDeque::new();
    let mut stack = vec![tree.root_node()];
    let mut visited = 0;
    while let Some(node) = stack.pop() {
        visited += 1;
        if visited > 12000 {
            break;
        }
        let kind = node.kind();
        let import = kind.contains("import") || kind == "use_declaration";
        let local =
            matches!(kind, "let_declaration" | "assignment" | "variable_declarator" | "parameter" | "self_parameter" | "typed_parameter");
        let mut in_scope = true;
        let mut parent = node.parent();
        while let Some(scope) = parent {
            if matches!(scope.kind(), "block" | "function_definition" | "function_item" | "method_definition" | "class_definition")
                && !(scope.start_byte() <= at && at <= scope.end_byte())
            {
                in_scope = false;
                break;
            }
            parent = scope.parent();
        }
        if import {
            ranked.add(7, kind, &text[node.byte_range()]);
        }
        if local && in_scope && node.end_byte() <= at {
            locals.push_back((kind.to_string(), clip(&text[node.byte_range()], 180)));
            if locals.len() > 12 {
                locals.pop_front();
            }
        }
        let mut walk = node.walk();
        let children: Vec<_> = node.children(&mut walk).collect();
        stack.extend(children.into_iter().rev());
    }
    for (kind, text) in locals.into_iter().rev() {
        ranked.add(2, &kind, &text);
    }
}
fn compact(value: &Value) -> String {
    match value {
        Value::String(s) => clip(s, 1600),
        Value::Array(a) => a.iter().take(20).map(compact).filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n"),
        Value::Object(o) => {
            let mut pieces = Vec::new();
            for field in ["name", "label", "detail", "contents", "value", "parameters", "signatures", "children", "items"] {
                if let Some(v) = o.get(field) {
                    let text = compact(v);
                    if !text.is_empty() {
                        pieces.push(text);
                    }
                }
            }
            clip(&pieces.join(" "), 3000)
        }
        _ => String::new(),
    }
}
pub struct Snapshot {
    pub buffer: Buffer,
    pub lang: Lang,
    pub cursor: usize,
    pub path: String,
    pub root: PathBuf,
    pub clients: Vec<Arc<LspClient>>,
    pub related: Vec<(String, ropey::Rope)>,
}
pub fn collect(snapshot: &Snapshot, cancel: &AtomicBool) -> Result<Context, String> {
    if !safe_path(&snapshot.root, Path::new(&snapshot.path)) {
        return Err("File is outside the project or excluded".into());
    }
    let mut context = window(&snapshot.buffer, snapshot.cursor, snapshot.lang.lsp_id());
    if sensitive(&context.prefix) || sensitive(&context.suffix) {
        return Err("Possible secret in code context; completion suppressed".into());
    }
    let mut ranked = Ranked::default();
    let text = snapshot.buffer.text();
    syntax_context(snapshot.lang, &text, snapshot.buffer.rope.char_to_byte(snapshot.cursor), &mut ranked);
    let deadline = Instant::now() + Duration::from_millis(650);
    // Query the receiver/identifier immediately before '.' or the current word.
    let mut symbol = snapshot.cursor;
    while symbol > 0 && matches!(snapshot.buffer.char(symbol - 1), Some('.' | ':' | ' ' | '(')) {
        symbol -= 1;
    }
    symbol = symbol.saturating_sub(1);
    for (method, capability, rank, receiver) in [
        ("textDocument/hover", "hoverProvider", 0, true),
        ("textDocument/signatureHelp", "signatureHelpProvider", 1, false),
        ("textDocument/completion", "completionProvider", 3, false),
        ("textDocument/definition", "definitionProvider", 4, true),
        ("textDocument/typeDefinition", "typeDefinitionProvider", 1, true),
        ("textDocument/documentSymbol", "documentSymbolProvider", 5, false),
        ("textDocument/references", "referencesProvider", 6, true),
    ] {
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled".into());
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        let Some(client) = snapshot.clients.iter().find(|c| c.has_capability(capability)) else { continue };
        let position = lsp::to_position(&snapshot.buffer, if receiver { symbol } else { snapshot.cursor }, client.utf8);
        let mut params = json!({"textDocument":{"uri":lsp::path_to_uri(&snapshot.path)},"position":position});
        if method.ends_with("references") {
            params["context"] = json!({"includeDeclaration":false});
        }
        if method.ends_with("completion") {
            params["context"] = json!({"triggerKind":1});
        }
        let Ok(value) = client.request_timeout(method, params, remaining.min(Duration::from_millis(90))) else { continue };
        if method.ends_with("definition") || method.ends_with("typeDefinition") || method.ends_with("references") {
            for (path, range) in lsp::parse_locations(&value).into_iter().take(3) {
                if cancel.load(Ordering::Relaxed) {
                    return Err("Cancelled".into());
                }
                if !safe_path(&snapshot.root, Path::new(&path)) {
                    continue;
                }
                let source = if let Some((_, rope)) = snapshot.related.iter().find(|(p, _)| p == &path) {
                    rope.to_string()
                } else {
                    let Ok(file) = std::fs::File::open(&path) else { continue };
                    let mut text = String::new();
                    if file.take(65536).read_to_string(&mut text).is_err() {
                        continue;
                    }
                    text
                };
                let line = range.start.line as usize;
                let fragment = source.lines().skip(line.saturating_sub(1)).take(8).collect::<Vec<_>>().join("\n");
                let relative = Path::new(&path).strip_prefix(&snapshot.root).unwrap_or(Path::new(&path)).display();
                ranked.add(rank, &format!("{method} {relative}:{}", line + 1), &fragment);
            }
        } else {
            ranked.add(rank, method, &compact(&value));
        }
    }
    context.semantic = ranked.finish(4000);
    Ok(context)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_window_and_position() {
        let b = Buffer::new("let 😀 = \u{43c}\u{438}\u{440};\nclient.");
        let at = 13;
        let c = window(&b, at, "rust");
        assert_eq!(c.prefix, b.slice(0..at));
        assert_eq!(c.suffix, b.slice(at..b.len_chars()));
        assert_eq!(lsp::from_position(&b, &lsp::to_position(&b, at, false), false), at);
    }
    #[test]
    fn rank_and_budget() {
        let mut r = Ranked::default();
        r.add(6, "far", &"x".repeat(1000));
        r.add(0, "hover", "ApiClient");
        let out = r.finish(100);
        assert!(out.starts_with("hover: ApiClient"));
        assert!(out.len() <= 100);
    }
    #[test]
    fn rust_and_python_scopes() {
        for (lang, text, expected, excluded) in [
            (
                Lang::Rust,
                "use api::Client;\nfn old() { let hidden = 1; }\nfn main() { let client: Client = make(); client. }",
                "client: Client",
                "hidden",
            ),
            (
                Lang::Python,
                "import api\ndef old():\n    hidden = 1\ndef main():\n    client: api.Client = make()\n    client.",
                "client: api.Client",
                "hidden",
            ),
        ] {
            let at = text.rfind("client.").unwrap() + 7;
            let mut r = Ranked::default();
            syntax_context(lang, text, at, &mut r);
            let out = r.finish(4000);
            assert!(out.contains(expected), "{out}");
            assert!(!out.contains(excluded), "{out}");
            assert!(out.contains(if lang == Lang::Rust { "api::Client" } else { "import api" }));
        }
    }
    #[test]
    fn privacy_guards() {
        assert!(sensitive("api_key = 'abcdefghijk'"));
        assert!(sensitive("-----BEGIN RSA PRIVATE KEY-----"));
        assert!(!sensitive("let client = ApiClient::new(config);"));
    }
    #[test]
    fn project_boundary_and_symlinks() {
        let dir = std::env::temp_dir().join(format!("lumen-inline-boundary-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("code.rs"), "fn main() {}").unwrap();
        std::fs::write(dir.join(".env.local"), "secret").unwrap();
        assert!(safe_path(&dir, &dir.join("code.rs")));
        assert!(!safe_path(&dir, &dir.join(".env.local")));
        assert!(!safe_path(&dir, Path::new(file!())));
        #[cfg(unix)]
        {
            let outside = dir.with_extension("outside");
            std::fs::write(&outside, "secret").unwrap();
            let _ = std::fs::remove_file(dir.join("link.rs"));
            std::os::unix::fs::symlink(&outside, dir.join("link.rs")).unwrap();
            assert!(!safe_path(&dir, &dir.join("link.rs")));
            std::fs::remove_file(outside).unwrap();
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn lsp_semantic_context_uses_capabilities_and_project_definitions() {
        use crate::host::Host;
        if !std::process::Command::new("python3").arg("--version").output().is_ok_and(|output| output.status.success()) {
            eprintln!("python3 unavailable; LSP fixture skipped");
            return;
        }
        let dir = std::env::temp_dir().join(format!("lumen-inline-lsp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("main.rs");
        let related = dir.join("api.rs");
        let log = dir.join("calls.jsonl");
        let text = "fn main() { let client: ApiClient = make(); client. }";
        std::fs::write(&path, text).unwrap();
        std::fs::write(&related,"pub struct ApiClient;\nimpl ApiClient {\n    pub fn send(&self, request: Request) -> Response {}\n}\nfn example(client: ApiClient) { client.send(request); }\n").unwrap();
        let script = dir.join("lsp.py");
        std::fs::write(&script,r#"
import sys,json,pathlib
root=pathlib.Path(sys.argv[1]); log=root/'calls.jsonl'
def send(value):
    body=json.dumps(value).encode();sys.stdout.buffer.write(('Content-Length: %d\r\n\r\n'%len(body)).encode()+body);sys.stdout.buffer.flush()
while True:
    n=None
    while True:
        line=sys.stdin.buffer.readline()
        if not line:sys.exit(0)
        if line.lower().startswith(b'content-length:'):n=int(line.split(b':')[1])
        if line in (b'\r\n',b'\n'):break
    msg=json.loads(sys.stdin.buffer.read(n)); method=msg.get('method'); result=None
    with log.open('a') as f:f.write(json.dumps({'method':method,'params':msg.get('params')})+'\n')
    if 'id' not in msg:continue
    if method=='initialize':result={'capabilities':{'hoverProvider':True,'completionProvider':{},'definitionProvider':True,'typeDefinitionProvider':True,'documentSymbolProvider':True,'referencesProvider':True}}
    elif method=='textDocument/hover':result={'contents':{'kind':'plaintext','value':'client: ApiClient'}}
    elif method=='textDocument/completion':result=[{'label':'send','detail':'fn send(&self, request: Request) -> Response'}]
    elif method in ('textDocument/definition','textDocument/typeDefinition','textDocument/references'):
        result=[{'uri':(root/'api.rs').as_uri(),'range':{'start':{'line':0,'character':0},'end':{'line':0,'character':20}}}, {'uri':(root.parent/'outside.rs').as_uri(),'range':{'start':{'line':0,'character':0},'end':{'line':0,'character':1}}}]
    elif method=='textDocument/documentSymbol':result=[{'name':'main','kind':12}]
    send({'jsonrpc':'2.0','id':msg['id'],'result':result})
"#).unwrap();
        let (tx, _rx) = crossbeam_channel::unbounded();
        let client = LspClient::start(
            &Host::Local,
            &dir.to_string_lossy(),
            "inline-fixture",
            "python3",
            &[script.to_string_lossy().into(), dir.to_string_lossy().into()],
            tx,
        )
        .unwrap();
        let cursor = text.find("client. }").unwrap() + 7;
        let snapshot = Snapshot {
            buffer: Buffer::new(text),
            lang: Lang::Rust,
            cursor,
            path: path.to_string_lossy().into(),
            root: dir.clone(),
            clients: vec![client.clone()],
            related: vec![],
        };
        let context = collect(&snapshot, &AtomicBool::new(false)).unwrap();
        client.shutdown();
        assert!(context.semantic.contains("client: ApiClient"), "{}", context.semantic);
        assert!(context.semantic.contains("fn send(&self, request: Request) -> Response"));
        assert!(context.semantic.contains("pub struct ApiClient"));
        assert!(context.semantic.contains("client.send(request)"));
        assert!(context.semantic.len() <= 4000);
        assert!(context.prompt().len() < 13000);
        let calls = std::fs::read_to_string(log).unwrap();
        let methods: Vec<_> =
            calls.lines().map(|line| serde_json::from_str::<Value>(line).unwrap()["method"].as_str().unwrap_or("").to_string()).collect();
        assert!(!methods.iter().any(|m| m == "textDocument/signatureHelp"), "unsupported methods must not be requested");
        for method in ["hover", "completion", "definition", "typeDefinition", "documentSymbol", "references"] {
            assert!(calls.contains(&format!("textDocument/{method}")));
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
