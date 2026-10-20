//! Markdown rendering for agent replies and hovers: headings, emphasis,
//! inline code, links, nested and task lists, quotes, tables, rules and
//! syntax-highlighted code blocks.

use std::{
    cell::RefCell,
    collections::HashMap,
    hash::{Hash, Hasher},
};

use eframe::egui::{self, Color32, CornerRadius, FontId, RichText, Stroke, TextFormat, text::LayoutJob};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::{
    syntax::{self, Lang},
    theme,
};

#[derive(Clone, Copy, Default)]
struct Style {
    strong: bool,
    emphasis: bool,
    strike: bool,
    code: bool,
    link: bool,
    heading: Option<HeadingLevel>,
    quote: bool,
    /// Index into `Writer::links` when this text is a clickable link.
    target: Option<u16>,
}

struct Writer<'u> {
    ui: &'u mut egui::Ui,
    size: f32,
    spans: Vec<(String, Style)>,
    style: Style,
    lists: Vec<Option<u64>>,
    prefix: Option<String>,
    quote: usize,
    code: Option<(String, String)>,
    table: Option<Vec<Vec<Vec<(String, Style)>>>>,
    links: Vec<String>,
    clicked: Option<String>,
    images: Option<&'u Images<'u>>,
    missing: Vec<String>,
    /// Inside `![alt](url)`: the url and the collected alt text.
    image: Option<(String, String)>,
}

/// Where Markdown images come from: files relative to `base_dir`, already
/// loaded into `cache`.
pub struct Images<'a> {
    pub base_dir: String,
    pub cache: &'a std::collections::HashMap<String, super::media::ImageEntry>,
}

pub struct Output {
    pub clicked: Option<String>,
    /// Image files that are not loaded yet.
    pub missing: Vec<String>,
}

/// Inline code that names a file, optionally with `:line`, `:line-line` or
/// `:line:col` (e.g. `make.bash:76`, `src/main.rs:10-20`).
pub fn looks_like_location(text: &str) -> bool {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"^(?:\.{0,2}/|~/)?[\w.\-/]*[A-Za-z_][\w\-]*\.[A-Za-z][\w]{0,7}(?::\d+(?:-\d+)?(?::\d+)?)?$").unwrap()
    });
    !text.contains(' ') && re.is_match(text)
}

fn lang_for_fence(name: &str) -> Lang {
    let name = name.split([',', ' ', '{']).next().unwrap_or("").to_lowercase();
    let ext = match name.as_str() {
        "rust" => "rs",
        "python" | "python3" => "py",
        "bash" | "shell" | "sh" | "zsh" | "console" => "sh",
        "javascript" | "jsx" => "js",
        "typescript" => "ts",
        "csharp" | "c#" => "cs",
        "c++" | "cxx" => "cpp",
        "golang" => "go",
        "yml" => "yaml",
        other => other,
    };
    Lang::from_path(&format!("x.{ext}"))
}

thread_local! {
    static HIGHLIGHT_CACHE: RefCell<HashMap<u64, Vec<syntax::Span>>> = RefCell::new(HashMap::new());
    static HIGHLIGHTER: RefCell<tree_sitter_highlight::Highlighter> = RefCell::new(tree_sitter_highlight::Highlighter::new());
}

fn highlighted(lang: Lang, code: &str) -> Vec<syntax::Span> {
    if lang == Lang::Plain || code.len() > 200_000 {
        return Vec::new();
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (lang as u8, code).hash(&mut hasher);
    let key = hasher.finish();
    if let Some(spans) = HIGHLIGHT_CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return spans;
    }
    let spans = HIGHLIGHTER.with(|h| syntax::highlight(&mut h.borrow_mut(), lang, code));
    HIGHLIGHT_CACHE.with(|c| {
        let mut cache = c.borrow_mut();
        if cache.len() > 400 {
            cache.clear();
        }
        cache.insert(key, spans.clone());
    });
    spans
}

