//! Inference is insertion-only. No tools, shell, file writes or provider logging.
use super::{Context, Settings};
use serde_json::{Value, json};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

pub const SYSTEM: &str = "You are an inline code completion engine.\nComplete the code at the cursor using the provided prefix, suffix, and semantic context.\nReturn ONLY the code that should be inserted at the cursor.\nDo not explain.\nDo not use Markdown.\nDo not repeat existing code.\nDo not invent unavailable APIs when semantic information is provided.\nRespect the surrounding code style, syntax, and types.\nReturn an empty completion if no useful continuation exists.";

#[derive(Default, Debug)]
pub struct Output {
    pub text: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost: Option<f64>,
}
pub type Inference<'a> = Pin<Box<dyn Future<Output = Result<Output, String>> + Send + 'a>>;
pub trait CompletionProvider: Send + Sync {
    fn complete<'a>(&'a self, context: &'a Context, settings: &'a Settings, cancel: Arc<AtomicBool>) -> Inference<'a>;
}
pub struct OpenRouterProvider {
    endpoint: String,
    key: String,
}
impl OpenRouterProvider {
    pub fn from_env() -> Result<Self, String> {
        let key = super::credentials::openrouter_key()?;
        Self::new("https://openrouter.ai/api/v1/chat/completions", key)
    }
    fn new(endpoint: &str, key: String) -> Result<Self, String> {
        if key.trim().is_empty() {
            return Err("OPENROUTER_API_KEY is not set".into());
        }
        Ok(Self { endpoint: endpoint.into(), key })
    }
}
impl CompletionProvider for OpenRouterProvider {
    fn complete<'a>(&'a self, context: &'a Context, settings: &'a Settings, cancel: Arc<AtomicBool>) -> Inference<'a> {
        Box::pin(async move {
            if cancel.load(Ordering::Relaxed) {
                return Err("Cancelled".into());
            }
            // Disable redirects: a bearer credential must never follow an arbitrary redirect.
            let client = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_millis(settings.timeout_ms.clamp(100, 30_000)))
                .connect_timeout(Duration::from_secs(3))
                .build()
                .map_err(|_| "HTTP client initialization failed")?;
            let request = async {
                let response = client
                    .post(&self.endpoint)
                    .bearer_auth(&self.key)
                    .header("X-Title", "Lumen Edit inline completion")
                    .json(&json!({"model": settings.model, "temperature": settings.temperature.clamp(0.0, 1.0),
                        "max_tokens": settings.max_tokens.clamp(1, 512), "stream": false,
                        "messages": [{"role":"system", "content":SYSTEM}, {"role":"user", "content": context.prompt()}]}))
                    .send()
                    .await
                    .map_err(|e| if e.is_timeout() { "OpenRouter timeout" } else { "OpenRouter network error" })?;
                let status = response.status();
                if !status.is_success() {
                    return Err(format!(
                        "OpenRouter HTTP {}{}",
                        status.as_u16(),
                        if status.as_u16() == 429 { " (rate limited)" } else { "" }
                    ));
                }
                // Never retain or print error bodies: they may echo the prompt.
                let mut response = response;
                let mut bytes = Vec::new();
                while let Some(chunk) = response.chunk().await.map_err(|_| "OpenRouter response read failed")? {
                    if bytes.len() + chunk.len() > 64 * 1024 {
                        return Err("OpenRouter response too large".into());
                    }
                    bytes.extend_from_slice(&chunk);
                }
                let value: Value = serde_json::from_slice(&bytes).map_err(|_| "Invalid OpenRouter JSON")?;
                let text = value.pointer("/choices/0/message/content").and_then(Value::as_str).ok_or("Missing completion text")?;
                Ok(Output {
                    text: sanitize(text, &context.prefix, &context.suffix)?,
                    input_tokens: value.pointer("/usage/prompt_tokens").and_then(Value::as_u64).unwrap_or(0),
                    output_tokens: value.pointer("/usage/completion_tokens").and_then(Value::as_u64).unwrap_or(0),
                    cost: value.pointer("/usage/cost").and_then(Value::as_f64).filter(|v| v.is_finite() && *v >= 0.0),
                })
            };
            tokio::pin!(request);
            loop {
                tokio::select! {
                    value = &mut request => return value,
                    _ = tokio::time::sleep(Duration::from_millis(20)) => {
                        if cancel.load(Ordering::Relaxed) { return Err("Cancelled".into()); }
                    }
                }
            }
        })
    }
}

