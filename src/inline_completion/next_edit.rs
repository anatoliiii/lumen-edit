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

use super::budget::{Ledger, Ticket};
use crate::text::UserEditEvent;

const HISTORY_ENTRIES: usize = 5;
const HISTORY_BYTES: usize = 16 * 1024;
const ENTRY_BYTES: usize = 4 * 1024;
const CONTEXT_BYTES: usize = 512;

/// One recent user edit as a small hunk: `before`/`after` surrounded by the
/// unchanged context that existed at edit time, starting at line `line`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EditHistoryEntry {
    pub start: usize,
    pub line: usize,
    pub context_before: String,
    pub before: String,
    pub after: String,
    pub context_after: String,
}
impl EditHistoryEntry {
    fn bytes(&self) -> usize {
        self.context_before.len() + self.before.len() + self.after.len() + self.context_after.len()
    }
    /// Caps the entry at ENTRY_BYTES: contexts lose their far ends (keeping
    /// `line` accurate), the edited texts keep their heads plus a marker.
    fn bounded(mut self) -> Self {
        if self.context_before.len() > CONTEXT_BYTES {
            let mut cut = self.context_before.len() - CONTEXT_BYTES;
            while !self.context_before.is_char_boundary(cut) {
                cut += 1;
            }
            self.line += self.context_before[..cut].matches('\n').count();
            self.context_before.drain(..cut);
        }
        truncate_head(&mut self.context_after, CONTEXT_BYTES);
        let room = ENTRY_BYTES.saturating_sub(self.context_before.len() + self.context_after.len()) / 2;
        let (before, after) = (self.before.len(), self.after.len());
        if before + after > room * 2 {
            // Give unused room of the smaller side to the larger one.
            let before_room = if after < room { room * 2 - after } else { room };
            let after_room = if before < room { room * 2 - before } else { room };
            truncate_head(&mut self.before, before_room);
            truncate_head(&mut self.after, after_room);
        }
        self
    }
}

const TRUNCATED: &str = "…\n";

fn truncate_head(text: &mut String, max: usize) {
    if text.len() <= max {
        return;
    }
    let mut cut = max.saturating_sub(TRUNCATED.len());
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    text.truncate(cut);
    text.push_str(TRUNCATED);
}

/// Recent user edits for one document, bounded in entries and bytes after
/// every push or merge.
#[derive(Clone, Debug, Default)]
pub struct EditHistory {
    entries: VecDeque<EditHistoryEntry>,
    bytes: usize,
}
impl EditHistory {
    pub fn entries(&self) -> impl Iterator<Item = &EditHistoryEntry> {
        self.entries.iter()
    }
    #[cfg(test)]
    fn bytes(&self) -> usize {
        self.bytes
    }
    pub fn push(&mut self, event: UserEditEvent) {
        if let Some(last) = self.entries.back_mut()
            && let Some(merged) = merge(last, &event)
            && merged.bytes() <= ENTRY_BYTES
        {
            self.bytes = self.bytes - last.bytes() + merged.bytes();
            *last = merged;
        } else {
            let entry = EditHistoryEntry {
                start: event.start,
                line: event.line,
                context_before: event.context_before,
                before: event.before,
                after: event.after,
                context_after: event.context_after,
            }
            .bounded();
            self.bytes += entry.bytes();
            self.entries.push_back(entry);
        }
        while self.entries.len() > 1 && (self.entries.len() > HISTORY_ENTRIES || self.bytes > HISTORY_BYTES) {
            match self.entries.pop_front() {
                Some(removed) => self.bytes -= removed.bytes(),
                None => unreachable!("entries.len() > 1"),
            }
        }
    }
}

/// Continues the previous entry for contiguous typing or deletion.
fn merge(last: &EditHistoryEntry, event: &UserEditEvent) -> Option<EditHistoryEntry> {
    let mut merged = last.clone();
    if event.typing && event.before.is_empty() && !last.after.is_empty() && event.start == last.start + last.after.chars().count() {
        merged.after.push_str(&event.after);
    } else if event.deleting && event.after.is_empty() && !last.before.is_empty() && last.after.is_empty() {
        if event.start + event.before.chars().count() == last.start {
            // Backspace: the deletion grows to the left.
            merged.before = format!("{}{}", event.before, last.before);
            merged.start = event.start;
            merged.line = event.line;
            merged.context_before = event.context_before.clone();
        } else if event.start == last.start {
            // Delete key: the deletion grows to the right.
            merged.before.push_str(&event.before);
            merged.context_after = event.context_after.clone();
        } else {
            return None;
        }
    } else {
        return None;
    }
    Some(merged)
}