impl Writer<'_> {
    fn format(&self, style: Style) -> TextFormat {
        let size = match style.heading {
            Some(HeadingLevel::H1) => self.size * 1.5,
            Some(HeadingLevel::H2) => self.size * 1.3,
            Some(HeadingLevel::H3) => self.size * 1.15,
            Some(_) => self.size * 1.05,
            None => self.size,
        };
        let mut color = if style.heading.is_some() || style.strong {
            Color32::WHITE
        } else if style.quote {
            theme::MUTED_TEXT
        } else {
            theme::TEXT
        };
        let mut format = TextFormat { font_id: FontId::proportional(size), color, italics: style.emphasis, ..Default::default() };
        if style.code {
            format.font_id = FontId::monospace(size - 1.0);
            format.background = theme::SURFACE;
            color = Color32::from_rgb(230, 192, 140);
            format.color = color;
        }
        if style.link {
            format.color = theme::ACCENT;
            format.underline = Stroke::new(1.0, theme::ACCENT.gamma_multiply(0.6));
        }
        if style.strike {
            format.strikethrough = Stroke::new(1.0, format.color);
        }
        format
    }

    fn job(&self, spans: &[(String, Style)]) -> LayoutJob {
        let mut job = LayoutJob::default();
        for (text, style) in spans {
            job.append(text, 0.0, self.format(*style));
        }
        job
    }

    fn indent(&self) -> f32 {
        self.lists.len().saturating_sub(1) as f32 * 18.0 + self.quote as f32 * 14.0
    }

    /// Emits the pending inline text as one wrapped paragraph.
    fn flush(&mut self) {
        let spans = std::mem::take(&mut self.spans);
        let prefix = self.prefix.take();
        if spans.iter().all(|(t, _)| t.trim().is_empty()) && prefix.is_none() {
            return;
        }
        let job = self.job(&spans);
        // Character ranges of link spans, for hit testing the laid-out text.
        let mut ranges: Vec<(std::ops::Range<usize>, String)> = Vec::new();
        let mut position = 0;
        for (text, style) in &spans {
            let len = text.chars().count();
            if let Some(target) = style.target.and_then(|t| self.links.get(t as usize)) {
                ranges.push((position..position + len, target.clone()));
            }
            position += len;
        }
        let indent = self.indent();
        let quote = self.quote > 0;
        let size = self.size;
        let mut clicked = None;
        self.ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if quote {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(3.0, size * 1.4), egui::Sense::hover());
                ui.painter().rect_filled(rect, 1.0, theme::ACCENT_DIM);
            }
            ui.add_space(indent);
            if let Some(prefix) = prefix {
                ui.label(RichText::new(prefix).size(size).color(theme::MUTED_TEXT));
            }
            let sense = if ranges.is_empty() { egui::Sense::hover() } else { egui::Sense::click() };
            let (pos, galley, response) = egui::Label::new(job).wrap().sense(sense).layout_in_ui(ui);
            ui.painter().galley(pos, galley.clone(), theme::TEXT);
            if !ranges.is_empty()
                && let Some(pointer) = response.hover_pos()
            {
                let index = galley.cursor_from_pos(pointer - pos).index.0;
                if let Some((_, target)) = ranges.iter().find(|(range, _)| range.contains(&index)) {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    if response.clicked() {
                        clicked = Some(target.clone());
                    }
                }
            }
        });
        if clicked.is_some() {
            self.clicked = clicked;
        }
    }

    fn code_block(&mut self, lang: &str, code: &str) {
        let code = code.trim_end_matches('\n');
        let language = lang_for_fence(lang);
        let spans = highlighted(language, code);
        let mono = FontId::monospace(self.size - 1.0);
        let mut job = LayoutJob::default();
        let mut position = 0;
        for span in &spans {
            if span.start > position {
                job.append(
                    &code[position..span.start],
                    0.0,
                    TextFormat { font_id: mono.clone(), color: theme::TEXT, ..Default::default() },
                );
            }
            if span.end > span.start && span.start >= position {
                job.append(
                    &code[span.start..span.end],
                    0.0,
                    TextFormat { font_id: mono.clone(), color: theme::syntax_color(span.highlight), ..Default::default() },
                );
                position = span.end;
            }
        }
        if position < code.len() {
            job.append(&code[position..], 0.0, TextFormat { font_id: mono, color: theme::TEXT, ..Default::default() });
        }
        let indent = self.indent();
        let label = lang.split([',', ' ']).next().unwrap_or("").to_string();
        let copy = code.to_string();
        let frame = egui::Frame::new()
            .fill(theme::EDITOR_BACKGROUND)
            .stroke(Stroke::new(1.0, theme::BORDER))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(egui::Margin::same(8))
            .outer_margin(egui::Margin { left: indent as i8, ..Default::default() });
        frame.show(self.ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&label).small().color(theme::FAINT_TEXT));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("копировать").clicked() {
                            ui.ctx().copy_text(copy.clone());
                        }
                    });
                });
                egui::ScrollArea::horizontal().id_salt(ui.next_auto_id()).auto_shrink([false, true]).show(ui, |ui| {
                    ui.add(egui::Label::new(job).extend());
                });
            });
        });
        self.ui.add_space(4.0);
    }

    fn table(&mut self, rows: Vec<Vec<Vec<(String, Style)>>>) {
        let jobs: Vec<Vec<LayoutJob>> = rows.iter().map(|row| row.iter().map(|cell| self.job(cell)).collect()).collect();
        let indent = self.indent();
        let frame = egui::Frame::new()
            .stroke(Stroke::new(1.0, theme::BORDER))
            .corner_radius(CornerRadius::same(4))
            .inner_margin(egui::Margin::same(6))
            .outer_margin(egui::Margin { left: indent as i8, ..Default::default() });
        egui::ScrollArea::horizontal().id_salt(self.ui.next_auto_id()).auto_shrink([true, true]).show(self.ui, |ui| {
            frame.show(ui, |ui| {
                egui::Grid::new(ui.next_auto_id()).striped(true).spacing([16.0, 4.0]).show(ui, |ui| {
                    for (index, row) in jobs.into_iter().enumerate() {
                        for mut job in row {
                            if index == 0 {
                                for section in &mut job.sections {
                                    section.format.color = Color32::WHITE;
                                }
                            }
                            ui.add(egui::Label::new(job).extend());
                        }
                        ui.end_row();
                    }
                });
            });
        });
        self.ui.add_space(4.0);
    }

    fn image(&mut self, url: &str, alt: &str) {
        let label = if alt.is_empty() { url.to_string() } else { alt.to_string() };
        let remote = url.starts_with("http://") || url.starts_with("https://");
        let Some(images) = self.images.filter(|_| !remote && !url.starts_with("data:")) else {
            // Shown as a link: external images are not downloaded.
            self.links.push(url.to_string());
            let style = Style { link: true, target: Some((self.links.len() - 1) as u16), ..self.style };
            self.spans.push((format!("▣ {label}"), style));
            return;
        };
        self.flush();
        let path = super::media::resolve_path(&images.base_dir, url.split(['?', '#']).next().unwrap_or(url));
        match images.cache.get(&path) {
            Some(super::media::ImageEntry::Ready(bytes)) => {
                let max = self.ui.available_width();
                let image = egui::Image::from_bytes(format!("bytes://{path}"), egui::load::Bytes::Shared(bytes.clone()))
                    .max_width(max)
                    .maintain_aspect_ratio(true)
                    .corner_radius(4);
                self.ui.add(image).on_hover_text(&label);
            }
            Some(super::media::ImageEntry::Failed(error)) => {
                self.ui.label(RichText::new(format!("▣ {label}: {error}")).small().color(theme::ERROR));
            }
            Some(super::media::ImageEntry::Loading) => {
                self.ui.label(RichText::new(format!("▣ {label}…")).small().color(theme::FAINT_TEXT));
            }
            None => {
                self.missing.push(path);
                self.ui.label(RichText::new(format!("▣ {label}…")).small().color(theme::FAINT_TEXT));
            }
        }
    }

    fn text(&mut self, text: &str) {
        if let Some((_, alt)) = &mut self.image {
            alt.push_str(text);
            return;
        }
        if let Some((_, code)) = &mut self.code {
            code.push_str(text);
            return;
        }
        let style = Style { quote: self.quote > 0, ..self.style };
        if let Some(rows) = &mut self.table {
            if let Some(cell) = rows.last_mut().and_then(|r| r.last_mut()) {
                cell.push((text.to_string(), style));
            }
            return;
        }
        self.spans.push((text.to_string(), style));
    }

    fn event(&mut self, event: Event) {
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => {}
                Tag::Heading { level, .. } => {
                    self.flush();
                    self.ui.add_space(if level <= HeadingLevel::H2 { 8.0 } else { 4.0 });
                    self.style.heading = Some(level);
                }
                Tag::BlockQuote(_) => {
                    self.flush();
                    self.quote += 1;
                }
                Tag::CodeBlock(kind) => {
                    self.flush();
                    let lang = match kind {
                        CodeBlockKind::Fenced(lang) => lang.to_string(),
                        CodeBlockKind::Indented => String::new(),
                    };
                    self.code = Some((lang, String::new()));
                }
                Tag::List(start) => {
                    self.flush();
                    self.lists.push(start);
                }
                Tag::Item => {
                    self.flush();
                    let depth = self.lists.len();
                    self.prefix = Some(match self.lists.last_mut() {
                        Some(Some(n)) => {
                            let label = format!("{n}.");
                            *n += 1;
                            label
                        }
                        _ => if depth > 1 { "◦" } else { "•" }.to_string(),
                    });
                }
                Tag::Table(_) => {
                    self.flush();
                    self.table = Some(Vec::new());
                }
                Tag::TableHead | Tag::TableRow => {
                    if let Some(rows) = &mut self.table {
                        rows.push(Vec::new());
                    }
                }
                Tag::TableCell => {
                    if let Some(row) = self.table.as_mut().and_then(|rows| rows.last_mut()) {
                        row.push(Vec::new());
                    }
                }
                Tag::Emphasis => self.style.emphasis = true,
                Tag::Strong => self.style.strong = true,
                Tag::Strikethrough => self.style.strike = true,
                Tag::Link { dest_url, .. } => {
                    self.links.push(dest_url.to_string());
                    self.style.link = true;
                    self.style.target = Some((self.links.len() - 1) as u16);
                }
                Tag::Image { dest_url, .. } => self.image = Some((dest_url.to_string(), String::new())),
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph => {
                    self.flush();
                    if self.lists.is_empty() {
                        self.ui.add_space(4.0);
                    }
                }
                TagEnd::Heading(level) => {
                    self.flush();
                    self.style.heading = None;
                    if level <= HeadingLevel::H2 {
                        let width = self.ui.available_width();
                        let (rect, _) = self.ui.allocate_exact_size(egui::vec2(width, 1.0), egui::Sense::hover());
                        self.ui.painter().rect_filled(rect, 0.0, theme::BORDER);
                    }
                    self.ui.add_space(4.0);
                }
                TagEnd::BlockQuote(_) => {
                    self.flush();
                    self.quote = self.quote.saturating_sub(1);
                }
                TagEnd::CodeBlock => {
                    if let Some((lang, code)) = self.code.take() {
                        self.code_block(&lang, &code);
                    }
                }
                TagEnd::List(_) => {
                    self.flush();
                    self.lists.pop();
                    if self.lists.is_empty() {
                        self.ui.add_space(4.0);
                    }
                }
                TagEnd::Item => self.flush(),
                TagEnd::Table => {
                    if let Some(rows) = self.table.take() {
                        self.table(rows);
                    }
                }
                TagEnd::Image => {
                    if let Some((url, alt)) = self.image.take() {
                        self.image(&url, &alt);
                    }
                }
                TagEnd::Emphasis => self.style.emphasis = false,
                TagEnd::Strong => self.style.strong = false,
                TagEnd::Strikethrough => self.style.strike = false,
                TagEnd::Link => {
                    self.style.link = false;
                    self.style.target = None;
                }
                _ => {}
            },
            Event::Text(text) => self.text(&text),
            Event::Code(code) => {
                let saved = self.style;
                self.style.code = true;
                if looks_like_location(&code) && self.style.target.is_none() {
                    self.links.push(code.to_string());
                    self.style.link = true;
                    self.style.target = Some((self.links.len() - 1) as u16);
                }
                self.text(&code);
                self.style = saved;
            }
            Event::InlineMath(math) | Event::DisplayMath(math) => self.text(&math),
            Event::Html(html) | Event::InlineHtml(html) => self.text(&html),
            Event::SoftBreak => self.text(" "),
            Event::HardBreak => self.text("\n"),
            Event::Rule => {
                self.flush();
                self.ui.add_space(4.0);
                self.ui.separator();
                self.ui.add_space(4.0);
            }
            Event::TaskListMarker(done) => self.prefix = Some(if done { "✓" } else { "☐" }.to_string()),
            Event::FootnoteReference(name) => self.text(&format!("[{name}]")),
        }
    }
}

