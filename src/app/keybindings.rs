//! The "Keyboard Shortcuts" window: every command with its keys, search by
//! name (English or Russian), id or keys, and recording new keys. Changes go
//! to `[keybindings]` in config.toml.

use std::collections::BTreeMap;

use eframe::egui::{self, Align, Event, Key, Layout, RichText, vec2};

use super::{
    App, Command,
    commands::ALL,
    keymap::{self, Chord, Keymap},
};
use crate::theme;

#[derive(Default)]
pub struct State {
    query: String,
    focus_search: bool,
    /// Pressed keys fill the search box instead of running commands.
    search_by_keys: bool,
    recording: Option<Recording>,
    /// Result of the last change: text and whether it is a warning.
    message: Option<(String, bool)>,
}

#[derive(Clone, Copy, PartialEq)]
struct Recording {
    command: Command,
    /// The chord being changed; `None` adds a new one.
    replace: Option<Chord>,
}

enum Change {
    Record(Recording),
    Remove(Command, Chord),
    Reset(Command),
    ResetAll,
}

pub fn open(app: &mut App) {
    let state = app.keybindings.get_or_insert_with(State::default);
    state.focus_search = true;
}

/// Opens the window with a search filled in (debug views).
pub fn open_with_query(app: &mut App, query: &str) {
    open(app);
    if let Some(state) = &mut app.keybindings {
        state.query = query.to_string();
    }
}

/// While keys are being recorded, takes this frame's key presses before
/// shortcuts and widgets see them. Returns true when it did.
pub fn capture_keys(app: &mut App, ctx: &egui::Context) -> bool {
    let Some(state) = &mut app.keybindings else { return false };
    if state.recording.is_none() && !state.search_by_keys {
        return false;
    }
    let pressed = ctx.input_mut(|input| {
        let pressed = pressed_chord(&input.events);
        input.events.retain(|e| !matches!(e, Event::Key { .. } | Event::Text(_) | Event::Copy | Event::Cut | Event::Paste(_)));
        pressed
    });
    let Some(chord) = pressed else { return true };
    if chord == Chord::plain(Key::Escape) {
        state.recording = None;
        state.search_by_keys = false;
        return true;
    }
    if state.search_by_keys {
        state.query = chord.label();
        return true;
    }
    let Some(recording) = state.recording else { return true };
    if chord.types_text() {
        let modifiers = if cfg!(target_os = "macos") { "Cmd, Ctrl / Opt" } else { "Ctrl / Alt" };
        state.message = Some((t!("{keys} types text: hold {modifiers} with it", keys = chord.label(), modifiers = modifiers), true));
        return true;
    }
    state.recording = None;
    let taken = assign(&mut app.config.keybindings, recording.command, recording.replace, chord);
    state.message = Some(if taken.is_empty() {
        (t!("{keys} → {command}", keys = chord.label(), command = recording.command.label()), false)
    } else {
        let names: Vec<&str> = taken.iter().map(|c| c.label()).collect();
        (
            t!(
                "{keys} → {command}; removed from: {names}",
                keys = chord.label(),
                command = recording.command.label(),
                names = names.join(", ")
            ),
            true,
        )
    });
    apply(app);
    true
}

/// The first chord pressed among `events`. Shift, Ctrl, Alt and Cmd come as
/// keys of their own: they only count together with the key they are held
/// with.
fn pressed_chord(events: &[Event]) -> Option<Chord> {
    events.iter().find_map(|event| match event {
        Event::Key { key, pressed: true, modifiers, .. } if !keymap::is_modifier(*key) => Some(Chord::from_event(*key, *modifiers)),
        // Ctrl+C / Ctrl+X / Ctrl+V arrive as clipboard events, not keys.
        _ => keymap::clipboard_chord(event),
    })
}

/// Makes the edited `[keybindings]` take effect and saves them.
fn apply(app: &mut App) {
    keymap::set(Keymap::new(&app.config.keybindings));
    app.config.save();
}

/// The chords a command's defaults parse to.
fn default_chords(command: Command) -> Vec<Chord> {
    keymap::defaults(command).iter().filter_map(|text| Chord::parse(text)).collect()
}

/// Stores the chords of a command, or drops the override when they are the
/// defaults again.
fn set_chords(overrides: &mut BTreeMap<String, Vec<String>>, command: Command, chords: &[Chord]) {
    if chords == default_chords(command).as_slice() {
        overrides.remove(&command.id());
    } else {
        overrides.insert(command.id(), chords.iter().map(|c| c.to_config()).collect());
    }
}