/// A single unified-diff hunk with the entry's real line numbers.
fn history_hunk(path: &str, h: &EditHistoryEntry) -> String {
    let old = format!("{}{}{}", h.context_before, h.before, h.context_after);
    let new = format!("{}{}{}", h.context_before, h.after, h.context_after);
    let count = |s: &str| s.split_inclusive('\n').count();
    let mut out = format!("--- {path}\n+++ {path}\n@@ -{},{} +{},{} @@\n", h.line + 1, count(&old), h.line + 1, count(&new));
    for change in similar::TextDiff::from_lines(&old, &new).iter_all_changes() {
        out.push_str(match change.tag() {
            similar::ChangeTag::Equal => " ",
            similar::ChangeTag::Delete => "-",
            similar::ChangeTag::Insert => "+",
        });
        out.push_str(change.value().trim_end_matches(['\r', '\n']));
        out.push('\n');
    }
    out
}

#[derive(Clone, Debug)]
pub struct Preview {
    pub doc_id: u64,
    pub version: u64,
    pub cursor: usize,
    pub range: Range<usize>,
    pub expected: String,
    pub replacement: String,
    pub rows: Vec<DiffRow>,
    pub metrics: RequestMetrics,
    /// Shown in the review when the syntax check could not fully vouch for it.
    pub warning: Option<String>,
}

#[derive(Clone, Debug)]
pub struct DiffRow {
    pub old_line: Option<usize>,
    pub new_line: Option<usize>,
    pub tag: similar::ChangeTag,
    pub text: String,
}

fn diff_rows(original: &str, replacement: &str, first_line: usize) -> Vec<DiffRow> {
    similar::TextDiff::from_lines(original, replacement)
        .iter_all_changes()
        .map(|change| DiffRow {
            old_line: change.old_index().map(|n| first_line + n),
            new_line: change.new_index().map(|n| first_line + n),
            tag: change.tag(),
            text: change.value().trim_end_matches('\n').trim_end_matches('\r').to_string(),
        })
        .collect()
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
    first_line: usize,
    content: Result<Reply, HttpFailure>,
    /// The request reached the provider, so its cost is owed.
    sent: bool,
    started: Instant,
    context_ms: u128,
}

#[derive(Clone, Debug)]
struct Reply {
    text: String,
    finish_reason: Option<String>,
    prompt_tokens: u64,
    completion_tokens: u64,
    cached_tokens: u64,
    cost: Option<f64>,
    http_ms: u128,
    /// Checked on the worker: the region with the file's line endings, or why
    /// it was rejected, plus an optional review warning.
    region: Result<String, String>,
    warning: Option<String>,
}

impl Default for Reply {
    fn default() -> Self {
        Self {
            text: String::new(),
            finish_reason: None,
            prompt_tokens: 0,
            completion_tokens: 0,
            cached_tokens: 0,
            cost: None,
            http_ms: 0,
            region: Err("Next Edit reply was not checked".into()),
            warning: None,
        }
    }
}

#[derive(Clone, Debug)]
struct HttpFailure {
    message: String,
    http_ms: u128,
}

/// A reserved request and the flag its worker sets right before sending.
type Reservation = (Ticket, Arc<AtomicBool>);

#[derive(Default)]
pub struct Controller {
    rx: Option<Receiver<RequestResult>>,
    cancel: Option<Arc<AtomicBool>>,
    reservation: Option<Reservation>,
    /// Cancelled requests whose reservation is resolved on the next tick.
    abandoned: Vec<Reservation>,
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
    lang: crate::syntax::Lang,
    history: Vec<EditHistoryEntry>,
    region_lines: usize,
    started: Instant,
}

