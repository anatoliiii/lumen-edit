//! Integrated terminal: a local PTY (running a shell, a task, or `ssh -t`)
//! parsed by `vt100` and painted with egui.

use std::{
    io::{Read, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use eframe::egui::{self, Color32, FontId, Key, Modifiers, Pos2, Rect, Sense, Stroke, Vec2, text::LayoutJob};
use portable_pty::{CommandBuilder, MasterPty, PtySize};

use crate::{
    app::{
        Command, Scope,
        keymap::{self, ClipboardAction},
    },
    theme,
};

#[derive(Default)]
struct Callbacks {
    responses: Vec<u8>,
    title: Option<String>,
}

impl vt100::Callbacks for Callbacks {
    fn set_window_title(&mut self, _: &mut vt100::Screen, title: &[u8]) {
        self.title = Some(String::from_utf8_lossy(title).into_owned());
    }

    fn unhandled_csi(&mut self, screen: &mut vt100::Screen, i1: Option<u8>, _i2: Option<u8>, params: &[&[u16]], c: char) {
        match (i1, c) {
            // Cursor position report; shells and TUIs wait for it.
            (None, 'n') if params.first().and_then(|p| p.first()) == Some(&6) => {
                let (row, col) = screen.cursor_position();
                self.responses.extend_from_slice(format!("\x1b[{};{}R", row + 1, col + 1).as_bytes());
            }
            (None, 'n') if params.first().and_then(|p| p.first()) == Some(&5) => {
                self.responses.extend_from_slice(b"\x1b[0n");
            }
            // Primary device attributes.
            (None, 'c') => self.responses.extend_from_slice(b"\x1b[?62;22c"),
            (Some(b'>'), 'c') => self.responses.extend_from_slice(b"\x1b[>0;0;0c"),
            _ => {}
        }
    }
}

type Parser = vt100::Parser<Callbacks>;

pub struct Terminal {
    pub title: String,
    pub cwd: String,
    parser: Arc<Mutex<Parser>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    master: Box<dyn MasterPty + Send>,
    killer: Box<dyn portable_pty::ChildKiller + Send + Sync>,
    pub alive: Arc<AtomicBool>,
    pub exit_code: Arc<Mutex<Option<u32>>>,
    size: (u16, u16),
    scrollback: usize,
    selection: Option<(Cell, Cell)>,
    selecting: bool,
    skip_text: bool,
    scroll_accum: f32,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
struct Cell {
    row: u16,
    col: u16,
}

pub struct TerminalOutput {
    /// Ctrl+click on `path:line[:col]`.
    pub open_location: Option<(String, usize, usize)>,
}

impl Terminal {
    pub fn spawn(
        title: String,
        cwd: String,
        command: CommandBuilder,
        ctx: egui::Context,
        alive: Option<Arc<AtomicBool>>,
    ) -> Result<Self, String> {
        let system = portable_pty::native_pty_system();
        let size = (24u16, 80u16);
        let pair = system.openpty(PtySize { rows: size.0, cols: size.1, pixel_width: 0, pixel_height: 0 }).map_err(|e| e.to_string())?;
        let mut child = pair.slave.spawn_command(command).map_err(|e| e.to_string())?;
        drop(pair.slave);
        let killer = child.clone_killer();
        let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
        let writer: Arc<Mutex<Box<dyn Write + Send>>> = Arc::new(Mutex::new(pair.master.take_writer().map_err(|e| e.to_string())?));
        let parser = Arc::new(Mutex::new(vt100::Parser::new_with_callbacks(size.0, size.1, 10_000, Callbacks::default())));
        let alive = alive.unwrap_or_else(|| Arc::new(AtomicBool::new(true)));
        alive.store(true, Ordering::Relaxed);
        let exit_code = Arc::new(Mutex::new(None));

        {
            let parser = parser.clone();
            let writer = writer.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let mut buffer = [0u8; 16 * 1024];
                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let responses = {
                                // Poison recovery is safe: a panic while processing a
                                // byte chunk leaves the vt100 screen one frame stale at
                                // worst; later input, resize and scroll operations
                                // self-heal it, so panicking every frame is worse.
                                let mut parser = parser.lock().unwrap_or_else(|e| e.into_inner());
                                parser.process(&buffer[..n]);
                                std::mem::take(&mut parser.callbacks_mut().responses)
                            };
                            if !responses.is_empty() {
                                let _ = writer.lock().unwrap_or_else(|e| e.into_inner()).write_all(&responses);
                            }
                            ctx.request_repaint();
                        }
                    }
                }
            });
        }
        {
            let alive = alive.clone();
            let exit_code = exit_code.clone();
            std::thread::spawn(move || {
                let status = child.wait();
                *exit_code.lock().unwrap_or_else(|e| e.into_inner()) = status.ok().map(|s| s.exit_code());
                alive.store(false, Ordering::Relaxed);
                ctx.request_repaint();
            });
        }

        Ok(Self {
            title,
            cwd,
            parser,
            writer,
            master: pair.master,
            killer,
            alive,
            exit_code,
            size,
            scrollback: 0,
            selection: None,
            selecting: false,
            skip_text: false,
            scroll_accum: 0.0,
        })
    }

    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }

    pub fn write(&self, data: &[u8]) {
        let _ = self.writer.lock().unwrap_or_else(|e| e.into_inner()).write_all(data);
    }

    pub fn kill(&mut self) {
        let _ = self.killer.kill();
    }

    /// Visible screen as text, trailing blank lines removed.
    pub fn screen_text(&self) -> String {
        let text = self.parser.lock().unwrap_or_else(|e| e.into_inner()).screen().contents();
        text.trim_end().to_string()
    }

    pub fn window_title(&self) -> Option<String> {
        self.parser.lock().unwrap_or_else(|e| e.into_inner()).callbacks().title.clone()
    }

    fn resize(&mut self, rows: u16, cols: u16) {
        if (rows, cols) == self.size || rows < 2 || cols < 2 {
            return;
        }
        self.size = (rows, cols);
        let _ = self.master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
        self.parser.lock().unwrap_or_else(|e| e.into_inner()).screen_mut().set_size(rows, cols);
        self.selection = None;
    }

    fn paste(&mut self, text: &str) {
        let bracketed = self.parser.lock().unwrap_or_else(|e| e.into_inner()).screen().bracketed_paste();
        let text = text.replace("\r\n", "\r").replace('\n', "\r");
        if bracketed {
            self.write(format!("\x1b[200~{text}\x1b[201~").as_bytes());
        } else {
            self.write(text.as_bytes());
        }
        self.scroll_to_bottom();
    }

    fn scroll_to_bottom(&mut self) {
        if self.scrollback != 0 {
            self.scrollback = 0;
            self.parser.lock().unwrap_or_else(|e| e.into_inner()).screen_mut().set_scrollback(0);
        }
    }

    fn selected_text(&self) -> Option<String> {
        let (a, b) = self.selection?;
        let (start, end) = if a <= b { (a, b) } else { (b, a) };
        let parser = self.parser.lock().unwrap_or_else(|e| e.into_inner());
        let text = parser.screen().contents_between(start.row, start.col, end.row, end.col + 1);
        (!text.is_empty()).then_some(text)
    }

    /// Paints the terminal into the available space and handles input.
    pub fn show(&mut self, ui: &mut egui::Ui, id: egui::Id, font_size: f32) -> TerminalOutput {
        let mut output = TerminalOutput { open_location: None };
        let font = FontId::monospace(font_size);
        let (char_w, row_h) = ui.fonts_mut(|f| (f.glyph_width(&font, 'M'), f.row_height(&font)));
        let rect = ui.available_rect_before_wrap();
        ui.allocate_rect(rect, Sense::hover());
        let response = ui.interact(rect, id, Sense::click_and_drag());
        let inner = rect.shrink2(Vec2::new(6.0, 4.0));
        let cols = ((inner.width() / char_w).floor() as u16).max(2);
        let rows = ((inner.height() / row_h).floor() as u16).max(2);
        self.resize(rows, cols);

        if response.clicked() || response.drag_started() {
            response.request_focus();
        }
        let focused = response.has_focus();
        if focused {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(id, egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true })
            });
        }

        let (mouse_mode, mouse_encoding, alternate) = {
            let parser = self.parser.lock().unwrap_or_else(|e| e.into_inner());
            let screen = parser.screen();
            (screen.mouse_protocol_mode(), screen.mouse_protocol_encoding(), screen.alternate_screen())
        };

        let cell_at = |pos: Pos2| -> Cell {
            let col = ((pos.x - inner.left()) / char_w).floor().clamp(0.0, (cols - 1) as f32) as u16;
            let row = ((pos.y - inner.top()) / row_h).floor().clamp(0.0, (rows - 1) as f32) as u16;
            Cell { row, col }
        };

        // ---- mouse
        let shift_held = ui.input(|i| i.modifiers.shift);
        let app_mouse = mouse_mode != vt100::MouseProtocolMode::None && !shift_held;
        if app_mouse {
            if let Some(pos) = response.interact_pointer_pos() {
                let cell = cell_at(pos);
                if response.drag_started() || response.clicked() {
                    self.send_mouse(0, cell, true, mouse_encoding);
                }
                if response.drag_stopped() || response.clicked() {
                    self.send_mouse(0, cell, false, mouse_encoding);
                } else if response.dragged()
                    && matches!(mouse_mode, vt100::MouseProtocolMode::ButtonMotion | vt100::MouseProtocolMode::AnyMotion)
                {
                    self.send_mouse(32, cell, true, mouse_encoding);
                }
            }
        } else {
            if response.drag_started()
                && let Some(pos) = response.interact_pointer_pos()
            {
                let cell = cell_at(pos);
                self.selection = Some((cell, cell));
                self.selecting = true;
            }
            if self.selecting
                && let Some(pos) = response.interact_pointer_pos()
                && let Some(selection) = &mut self.selection
            {
                selection.1 = cell_at(pos);
            }
            if response.drag_stopped() {
                self.selecting = false;
            }
            if response.clicked() {
                self.selection = None;
                if ui.input(|i| i.modifiers.command)
                    && let Some(pos) = response.interact_pointer_pos()
                {
                    output.open_location = self.location_at(cell_at(pos));
                }
            }
        }

        if response.hovered() {
            self.scroll_accum += ui.input(|i| i.smooth_scroll_delta.y);
            let lines = (self.scroll_accum / row_h).trunc() as i32;
            if lines != 0 {
                self.scroll_accum -= lines as f32 * row_h;
                if app_mouse {
                    if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
                        let button = if lines > 0 { 64 } else { 65 };
                        for _ in 0..lines.unsigned_abs().min(5) {
                            self.send_mouse(button, cell_at(pos), true, mouse_encoding);
                        }
                    }
                } else if alternate {
                    let key: &[u8] = if lines > 0 { b"\x1bOA" } else { b"\x1bOB" };
                    for _ in 0..lines.unsigned_abs().min(5) {
                        self.write(key);
                    }
                } else {
                    let target = (self.scrollback as i64 + lines as i64).max(0) as usize;
                    let mut parser = self.parser.lock().unwrap_or_else(|e| e.into_inner());
                    parser.screen_mut().set_scrollback(target);
                    self.scrollback = parser.screen().scrollback();
                }
            }
        }

        // ---- keyboard
        if focused {
            let events = ui.input(|i| i.events.clone());
            let modifiers = ui.input(|i| i.modifiers);
            for event in events {
                self.handle_event(ui, &event, modifiers);
            }
        }

        // ---- paint
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, theme::TERMINAL_BACKGROUND);
        let parser = self.parser.lock().unwrap_or_else(|e| e.into_inner());
        let screen = parser.screen();
        let selection = self.selection.map(|(a, b)| if a <= b { (a, b) } else { (b, a) });
        for row in 0..rows.min(screen.size().0) {
            let y = inner.top() + row as f32 * row_h;
            let mut job = LayoutJob::default();
            let mut col = 0u16;
            while col < cols.min(screen.size().1) {
                let Some(cell) = screen.cell(row, col) else { break };
                if cell.is_wide_continuation() {
                    col += 1;
                    continue;
                }
                let (mut fg, mut bg) = (color(cell.fgcolor(), true, cell.bold()), color(cell.bgcolor(), false, false));
                if cell.inverse() {
                    std::mem::swap(&mut fg, &mut bg);
                    if bg == Color32::TRANSPARENT {
                        bg = theme::TERMINAL_FOREGROUND;
                    }
                    if fg == Color32::TRANSPARENT {
                        fg = theme::TERMINAL_BACKGROUND;
                    }
                }
                if cell.dim() {
                    fg = fg.gamma_multiply(0.6);
                }
                let selected = selection.is_some_and(|(a, b)| Cell { row, col } >= a && Cell { row, col } <= b);
                let width = if cell.is_wide() { 2.0 } else { 1.0 };
                let cell_rect = Rect::from_min_size(Pos2::new(inner.left() + col as f32 * char_w, y), Vec2::new(char_w * width, row_h));
                if selected {
                    painter.rect_filled(cell_rect, 0.0, theme::SELECTION);
                } else if bg != Color32::TRANSPARENT {
                    painter.rect_filled(cell_rect, 0.0, bg);
                }
                let contents = cell.contents();
                let text = if contents.is_empty() { " " } else { contents };
                let mut format = egui::TextFormat { font_id: font.clone(), color: fg, italics: cell.italic(), ..Default::default() };
                if cell.underline() {
                    format.underline = Stroke::new(1.0, fg);
                }
                // Pad wide glyphs so the following cells stay on the grid.
                job.append(text, 0.0, format);
                if cell.is_wide() {
                    col += 1;
                }
                col += 1;
            }
            let galley = painter.layout_job(job);
            painter.galley(Pos2::new(inner.left(), y), galley, theme::TERMINAL_FOREGROUND);
        }

        if self.scrollback == 0 && !screen.hide_cursor() {
            let (row, col) = screen.cursor_position();
            let cursor = Rect::from_min_size(
                Pos2::new(inner.left() + col as f32 * char_w, inner.top() + row as f32 * row_h),
                Vec2::new(char_w, row_h),
            );
            if focused {
                painter.rect_filled(cursor, 0.0, theme::TERMINAL_CURSOR.gamma_multiply(0.75));
            } else {
                painter.rect_stroke(cursor, 0.0, Stroke::new(1.0, theme::TERMINAL_CURSOR), egui::StrokeKind::Inside);
            }
        }
        if self.scrollback > 0 {
            let label = t!("{n1} lines ↑", n1 = self.scrollback);
            painter.text(
                rect.right_top() + Vec2::new(-10.0, 6.0),
                egui::Align2::RIGHT_TOP,
                label,
                FontId::proportional(11.0),
                theme::MUTED_TEXT,
            );
        }
        drop(parser);

        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
        }
        output
    }

    /// Copies the selected text; false when nothing is selected.
    pub fn copy_selection(&mut self, ctx: &egui::Context) -> bool {
        match self.selected_text() {
            Some(text) => {
                ctx.copy_text(text);
                self.selection = None;
                true
            }
            None => false,
        }
    }

    /// The terminal's own commands: copy the selection, paste the clipboard.
    fn run_command(&mut self, ctx: &egui::Context, command: Command) {
        match command {
            Command::TerminalCopy => {
                self.copy_selection(ctx);
            }
            // The text arrives next frame as a paste event.
            Command::TerminalPaste => keymap::request_paste(ctx),
            _ => {}
        }
    }

    fn handle_event(&mut self, ui: &egui::Ui, event: &egui::Event, modifiers: Modifiers) {
        let keys = keymap::current();
        // Ctrl+C / Ctrl+X / Ctrl+V go by the keymap: copy and paste, another
        // command, or (given to none) the control character for the program.
        if !keymap::requested_paste(ui.ctx())
            && let Some(action) = keymap::clipboard_action(&keys, Scope::Terminal, event)
        {
            match action {
                ClipboardAction::Native => {}
                ClipboardAction::Run(command) => return self.run_command(ui.ctx(), command),
                ClipboardAction::Unbound => {
                    let byte = match event {
                        egui::Event::Copy => 0x03,
                        egui::Event::Cut => 0x18,
                        _ => 0x16,
                    };
                    self.write(&[byte]);
                    self.scroll_to_bottom();
                    return;
                }
            }
        }
        // Keys given to the terminal's copy and paste (Ctrl+Shift+C / V).
        if let egui::Event::Key { key, physical_key, pressed: true, modifiers, .. } = event
            && let Some(command) = keys.lookup(Scope::Terminal, *key, *physical_key, *modifiers)
        {
            return self.run_command(ui.ctx(), command);
        }
        match event {
            egui::Event::Text(text) => {
                if !self.skip_text {
                    self.write(text.as_bytes());
                    self.scroll_to_bottom();
                }
                self.skip_text = false;
            }
            egui::Event::Copy => {
                if let Some(text) = self.selected_text() {
                    ui.ctx().copy_text(text);
                    self.selection = None;
                } else if !modifiers.shift {
                    self.write(b"\x03");
                    self.scroll_to_bottom();
                }
            }
            egui::Event::Cut => self.write(b"\x18"),
            egui::Event::Paste(text) => self.paste(text),
            egui::Event::Key { key, pressed: true, modifiers, .. } => {
                let app_cursor = self.parser.lock().unwrap_or_else(|e| e.into_inner()).screen().application_cursor();
                if let Some(bytes) = key_bytes(*key, *modifiers, app_cursor) {
                    self.skip_text = modifiers.alt && !modifiers.ctrl;
                    self.write(&bytes);
                    self.scroll_to_bottom();
                    self.selection = None;
                }
            }
            _ => {}
        }
    }

    fn send_mouse(&self, button: u8, cell: Cell, pressed: bool, encoding: vt100::MouseProtocolEncoding) {
        let (x, y) = (cell.col + 1, cell.row + 1);
        match encoding {
            vt100::MouseProtocolEncoding::Sgr => {
                let end = if pressed { 'M' } else { 'm' };
                self.write(format!("\x1b[<{button};{x};{y}{end}").as_bytes());
            }
            _ => {
                let b = if pressed { button } else { 3 };
                if x < 223 && y < 223 {
                    self.write(&[0x1b, b'[', b'M', 32 + b, 32 + x as u8, 32 + y as u8]);
                }
            }
        }
    }

    fn location_at(&self, cell: Cell) -> Option<(String, usize, usize)> {
        let parser = self.parser.lock().unwrap_or_else(|e| e.into_inner());
        let row_text = parser.screen().contents_between(cell.row, 0, cell.row, self.size.1);
        drop(parser);
        static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        let re = RE.get_or_init(|| {
            // The pattern is a literal in this source; a failure is a build-time bug.
            regex::Regex::new(r"([A-Za-z0-9_./~+-]+\.[A-Za-z0-9]+):(\d+)(?::(\d+))?").unwrap_or_else(|e| panic!("invalid link regex: {e}"))
        });
        let col = cell.col as usize;
        for found in re.captures_iter(&row_text) {
            let Some(whole) = found.get(0) else { continue };
            let start = row_text[..whole.start()].chars().count();
            let end = start + whole.as_str().chars().count();
            if (start..=end).contains(&col) {
                let line = found[2].parse().unwrap_or(1);
                let column = found.get(3).and_then(|c| c.as_str().parse().ok()).unwrap_or(1);
                return Some((found[1].to_string(), line, column));
            }
        }
        None
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        self.kill();
    }
}