/// Binds `chord` to `command` (in place of `replace`) and takes it away from
/// the commands that would react to the same keys; returns those.
fn assign(overrides: &mut BTreeMap<String, Vec<String>>, command: Command, replace: Option<Chord>, chord: Chord) -> Vec<Command> {
    let keymap = Keymap::new(overrides);
    let taken = keymap.conflicts(command, chord);
    for &other in &taken {
        let rest: Vec<Chord> = keymap.chords(other).iter().copied().filter(|c| *c != chord).collect();
        set_chords(overrides, other, &rest);
    }
    let mut chords = keymap.chords(command).to_vec();
    match replace.and_then(|old| chords.iter().position(|c| *c == old)) {
        Some(index) => chords[index] = chord,
        None => chords.push(chord),
    }
    let mut unique = Vec::new();
    for chord in chords {
        if !unique.contains(&chord) {
            unique.push(chord);
        }
    }
    set_chords(overrides, command, &unique);
    taken
}

fn remove(overrides: &mut BTreeMap<String, Vec<String>>, command: Command, chord: Chord) {
    let rest: Vec<Chord> = Keymap::new(overrides).chords(command).iter().copied().filter(|c| *c != chord).collect();
    set_chords(overrides, command, &rest);
}

pub fn window(ctx: &egui::Context, app: &mut App) {
    let Some(mut state) = app.keybindings.take() else { return };
    let keymap = keymap::current();
    let overrides = app.config.keybindings.clone();
    let mut open = true;
    let mut change = None;
    // A fixed width (only the height can be changed): the rows wrap inside
    // it instead of ever pushing the window wider.
    let width = (ctx.content_rect().width() - 48.0).clamp(360.0, 780.0);
    egui::Window::new(t!("Keyboard Shortcuts"))
        .open(&mut open)
        .collapsible(false)
        .resizable([false, true])
        .default_width(width)
        .min_width(width)
        .max_width(width)
        .default_height(560.0)
        .show(ctx, |ui| {
            search_row(ui, &mut state);
            for problem in &keymap.problems {
                ui.label(RichText::new(problem).small().color(theme::WARNING));
            }
            if let Some((text, warning)) = &state.message {
                ui.label(RichText::new(text).small().color(if *warning { theme::WARNING } else { theme::SUCCESS }));
            }
            ui.separator();

            let commands: Vec<Command> = ALL.iter().copied().filter(|c| keymap::matches_query(&keymap, *c, &state.query)).collect();
            let footer = 72.0;
            egui::ScrollArea::vertical().auto_shrink(false).max_height(ui.available_height() - footer).show(ui, |ui| {
                if commands.is_empty() {
                    ui.label(RichText::new(t!("No commands found")).color(theme::MUTED_TEXT));
                }
                for command in commands {
                    let overridden = overrides.contains_key(&command.id());
                    command_row(ui, &keymap, &state, command, overridden, &mut change);
                    ui.separator();
                }
            });

            ui.separator();
            // Everything below wraps: a line that cannot would widen the window.
            if !overrides.is_empty() && ui.button(t!("Reset all")).clicked() {
                change = Some(Change::ResetAll);
            }
            ui.label(
                RichText::new(t!("Fixed keys: arrows, Home/End, Page Up/Down, Backspace, Delete, Enter, Tab and Escape."))
                    .small()
                    .color(theme::MUTED_TEXT),
            );
            let path = crate::config::config_path().display().to_string();
            let path = match dirs::home_dir().map(|h| h.display().to_string()) {
                Some(home) if path.starts_with(&home) => format!("~{}", &path[home.len()..]),
                _ => path,
            };
            let text = t!("Saved in [keybindings] of {path}", path = path);
            let format = egui::TextFormat { font_id: egui::FontId::proportional(11.0), color: theme::FAINT_TEXT, ..Default::default() };
            let mut job = egui::text::LayoutJob::single_section(text, format);
            // A path has no spaces to wrap at.
            job.wrap = egui::text::TextWrapping { max_width: ui.available_width(), break_anywhere: true, ..Default::default() };
            ui.label(job);
        });

    match change {
        Some(Change::Record(recording)) => {
            state.recording = Some(recording);
            state.search_by_keys = false;
            state.message = None;
        }
        Some(Change::Remove(command, chord)) => {
            remove(&mut app.config.keybindings, command, chord);
            state.message = None;
            apply(app);
        }
        Some(Change::Reset(command)) => {
            app.config.keybindings.remove(&command.id());
            state.message = None;
            apply(app);
        }
        Some(Change::ResetAll) => {
            app.config.keybindings.clear();
            state.message = Some((t!("All shortcuts are back to the defaults").into(), false));
            apply(app);
        }
        None => {}
    }
    if open {
        app.keybindings = Some(state);
    }
}