impl Controller {
    fn record_metrics(&mut self, metrics: RequestMetrics) {
        self.metrics.add_latency(metrics.total_latency_ms);
        self.metrics.latest = metrics;
    }
    #[allow(clippy::too_many_arguments)]
    pub fn launch(
        &mut self,
        project: &Path,
        path: &str,
        doc_id: u64,
        buffer: &crate::text::Buffer,
        lang: crate::syntax::Lang,
        history: &[EditHistoryEntry],
        region_lines: usize,
        ticket: Ticket,
    ) {
        self.invalidate();
        let started = Instant::now();
        let cursor = buffer.primary().head;
        let cancel = Arc::new(AtomicBool::new(false));
        let token = cancel.clone();
        let sent = Arc::new(AtomicBool::new(false));
        let sending = sent.clone();
        let (tx, rx) = unbounded();
        let snapshot = RequestSnapshot {
            project: project.to_path_buf(),
            path: path.to_string(),
            doc_id,
            version: buffer.version,
            cursor,
            rope: buffer.rope.clone(),
            lang,
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
                        .and_then(|rt| {
                            // Pairs with `invalidate`: either it sees `sent`, or
                            // this sees the cancellation and never sends.
                            sending.store(true, Ordering::SeqCst);
                            if token.load(Ordering::SeqCst) {
                                sending.store(false, Ordering::SeqCst);
                                return Err(HttpFailure { message: "Cancelled".into(), http_ms: 0 });
                            }
                            rt.block_on(request(endpoint, key, body, token))
                        })
                        .map(|mut reply| {
                            let checked = evaluate(snapshot.lang, &snapshot.rope, range.clone(), &original, &reply);
                            match checked {
                                Ok((region, warning)) => {
                                    reply.region = Ok(region);
                                    reply.warning = warning;
                                }
                                Err(error) => reply.region = Err(error),
                            }
                            reply
                        });
                    (range, original, result)
                }
                Err(error) => (0..0, String::new(), Err(HttpFailure { message: error, http_ms: 0 })),
            };
            let first_line = snapshot.rope.char_to_line(range.start) + 1;
            let sent = sending.load(Ordering::SeqCst);
            let _ =
                tx.send(RequestResult { first_line, doc_id, version, cursor, range, original, content: result, sent, started, context_ms });
        });
        self.rx = Some(rx);
        self.cancel = Some(cancel);
        self.reservation = Some((ticket, sent));
        self.pending_identity = Some((doc_id, buffer.version, cursor));
        self.busy = true;
        self.metrics.requests += 1;
        self.status = "Next Edit is generating a preview…".into();
    }

    pub fn invalidate(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel.store(true, Ordering::SeqCst);
        }
        self.abandoned.extend(self.reservation.take());
        self.rx = None;
        self.busy = false;
        self.pending_identity = None;
    }

    /// Returns the reservation of a request cancelled before sending; a sent
    /// one keeps it, since the provider may still bill it.
    pub fn settle_abandoned(&mut self, ledger: &mut Ledger) {
        for (ticket, sent) in self.abandoned.drain(..) {
            if sent.load(Ordering::SeqCst) { ledger.settle(ticket, None) } else { ledger.release(ticket) }
        }
    }

    pub fn poll(&mut self, current: Option<(u64, u64, usize)>, ledger: &mut Ledger) -> Option<Preview> {
        let result = self.rx.as_ref()?.try_recv().ok()?;
        self.busy = false;
        self.cancel = None;
        self.pending_identity = None;
        self.rx = None;
        if let Some((ticket, _)) = self.reservation.take() {
            if result.sent { ledger.settle(ticket, result.content.as_ref().ok().and_then(|r| r.cost)) } else { ledger.release(ticket) }
        }
        if let Err(e) = &result.content
            && e.message.contains("HTTP 429")
        {
            ledger.backoff();
        }
        if current != Some((result.doc_id, result.version, result.cursor)) {
            self.record_metrics(RequestMetrics {
                context_prepare_ms: result.context_ms,
                total_latency_ms: result.started.elapsed().as_millis(),
                ..Default::default()
            });
            self.status = "Next Edit discarded: document changed".into();
            return None;
        }
        let reply = match result.content {
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
            http_latency_ms: reply.http_ms,
            total_latency_ms: result.started.elapsed().as_millis(),
            prompt_tokens: reply.prompt_tokens,
            completion_tokens: reply.completion_tokens,
            cached_tokens: reply.cached_tokens,
            ..Default::default()
        };
        let normalized = match reply.region {
            Ok(s) => s,
            Err(e) => {
                self.metrics.errors += 1;
                metrics.diff_prepare_ms = diff_started.elapsed().as_millis();
                self.record_metrics(metrics);
                self.status = e;
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
        let rows = diff_rows(&result.original, &normalized, result.first_line);
        let diff_ms = diff_started.elapsed().as_millis();
        metrics.diff_prepare_ms = diff_ms;
        self.record_metrics(metrics.clone());
        self.status = "Review Next Edit, then accept or dismiss it".into();
        Some(Preview {
            doc_id: result.doc_id,
            version: result.version,
            cursor: result.cursor,
            range,
            expected,
            replacement,
            rows,
            metrics,
            warning: reply.warning,
        })
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel.store(true, Ordering::SeqCst);
        }
    }
}