fn key_bytes(key: Key, modifiers: Modifiers, app_cursor: bool) -> Option<Vec<u8>> {
    let modifier_code = 1 + modifiers.shift as u8 + 2 * modifiers.alt as u8 + 4 * modifiers.ctrl as u8;
    let csi = |final_byte: char, code: Option<u8>| -> Vec<u8> {
        if modifier_code > 1 {
            match code {
                Some(n) => format!("\x1b[{n};{modifier_code}~").into_bytes(),
                None => format!("\x1b[1;{modifier_code}{final_byte}").into_bytes(),
            }
        } else {
            match code {
                Some(n) => format!("\x1b[{n}~").into_bytes(),
                None if app_cursor && "ABCDHF".contains(final_byte) => format!("\x1bO{final_byte}").into_bytes(),
                None => format!("\x1b[{final_byte}").into_bytes(),
            }
        }
    };
    let bytes = match key {
        Key::Enter => b"\r".to_vec(),
        Key::Backspace if modifiers.ctrl || modifiers.alt => b"\x1b\x7f".to_vec(),
        Key::Backspace => b"\x7f".to_vec(),
        Key::Tab if modifiers.shift => b"\x1b[Z".to_vec(),
        Key::Tab => b"\t".to_vec(),
        Key::Escape => b"\x1b".to_vec(),
        Key::ArrowUp => csi('A', None),
        Key::ArrowDown => csi('B', None),
        Key::ArrowRight => csi('C', None),
        Key::ArrowLeft => csi('D', None),
        Key::Home => csi('H', None),
        Key::End => csi('F', None),
        Key::Insert => csi('~', Some(2)),
        Key::Delete => csi('~', Some(3)),
        Key::PageUp => csi('~', Some(5)),
        Key::PageDown => csi('~', Some(6)),
        Key::F1 => b"\x1bOP".to_vec(),
        Key::F2 => b"\x1bOQ".to_vec(),
        Key::F3 => b"\x1bOR".to_vec(),
        Key::F4 => b"\x1bOS".to_vec(),
        Key::F5 => csi('~', Some(15)),
        Key::F6 => csi('~', Some(17)),
        Key::F7 => csi('~', Some(18)),
        Key::F8 => csi('~', Some(19)),
        Key::F9 => csi('~', Some(20)),
        Key::F10 => csi('~', Some(21)),
        Key::F11 => csi('~', Some(23)),
        Key::F12 => csi('~', Some(24)),
        _ => {
            let name = key.name();
            let mut chars = name.chars();
            let c = chars.next()?;
            if chars.next().is_some() {
                // Punctuation keys used with Ctrl.
                return match (modifiers.ctrl, key) {
                    (true, Key::OpenBracket) => Some(vec![0x1b]),
                    (true, Key::Backslash) => Some(vec![0x1c]),
                    (true, Key::CloseBracket) => Some(vec![0x1d]),
                    (true, Key::Space) => Some(vec![0]),
                    _ => None,
                };
            }
            if modifiers.ctrl && c.is_ascii_alphabetic() {
                let code = (c.to_ascii_lowercase() as u8) & 0x1f;
                if modifiers.alt { vec![0x1b, code] } else { vec![code] }
            } else if modifiers.alt && !modifiers.ctrl && c.is_ascii_alphanumeric() {
                let c = if modifiers.shift { c.to_ascii_uppercase() } else { c.to_ascii_lowercase() };
                vec![0x1b, c as u8]
            } else {
                return None;
            }
        }
    };
    Some(bytes)
}