fn search_row(ui: &mut egui::Ui, state: &mut State) {
    // Right to left: the buttons first, the search box takes what is left,
    // so the row never pushes the window wider.
    let row = vec2(ui.available_width(), ui.spacing().interact_size.y);
    ui.allocate_ui_with_layout(row, Layout::right_to_left(Align::Center), |ui| {
        if !state.query.is_empty() && ui.small_button("×").on_hover_text(t!("Clear")).clicked() {
            state.query.clear();
            state.focus_search = true;
        }
        let label = if state.search_by_keys { t!("● Recording keys") } else { t!("Record keys") };
        if ui
            .selectable_label(state.search_by_keys, label)
            .on_hover_text(t!("Search by pressing the shortcut itself; Escape stops"))
            .clicked()
        {
            state.search_by_keys = !state.search_by_keys;
            state.recording = None;
            state.focus_search = !state.search_by_keys;
        }
        let hint = if state.search_by_keys {
            t!("Press the keys to look for…").to_string()
        } else {
            let example = Chord::parse("Ctrl+Shift+P").map(|c| c.label().to_lowercase()).unwrap_or_default();
            t!("Search: name in English or Russian, id or keys ({example})", example = example)
        };
        // A fixed id: "×" appears before the box once a letter is typed,
        // and an id taken from the position would lose the focus.
        let edit =
            egui::TextEdit::singleline(&mut state.query).id_salt("keybindings_search").hint_text(hint).desired_width(ui.available_width());
        let edit = ui.add_enabled(!state.search_by_keys, edit);
        if state.focus_search {
            edit.request_focus();
            state.focus_search = false;
        }
    });
}

/// One command: its names on the left, its keys in the middle, the add and
/// reset buttons on the right. The columns add up to exactly the width at
/// hand: anything wider would make the window grow every frame.
fn command_row(ui: &mut egui::Ui, keymap: &Keymap, state: &State, command: Command, overridden: bool, change: &mut Option<Change>) {
    let gap = 12.0;
    let buttons = 52.0;
    let rest = (ui.available_width() - buttons - 2.0 * gap).max(0.0);
    let name_width = (rest * 0.5).floor();
    let keys_width = rest - name_width;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.allocate_ui_with_layout(vec2(name_width, 0.0), Layout::top_down(Align::Min), |ui| {
            ui.set_width(name_width);
            names(ui, command, overridden);
        });
        ui.add_space(gap);
        ui.allocate_ui_with_layout(vec2(keys_width, 0.0), Layout::left_to_right(Align::Center).with_main_wrap(true), |ui| {
            ui.set_width(keys_width);
            ui.spacing_mut().item_spacing.x = 6.0;
            keys(ui, keymap, state, command, change);
        });
        ui.add_space(gap);
        ui.allocate_ui_with_layout(vec2(buttons, 0.0), Layout::left_to_right(Align::Center), |ui| {
            ui.set_width(buttons);
            ui.spacing_mut().item_spacing.x = 6.0;
            if ui.small_button("+").on_hover_text(t!("Add a shortcut")).clicked() {
                *change = Some(Change::Record(Recording { command, replace: None }));
            }
            if overridden && ui.small_button("↺").on_hover_text(t!("Reset to default")).clicked() {
                *change = Some(Change::Reset(command));
            }
        });
    });
}

/// The name in the current language, then the other language, id and scope.
fn names(ui: &mut egui::Ui, command: Command, overridden: bool) {
    let name = RichText::new(command.label());
    ui.label(if overridden { name.color(theme::ACCENT) } else { name });
    let english = command.label_key();
    let other = if command.label() == english { crate::i18n::russian_of(english) } else { english };
    let mut detail = command.id();
    if other != command.label() {
        detail = format!("{other} · {detail}");
    }
    ui.label(RichText::new(format!("{detail} · {}", command.scope().label())).small().color(theme::FAINT_TEXT));
}

