use crate::{
    inline_completion::{
        Context, ProviderMode, Settings,
        provider::{CompletionProvider, InceptionFimProvider, OpenRouterProvider, Output},
        semantic::{self, Snapshot},
    },
    text::{Buffer, Selection},
};
use crossbeam_channel::{Receiver, Sender, unbounded};
use eframe::egui;
use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Identity {
    pub project: String,
    pub doc: u64,
    pub version: u64,
    pub cursor: usize,
    pub epoch: u64,
}
#[derive(Clone)]
pub struct Ghost {
    pub identity: Identity,
    pub text: String,
}
impl Ghost {
    pub fn valid(&self, doc: u64, buffer: &Buffer) -> bool {
        self.identity.doc == doc
            && self.identity.version == buffer.version
            && self.identity.cursor == buffer.primary().head
            && buffer.selections.len() == 1
            && buffer.primary().is_empty()
    }
    pub fn accept(self, doc: u64, buffer: &mut Buffer) -> bool {
        if !self.valid(doc, buffer) {
            return false;
        }
        let at = self.identity.cursor;
        let end = at + self.text.chars().count();
        buffer.apply_edits_with_selection(vec![(at..at, self.text)], vec![Selection::cursor(end)]);
        true
    }
}
#[derive(Default)]
pub struct Metrics {
    pub requests: u64,
    pub success: u64,
    pub errors: u64,
    pub accepted: u64,
    pub rejected: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    pub reserved_usd: f64,
    pub latency_ms: u128,
    pub debounce_ms: u128,
    pub context_prepare_ms: u128,
    pub http_request_ms: u128,
    pub total_latency_ms: u128,
    pub display_ms: u128,
    pub cached_tokens: u64,
    pub cache_hits: u64,
    pub rejection_reason: String,
    latency_samples: VecDeque<u128>,
}
struct Done {
    identity: Identity,
    serial: u64,
    context: Option<Context>,
    result: Result<Output, String>,
    latency: Duration,
    debounce_ms: u128,
    context_prepare_ms: u128,
    charged: bool,
}
struct Active {
    cancel: Arc<AtomicBool>,
    busy: Arc<AtomicBool>,
}
impl Drop for Active {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
struct Cached {
    key: u64,
    text: String,
    created: Instant,
}
struct CompletionCache {
    entries: VecDeque<Cached>,
}
impl CompletionCache {
    fn key(context: &Context, settings: &Settings) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        context.language.hash(&mut hash);
        context.semantic.hash(&mut hash);
        context.prefix.hash(&mut hash);
        context.suffix.hash(&mut hash);
        settings.model.hash(&mut hash);
        settings.provider.hash(&mut hash);
        settings.temperature.to_bits().hash(&mut hash);
        settings.max_tokens.hash(&mut hash);
        hash.finish()
    }
    #[cfg(test)]
    fn get(&self, key: u64) -> Option<String> {
        self.entries.iter().find(|e| e.key == key && e.created.elapsed() < Duration::from_secs(30)).map(|e| e.text.clone())
    }
    fn put(&mut self, key: u64, text: String) {
        self.entries.retain(|e| e.key != key);
        self.entries.push_front(Cached { key, text, created: Instant::now() });
        self.entries.truncate(16);
    }
}
pub struct Controller {
    observed: Option<Identity>,
    retry: Option<(String, u64)>,
    since: Instant,
    serial: u64,
    issued: bool,
    cooldown: Option<Instant>,
    active: Option<Active>,
    tx: Sender<Done>,
    rx: Receiver<Done>,
    cache: CompletionCache,
    recent: VecDeque<(u64, Instant)>,
    pub metrics: Metrics,
    pub status: String,
    pub revision: u64,
    /// UI drains these counters without storing code or responses in metrics.
    pub settings_fingerprint: String,
}
impl Default for Controller {
    fn default() -> Self {
        let (tx, rx) = unbounded();
        Self {
            observed: None,
            retry: None,
            since: Instant::now(),
            serial: 0,
            issued: false,
            cooldown: None,
            active: None,
            tx,
            rx,
            cache: CompletionCache { entries: VecDeque::new() },
            recent: VecDeque::new(),
            metrics: Metrics::default(),
            status: String::new(),
            revision: 0,
            settings_fingerprint: String::new(),
        }
    }
}
impl Controller {
    pub fn changed_version(&self, next: Option<&Identity>) -> bool {
        match (&self.observed, next) {
            (Some(old), Some(new)) => old.doc == new.doc && old.version != new.version,
            _ => false,
        }
    }
    pub fn observe(&mut self, identity: Option<Identity>, edited: bool) {
        let retry = identity.as_ref().is_some_and(|id| self.retry.as_ref() == Some(&(id.project.clone(), id.doc)));
        if identity.is_some() {
            self.retry = None;
        }
        let edited = edited || retry;
        if self.observed != identity {
            if self.observed.as_ref().map(|i| &i.project) != identity.as_ref().map(|i| &i.project) {
                self.cache.entries.clear();
            }
            // Any cursor, document or buffer identity change cancels the active
            // request; the serial check independently rejects late responses.
            if let Some(active) = &self.active {
                active.cancel.store(true, Ordering::Relaxed);
            }
            self.serial += 1;
            self.issued = !edited;
            self.since = Instant::now();
            self.observed = identity;
        }
    }
    pub fn reject(&mut self) {
        self.serial += 1;
        self.issued = true;
        if let Some(active) = &self.active {
            active.cancel.store(true, Ordering::Relaxed);
        }
        self.metrics.rejected += 1;
    }
    pub fn invalidate(&mut self) {
        let retry = if self.busy() || !self.issued { self.observed.as_ref().map(|id| (id.project.clone(), id.doc)) } else { None };
        self.revision = self.revision.wrapping_add(1);
        self.observe(None, false);
        self.retry = retry;
        self.cache.entries.clear();
    }
    pub fn waiting(&self) -> bool {
        !self.issued && self.cooldown.is_some_and(|until| until > Instant::now())
    }
    pub fn busy(&self) -> bool {
        self.active.as_ref().is_some_and(|a| a.busy.load(Ordering::Acquire))
    }
    pub fn wake_after(&self, settings: &Settings) -> Option<Duration> {
        if self.issued || self.busy() {
            return None;
        }
        if let Some(until) = self.cooldown
            && until > Instant::now()
        {
            return Some(until.saturating_duration_since(Instant::now()));
        }
        let debounce = Duration::from_millis(settings.debounce_ms.clamp(50, 2000));
        let wait = debounce.saturating_sub(self.since.elapsed());
        (!wait.is_zero()).then_some(wait)
    }
    pub fn ready(&self, settings: &Settings, manual: bool) -> bool {
        self.observed.is_some()
            && self.cooldown.is_none_or(|at| Instant::now() >= at)
            && (manual || (!self.issued && self.since.elapsed() >= Duration::from_millis(settings.debounce_ms.clamp(50, 2000))))
            && self.active.as_ref().is_none_or(|a| !a.busy.load(Ordering::Acquire))
    }
    pub fn poll(&mut self, settings: &Settings) -> Option<Ghost> {
        let mut ghost = None;
        while let Ok(done) = self.rx.try_recv() {
            if done.serial != self.serial && self.status == "AI completion…" {
                self.status.clear();
            }
            let total_latency_ms = done.debounce_ms + done.latency.as_millis();
            self.metrics.latency_ms = total_latency_ms;
            self.metrics.total_latency_ms = total_latency_ms;
            self.metrics.display_ms = total_latency_ms;
            self.metrics.debounce_ms = done.debounce_ms;
            self.metrics.context_prepare_ms = done.context_prepare_ms;
            self.metrics.http_request_ms = match &done.result {
                Ok(output) => output.http_request_ms,
                Err(_) if done.charged => done.latency.as_millis().saturating_sub(done.context_prepare_ms),
                Err(_) => 0,
            };
            self.metrics.latency_samples.push_back(total_latency_ms);
            self.metrics.latency_samples.truncate(128);
            if !done.charged {
                self.recent.retain(|(serial, _)| *serial != done.serial);
            }
            if done.charged {
                self.metrics.requests += 1;
                self.metrics.reserved_usd += settings.reserve_per_request_usd.max(0.001);
            }
            match done.result {
                Ok(output) => {
                    self.metrics.success += 1;
                    if !done.charged {
                        self.metrics.cache_hits += 1;
                    }
                    self.metrics.input_tokens += output.input_tokens;
                    self.metrics.output_tokens += output.output_tokens;
                    self.metrics.cached_tokens += output.cached_tokens;
                    if let Some(cost) = output.cost {
                        self.metrics.cost_usd += cost;
                        if done.charged {
                            self.metrics.reserved_usd = (self.metrics.reserved_usd - settings.reserve_per_request_usd.max(0.001)).max(0.0);
                        }
                    }
                    if done.serial == self.serial && self.observed.as_ref() == Some(&done.identity) {
                        if let Some(context) = done.context {
                            self.cache.put(CompletionCache::key(&context, settings), output.text.clone());
                        }
                        self.status.clear();
                        if !output.text.is_empty() {
                            self.metrics.rejection_reason.clear();
                            ghost = Some(Ghost { identity: done.identity, text: output.text });
                        } else {
                            self.metrics.rejection_reason = output.rejection_reason.unwrap_or_else(|| "empty FIM response".into());
                        }
                    }
                }
                Err(error) => {
                    if error.contains("HTTP 429") {
                        self.cooldown = Some(Instant::now() + Duration::from_secs(30));
                        self.issued = false;
                    }
                    if error != "Cancelled" {
                        self.metrics.errors += 1;
                    }
                    if done.serial == self.serial {
                        self.metrics.rejection_reason = error.clone();
                        self.status = error;
                    }
                }
            }
        }
        ghost
    }
    pub fn launch(&mut self, snapshot: Snapshot, settings: Settings, ctx: egui::Context) {
        self.issued = true;
        self.recent.retain(|(_, at)| at.elapsed() < Duration::from_secs(60));
        if self.recent.len() >= settings.requests_per_minute.min(60) {
            if let Some((_, oldest)) = self.recent.front() {
                self.cooldown = Some(*oldest + Duration::from_secs(60));
                self.issued = false;
                self.status = "AI completion rate limit; automatically retrying after cooldown".into();
            }
            return;
        }
        if self.metrics.requests >= settings.requests_per_session {
            self.status = "AI completion session request limit reached; increase requests_per_session in settings".into();
            return;
        }
        if self.metrics.cost_usd + self.metrics.reserved_usd + settings.reserve_per_request_usd.max(0.001) > settings.session_budget_usd {
            self.status = "AI completion session spending budget reached".into();
            return;
        }
        let Some(identity) = self.observed.clone() else { return };
        // Exact semantic cache lookup is performed on the worker; this UI only
        // clones a small bounded cache, never parses syntax or waits for LSP.
        let cached: Vec<_> =
            self.cache.entries.iter().filter(|e| e.created.elapsed() < Duration::from_secs(30)).map(|e| (e.key, e.text.clone())).collect();
        let cancel = Arc::new(AtomicBool::new(false));
        let busy = Arc::new(AtomicBool::new(true));
        self.active = Some(Active { cancel: cancel.clone(), busy: busy.clone() });
        let tx = self.tx.clone();
        self.serial += 1;
        let serial = self.serial;
        let debounce_ms = self.since.elapsed().as_millis();
        self.recent.push_back((serial, Instant::now()));
        self.status = "AI completion…".into();
        std::thread::spawn(move || {
            let started = Instant::now();
            let context_started = Instant::now();
            let mut context = None;
            let mut charged = false;
            let collected = if settings.provider == ProviderMode::OpenRouterChat {
                semantic::collect(&snapshot, &cancel)
            } else {
                semantic::collect_local(&snapshot)
            };
            let context_prepare_ms = context_started.elapsed().as_millis();
            let result = collected
                .and_then(|collected| {
                    if cancel.load(Ordering::Relaxed) {
                        return Err("Cancelled".into());
                    }
                    let key = CompletionCache::key(&collected, &settings);
                    if let Some((_, text)) = cached.iter().find(|(k, _)| *k == key) {
                        context = Some(collected);
                        return Ok(Output { text: text.clone(), ..Default::default() });
                    }
                    let provider: Box<dyn CompletionProvider> = match settings.provider {
                        ProviderMode::MercuryEdit2 => Box::new(InceptionFimProvider::from_project(&snapshot.root)?),
                        ProviderMode::OpenRouterChat => Box::new(OpenRouterProvider::from_env()?),
                    };
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|_| "Completion runtime failed".to_string())?;
                    charged = true;
                    let output = rt.block_on(provider.complete(&collected, &settings, cancel.clone()));
                    context = Some(collected);
                    output
                })
                .map(|mut output| {
                    let source = snapshot.buffer.text();
                    let at = snapshot.buffer.rope.char_to_byte(snapshot.cursor);
                    output.text = super::validation::normalize(snapshot.lang, &source, at, &output.text);
                    if !super::validation::acceptable(snapshot.lang, &source, at, &output.text) {
                        output.text.clear();
                        output.rejection_reason = Some("completion failed syntax validation".into());
                    }
                    output
                });
            let _ =
                tx.send(Done { identity, serial, context, result, latency: started.elapsed(), debounce_ms, context_prepare_ms, charged });
            busy.store(false, Ordering::Release);
            ctx.request_repaint();
        });
    }
}