fn color(color: vt100::Color, foreground: bool, bold: bool) -> Color32 {
    match color {
        vt100::Color::Default => {
            if foreground {
                theme::TERMINAL_FOREGROUND
            } else {
                Color32::TRANSPARENT
            }
        }
        vt100::Color::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
        vt100::Color::Idx(index) => {
            let index = if bold && foreground && index < 8 { index + 8 } else { index };
            indexed_color(index)
        }
    }
}

pub fn indexed_color(index: u8) -> Color32 {
    const ANSI: [(u8, u8, u8); 16] = [
        (30, 33, 39),
        (224, 108, 117),
        (152, 195, 121),
        (229, 192, 123),
        (97, 175, 239),
        (198, 120, 221),
        (86, 182, 194),
        (200, 204, 212),
        (92, 99, 112),
        (240, 130, 140),
        (170, 215, 140),
        (240, 210, 140),
        (120, 190, 250),
        (215, 140, 235),
        (110, 200, 210),
        (240, 242, 245),
    ];
    match index {
        0..=15 => {
            let (r, g, b) = ANSI[index as usize];
            Color32::from_rgb(r, g, b)
        }
        16..=231 => {
            let i = index - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            Color32::from_rgb(level(i / 36), level((i / 6) % 6), level(i % 6))
        }
        _ => {
            let gray = 8 + (index - 232) * 10;
            Color32::from_rgb(gray, gray, gray)
        }
    }
}
