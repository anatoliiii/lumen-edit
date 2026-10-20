use eframe::egui::Color32;

use crate::theme;

pub fn time_ago(unix_seconds: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let delta = (now - unix_seconds).max(0);
    match delta {
        0..=59 => "только что".into(),
        60..=3599 => format!("{} мин назад", delta / 60),
        3600..=86_399 => format!("{} ч назад", delta / 3600),
        86_400..=2_591_999 => format!("{} дн назад", delta / 86_400),
        2_592_000..=31_535_999 => format!("{} мес назад", delta / 2_592_000),
        _ => format!("{} г назад", delta / 31_536_000),
    }
}

/// Small colored badge text for a file type.
pub fn file_icon(name: &str) -> (&'static str, Color32) {
    let lower = name.to_ascii_lowercase();
    if lower == "cargo.toml" || lower == "cargo.lock" {
        return ("⚙", Color32::from_rgb(222, 125, 87));
    }
    if lower == "makefile" || lower == "cmakelists.txt" || lower == "justfile" {
        return ("⚙", Color32::from_rgb(160, 170, 185));
    }
    if lower == "dockerfile" || lower.starts_with("docker-compose") {
        return ("◈", Color32::from_rgb(80, 160, 230));
    }
    if lower.starts_with(".git") {
        return ("◆", Color32::from_rgb(230, 100, 70));
    }
    let ext = lower.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    match ext {
        "rs" => ("R", Color32::from_rgb(222, 125, 87)),
        "py" | "pyi" => ("P", Color32::from_rgb(90, 160, 220)),
        "go" => ("G", Color32::from_rgb(80, 190, 220)),
        "c" | "h" => ("C", Color32::from_rgb(110, 150, 210)),
        "cpp" | "cc" | "cxx" | "hpp" | "hh" => ("C", Color32::from_rgb(150, 120, 220)),
        "cs" => ("#", Color32::from_rgb(120, 190, 100)),
        "js" | "mjs" | "cjs" | "jsx" => ("J", Color32::from_rgb(230, 200, 90)),
        "ts" | "tsx" | "mts" => ("T", Color32::from_rgb(70, 140, 220)),
        "html" | "htm" => ("<", Color32::from_rgb(230, 120, 80)),
        "css" | "scss" | "sass" => ("#", Color32::from_rgb(90, 150, 230)),
        "json" | "jsonc" => ("{", Color32::from_rgb(220, 180, 90)),
        "toml" | "yaml" | "yml" | "ini" | "conf" | "cfg" => ("≡", Color32::from_rgb(160, 170, 185)),
        "md" | "markdown" | "txt" | "rst" => ("¶", Color32::from_rgb(110, 170, 230)),
        "sh" | "bash" | "zsh" | "fish" => ("$", Color32::from_rgb(130, 200, 120)),
        "lock" => ("⚿", theme::MUTED_TEXT),
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "ico" | "webp" => ("▣", Color32::from_rgb(180, 120, 200)),
        "sql" => ("D", Color32::from_rgb(220, 160, 90)),
        _ => ("·", theme::MUTED_TEXT),
    }
}

/// Fuzzy score with matched char positions, or `None` if no match.
pub struct Fuzzy {
    matcher: nucleo_matcher::Matcher,
}

impl Default for Fuzzy {
    fn default() -> Self {
        Self { matcher: nucleo_matcher::Matcher::new(nucleo_matcher::Config::DEFAULT.match_paths()) }
    }
}

impl Fuzzy {
    pub fn score(&mut self, haystack: &str, needle: &str) -> Option<(u32, Vec<u32>)> {
        if needle.is_empty() {
            return Some((0, Vec::new()));
        }
        let pattern = nucleo_matcher::pattern::Pattern::parse(
            needle,
            nucleo_matcher::pattern::CaseMatching::Smart,
            nucleo_matcher::pattern::Normalization::Smart,
        );
        let mut buf = Vec::new();
        let hay = nucleo_matcher::Utf32Str::new(haystack, &mut buf);
        let mut indices = Vec::new();
        let score = pattern.indices(hay, &mut self.matcher, &mut indices)?;
        indices.sort_unstable();
        indices.dedup();
        Some((score, indices))
    }
}

/// Rich text with matched characters highlighted.
pub fn highlighted_label(text: &str, matches: &[u32], base: Color32, highlight: Color32, size: f32) -> eframe::egui::text::LayoutJob {
    use eframe::egui::{FontId, TextFormat, text::LayoutJob};
    let mut job = LayoutJob::default();
    for (i, c) in text.chars().enumerate() {
        let matched = matches.binary_search(&(i as u32)).is_ok();
        let mut buf = [0u8; 4];
        job.append(
            c.encode_utf8(&mut buf),
            0.0,
            TextFormat { font_id: FontId::proportional(size), color: if matched { highlight } else { base }, ..Default::default() },
        );
    }
    job
}

pub fn truncate_middle(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let keep = max.saturating_sub(1) / 2;
    let start: String = text.chars().take(keep).collect();
    let end: String = text.chars().skip(count - keep).collect();
    format!("{start}…{end}")
}
