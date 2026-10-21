//! Teleport: a two-pane file transfer tab between any connected machines
//! (this computer, the project's SSH host, other SSH hosts). Files stream as
//! `tar | tar` through the existing connections, keeping permissions and
//! symlinks, with progress and cancel.

use std::{
    collections::BTreeSet,
    io::{Read, Write},
    process::{Child, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Instant,
};

use eframe::egui::{self, Align2, CornerRadius, FontId, Id, RichText, Sense, Stroke, pos2, vec2};

use super::{App, Dialog, Msg, Tab};
use crate::{
    host::{self, Host, shell_quote},
    theme,
};

/// Directory entry shown in a pane.
#[derive(Clone)]
struct DirEntryLite {
    name: String,
    is_dir: bool,
}

pub struct Pane {
    pub host: Host,
    pub path: String,
    input: String,
    entries: Option<Result<Vec<DirEntryLite>, String>>,
    selected: BTreeSet<String>,
    anchor: Option<usize>,
    generation: u64,
}

impl Pane {
    fn new(host: Host, path: String) -> Self {
        Self { input: path.clone(), host, path, entries: None, selected: BTreeSet::new(), anchor: None, generation: 0 }
    }
}

pub struct Transfer {
    pub label: String,
    pub total: Option<u64>,
    pub done: Arc<AtomicU64>,
    pub started: Instant,
    pub finished: Option<Result<(), String>>,
    cancel: Arc<AtomicBool>,
    finished_at: Option<Instant>,
}

pub struct TeleportTab {
    pub id: u64,
    pub panes: [Pane; 2],
    pub transfers: Vec<Transfer>,
    shared: Arc<Mutex<Vec<(u64, Result<(), String>)>>>,
}

/// Every machine the editor is connected to.
pub fn known_hosts(app: &App) -> Vec<Host> {
    let mut hosts = vec![Host::Local];
    let mut push = |host: &Host| {
        if !hosts.iter().any(|h| h.same(host)) {
            hosts.push(host.clone());
        }
    };
    if let Some(workspace) = &app.workspace {
        push(&workspace.host);
    }
    for host in &app.known_hosts {
        push(host);
    }
    hosts
}

pub fn open(app: &mut App) {
    if let Some(index) = app.tabs.iter().position(|t| matches!(t, Tab::Teleport(_))) {
        app.activate_tab(index);
        return;
    }
    let local_home = Host::Local.home();
    let (right_host, right_path) = match &app.workspace {
        Some(w) => (w.host.clone(), w.root.clone()),
        None => (Host::Local, local_home.clone()),
    };
    // Local on the left; the project on the right (if it is remote, that is
    // the interesting direction).
    let left_path = if right_host.is_remote() {
        local_home
    } else {
        std::env::current_dir().map(|p| crate::platform::host_path(&p)).unwrap_or(local_home)
    };
    let id = app.next_id();
    let mut tab = TeleportTab {
        id,
        panes: [Pane::new(Host::Local, left_path), Pane::new(right_host, right_path)],
        transfers: Vec::new(),
        shared: Arc::default(),
    };
    for side in 0..2 {
        list(&app.bus, &mut tab, side);
    }
    app.tabs.push(Tab::Teleport(Box::new(tab)));
    app.active_tab = Some(app.tabs.len() - 1);
}

fn list(bus: &super::Bus, tab: &mut TeleportTab, side: usize) {
    let pane = &mut tab.panes[side];
    pane.generation += 1;
    pane.entries = None;
    pane.selected.clear();
    pane.anchor = None;
    pane.input = pane.path.clone();
    let host = pane.host.clone();
    let path = pane.path.clone();
    let generation = pane.generation;
    let tab_id = tab.id;
    bus.spawn(move || {
        let result = host
            .read_dir(&path)
            .map(|entries| entries.into_iter().map(|e| DirEntryLite { name: e.name, is_dir: e.is_dir }).collect::<Vec<_>>());
        Msg::Run(Box::new(move |app: &mut App| {
            if let Some(Tab::Teleport(tab)) = app.tabs.iter_mut().find(|t| matches!(t, Tab::Teleport(t) if t.id == tab_id)) {
                let pane = &mut tab.panes[side];
                if pane.generation == generation {
                    pane.entries = Some(result);
                }
            }
        }))
    });
}

/// What to move where.
pub struct Job {
    pub src: Host,
    pub src_dir: String,
    pub names: Vec<String>,
    pub dst: Host,
    pub dst_dir: String,
    /// Delete the sources once the copy succeeded.
    pub move_after: bool,
}

/// Starts streaming the job's files in the background.
pub fn start_transfer(app: &mut App, tab_id: u64, job: Job) {
    let Job { src, src_dir, names, dst, dst_dir, move_after } = job;
    if names.is_empty() {
        return;
    }
    if src.same(&dst) && src_dir == dst_dir {
        app.notify(t!("Source and destination are the same"), true);
        return;
    }
    let label = format!(
        "{} {} → {}:{}",
        if move_after { t!("Moving") } else { t!("Copying") },
        if names.len() == 1 { names[0].clone() } else { t!("{n1} items", n1 = names.len()) },
        if dst.is_remote() { dst.label() } else { t!("local").into() },
        dst_dir
    );
    let done = Arc::new(AtomicU64::new(0));
    let cancel = Arc::new(AtomicBool::new(false));
    let transfer_id = app.next_id();
    let Some(Tab::Teleport(tab)) = app.tabs.iter_mut().find(|t| matches!(t, Tab::Teleport(t) if t.id == tab_id)) else { return };
    tab.transfers.push(Transfer {
        label,
        total: None,
        done: done.clone(),
        started: Instant::now(),
        finished: None,
        cancel: cancel.clone(),
        finished_at: None,
    });
    let index = tab.transfers.len() - 1;
    let shared = tab.shared.clone();
    let total_slot = Arc::new(Mutex::new(None::<u64>));
    let total_for_ui = total_slot.clone();
    let bus = app.bus.clone();
    std::thread::spawn(move || {
        let quoted: Vec<String> = names.iter().map(|n| shell_quote(n)).collect();
        let list = quoted.join(" ");
        // Size first, for a percentage.
        if let Ok(out) = host::run_with_input(
            &mut src.shell(&src_dir, &format!("du -skL -- {list} 2>/dev/null | awk '{{s+=$1}} END {{print s*1024}}'")),
            None,
        ) {
            *total_slot.lock().unwrap() = String::from_utf8_lossy(&out).trim().parse().ok();
        }
        let result = pipe(&src, &src_dir, &list, &dst, &dst_dir, &done, &cancel).and_then(|_| {
            if move_after { host::run_with_input(&mut src.shell(&src_dir, &format!("rm -rf -- {list}")), None).map(|_| ()) } else { Ok(()) }
        });
        shared.lock().unwrap().push((transfer_id, result));
        bus.send(Msg::Run(Box::new(move |app: &mut App| finish(app, tab_id, index, total_for_ui))));
    });
}

fn pipe(src: &Host, src_dir: &str, list: &str, dst: &Host, dst_dir: &str, done: &AtomicU64, cancel: &AtomicBool) -> Result<(), String> {
    let mut reader: Child = src
        .shell(src_dir, &format!("tar -cf - -- {list}"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let mut writer: Child = dst
        .shell("/", &format!("mkdir -p {dir} && cd {dir} && tar -xf -", dir = shell_quote(dst_dir)))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let mut from = reader.stdout.take().unwrap();
    let mut to = writer.stdin.take().unwrap();
    let mut buffer = vec![0u8; 256 * 1024];
    let mut aborted = false;
    loop {
        if cancel.load(Ordering::Relaxed) {
            aborted = true;
            break;
        }
        match from.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => {
                if to.write_all(&buffer[..n]).is_err() {
                    break;
                }
                done.fetch_add(n as u64, Ordering::Relaxed);
            }
            Err(_) => break,
        }
    }
    drop(to);
    if aborted {
        let _ = reader.kill();
        let _ = writer.kill();
        let _ = reader.wait();
        let _ = writer.wait();
        return Err(t!("cancelled").into());
    }
    let read_status = reader.wait_with_output().map_err(|e| e.to_string())?;
    let write_status = writer.wait_with_output().map_err(|e| e.to_string())?;
    if !read_status.status.success() {
        return Err(t!("read: {n1}", n1 = String::from_utf8_lossy(&read_status.stderr).trim()));
    }
    if !write_status.status.success() {
        return Err(t!("write: {n1}", n1 = String::from_utf8_lossy(&write_status.stderr).trim()));
    }
    Ok(())
}

fn finish(app: &mut App, tab_id: u64, index: usize, total: Arc<Mutex<Option<u64>>>) {
    let Some(Tab::Teleport(tab)) = app.tabs.iter_mut().find(|t| matches!(t, Tab::Teleport(t) if t.id == tab_id)) else { return };
    let results: Vec<_> = tab.shared.lock().unwrap().drain(..).collect();
    if let Some(transfer) = tab.transfers.get_mut(index) {
        transfer.total = transfer.total.or(*total.lock().unwrap());
        if let Some((_, result)) = results.into_iter().last() {
            transfer.finished = Some(result.clone());
            transfer.finished_at = Some(Instant::now());
            if let Err(error) = result {
                app.output.push((format!("{}: {error}", transfer.label), true));
            }
        }
    }
    // Refresh both panes and the explorer.
    let bus = app.bus.clone();
    if let Some(Tab::Teleport(tab)) = app.tabs.iter_mut().find(|t| matches!(t, Tab::Teleport(t) if t.id == tab_id)) {
        for side in 0..2 {
            list(&bus, tab, side);
        }
    }
    app.reload_explorer();
}

fn human(bytes: u64) -> String {
    let units = [t!("B"), t!("KB"), t!("MB"), t!("GB"), t!("TB")];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < units.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 { t!("{bytes} B", bytes = bytes) } else { format!("{value:.1} {}", units[unit]) }
}

enum Action {
    Navigate(usize, String),
    SetHost(usize, Host),
    Refresh(usize),
    Transfer { from: usize, names: Vec<String>, move_after: bool },
    Delete(usize, Vec<String>),
    NewFolder(usize),
    ConnectHost,
    OpenFile(usize, String),
}

pub fn show(ui: &mut egui::Ui, app: &mut App, index: usize) {
    let hosts = known_hosts(app);
    let Some(Tab::Teleport(tab)) = app.tabs.get_mut(index) else { return };
    let tab_id = tab.id;
    let mut actions: Vec<Action> = Vec::new();
    let total = ui.available_rect_before_wrap();
    let queue_height = if tab.transfers.is_empty() { 0.0 } else { (tab.transfers.len().min(4) as f32 * 34.0 + 28.0).min(180.0) };
    let toolbar_height = 36.0;
    let panes_rect = egui::Rect::from_min_max(total.min + vec2(0.0, toolbar_height), total.max - vec2(0.0, queue_height));
    ui.allocate_rect(total, Sense::hover());

    // Toolbar.
    let toolbar = egui::Rect::from_min_size(total.min, vec2(total.width(), toolbar_height));
    let mut bar = ui.new_child(
        egui::UiBuilder::new().max_rect(toolbar.shrink2(vec2(12.0, 4.0))).layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    bar.label(RichText::new(t!("⇄ Teleport")).strong());
    bar.label(RichText::new(t!("drag files between the panes or use the buttons")).small().color(theme::FAINT_TEXT));
    bar.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if ui.button(t!("+ Connect host…")).clicked() {
            actions.push(Action::ConnectHost);
        }
    });

    let half = panes_rect.width() / 2.0;
    for side in 0..2 {
        let rect = egui::Rect::from_min_size(panes_rect.min + vec2(side as f32 * half, 0.0), vec2(half, panes_rect.height())).shrink(6.0);
        let mut pane_ui = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::top_down(egui::Align::Min)));
        pane_ui.set_clip_rect(rect);
        pane(&mut pane_ui, tab, side, &hosts, &mut actions);
    }

    // Transfer queue.
    if !tab.transfers.is_empty() {
        let rect = egui::Rect::from_min_max(pos2(total.left(), panes_rect.bottom()), total.max).shrink2(vec2(12.0, 4.0));
        let mut queue = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::top_down(egui::Align::Min)));
        queue.horizontal(|ui| {
            ui.label(RichText::new(t!("TRANSFERS")).small().strong().color(theme::MUTED_TEXT));
            if ui.small_button(t!("clear finished")).clicked() {
                tab.transfers.retain(|t| t.finished.is_none());
            }
        });
        egui::ScrollArea::vertical().id_salt(("teleport_queue", tab_id)).show(&mut queue, |ui| {
            for transfer in tab.transfers.iter().rev() {
                let done = transfer.done.load(Ordering::Relaxed);
                let elapsed = transfer.finished_at.unwrap_or_else(Instant::now).duration_since(transfer.started).as_secs_f64().max(0.1);
                let speed = done as f64 / elapsed;
                ui.horizontal(|ui| {
                    let (status, color) = match &transfer.finished {
                        None => ("…", theme::ACCENT),
                        Some(Ok(())) => ("✓", theme::SUCCESS),
                        Some(Err(_)) => ("✗", theme::ERROR),
                    };
                    ui.label(RichText::new(status).color(color));
                    ui.add_sized([320.0, 18.0], egui::Label::new(RichText::new(&transfer.label).small()).truncate());
                    let (bar_rect, _) = ui.allocate_exact_size(vec2(180.0, 8.0), Sense::hover());
                    ui.painter().rect_filled(bar_rect, CornerRadius::same(3), theme::SURFACE);
                    let fraction = match (transfer.finished.as_ref(), transfer.total) {
                        (Some(Ok(())), _) => 1.0,
                        (_, Some(total)) if total > 0 => (done as f32 / total as f32).min(1.0),
                        _ => 0.0,
                    };
                    let mut filled = bar_rect;
                    filled.set_width(bar_rect.width() * fraction);
                    ui.painter().rect_filled(filled, CornerRadius::same(3), color);
                    let detail = match &transfer.finished {
                        Some(Err(error)) => error.clone(),
                        _ => t!("{n1} · {n2}/s", n1 = human(done), n2 = human(speed as u64)),
                    };
                    ui.label(RichText::new(detail).small().color(theme::MUTED_TEXT));
                    if transfer.finished.is_none() && ui.small_button(t!("cancel")).clicked() {
                        transfer.cancel.store(true, Ordering::Relaxed);
                    }
                });
            }
        });
        if tab.transfers.iter().any(|t| t.finished.is_none()) {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
        }
    }

    apply(app, index, tab_id, actions);
}

