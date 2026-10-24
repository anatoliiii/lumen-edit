//! Manual, preview-first Next Edit predictions. Kept independent from FIM's
//! insertion controller and Tab handling.
use std::{
    collections::VecDeque,
    ops::Range,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use crossbeam_channel::{Receiver, unbounded};
use serde_json::{Value, json};

#[derive(Clone, Debug)]
pub struct EditHistoryEntry {
    pub start: usize,
    pub before: String,
    pub after: String,
}

#[derive(Clone, Debug)]
pub struct Preview {
    pub doc_id: u64,
    pub version: u64,
    pub cursor: usize,
    pub range: Range<usize>,
    pub expected: String,
    pub replacement: String,
    pub diff: String,
    pub metrics: RequestMetrics,
}

#[derive(Clone, Debug, Default)]
pub struct RequestMetrics {
    pub context_prepare_ms: u128,
    pub http_latency_ms: u128,
    pub diff_prepare_ms: u128,
    pub total_latency_ms: u128,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cached_tokens: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Metrics {
    pub requests: u64,
    pub errors: u64,
    pub latest: RequestMetrics,
    latencies: VecDeque<u128>,
}
impl Metrics {
    pub fn add_latency(&mut self, ms: u128) {
        self.latencies.push_back(ms);
        while self.latencies.len() > 128 {
            self.latencies.pop_front();
        }
    }
    pub fn percentiles(&self) -> (u128, u128) {
        if self.latencies.is_empty() {
            return (0, 0);
        }
        let mut values: Vec<_> = self.latencies.iter().copied().collect();
        values.sort_unstable();
        let p = |q: f64| values[((values.len() as f64 * q).ceil() as usize).saturating_sub(1).min(values.len() - 1)];
        (p(0.50), p(0.95))
    }
}

#[derive(Clone, Debug)]
struct RequestResult {
    doc_id: u64,
    version: u64,
    cursor: usize,
    range: Range<usize>,
    original: String,
    content: Result<(String, u64, u64, u64, u128), HttpFailure>,
    started: Instant,
    context_ms: u128,
}

#[derive(Clone, Debug)]
struct HttpFailure {
    message: String,
    http_ms: u128,
}

#[derive(Default)]
pub struct Controller {
    rx: Option<Receiver<RequestResult>>,
    cancel: Option<Arc<AtomicBool>>,
    pub busy: bool,
    pub status: String,
    pub metrics: Metrics,
    pub(super) pending_identity: Option<(u64, u64, usize)>,
}

#[derive(Clone, Debug)]
struct RequestSnapshot {
    project: std::path::PathBuf,
    path: String,
    doc_id: u64,
    version: u64,
    cursor: usize,
    rope: ropey::Rope,
    history: Vec<EditHistoryEntry>,
    region_lines: usize,
    started: Instant,
}

impl Controller {
    fn record_metrics(&mut self, metrics: RequestMetrics) {
        self.metrics.add_latency(metrics.total_latency_ms);
        self.metrics.latest = metrics;
    }
    pub fn launch(
        &mut self,
        project: &Path,
        path: &str,
        doc_id: u64,
        buffer: &crate::text::Buffer,
        history: &[EditHistoryEntry],
        region_lines: usize,
    ) {
        self.invalidate();
        let started = Instant::now();
        let cursor = buffer.primary().head;
        let cancel = Arc::new(AtomicBool::new(false));
        let token = cancel.clone();
        let (tx, rx) = unbounded();
        let snapshot = RequestSnapshot {
            project: project.to_path_buf(),
            path: path.to_string(),
            doc_id,
            version: buffer.version,
            cursor,
            rope: buffer.rope.clone(),
            history: history.to_vec(),
            region_lines,
            started,
        };
        std::thread::spawn(move || {
            let context_started = Instant::now();
            let doc_id = snapshot.doc_id;
            let version = snapshot.version;
            let cursor = snapshot.cursor;
            let started = snapshot.started;
            let prepared = (|| {
                if token.load(Ordering::Relaxed) {
                    return Err("Cancelled".to_string());
                }
                let project = snapshot.project.as_path();
                let path = Path::new(&snapshot.path);
                if !super::semantic::safe_path(project, path) {
                    return Err("Next Edit blocked: file is outside the allowed project or is sensitive".into());
                }
                if snapshot.rope.len_bytes() > 256 * 1024 {
                    return Err("Next Edit blocked: file exceeds the 256 KiB context limit".into());
                }
                let full = snapshot.rope.to_string();
                if super::semantic::sensitive(&full) {
                    return Err("Next Edit blocked: file appears to contain secrets".into());
                }
                let safe_history = snapshot
                    .history
                    .iter()
                    .filter(|h| !super::semantic::sensitive(&h.before) && !super::semantic::sensitive(&h.after))
                    .cloned()
                    .collect::<Vec<_>>();
                let relative = path
                    .canonicalize()
                    .ok()
                    .and_then(|p| project.canonicalize().ok().and_then(|root| p.strip_prefix(root).ok().map(|p| p.display().to_string())))
                    .unwrap_or_else(|| path.file_name().unwrap_or_default().to_string_lossy().into_owned());
                let (range, original, body) = build_request(&relative, &full, cursor, &safe_history, snapshot.region_lines)?;
                let (endpoint, key) = super::credentials::inception_route(project, "/v1/edit/completions")?;
                Ok((range, original, endpoint, key, body))
            })();
            let context_ms = context_started.elapsed().as_millis();
            let (range, original, result) = match prepared {
                Ok((range, original, endpoint, key, body)) => {
                    let result = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|_| HttpFailure { message: "Next Edit runtime initialization failed".to_string(), http_ms: 0 })
                        .and_then(|rt| rt.block_on(request(endpoint, key, body, token)));
                    (range, original, result)
                }
                Err(error) => (0..0, String::new(), Err(HttpFailure { message: error, http_ms: 0 })),
            };
            let _ = tx.send(RequestResult { doc_id, version, cursor, range, original, content: result, started, context_ms });
        });
        self.rx = Some(rx);
        self.cancel = Some(cancel);
        self.pending_identity = Some((doc_id, buffer.version, cursor));
        self.busy = true;
        self.metrics.requests += 1;
        self.status = "Next Edit is generating a preview…".into();
    }

    pub fn invalidate(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        self.busy = false;
        self.pending_identity = None;
    }

    pub fn poll(&mut self, current: Option<(u64, u64, usize)>) -> Option<Preview> {
        let result = self.rx.as_ref()?.try_recv().ok()?;
        self.busy = false;
        self.cancel = None;
        self.pending_identity = None;
        self.rx = None;
        if current != Some((result.doc_id, result.version, result.cursor)) {
            self.record_metrics(RequestMetrics {
                context_prepare_ms: result.context_ms,
                total_latency_ms: result.started.elapsed().as_millis(),
                ..Default::default()
            });
            self.status = "Next Edit discarded: document changed".into();
            return None;
        }
        let (content, prompt_tokens, completion_tokens, cached_tokens, http_ms) = match result.content {
            Ok(v) => v,
            Err(e) => {
                self.metrics.errors += 1;
                self.record_metrics(RequestMetrics {
                    context_prepare_ms: result.context_ms,
                    http_latency_ms: e.http_ms,
                    total_latency_ms: result.started.elapsed().as_millis(),
                    ..Default::default()
                });
                self.status = e.message;
                return None;
            }
        };
        let diff_started = Instant::now();
        let mut metrics = RequestMetrics {
            context_prepare_ms: result.context_ms,
            http_latency_ms: http_ms,
            total_latency_ms: result.started.elapsed().as_millis(),
            prompt_tokens,
            completion_tokens,
            cached_tokens,
            ..Default::default()
        };
        let normalized = match parse_region(&content) {
            Ok(s) => s,
            Err(e) => {
                self.metrics.errors += 1;
                metrics.diff_prepare_ms = diff_started.elapsed().as_millis();
                self.record_metrics(metrics);
                self.status = e.into();
                return None;
            }
        };
        if normalized == result.original {
            metrics.diff_prepare_ms = diff_started.elapsed().as_millis();
            self.record_metrics(metrics);
            self.status = "Next Edit returned no changes".into();
            return None;
        }
        let (range, expected, replacement) = minimal_change(&result.original, &normalized, result.range.start);
        if expected == replacement {
            metrics.diff_prepare_ms = diff_started.elapsed().as_millis();
            self.record_metrics(metrics);
            self.status = "Next Edit returned no changes".into();
            return None;
        }
        let diff = similar::TextDiff::from_lines(&result.original, &normalized).unified_diff().context_radius(3).to_string();
        let diff_ms = diff_started.elapsed().as_millis();
        metrics.diff_prepare_ms = diff_ms;
        self.record_metrics(metrics.clone());
        self.status = "Review Next Edit, then accept or dismiss it".into();
        Some(Preview { doc_id: result.doc_id, version: result.version, cursor: result.cursor, range, expected, replacement, diff, metrics })
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
    }
}

impl Preview {
    pub fn is_current(&self, doc_id: u64, buffer: &crate::text::Buffer) -> bool {
        self.doc_id == doc_id
            && self.version == buffer.version
            && self.cursor == buffer.primary().head
            && buffer.slice(self.range.clone()) == self.expected
    }
}

pub fn build_request(
    path: &str,
    file: &str,
    cursor: usize,
    history: &[EditHistoryEntry],
    region_lines: usize,
) -> Result<(Range<usize>, String, Value), String> {
    let chars: Vec<char> = file.chars().collect();
    let cursor = cursor.min(chars.len());
    let line_at = |at: usize| chars[..at].iter().filter(|&&c| c == '\n').count();
    let line = line_at(cursor);
    let lines = region_lines.clamp(1, 15);
    let start_line = line.saturating_sub(5.min(lines.saturating_sub(1)));
    let end_line = (start_line + lines).min(file.lines().count());
    let mut start = file_line_char_start(file, start_line);
    let mut end = if end_line >= file.lines().count() { chars.len() } else { file_line_char_start(file, end_line) };
    // Bound pathological/minified lines while keeping the cursor inside the
    // editable window and preserving character boundaries.
    if end.saturating_sub(start) > 8192 {
        start = cursor.saturating_sub(4096).max(start);
        end = (cursor + 4096).min(end);
    }
    let range = start..end;
    let original: String = chars[start..end].iter().collect();
    let cursor_local = cursor.saturating_sub(start).min(original.chars().count());
    let mut above_start = file_line_char_start(file, start_line.saturating_sub(30));
    above_start = above_start.min(start);
    let below_line = (end_line + 30).min(file.lines().count());
    let below_end = if below_line >= file.lines().count() { chars.len() } else { file_line_char_start(file, below_line) };
    let before: String = chars[above_start.max(start.saturating_sub(4096))..start].iter().collect();
    let after: String = chars[end..below_end.min(end.saturating_add(4096))].iter().collect();
    let region_before: String = original.chars().take(cursor_local).collect();
    let region_after: String = original.chars().skip(cursor_local).collect();
    let diff_history = history
        .iter()
        .rev()
        .take(5)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|h| {
            let before = format!("{}{}", " ".repeat(h.start.min(80)), h.before);
            let after = format!("{}{}", " ".repeat(h.start.min(80)), h.after);
            similar::TextDiff::from_lines(&before, &after).unified_diff().context_radius(2).to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");
    let content = format!(
        "<|recently_viewed_code_snippets|>\n\n<|/recently_viewed_code_snippets|>\n\n<|current_file_content|>\ncurrent_file_path: {path}\n{before}<|code_to_edit|>\n{region_before}\n<|cursor|>\n{region_after}<|/code_to_edit|>\n{after}\n<|/current_file_content|>\n\n<|edit_diff_history|>\n{diff_history}\n<|/edit_diff_history|>"
    );
    let body = json!({"model":"mercury-edit-2","messages":[{"role":"user","content":content}],"max_tokens":256});
    Ok((range, original, body))
}

fn file_line_char_start(file: &str, line: usize) -> usize {
    file.lines().take(line).map(|l| l.chars().count() + 1).sum()
}
pub fn parse_region(response: &str) -> Result<String, &'static str> {
    let text = response.trim();
    let Some(rest) = text.strip_prefix("```") else {
        return Err("Next Edit response must contain one fenced code region");
    };
    let Some((_, body)) = rest.split_once('\n') else {
        return Err("Next Edit returned malformed Markdown fence");
    };
    let Some(body) = body.strip_suffix("```") else {
        return Err("Next Edit returned malformed Markdown fence");
    };
    if body.contains("```") {
        return Err("Next Edit returned multiple Markdown fences");
    }
    Ok(body.to_string())
}

fn minimal_change(original: &str, new: &str, base: usize) -> (Range<usize>, String, String) {
    let a: Vec<char> = original.chars().collect();
    let b: Vec<char> = new.chars().collect();
    let mut prefix = 0;
    while prefix < a.len().min(b.len()) && a[prefix] == b[prefix] {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < a.len() - prefix && suffix < b.len() - prefix && a[a.len() - 1 - suffix] == b[b.len() - 1 - suffix] {
        suffix += 1;
    }
    let replacement: String = b[prefix..b.len() - suffix].iter().collect();
    let expected: String = a[prefix..a.len() - suffix].iter().collect();
    (base + prefix..base + a.len() - suffix, expected, replacement)
}

async fn request(
    endpoint: String,
    key: String,
    body: Value,
    cancel: Arc<AtomicBool>,
) -> Result<(String, u64, u64, u64, u128), HttpFailure> {
    let started = Instant::now();
    if cancel.load(Ordering::Relaxed) {
        return Err(HttpFailure { message: "Cancelled".into(), http_ms: 0 });
    }
    let client = super::provider::shared_client().map_err(|message| HttpFailure { message, http_ms: started.elapsed().as_millis() })?;
    let operation = async {
        let response = client
            .post(endpoint)
            .bearer_auth(key)
            .json(&body)
            .send()
            .await
            .map_err(|e| if e.is_timeout() { "Next Edit request timed out" } else { "Next Edit network error" })?;
        if !response.status().is_success() {
            let code = response.status().as_u16();
            return Err(match code {
                401 => "Inception Edit HTTP 401 (invalid API key)",
                402 => "Inception Edit HTTP 402 (account or payment required)",
                404 => "Inception Edit HTTP 404 (endpoint or model not found)",
                429 => "Inception Edit HTTP 429 (rate limited)",
                500..=599 => "Inception Edit provider server error",
                _ => "Inception Edit request failed",
            }
            .to_string());
        }
        let mut response = response;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| "Inception Edit response read failed")? {
            if bytes.len() + chunk.len() > 256 * 1024 {
                return Err("Inception Edit response too large".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| "Invalid Inception Edit JSON")?;
        let text = value.pointer("/choices/0/message/content").and_then(Value::as_str).ok_or("Missing Inception Edit response content")?;
        Ok((
            text.to_string(),
            value.pointer("/usage/prompt_tokens").and_then(Value::as_u64).unwrap_or(0),
            value.pointer("/usage/completion_tokens").and_then(Value::as_u64).unwrap_or(0),
            value
                .pointer("/usage/cached_tokens")
                .or_else(|| value.pointer("/usage/cached_input_tokens"))
                .and_then(Value::as_u64)
                .unwrap_or(0),
            started.elapsed().as_millis(),
        ))
    };
    tokio::pin!(operation);
    let result = tokio::select! { r = &mut operation => r, _ = async { while !cancel.load(Ordering::Relaxed) { tokio::time::sleep(std::time::Duration::from_millis(20)).await; } } => Err("Cancelled".into()) };
    result.map_err(|message| HttpFailure { message, http_ms: started.elapsed().as_millis() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };
    #[test]
    fn request_uses_official_markers_and_bounded_region() {
        let src = (0..40).map(|n| format!("line{n}\n")).collect::<String>();
        let (range, original, body) =
            build_request("src/main.rs", &src, src.chars().take_while(|&c| c != '\n').count() + 1, &[], 15).unwrap();
        let content = body.pointer("/messages/0/content").unwrap().as_str().unwrap();
        assert!(content.contains("<|recently_viewed_code_snippets|>"));
        assert!(content.contains("<|current_file_content|>"));
        assert!(content.contains("<|code_to_edit|>"));
        assert!(content.contains("<|cursor|>"));
        assert!(content.contains("<|edit_diff_history|>"));
        assert!(original.lines().count() <= 15);
        assert!(range.end > range.start);
        assert_eq!(body["model"], "mercury-edit-2");
        assert_eq!(body["max_tokens"], 256);
    }
    #[test]
    fn history_is_chronological_and_limited_to_five() {
        let history =
            (0..7).map(|n| EditHistoryEntry { start: 0, before: format!("old{n}\n"), after: format!("new{n}\n") }).collect::<Vec<_>>();
        let (_, _, body) = build_request("f.rs", "x\n", 0, &history, 15).unwrap();
        let p = body.pointer("/messages/0/content").unwrap().as_str().unwrap();
        assert!(!p.contains("old1"));
        assert!(p.find("old2").unwrap() < p.find("old6").unwrap());
    }
    #[test]
    fn parses_markdown_or_rejects_bad_fence() {
        assert_eq!(parse_region("```rust\nfn main() {}\n```").unwrap(), "fn main() {}\n");
        assert!(parse_region("```rust\nmissing").is_err());
    }
    #[test]
    fn minimal_diff_is_utf8_safe_and_empty_for_identical() {
        let old = format!("a{}c", '\u{1f642}');
        let new = format!("a{}c", '\u{1f980}');
        assert_eq!(minimal_change(&old, &new, 3), (4..5, "\u{1f642}".into(), "\u{1f980}".into()));
        assert_eq!(minimal_change("same", "same", 0), (4..4, String::new(), String::new()));
    }

    #[test]
    fn rust_go_python_contexts_keep_language_and_cursor_code() {
        for (path, source) in [
            ("main.rs", "fn main() {\n    println!(\"ok\");\n}\n"),
            ("main.go", "package main\nfunc main() {\n    println(\"ok\")\n}\n"),
            ("main.py", "def main():\n    print('ok')\n"),
        ] {
            let cursor = source.chars().take_while(|&c| c != '\n').count();
            let (_, _, body) = build_request(path, source, cursor, &[], 15).unwrap();
            let context = body.pointer("/messages/0/content").unwrap().as_str().unwrap();
            assert!(context.contains(path));
            assert!(context.contains("<|cursor|>"));
            assert!(source.lines().last().is_some_and(|line| context.contains(line)));
        }
    }

    #[test]
    fn prediction_apply_is_atomic_undoable_and_excluded_from_user_history() {
        let old = format!("a{}c", '\u{1f642}');
        let new = format!("a{}c", '\u{1f980}');
        let mut buffer = crate::text::Buffer::new(&old);
        buffer.apply_prediction_mapped(vec![(1..2, "\u{1f980}".into())]);
        assert_eq!(buffer.text(), new);
        assert!(buffer.take_user_edit_events().is_empty());
        buffer.undo();
        assert_eq!(buffer.text(), old);
        buffer.redo();
        assert_eq!(buffer.text(), new);
    }

    #[test]
    fn stale_preview_is_rejected_after_any_buffer_change() {
        let mut buffer = crate::text::Buffer::new("abc");
        let preview = Preview {
            doc_id: 7,
            version: buffer.version,
            cursor: 0,
            range: 1..2,
            expected: "b".into(),
            replacement: "B".into(),
            diff: String::new(),
            metrics: Default::default(),
        };
        assert!(preview.is_current(7, &buffer));
        buffer.type_text("x");
        assert!(!preview.is_current(7, &buffer));
    }

    fn mock_call(status: &str, body: &str) -> Result<(String, u64, u64, u64, u128), HttpFailure> {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let status = status.to_string();
        let body = body.to_string();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(std::time::Duration::from_secs(2))).unwrap();
            let mut req = Vec::new();
            let mut chunk = [0u8; 2048];
            let mut expected_len = None;
            loop {
                let n = stream.read(&mut chunk).unwrap_or(0);
                if n == 0 {
                    break;
                }
                req.extend_from_slice(&chunk[..n]);
                if let Some(end) = req.windows(4).position(|x| x == b"\r\n\r\n") {
                    if expected_len.is_none() {
                        let h = String::from_utf8_lossy(&req[..end]);
                        expected_len = Some(
                            h.lines()
                                .find_map(|line| {
                                    line.to_ascii_lowercase().strip_prefix("content-length:").and_then(|n| n.trim().parse::<usize>().ok())
                                })
                                .unwrap_or(0),
                        );
                    }
                    if req.len() >= end + 4 + expected_len.unwrap_or(0) {
                        break;
                    }
                }
            }
            let headers = String::from_utf8_lossy(&req);
            assert!(headers.to_ascii_lowercase().contains("authorization: bearer test-secret"));
            assert!(headers.starts_with("POST /v1/edit/completions HTTP/1.1"));
            let payload = headers.split_once("\r\n\r\n").unwrap().1;
            assert!(payload.contains("\"model\":\"mercury-edit-2\""));
            assert!(payload.contains("\"role\":\"user\""));
            assert!(!payload.contains("fim/completions"));
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
        });
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let result = rt.block_on(request(
            format!("http://{addr}/v1/edit/completions"),
            "test-secret".into(),
            json!({"model":"mercury-edit-2","messages":[{"role":"user","content":"fixture"}],"max_tokens":256}),
            Arc::new(AtomicBool::new(false)),
        ));
        server.join().unwrap();
        result
    }

    #[test]
    fn mock_http_success_and_api_error_classes() {
        let ok=mock_call("200 OK",r#"{"choices":[{"message":{"content":"```rust\nupdated\n```"}}],"usage":{"prompt_tokens":8,"completion_tokens":2,"cached_tokens":3}}"#).unwrap();
        assert_eq!(ok.0, "```rust\nupdated\n```");
        assert_eq!((ok.1, ok.2, ok.3), (8, 2, 3));
        for (code, label) in
            [(401, "invalid API key"), (402, "payment required"), (404, "not found"), (429, "rate limited"), (503, "server error")]
        {
            let err = mock_call(&format!("{code} Error"), "{}").unwrap_err();
            assert!(err.message.contains(label), "HTTP {code}: {}", err.message);
        }
    }

    #[test]
    fn obsolete_http_request_is_cancelled() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(std::time::Duration::from_secs(2))).unwrap();
            let mut req = [0u8; 4096];
            let _ = stream.read(&mut req);
            thread::sleep(std::time::Duration::from_millis(250));
            let body = r#"{"choices":[{"message":{"content":"```rust\nnew\n```"}}]}"#;
            let response = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            let _ = stream.write_all(response.as_bytes());
        });
        let cancel = Arc::new(AtomicBool::new(false));
        let token = cancel.clone();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let result = rt.block_on(async move {
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(40)).await;
                token.store(true, Ordering::Relaxed);
            });
            request(format!("http://{addr}/v1/edit/completions"), "fixture".into(), json!({"model":"mercury-edit-2"}), cancel).await
        });
        server.join().unwrap();
        assert!(result.unwrap_err().message.contains("Cancelled"));
    }

    #[test]
    #[ignore = "manual live Inception API smoke test; reports timing only, never code or credentials"]
    fn live_inception_next_edit_smoke_timing_only() {
        let total_started = Instant::now();
        let project = Path::new(env!("CARGO_MANIFEST_DIR"));
        let (endpoint, key) = super::super::credentials::inception_route(project, "/v1/edit/completions")
            .expect(".env-fim or environment configuration required");
        let source = "fn square(x: i32) -> i32 {\n    x *\n}\n";
        let cursor = source.find('*').unwrap() + 1;
        let context_started = Instant::now();
        let (_, _, body) = build_request("src/square.rs", source, cursor, &[], 15).unwrap();
        let context_ms = context_started.elapsed().as_millis();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let result =
            rt.block_on(request(endpoint, key, body, Arc::new(AtomicBool::new(false)))).expect("live Inception Edit API request failed");
        let _region = parse_region(&result.0).expect("API response was not a fenced updated region");
        eprintln!(
            "Next Edit smoke: context {} ms; HTTP {} ms; total {} ms; prompt {} tokens; completion {} tokens; cached {} tokens; response {} chars",
            context_ms,
            result.4,
            total_started.elapsed().as_millis(),
            result.1,
            result.2,
            result.3,
            result.0.chars().count()
        );
    }
}