/// The command's chords: click one to change it, × removes it.
fn keys(ui: &mut egui::Ui, keymap: &Keymap, state: &State, command: Command, change: &mut Option<Change>) {
    let recording = |replace| state.recording == Some(Recording { command, replace });
    let held = keymap::held_label(ui.input(|i| i.modifiers));
    let waiting = || {
        let text = if held.is_empty() { t!("press keys… (Escape cancels)").to_string() } else { format!("{held}…") };
        egui::Label::new(RichText::new(text).monospace().color(theme::ACCENT)).wrap()
    };
    let chords = keymap.chords(command);
    for &chord in chords {
        if recording(Some(chord)) {
            ui.add(waiting());
            continue;
        }
        let taken = keymap.conflicts(command, chord);
        let mut text = RichText::new(chord.label()).monospace();
        if !taken.is_empty() {
            text = text.color(theme::WARNING);
        }
        let mut response = ui.add(egui::Button::new(text).small()).on_hover_text(t!("Click to change"));
        if !taken.is_empty() {
            let names: Vec<&str> = taken.iter().map(|c| c.label()).collect();
            response = response.on_hover_text(t!("Also bound to: {names}", names = names.join(", ")));
        }
        if response.clicked() {
            *change = Some(Change::Record(Recording { command, replace: Some(chord) }));
        }
        if ui.small_button("×").on_hover_text(t!("Remove")).clicked() {
            *change = Some(Change::Remove(command, chord));
        }
    }
    if recording(None) {
        ui.add(waiting());
    } else if chords.is_empty() {
        ui.label(RichText::new("—").color(theme::FAINT_TEXT));
    }
}

#[cfg(test)]
mod tests {
    use eframe::egui::Modifiers;

    use super::*;

    fn chord(text: &str) -> Chord {
        Chord::parse(text).unwrap()
    }

    /// Regression: "×" appeared before the search box after the first letter
    /// and the box lost the focus.
    #[test]
    fn search_box_keeps_the_focus_while_typing() {
        let ctx = egui::Context::default();
        let mut state = State { focus_search: true, ..Default::default() };
        for text in ["", "a", "\u{431}", "c", "\u{432}"] {
            let mut input = egui::RawInput::default();
            if !text.is_empty() {
                input.events.push(Event::Text(text.into()));
            }
            let mut output = ctx.run_ui(input, |ui| search_row(ui, &mut state));
            output.textures_delta.clear();
        }
        assert_eq!(state.query, "a\u{431}c\u{432}");
    }

    #[test]
    fn recording_waits_for_the_key_held_with_the_modifiers() {
        let mac = cfg!(target_os = "macos");
        let key = |key, modifiers| Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers };
        // Cmd+F on a Mac (Ctrl+F elsewhere): the modifier arrives first.
        let primary = Modifiers { command: true, mac_cmd: mac, ctrl: !mac, ..Modifiers::NONE };
        let modifier = if mac { Key::SuperLeft } else { Key::ControlLeft };
        assert_eq!(pressed_chord(&[key(modifier, primary)]), None);
        assert_eq!(pressed_chord(&[key(modifier, primary), key(Key::F, primary)]), Chord::parse("Ctrl+F"));
        // The Control key of a Mac with backtick.
        let control = Modifiers { ctrl: true, command: !mac, ..Modifiers::NONE };
        let chord = pressed_chord(&[key(Key::ControlLeft, control), key(Key::Backtick, control)]).unwrap();
        assert_eq!(chord, Chord::parse("MacCtrl+`").unwrap());
        assert!(!chord.types_text());
        assert_eq!(chord.label(), "Ctrl+`");
        // Ctrl+V comes as a paste, not as a key press.
        assert_eq!(pressed_chord(&[Event::Paste("text".into())]), Chord::parse("Ctrl+V"));
    }

    #[test]
    fn assigning_takes_the_keys_from_other_commands() {
        let mut overrides = BTreeMap::new();
        let taken = assign(&mut overrides, Command::GotoLine, None, chord("Ctrl+P"));
        assert_eq!(taken, vec![Command::QuickOpen]);
        let keymap = Keymap::new(&overrides);
        assert_eq!(keymap.chords(Command::GotoLine), [chord("Ctrl+G"), chord("Ctrl+P")]);
        assert!(keymap.chords(Command::QuickOpen).is_empty());
        assert_eq!(overrides["quick_open"], Vec::<String>::new());
    }

    #[test]
    fn replacing_and_going_back_to_the_defaults_forgets_the_override() {
        let mut overrides = BTreeMap::new();
        assign(&mut overrides, Command::Save, Some(chord("Ctrl+S")), chord("Ctrl+Alt+F9"));
        assert_eq!(overrides["save"], vec!["Ctrl+Alt+F9".to_string()]);
        assign(&mut overrides, Command::Save, Some(chord("Ctrl+Alt+F9")), chord("Ctrl+S"));
        assert!(overrides.is_empty(), "{overrides:?}");
        remove(&mut overrides, Command::Redo, chord("Ctrl+Y"));
        assert_eq!(overrides["redo"], vec!["Ctrl+Shift+Z".to_string()]);
    }

    #[test]
    fn editor_and_commit_keys_do_not_take_from_each_other() {
        let mut overrides = BTreeMap::new();
        let taken = assign(&mut overrides, Command::GitCommit, None, chord("Ctrl+Shift+Enter"));
        assert!(taken.is_empty(), "{taken:?}");
    }
}