fn pane(ui: &mut egui::Ui, tab: &mut TeleportTab, side: usize, hosts: &[Host], actions: &mut Vec<Action>) {
    let other = 1 - side;
    let arrow = if side == 0 { t!("Copy →") } else { t!("← Copy") };
    egui::Frame::new()
        .fill(theme::SIDEBAR_BACKGROUND)
        .stroke(Stroke::new(1.0, theme::BORDER))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::same(6))
        .show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            let pane = &mut tab.panes[side];
            ui.horizontal(|ui| {
                let current = if pane.host.is_remote() { pane.host.label() } else { t!("⌂ This computer").into() };
                egui::ComboBox::from_id_salt(("teleport_host", tab.id, side)).selected_text(current).width(150.0).show_ui(ui, |ui| {
                    for host in hosts {
                        let label = if host.is_remote() { host.label() } else { t!("⌂ This computer").into() };
                        if ui.selectable_label(host.same(&pane.host), label).clicked() {
                            actions.push(Action::SetHost(side, host.clone()));
                        }
                    }
                });
                if ui.small_button("⇧").on_hover_text(t!("Up")).clicked() {
                    actions.push(Action::Navigate(side, host::parent(&pane.path)));
                }
                if ui.small_button("⌂").on_hover_text(t!("Home folder")).clicked() {
                    actions.push(Action::Navigate(side, pane.host.home()));
                }
                if ui.small_button("⟳").on_hover_text(t!("Refresh")).clicked() {
                    actions.push(Action::Refresh(side));
                }
            });
            // Path on its own row so long paths stay readable.
            let field = ui.add(egui::TextEdit::singleline(&mut pane.input).desired_width(f32::INFINITY));
            if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                actions.push(Action::Navigate(side, pane.input.trim().to_string()));
            }
            let selection: Vec<String> = pane.selected.iter().cloned().collect();
            ui.horizontal_wrapped(|ui| {
                let any = !selection.is_empty();
                if ui.add_enabled(any, egui::Button::new(arrow)).clicked() {
                    actions.push(Action::Transfer { from: side, names: selection.clone(), move_after: false });
                }
                if ui.add_enabled(any, egui::Button::new(t!("Move"))).on_hover_text(t!("Copy and delete the source")).clicked() {
                    actions.push(Action::Transfer { from: side, names: selection.clone(), move_after: true });
                }
                if ui.add_enabled(any, egui::Button::new(RichText::new(t!("Delete")).color(theme::ERROR))).clicked() {
                    actions.push(Action::Delete(side, selection.clone()));
                }
                if ui.button(t!("+ Folder")).clicked() {
                    actions.push(Action::NewFolder(side));
                }
                if any {
                    ui.label(RichText::new(t!("selected: {n1}", n1 = selection.len())).small().color(theme::MUTED_TEXT));
                }
            });
            ui.separator();

            // Drop target: the whole pane accepts rows dragged from the other one.
            let drop_zone = ui.available_rect_before_wrap();
            let dragged = egui::DragAndDrop::payload::<(u64, usize, Vec<String>)>(ui.ctx());
            let hovering_drop = dragged.as_ref().is_some_and(|p| p.0 == tab.id && p.1 == other) && ui.rect_contains_pointer(drop_zone);
            if hovering_drop {
                ui.painter().rect_stroke(drop_zone, CornerRadius::same(4), Stroke::new(2.0, theme::ACCENT), egui::StrokeKind::Inside);
            }
            if hovering_drop
                && ui.input(|i| i.pointer.any_released())
                && let Some(payload) = egui::DragAndDrop::take_payload::<(u64, usize, Vec<String>)>(ui.ctx())
            {
                actions.push(Action::Transfer { from: payload.1, names: payload.2.clone(), move_after: false });
            }

            let pane = &mut tab.panes[side];
            let entries = match &pane.entries {
                None => {
                    ui.spinner();
                    return;
                }
                Some(Err(error)) => {
                    ui.label(RichText::new(error).color(theme::ERROR));
                    return;
                }
                Some(Ok(entries)) => entries.clone(),
            };
            let row_h = 22.0;
            egui::ScrollArea::vertical().id_salt(("teleport_list", tab.id, side)).auto_shrink([false, false]).show_rows(
                ui,
                row_h,
                entries.len(),
                |ui, range| {
                    for i in range {
                        let entry = &entries[i];
                        let selected = pane.selected.contains(&entry.name);
                        let id = Id::unique(("teleport_row", tab.id, side, &entry.name));
                        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::hover());
                        let response = ui.interact(rect, id, Sense::click_and_drag());
                        if selected {
                            ui.painter().rect_filled(rect, CornerRadius::same(3), theme::ACCENT_DIM);
                        } else if response.hovered() {
                            ui.painter().rect_filled(rect, CornerRadius::same(3), theme::SURFACE);
                        }
                        if entry.is_dir {
                            super::icons::folder(ui.painter(), rect.left_center() + vec2(12.0, 0.0), false);
                        } else {
                            super::icons::file(ui.painter(), rect.left_center() + vec2(12.0, 0.0), &format!("/{}", entry.name));
                        }
                        ui.painter().text(
                            rect.left_center() + vec2(26.0, 0.0),
                            Align2::LEFT_CENTER,
                            &entry.name,
                            FontId::proportional(13.0),
                            theme::TEXT,
                        );
                        let modifiers = ui.input(|i| i.modifiers);
                        if response.clicked() {
                            if modifiers.command {
                                if !pane.selected.remove(&entry.name) {
                                    pane.selected.insert(entry.name.clone());
                                }
                                pane.anchor = Some(i);
                            } else if modifiers.shift
                                && let Some(anchor) = pane.anchor
                            {
                                let (a, b) = if anchor < i { (anchor, i) } else { (i, anchor) };
                                for e in &entries[a..=b] {
                                    pane.selected.insert(e.name.clone());
                                }
                            } else {
                                pane.selected.clear();
                                pane.selected.insert(entry.name.clone());
                                pane.anchor = Some(i);
                            }
                        }
                        if response.double_clicked() {
                            if entry.is_dir {
                                actions.push(Action::Navigate(side, host::join(&pane.path, &entry.name)));
                            } else {
                                actions.push(Action::OpenFile(side, host::join(&pane.path, &entry.name)));
                            }
                        }
                        if response.drag_started() {
                            // Drag the selection, or this row if it is not selected.
                            let names = if selected { pane.selected.iter().cloned().collect() } else { vec![entry.name.clone()] };
                            egui::DragAndDrop::set_payload(ui.ctx(), (tab.id, side, names));
                        }
                        if response.dragged()
                            && let Some(pointer) = ui.ctx().pointer_latest_pos()
                            && let Some(payload) = egui::DragAndDrop::payload::<(u64, usize, Vec<String>)>(ui.ctx())
                        {
                            let text = if payload.2.len() == 1 { payload.2[0].clone() } else { t!("{n1} items", n1 = payload.2.len()) };
                            egui::Area::new(Id::unique("teleport_drag"))
                                .order(egui::Order::Tooltip)
                                .fixed_pos(pointer + vec2(12.0, 8.0))
                                .show(ui.ctx(), |ui| {
                                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                                        ui.label(format!("⇄ {text}"));
                                    });
                                });
                        }
                    }
                },
            );
        });
}