/// Turns a raw reply into the replacement region: rejects truncated output
/// and replacements that add syntax errors to the whole document.
fn evaluate(
    lang: crate::syntax::Lang,
    rope: &ropey::Rope,
    range: Range<usize>,
    original: &str,
    reply: &Reply,
) -> Result<(String, Option<String>), String> {
    if reply.finish_reason.as_deref() == Some("length") {
        return Err("Next Edit response was cut off by the output token limit".into());
    }
    let region = parse_region(&reply.text).map_err(String::from)?;
    let region = match_line_endings(&region, original, rope);
    if region == original {
        return Ok((region, None));
    }
    let source = rope.to_string();
    let bytes = rope.char_to_byte(range.start)..rope.char_to_byte(range.end);
    let warning = super::validation::check_replacement(lang, &source, bytes, &region)?;
    Ok((region, warning.map(String::from)))
}

/// Models answer with `\n`; a CRLF file must get CRLF back, and vice versa.
fn match_line_endings(region: &str, original: &str, rope: &ropey::Rope) -> String {
    let crlf = if original.contains('\n') { original.contains("\r\n") } else { rope.chars().any(|c| c == '\r') };
    let lf = region.replace("\r\n", "\n");
    if crlf { lf.replace('\n', "\r\n") } else { lf }
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
    // Char offset of each line start, plus the end of the file; real
    // terminator lengths, so CRLF and Unicode lines are measured exactly.
    let mut starts = vec![0];
    for piece in file.split_inclusive('\n') {
        let prev = starts[starts.len() - 1];
        starts.push(prev + piece.chars().count());
    }
    let line_count = starts.len() - 1;
    let line_start = |line: usize| starts[line.min(line_count)];
    let line = chars[..cursor].iter().filter(|&&c| c == '\n').count();
    let lines = region_lines.clamp(1, 15);
    let start_line = line.saturating_sub(5.min(lines.saturating_sub(1)));
    let end_line = (start_line + lines).min(line_count);
    let mut start = line_start(start_line);
    let mut end = line_start(end_line).max(start);
    // Bound pathological/minified lines while keeping the cursor inside the
    // editable window and preserving character boundaries.
    if end.saturating_sub(start) > 8192 {
        start = cursor.saturating_sub(4096).max(start);
        end = (cursor + 4096).min(end);
    }
    let range = start..end;
    let original: String = chars[start..end].iter().collect();
    let cursor_local = cursor.saturating_sub(start).min(original.chars().count());
    let above_start = line_start(start_line.saturating_sub(30)).min(start);
    let below_end = line_start(end_line + 30).max(end);
    let before: String = chars[above_start.max(start.saturating_sub(4096))..start].iter().collect();
    let after: String = chars[end..below_end.min(end.saturating_add(4096))].iter().collect();
    let region_before: String = original.chars().take(cursor_local).collect();
    let region_after: String = original.chars().skip(cursor_local).collect();
    let skip = history.len().saturating_sub(HISTORY_ENTRIES);
    let diff_history = history[skip..].iter().map(|h| history_hunk(path, h)).collect::<Vec<_>>().join("\n");
    let content = format!(
        "<|recently_viewed_code_snippets|>\n\n<|/recently_viewed_code_snippets|>\n\n<|current_file_content|>\ncurrent_file_path: {path}\n{before}<|code_to_edit|>\n{region_before}\n<|cursor|>\n{region_after}<|/code_to_edit|>\n{after}\n<|/current_file_content|>\n\n<|edit_diff_history|>\n{diff_history}\n<|/edit_diff_history|>"
    );
    let body = json!({"model":"mercury-edit-2","messages":[{"role":"user","content":content}],"max_tokens":output_budget(&original)});
    Ok((range, original, body))
}