impl Metrics {
    pub fn latency_percentiles(&self) -> (u128, u128) {
        if self.latency_samples.is_empty() {
            return (0, 0);
        }
        let mut values: Vec<_> = self.latency_samples.iter().copied().collect();
        values.sort_unstable();
        let at = |p: usize| values[((values.len() - 1) * p).div_ceil(100)];
        (at(50), at(95))
    }
}
impl Drop for Controller {
    fn drop(&mut self) {
        if let Some(active) = &self.active {
            active.cancel.store(true, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn identity(b: &Buffer) -> Identity {
        Identity { project: "local|test".into(), doc: 1, version: b.version, cursor: b.primary().head, epoch: 0 }
    }
    #[test]
    fn atomic_unicode_multiline_and_undo() {
        let mut b = Buffer::new("fn main() {\n    \n}");
        b.set_cursor(16, false);
        let old = b.text();
        let ghost = Ghost { identity: identity(&b), text: "println!(\"\u{43c}\u{438}\u{440}😀\");\n    next();".into() };
        assert!(ghost.accept(1, &mut b));
        assert!(b.text().contains("\u{43c}\u{438}\u{440}😀"));
        b.undo();
        assert_eq!(b.text(), old);
        b.redo();
        assert!(b.text().contains("next();"));
    }
    #[test]
    fn background_agent_edit_invalidates() {
        let mut b = Buffer::new("client.");
        b.set_cursor(7, false);
        let g = Ghost { identity: identity(&b), text: "send()".into() };
        b.apply_edits(vec![(0..0, "// agent\n".into())]);
        assert!(!g.accept(1, &mut b));
    }
    #[test]
    fn stale_cursor_and_closed_document() {
        let mut b = Buffer::new("abc");
        let id = identity(&b);
        let g = Ghost { identity: id.clone(), text: "x".into() };
        b.set_cursor(1, false);
        assert!(!g.valid(1, &b));
        assert!(!g.valid(2, &b));
        let mut c = Controller::default();
        c.observe(Some(id.clone()), true);
        let serial = c.serial;
        c.observe(None, false);
        c.tx.send(Done {
            identity: id,
            serial,
            context: None,
            result: Ok(Output { text: "late".into(), ..Default::default() }),
            latency: Duration::ZERO,
            debounce_ms: 0,
            context_prepare_ms: 0,
            charged: false,
        })
        .unwrap();
        assert!(c.poll(&Settings::default()).is_none());
    }
    #[test]
    fn bounded_cache_and_context_keys() {
        let mut cache = CompletionCache { entries: VecDeque::new() };
        for n in 0..30 {
            cache.put(n, "test".into());
        }
        assert_eq!(cache.entries.len(), 16);
        assert!(cache.get(0).is_none());
        assert_eq!(cache.get(29), Some("test".into()));
    }
    #[test]
    fn debounce_single_flight_and_filesystem_retry() {
        let b = Buffer::new("client.");
        let mut id = identity(&b);
        let mut c = Controller::default();
        let settings = Settings::default();
        c.observe(Some(id.clone()), true);
        assert!(!c.ready(&settings, false));
        c.since = Instant::now() - Duration::from_millis(200);
        assert!(c.ready(&settings, false));
        let cancel = Arc::new(AtomicBool::new(false));
        let busy = Arc::new(AtomicBool::new(true));
        c.active = Some(Active { cancel: cancel.clone(), busy: busy.clone() });
        assert!(!c.ready(&settings, true));
        c.invalidate();
        assert!(cancel.load(Ordering::Relaxed));
        id.epoch += 1;
        c.observe(Some(id), false);
        busy.store(false, Ordering::Release);
        c.since = Instant::now() - Duration::from_millis(200);
        assert!(c.ready(&settings, false));
        c.observe(None, false);
        assert!(!c.ready(&settings, true));
    }
    #[test]
    fn typing_cancels_request_and_discards_its_stale_answer() {
        let b = Buffer::new("client.");
        let old = identity(&b);
        let mut next = old.clone();
        next.version += 1;
        next.cursor += 1;
        let mut c = Controller::default();
        c.observe(Some(old.clone()), true);
        let serial = c.serial;
        let cancel = Arc::new(AtomicBool::new(false));
        let busy = Arc::new(AtomicBool::new(true));
        c.active = Some(Active { cancel: cancel.clone(), busy: busy.clone() });
        c.observe(Some(next), true);
        assert!(cancel.load(Ordering::Relaxed));
        c.tx.send(Done {
            identity: old,
            serial,
            context: None,
            result: Ok(Output { text: "stale()".into(), cost: Some(0.001), ..Default::default() }),
            latency: Duration::ZERO,
            debounce_ms: 0,
            context_prepare_ms: 0,
            charged: true,
        })
        .unwrap();
        busy.store(false, Ordering::Release);
        assert!(c.poll(&Settings::default()).is_none());
        assert_eq!(c.metrics.requests, 1);
        c.since = Instant::now() - Duration::from_secs(1);
        assert!(c.ready(&Settings::default(), false));
    }
    #[test]
    fn minute_limit_schedules_automatic_retry() {
        let b = Buffer::new("");
        let mut c = Controller::default();
        c.observe(Some(identity(&b)), true);
        c.recent.push_back((0, Instant::now()));
        let settings = Settings { requests_per_minute: 1, ..Default::default() };
        let snapshot = Snapshot {
            buffer: b,
            lang: crate::syntax::Lang::Rust,
            cursor: 0,
            path: String::new(),
            root: Default::default(),
            clients: vec![],
            related: vec![],
        };
        c.launch(snapshot, settings.clone(), egui::Context::default());
        assert!(!c.issued);
        assert!(c.wake_after(&settings).is_some());
        assert!(!c.ready(&settings, false));
        c.cooldown = Some(Instant::now() - Duration::from_secs(1));
        c.since = Instant::now() - Duration::from_secs(1);
        assert!(c.ready(&settings, false));
    }
    #[test]
    fn cancelled_collection_does_not_spend_request_quota() {
        let b = Buffer::new("");
        let mut c = Controller::default();
        let id = identity(&b);
        c.observe(Some(id.clone()), true);
        let serial = c.serial;
        c.recent.push_back((serial, Instant::now()));
        c.tx.send(Done {
            identity: id,
            serial,
            context: None,
            result: Err("Cancelled".into()),
            latency: Duration::ZERO,
            debounce_ms: 0,
            context_prepare_ms: 0,
            charged: false,
        })
        .unwrap();
        c.poll(&Settings::default());
        assert!(c.recent.is_empty());
        assert_eq!(c.metrics.requests, 0);
    }
}