fn apply(app: &mut App, index: usize, tab_id: u64, actions: Vec<Action>) {
    for action in actions {
        match action {
            Action::Navigate(side, path) => {
                let bus = app.bus.clone();
                if let Some(Tab::Teleport(tab)) = app.tabs.get_mut(index) {
                    tab.panes[side].path = path;
                    list(&bus, tab, side);
                }
            }
            Action::SetHost(side, host) => {
                let bus = app.bus.clone();
                if let Some(Tab::Teleport(tab)) = app.tabs.get_mut(index) {
                    tab.panes[side].path = host.home();
                    tab.panes[side].host = host;
                    list(&bus, tab, side);
                }
            }
            Action::Refresh(side) => {
                let bus = app.bus.clone();
                if let Some(Tab::Teleport(tab)) = app.tabs.get_mut(index) {
                    list(&bus, tab, side);
                }
            }
            Action::Transfer { from, names, move_after } => {
                let Some(Tab::Teleport(tab)) = app.tabs.get(index) else { continue };
                let (src, dst) = (&tab.panes[from], &tab.panes[1 - from]);
                let (src_host, src_dir, dst_host, dst_dir) = (src.host.clone(), src.path.clone(), dst.host.clone(), dst.path.clone());
                start_transfer(app, tab_id, Job { src: src_host, src_dir, names, dst: dst_host, dst_dir, move_after });
            }
            Action::Delete(side, names) => {
                let Some(Tab::Teleport(tab)) = app.tabs.get(index) else { continue };
                let host = tab.panes[side].host.clone();
                let dir = tab.panes[side].path.clone();
                let count = names.len();
                app.dialog = Some(Dialog::Confirm {
                    title: t!("Delete").into(),
                    message: t!("Delete {count} item(s) in {dir}? This cannot be undone.", count = count, dir = dir),
                    action: Box::new(move |app: &mut App| {
                        let list: Vec<String> = names.iter().map(|n| shell_quote(n)).collect();
                        app.bus.spawn(move || {
                            let result = host::run_with_input(&mut host.shell(&dir, &format!("rm -rf -- {}", list.join(" "))), None);
                            Msg::Run(Box::new(move |app: &mut App| {
                                if let Err(error) = result {
                                    app.notify(error, true);
                                }
                                let bus = app.bus.clone();
                                if let Some(Tab::Teleport(tab)) =
                                    app.tabs.iter_mut().find(|t| matches!(t, Tab::Teleport(t) if t.id == tab_id))
                                {
                                    list_side(&bus, tab, side);
                                }
                                app.reload_explorer();
                            }))
                        });
                    }),
                });
            }
            Action::NewFolder(side) => {
                let Some(Tab::Teleport(tab)) = app.tabs.get(index) else { continue };
                let host = tab.panes[side].host.clone();
                let dir = tab.panes[side].path.clone();
                app.dialog = Some(Dialog::Input {
                    title: t!("New folder").into(),
                    value: String::new(),
                    focus: true,
                    action: Box::new(move |app: &mut App, name: String| {
                        let path = host::join(&dir, &name);
                        app.bus.spawn(move || {
                            let result = host.create_dir(&path);
                            Msg::Run(Box::new(move |app: &mut App| {
                                if let Err(error) = result {
                                    app.notify(error, true);
                                }
                                let bus = app.bus.clone();
                                if let Some(Tab::Teleport(tab)) =
                                    app.tabs.iter_mut().find(|t| matches!(t, Tab::Teleport(t) if t.id == tab_id))
                                {
                                    list_side(&bus, tab, side);
                                }
                            }))
                        });
                    }),
                });
            }
            Action::ConnectHost => {
                let mut state = super::welcome::ConnectState::input(&app.config);
                state.register_only = true;
                app.connect = Some(state);
            }
            Action::OpenFile(side, path) => {
                let Some(Tab::Teleport(tab)) = app.tabs.get(index) else { continue };
                // Files on the project host open in the editor.
                if app.workspace.as_ref().is_some_and(|w| w.host.same(&tab.panes[side].host)) {
                    app.open_file(path, None);
                } else {
                    app.notify(t!("Only files of the project host can be opened in the editor"), false);
                }
            }
        }
    }
}