/// The model rewrites the whole region: allow roughly two tokens per three
/// characters of it plus room for fences and growth.
fn output_budget(region: &str) -> u32 {
    (region.chars().count() as u32 * 2 / 3 + 128).clamp(256, 4096)
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

async fn request(endpoint: String, key: String, body: Value, cancel: Arc<AtomicBool>) -> Result<Reply, HttpFailure> {
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
        Ok(Reply {
            text: text.to_string(),
            finish_reason: value.pointer("/choices/0/finish_reason").and_then(Value::as_str).map(String::from),
            prompt_tokens: value.pointer("/usage/prompt_tokens").and_then(Value::as_u64).unwrap_or(0),
            completion_tokens: value.pointer("/usage/completion_tokens").and_then(Value::as_u64).unwrap_or(0),
            cached_tokens: value
                .pointer("/usage/cached_tokens")
                .or_else(|| value.pointer("/usage/cached_input_tokens"))
                .and_then(Value::as_u64)
                .unwrap_or(0),
            cost: value.pointer("/usage/cost").and_then(Value::as_f64),
            http_ms: started.elapsed().as_millis(),
            ..Default::default()
        })
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
    fn cancelled_ready_response_cannot_resurrect_preview() {
        let (tx, rx) = unbounded();
        let mut controller = Controller::default();
        controller.rx = Some(rx);
        controller.busy = true;
        tx.send(RequestResult {
            doc_id: 1,
            version: 0,
            cursor: 0,
            range: 0..2,
            original: "a\n".into(),
            first_line: 1,
            content: Ok(Reply { text: "```rust\nb\n```".into(), region: Ok("b\n".into()), ..Default::default() }),
            sent: true,
            started: Instant::now(),
            context_ms: 0,
        })
        .unwrap_or_else(|e| panic!("send failed: {e:?}"));
        controller.invalidate();
        assert!(controller.poll(Some((1, 0, 0)), &mut Ledger::default()).is_none());
        assert!(!controller.busy);
    }

    #[test]
    fn cancellation_resolves_reservation_by_whether_it_was_sent() {
        let settings = super::super::Settings::default();
        let mut ledger = Ledger::default();
        let mut controller = Controller::default();
        let unsent = ledger.reserve(&settings).unwrap_or_else(|e| panic!("reserve failed: {e:?}"));
        controller.reservation = Some((unsent, Arc::new(AtomicBool::new(false))));
        controller.invalidate();
        let sent = ledger.reserve(&settings).unwrap_or_else(|e| panic!("reserve failed: {e:?}"));
        controller.reservation = Some((sent, Arc::new(AtomicBool::new(true))));
        controller.invalidate();
        controller.settle_abandoned(&mut ledger);
        assert_eq!(ledger.requests, 1);
        assert!((ledger.reserved_usd - settings.reserve_per_request_usd).abs() < 1e-9);
    }

    #[test]
    fn crlf_region_boundaries_are_exact() {
        let src = "a\r\nb\r\nc";
        let (range, original, _) = build_request("f.rs", src, 6, &[], 1).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((range, original.as_str()), (6..7, "c"));
        let (range, original, _) = build_request("f.rs", src, 3, &[], 1).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((range, original.as_str()), (3..6, "b\r\n"));
        let src = "\u{44f}\u{44f}\r\n\u{1f642}\r\nend\r\n";
        let cursor = src.chars().position(|c| c == 'e').unwrap_or_else(|| panic!("expected 'e'"));
        let (range, original, _) = build_request("f.rs", src, cursor, &[], 1).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(original, "end\r\n");
        assert_eq!(range, cursor..cursor + 5);
    }

    #[test]
    fn region_edges_empty_file_trailing_newline_and_no_newline() {
        assert_eq!(build_request("f.rs", "", 0, &[], 15).unwrap_or_else(|e| panic!("{e}")).0, 0..0);
        // Cursor on the empty line after a final newline.
        let (range, original, _) = build_request("f.rs", "a\nb\n", 4, &[], 15).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((range, original.as_str()), (0..4, "a\nb\n"));
        let (range, _, _) = build_request("f.rs", "a\nb\n", 4, &[], 1).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(range, 4..4);
        let (range, original, _) = build_request("f.rs", "a\nb", 3, &[], 1).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((range, original.as_str()), (2..3, "b"));
    }

    #[test]
    fn crlf_style_is_restored_in_model_reply() {
        let rope = ropey::Rope::from_str("x\r\ny\r\n");
        assert_eq!(match_line_endings("a\nb\n", "x\r\ny\r\n", &rope), "a\r\nb\r\n");
        assert_eq!(match_line_endings("a\r\nb\n", "x\ny\n", &ropey::Rope::from_str("x\ny\n")), "a\nb\n");
        assert_eq!(match_line_endings("a\n", "", &rope), "a\r\n");
    }

    #[test]
    fn truncated_or_breaking_reply_is_rejected() {
        let rope = ropey::Rope::from_str("fn a() {\n    x();\n}\n");
        let range = 9..18;
        let original = "    x();\n";
        let reply =
            |text: &str, finish: Option<&str>| Reply { text: text.into(), finish_reason: finish.map(String::from), ..Default::default() };
        let good = reply("```rust\n    y();\n```", Some("stop"));
        assert_eq!(
            evaluate(crate::syntax::Lang::Rust, &rope, range.clone(), original, &good).unwrap_or_else(|e| panic!("{e}")),
            ("    y();\n".into(), None)
        );
        let cut = reply("```rust\n    y();\n```", Some("length"));
        assert!(matches!(evaluate(crate::syntax::Lang::Rust, &rope, range.clone(), original, &cut), Err(e) if e.contains("cut off")));
        let broken = reply("```rust\n    y(;\n}\nfn z( {\n```", None);
        assert!(matches!(evaluate(crate::syntax::Lang::Rust, &rope, range, original, &broken), Err(e) if e.contains("syntax")));
    }

    #[test]
    fn output_budget_grows_with_the_region() {
        assert_eq!(output_budget("short\n"), 256);
        let long = "x".repeat(3000);
        assert!(output_budget(&long) >= 2000);
        assert_eq!(output_budget(&"x".repeat(100_000)), 4096);
    }

    fn event(start: usize, before: &str, after: &str, typing: bool, deleting: bool) -> UserEditEvent {
        UserEditEvent {
            start,
            before: before.into(),
            after: after.into(),
            typing,
            deleting,
            line: 0,
            context_before: String::new(),
            context_after: String::new(),
        }
    }

    #[test]
    fn history_merges_typing_and_deletions_and_stays_bounded() {
        let mut history = EditHistory::default();
        for (n, c) in "hello".chars().enumerate() {
            history.push(event(n, "", &c.to_string(), true, false));
        }
        assert_eq!(history.entries().count(), 1);
        assert_eq!(history.entries().next().unwrap_or_else(|| panic!("expected entry")).after, "hello");
        // Backspace and Delete grow one deletion.
        let mut history = EditHistory::default();
        history.push(event(5, "e", "", false, true));
        history.push(event(4, "d", "", false, true));
        history.push(event(4, "f", "", false, true));
        assert_eq!(history.entries().map(|e| (e.start, e.before.as_str())).collect::<Vec<_>>(), [(4, "def")]);
        // A long typing run stops merging at the entry cap.
        let mut history = EditHistory::default();
        for n in 0..10_000 {
            history.push(event(n, "", "\u{44f}", true, false));
            assert!(history.bytes() <= HISTORY_BYTES);
            assert!(history.entries().all(|e| e.bytes() <= ENTRY_BYTES));
        }
        assert_eq!(history.bytes(), history.entries().map(EditHistoryEntry::bytes).sum::<usize>());
        // One huge paste is truncated on a char boundary, not dropped.
        let mut history = EditHistory::default();
        history.push(event(0, "", &"\u{1f642}".repeat(100_000), false, false));
        let entry = history.entries().next().unwrap_or_else(|| panic!("expected entry"));
        assert!(entry.bytes() <= ENTRY_BYTES);
        assert!(entry.after.ends_with(TRUNCATED));
        // Only as many old entries are dropped as the limits require.
        let mut history = EditHistory::default();
        for n in 0..4 {
            history.push(event(n * 100, "", &"x".repeat(3584), false, false));
        }
        assert_eq!(history.entries().count(), 4);
        history.push(event(900, "", &"x".repeat(3584), false, false));
        assert_eq!(history.entries().count(), 4);
        assert!(history.bytes() <= HISTORY_BYTES);
    }

    #[test]
    fn history_hunk_uses_real_line_numbers_and_context() {
        let mut buffer = crate::text::Buffer::new("l1\nl2\nl3\nl4 old\nl5\nl6\nl7\n");
        let at = buffer.text().find("old").unwrap_or_else(|| panic!("expected \"old\""));
        buffer.apply_edits_with_selection(vec![(at..at + 3, "new".into())], vec![crate::text::Selection::cursor(at + 3)]);
        let mut history = EditHistory::default();
        for event in buffer.take_user_edit_events() {
            history.push(event);
        }
        let entry = history.entries().next().unwrap_or_else(|| panic!("expected entry"));
        let hunk = history_hunk("src/f.rs", entry);
        assert!(hunk.starts_with("--- src/f.rs\n+++ src/f.rs\n@@ -2,5 +2,5 @@\n"), "{hunk}");
        assert!(hunk.contains("\n-l4 old\n+l4 new\n"));
        assert!(hunk.contains(" l2\n") && hunk.contains(" l6\n") && !hunk.contains("l7"));
        let src = (0..200).map(|n| format!("line{n}\n")).collect::<String>();
        let (_, _, body) = build_request("src/f.rs", &src, 0, std::slice::from_ref(entry), 15).unwrap_or_else(|e| panic!("{e}"));
        assert!(
            body.pointer("/messages/0/content")
                .unwrap_or_else(|| panic!("expected content"))
                .as_str()
                .unwrap_or_else(|| panic!("expected string"))
                .contains("@@ -2,5 +2,5 @@")
        );
    }

    #[test]
    fn bounded_entry_keeps_line_number_of_trimmed_context() {
        let entry =
            EditHistoryEntry { line: 10, context_before: format!("{}\nkept", "a\n".repeat(400)), after: "x".into(), ..Default::default() }
                .bounded();
        assert!(entry.context_before.len() <= CONTEXT_BYTES);
        let dropped = 401 - entry.context_before.matches('\n').count();
        assert_eq!(entry.line, 10 + dropped);
    }

    #[test]
    fn review_rows_use_file_numbers_and_preserve_indentation() {
        let rows = diff_rows("same\n    old\n", "same\n    \u{43d}\u{43e}\u{432}\u{44b}\u{439}\n", 35);
        assert_eq!(rows.len(), 3);
        assert_eq!((rows[0].old_line, rows[0].new_line), (Some(35), Some(35)));
        assert_eq!((rows[1].old_line, rows[1].new_line), (Some(36), None));
        assert_eq!((rows[2].old_line, rows[2].new_line), (None, Some(36)));
        assert_eq!(rows[2].text, "    \u{43d}\u{43e}\u{432}\u{44b}\u{439}");
    }

    #[test]
    fn request_uses_official_markers_and_bounded_region() {
        let src = (0..40).map(|n| format!("line{n}\n")).collect::<String>();
        let (range, original, body) = build_request("src/main.rs", &src, src.chars().take_while(|&c| c != '\n').count() + 1, &[], 15)
            .unwrap_or_else(|e| panic!("{e}"));
        let content = body
            .pointer("/messages/0/content")
            .unwrap_or_else(|| panic!("expected content"))
            .as_str()
            .unwrap_or_else(|| panic!("expected string"));
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
        let history = (0..7)
            .map(|n| EditHistoryEntry { before: format!("old{n}\n"), after: format!("new{n}\n"), ..Default::default() })
            .collect::<Vec<_>>();
        let (_, _, body) = build_request("f.rs", "x\n", 0, &history, 15).unwrap_or_else(|e| panic!("{e}"));
        let p = body
            .pointer("/messages/0/content")
            .unwrap_or_else(|| panic!("expected content"))
            .as_str()
            .unwrap_or_else(|| panic!("expected string"));
        assert!(!p.contains("old1"));
        let old2 = p.find("old2").unwrap_or_else(|| panic!("expected old2"));
        let old6 = p.find("old6").unwrap_or_else(|| panic!("expected old6"));
        assert!(old2 < old6);
    }
    #[test]
    fn parses_markdown_or_rejects_bad_fence() {
        assert_eq!(parse_region("```rust\nfn main() {}\n```").unwrap_or_else(|e| panic!("{e}")), "fn main() {}\n");
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
            let (_, _, body) = build_request(path, source, cursor, &[], 15).unwrap_or_else(|e| panic!("{e}"));
            let context = body
                .pointer("/messages/0/content")
                .unwrap_or_else(|| panic!("expected content"))
                .as_str()
                .unwrap_or_else(|| panic!("expected string"));
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
            rows: Vec::new(),
            metrics: Default::default(),
            warning: None,
        };
        assert!(preview.is_current(7, &buffer));
        buffer.type_text("x");
        assert!(!preview.is_current(7, &buffer));
    }

    fn mock_call(status: &str, body: &str) -> Result<Reply, HttpFailure> {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|e| panic!("bind failed: {e}"));
        let addr = listener.local_addr().unwrap_or_else(|e| panic!("local_addr failed: {e}"));
        let status = status.to_string();
        let body = body.to_string();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap_or_else(|e| panic!("accept failed: {e}"));
            stream.set_read_timeout(Some(std::time::Duration::from_secs(2))).unwrap_or_else(|e| panic!("set_read_timeout failed: {e}"));
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
            let payload = headers.split_once("\r\n\r\n").unwrap_or_else(|| panic!("missing header separator")).1;
            assert!(payload.contains("\"model\":\"mercury-edit-2\""));
            assert!(payload.contains("\"role\":\"user\""));
            assert!(!payload.contains("fim/completions"));
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap_or_else(|e| panic!("write failed: {e}"));
        });
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap_or_else(|e| panic!("runtime build failed: {e}"));
        let result = rt.block_on(request(
            format!("http://{addr}/v1/edit/completions"),
            "test-secret".into(),
            json!({"model":"mercury-edit-2","messages":[{"role":"user","content":"fixture"}],"max_tokens":256}),
            Arc::new(AtomicBool::new(false)),
        ));
        server.join().unwrap_or_else(|e| panic!("server join failed: {e:?}"));
        result
    }

    #[test]
    fn mock_http_success_and_api_error_classes() {
        let ok=mock_call("200 OK",r#"{"choices":[{"message":{"content":"```rust\nupdated\n```"},"finish_reason":"length"}],"usage":{"prompt_tokens":8,"completion_tokens":2,"cached_tokens":3}}"#).unwrap_or_else(|e| panic!("mock call failed: {e:?}"));
        assert_eq!(ok.text, "```rust\nupdated\n```");
        assert_eq!((ok.prompt_tokens, ok.completion_tokens, ok.cached_tokens), (8, 2, 3));
        assert_eq!(ok.finish_reason.as_deref(), Some("length"));
        for (code, label) in
            [(401, "invalid API key"), (402, "payment required"), (404, "not found"), (429, "rate limited"), (503, "server error")]
        {
            let err = match mock_call(&format!("{code} Error"), "{}") {
                Err(e) => e,
                Ok(_) => panic!("expected HTTP error"),
            };
            assert!(err.message.contains(label), "HTTP {code}: {}", err.message);
        }
    }

    #[test]
    fn obsolete_http_request_is_cancelled() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|e| panic!("bind failed: {e}"));
        let addr = listener.local_addr().unwrap_or_else(|e| panic!("local_addr failed: {e}"));
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap_or_else(|e| panic!("accept failed: {e}"));
            stream.set_read_timeout(Some(std::time::Duration::from_secs(2))).unwrap_or_else(|e| panic!("set_read_timeout failed: {e}"));
            let mut req = [0u8; 4096];
            let _ = stream.read(&mut req);
            thread::sleep(std::time::Duration::from_millis(250));
            let body = r#"{"choices":[{"message":{"content":"```rust\nnew\n```"}}]}"#;
            let response = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            let _ = stream.write_all(response.as_bytes());
        });
        let cancel = Arc::new(AtomicBool::new(false));
        let token = cancel.clone();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap_or_else(|e| panic!("runtime build failed: {e}"));
        let result = rt.block_on(async move {
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(40)).await;
                token.store(true, Ordering::Relaxed);
            });
            request(format!("http://{addr}/v1/edit/completions"), "fixture".into(), json!({"model":"mercury-edit-2"}), cancel).await
        });
        server.join().unwrap_or_else(|e| panic!("server join failed: {e:?}"));
        assert!(matches!(result, Err(e) if e.message.contains("Cancelled")));
    }

    #[test]
    #[ignore = "manual live Inception API smoke test; reports timing only, never code or credentials"]
    fn live_inception_next_edit_smoke_timing_only() {
        let total_started = Instant::now();
        let project = Path::new(env!("CARGO_MANIFEST_DIR"));
        let (endpoint, key) = super::super::credentials::inception_route(project, "/v1/edit/completions")
            .unwrap_or_else(|e| panic!(".env-fim or environment configuration required: {e}"));
        let source = "fn square(x: i32) -> i32 {\n    x *\n}\n";
        let cursor = source.find('*').unwrap_or_else(|| panic!("expected '*'")) + 1;
        let context_started = Instant::now();
        let (_, _, body) = build_request("src/square.rs", source, cursor, &[], 15).unwrap_or_else(|e| panic!("{e}"));
        let context_ms = context_started.elapsed().as_millis();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap_or_else(|e| panic!("runtime build failed: {e}"));
        let result = rt
            .block_on(request(endpoint, key, body, Arc::new(AtomicBool::new(false))))
            .unwrap_or_else(|e| panic!("live Inception Edit API request failed: {e:?}"));
        let _region = parse_region(&result.text).unwrap_or_else(|e| panic!("API response was not a fenced updated region: {e}"));
        eprintln!(
            "Next Edit smoke: context {} ms; HTTP {} ms; total {} ms; prompt {} tokens; completion {} tokens; cached {} tokens; response {} chars",
            context_ms,
            result.http_ms,
            total_started.elapsed().as_millis(),
            result.prompt_tokens,
            result.completion_tokens,
            result.cached_tokens,
            result.text.chars().count()
        );
    }
}