pub fn sanitize(raw: &str, prefix: &str, suffix: &str) -> Result<String, String> {
    let text = raw.replace("\r\n", "\n");
    if text.len() > 8192 || text.contains("```") || text.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
        return Err("Invalid completion format".into());
    }
    if text.trim().is_empty() {
        return Ok(String::new());
    }
    // Models sometimes return the line's existing indentation. It belongs
    // to the prefix, so remove exactly one overlap rather than doubling it.
    let line = prefix.rsplit('\n').next().unwrap_or(prefix);
    let text = if !line.is_empty() && line.chars().all(|c| c == ' ' || c == '\t') {
        let overlap = line.chars().zip(text.chars()).take_while(|(a, b)| a == b).map(|(c, _)| c.len_utf8()).sum::<usize>();
        text[overlap..].to_string()
    } else {
        text
    };
    // Reject echoes rather than guessing at an edit range.
    if (!line.trim().is_empty() && line.len() >= 8 && text.starts_with(line)) || (prefix.len() >= 8 && text.starts_with(prefix)) {
        return Err("Completion repeats prefix".into());
    }
    // Remove a duplicated suffix only at a UTF-8 boundary; never remove internal code.
    let mut text = text;
    for (n, _) in text.char_indices().skip(1) {
        let tail = &text[n..];
        if (tail.len() >= 2 || matches!(tail, ")" | "]" | "}" | ";")) && suffix.starts_with(tail) {
            text.truncate(n);
            break;
        }
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_key() {
        assert!(OpenRouterProvider::new("unused", String::new()).is_err());
    }
    #[test]
    fn untrusted_output() {
        assert_eq!(sanitize("    return x * x", "def square(x):\n     ", "\n").unwrap(), "return x * x");
        assert_eq!(sanitize("    return x * x", "def square(x):\n    ", "\n").unwrap(), "return x * x");
        assert!(sanitize("```rust\nx\n```", "", "").is_err());
        assert!(sanitize("let client = client.send()", "let client = ", "").is_err());
        assert_eq!(sanitize("send());", "client.", ");").unwrap(), "send()");
        assert_eq!(sanitize("😀\n    \u{43c}\u{438}\u{440}", "", "").unwrap(), "😀\n    \u{43c}\u{438}\u{440}");
    }
    async fn mock(status: &str, body: &str, delay: u64, timeout: u64) -> Result<Output, String> {
        use std::{
            io::{Read, Write},
            net::TcpListener,
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/", listener.local_addr().unwrap());
        let status = status.to_string();
        let body = body.to_string();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0; 16384];
            let _ = stream.read(&mut buf);
            std::thread::sleep(Duration::from_millis(delay));
            let _ = write!(stream, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        });
        let settings = Settings { timeout_ms: timeout, ..Default::default() };
        OpenRouterProvider::new(&endpoint, "test-not-secret".into())
            .unwrap()
            .complete(&Context::default(), &settings, Arc::new(AtomicBool::new(false)))
            .await
    }
    #[test]
    fn network_contract() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let out = mock(
                "200 OK",
                r#"{"choices":[{"message":{"content":"hello"}}],"usage":{"prompt_tokens":12,"completion_tokens":2,"cost":0.0001}}"#,
                0,
                1000,
            )
            .await
            .unwrap();
            assert_eq!(out.text, "hello");
            assert_eq!(out.input_tokens, 12);
            assert_eq!(out.cost, Some(0.0001));
            assert!(mock("429 Too Many Requests", "sensitive body", 0, 1000).await.unwrap_err().contains("429"));
            assert!(mock("200 OK", "bad json", 0, 1000).await.is_err());
            assert!(mock("200 OK", "{}", 250, 100).await.unwrap_err().contains("timeout"));
        });
    }
    #[test]
    fn cancellation_interrupts_slow_network() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/", listener.local_addr().unwrap());
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut bytes = [0; 8192];
            let _ = stream.read(&mut bytes);
            let _ = started_tx.send(());
            std::thread::sleep(Duration::from_millis(500));
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}");
        });
        let cancel = Arc::new(AtomicBool::new(false));
        let signal = cancel.clone();
        std::thread::spawn(move || {
            started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
            signal.store(true, Ordering::Relaxed);
        });
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let at = std::time::Instant::now();
        let provider = OpenRouterProvider::new(&endpoint, "test-fixture".into()).unwrap();
        assert_eq!(rt.block_on(provider.complete(&Context::default(), &Settings::default(), cancel)).unwrap_err(), "Cancelled");
        assert!(at.elapsed() < Duration::from_millis(400));
    }

    #[test]
    #[ignore = "Explicit real cloud test; sends only this public synthetic example"]
    fn real_openrouter() {
        let provider = OpenRouterProvider::from_env().expect("Set OPENROUTER_API_KEY");
        let context = Context {
            language: "rust".into(),
            prefix: "fn square(x: i32) -> i32 {\n    ".into(),
            suffix: "\n}\n".into(),
            ..Default::default()
        };
        let now = std::time::Instant::now();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let output = rt.block_on(provider.complete(&context, &Settings::default(), Arc::new(AtomicBool::new(false)))).unwrap();
        assert!(!output.text.trim().is_empty());
        eprintln!(
            "Inline completion latency: {:?}; input={} output={} cost={:?}",
            now.elapsed(),
            output.input_tokens,
            output.output_tokens,
            output.cost
        );
    }
}