fn list_side(bus: &super::Bus, tab: &mut TeleportTab, side: usize) {
    list(bus, tab, side);
}

/// Files dropped from the desktop: upload into `dir` on the project host.
pub fn upload_dropped(app: &mut App, paths: Vec<std::path::PathBuf>, dir: String) {
    let Some(host) = app.host() else { return };
    // Group by parent folder so each tar stream has one source directory.
    let mut groups: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for path in paths {
        let parent = path.parent().map(crate::platform::host_path).unwrap_or_else(|| "/".into());
        if let Some(name) = path.file_name() {
            groups.entry(parent).or_default().push(name.to_string_lossy().into_owned());
        }
    }
    // Show progress in the Teleport tab.
    if !app.tabs.iter().any(|t| matches!(t, Tab::Teleport(_))) {
        let active = app.active_tab;
        open(app);
        app.active_tab = active.or(app.active_tab);
    }
    let Some(tab_id) = app.tabs.iter().find_map(|t| if let Tab::Teleport(t) = t { Some(t.id) } else { None }) else { return };
    for (parent, names) in groups {
        start_transfer(
            app,
            tab_id,
            Job { src: Host::Local, src_dir: parent, names, dst: host.clone(), dst_dir: dir.clone(), move_after: false },
        );
    }
    app.notify(t!("Uploading to {dir} — progress is in the Teleport tab", dir = dir), false);
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn streams_tree_with_permissions_and_symlinks() {
        use std::os::unix::fs::PermissionsExt;
        let base = std::env::temp_dir().join(format!("lumen-teleport-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let src = base.join("src");
        std::fs::create_dir_all(src.join("tree/deep")).unwrap();
        std::fs::write(src.join("tree/deep/data.bin"), vec![7u8; 300_000]).unwrap();
        std::fs::write(src.join("run.sh"), "#!/bin/sh\necho hi\n").unwrap();
        std::fs::set_permissions(src.join("run.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
        std::os::unix::fs::symlink("run.sh", src.join("link")).unwrap();
        let dst = base.join("dst");
        let done = AtomicU64::new(0);
        let cancel = AtomicBool::new(false);
        let list = ["tree", "run.sh", "link"].iter().map(|n| shell_quote(n)).collect::<Vec<_>>().join(" ");
        pipe(&Host::Local, &src.to_string_lossy(), &list, &Host::Local, &dst.to_string_lossy(), &done, &cancel).unwrap();
        assert_eq!(std::fs::read(dst.join("tree/deep/data.bin")).unwrap().len(), 300_000);
        assert_eq!(std::fs::metadata(dst.join("run.sh")).unwrap().permissions().mode() & 0o777, 0o755);
        assert_eq!(std::fs::read_link(dst.join("link")).unwrap().to_string_lossy(), "run.sh");
        assert!(done.load(Ordering::Relaxed) >= 300_000);
        // Cancelled transfers report an error.
        cancel.store(true, Ordering::Relaxed);
        assert!(
            pipe(&Host::Local, &src.to_string_lossy(), &list, &Host::Local, &base.join("dst2").to_string_lossy(), &done, &cancel).is_err()
        );
        let _ = std::fs::remove_dir_all(&base);
    }
}