/// Renders Markdown into `ui` at the given base font size. Returns the
/// target of a link (URL or file location) clicked this frame.
pub fn render(ui: &mut egui::Ui, text: &str, size: f32) -> Option<String> {
    render_with(ui, text, size, None).clicked
}

/// Like `render`, with local images resolved through `images`.
pub fn render_with(ui: &mut egui::Ui, text: &str, size: f32, images: Option<&Images>) -> Output {
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 3.0;
        let mut writer = Writer {
            ui,
            size,
            spans: Vec::new(),
            style: Style::default(),
            lists: Vec::new(),
            prefix: None,
            quote: 0,
            code: None,
            table: None,
            links: Vec::new(),
            clicked: None,
            images,
            missing: Vec::new(),
            image: None,
        };
        for event in Parser::new_ext(text, options) {
            writer.event(event);
        }
        // Unterminated code fence while streaming.
        if let Some((lang, code)) = writer.code.take() {
            writer.code_block(&lang, &code);
        }
        writer.flush();
        Output { clicked: writer.clicked, missing: writer.missing }
    })
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_everything_without_panicking() {
        let text = "# Title\n\nSome **bold**, *italic*, `code`, ~~gone~~ and [link](http://x).\n\n- one\n- two\n  1. nested\n  2. more\n- [x] done\n- [ ] todo\n\n> quote\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n```rust\nfn main() {}\n```\n\n---\n\n```py\nunterminated";
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let _ = render(ui, text, 14.0);
        });
        output.textures_delta.clear();
    }

    #[test]
    fn recognises_locations() {
        for yes in ["make.bash:76", "src/main.rs:10-20", "Cargo.toml", "./a/b.py:3:7", "01.prepare.before.package.bash:65-73", "~/x.sh"] {
            assert!(looks_like_location(yes), "{yes}");
        }
        for no in ["0.2.0", "passwd", "a b.rs", "u32", "user", "chroot ${BUILD_DIR}"] {
            assert!(!looks_like_location(no), "{no}");
        }
    }

    #[test]
    fn fence_languages() {
        assert_eq!(lang_for_fence("rust"), Lang::Rust);
        assert_eq!(lang_for_fence("bash"), Lang::Bash);
        assert_eq!(lang_for_fence("typescript"), Lang::TypeScript);
        assert_eq!(lang_for_fence(""), Lang::Plain);
    }
}
