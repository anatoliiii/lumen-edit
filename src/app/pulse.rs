//! Server pulse: live CPU, memory, disks and top processes of the project
//! host (local or SSH), sampled from /proc (sysctl on macOS) with one small
//! shell script.

use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

use eframe::egui::{self, Color32, CornerRadius, RichText, Sense, Shape, Stroke, pos2, vec2};
use serde_json::{Value, json};

use super::{App, Dialog, Msg};
use crate::{host::Host, theme};

const INTERVAL: Duration = Duration::from_secs(3);
const HISTORY: usize = 60;

const SCRIPT: &str = r#"LC_ALL=C
if [ "$(uname -s)" = Darwin ]; then
# macOS has no /proc: the same sections from sysctl, vm_stat and top.
top -l 2 -n 0 -s 1 | awk '/^CPU usage/ { idle = $7 } END { sub("%", "", idle); print "cpu% " 100 - idle }'; echo ---
pages=$(vm_stat | awk '/page size of/ { size = $8 } /^Pages (free|inactive|speculative|purgeable)/ { gsub("\\.", "", $NF); free += $NF } END { print free * size / 1024 }')
echo "MemTotal: $(($(sysctl -n hw.memsize) / 1024)) kB"; echo "MemAvailable: ${pages%.*} kB"
sysctl -n vm.swapusage | awk '{ sub("M", "", $3); sub("M", "", $9); printf "SwapTotal: %d kB\nSwapFree: %d kB\n", $3 * 1024, $9 * 1024 }'; echo ---
sysctl -n vm.loadavg | tr -d '{}'; echo ---
echo $(($(date +%s) - $(sysctl -n kern.boottime | sed 's/^{ sec = \([0-9]*\).*/\1/'))); echo ---
sysctl -n hw.ncpu; echo ---
df -P -k -l 2>/dev/null | tail -n +2 | grep -v -e '^devfs' -e '^map ' -e ' /System/Volumes/[^D]' -e ' /Library/Developer/'; echo ---
df -P -k . 2>/dev/null | tail -n 1; echo ---
ps -Aro pid=,pcpu=,pmem=,rss=,comm= 2>/dev/null | head -n 8 | awk '{ name = $5; for (i = 6; i <= NF; i++) name = name " " $i; sub(".*/", "", name); print $1, $2, $3, $4, name }'; echo ---
uname -n
else
head -n1 /proc/stat; echo ---
grep -E '^(MemTotal|MemAvailable|SwapTotal|SwapFree):' /proc/meminfo; echo ---
cat /proc/loadavg; echo ---
cut -d' ' -f1 /proc/uptime; echo ---
nproc 2>/dev/null || grep -c ^processor /proc/cpuinfo; echo ---
df -P -k -x tmpfs -x devtmpfs -x overlay -x squashfs -x efivarfs 2>/dev/null | tail -n +2; echo ---
df -P -k . 2>/dev/null | tail -n 1; echo ---
ps -eo pid=,pcpu=,pmem=,rss=,comm= --sort=-pcpu 2>/dev/null | head -n 8; echo ---
cat /proc/sys/kernel/hostname 2>/dev/null || uname -n
fi
true"#;

#[derive(Clone, Default)]
pub struct Disk {
    pub mount: String,
    pub used_kb: u64,
    pub total_kb: u64,
}

impl Disk {
    pub fn percent(&self) -> f32 {
        if self.total_kb == 0 { 0.0 } else { self.used_kb as f32 * 100.0 / self.total_kb as f32 }
    }
}

#[derive(Clone)]
pub struct Process {
    pub pid: u32,
    pub cpu: f32,
    pub mem: f32,
    pub rss_kb: u64,
    pub name: String,
}

#[derive(Clone, Default)]
struct Sample {
    cpu_idle: u64,
    cpu_total: u64,
    mem_total_kb: u64,
    mem_available_kb: u64,
    swap_total_kb: u64,
    swap_free_kb: u64,
    /// Busy percent measured by the host itself (macOS has no counters).
    cpu_busy: Option<f32>,
    load: [f32; 3],
    uptime_s: u64,
    cores: u32,
    disks: Vec<Disk>,
    project_disk: Option<Disk>,
    processes: Vec<Process>,
    hostname: String,
}

fn parse(output: &str) -> Sample {
    let sections: Vec<&str> = output.split("---\n").collect();
    let section = |i: usize| sections.get(i).copied().unwrap_or("");
    let mut sample = Sample::default();

    if let Some(busy) = section(0).trim().strip_prefix("cpu% ") {
        sample.cpu_busy = busy.trim().parse().ok();
    }
    let cpu: Vec<u64> = section(0).split_whitespace().skip(1).filter_map(|v| v.parse().ok()).collect();
    if cpu.len() >= 4 {
        // user nice system idle iowait irq softirq steal
        sample.cpu_idle = cpu[3] + cpu.get(4).copied().unwrap_or(0);
        sample.cpu_total = cpu.iter().take(8).sum();
    }
    for line in section(1).lines() {
        let mut parts = line.split_whitespace();
        let key = parts.next().unwrap_or("");
        let value: u64 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
        match key {
            "MemTotal:" => sample.mem_total_kb = value,
            "MemAvailable:" => sample.mem_available_kb = value,
            "SwapTotal:" => sample.swap_total_kb = value,
            "SwapFree:" => sample.swap_free_kb = value,
            _ => {}
        }
    }
    let load: Vec<f32> = section(2).split_whitespace().take(3).filter_map(|v| v.parse().ok()).collect();
    if load.len() == 3 {
        sample.load = [load[0], load[1], load[2]];
    }
    sample.uptime_s = section(3).trim().parse::<f64>().map(|v| v as u64).unwrap_or(0);
    sample.cores = section(4).trim().parse().unwrap_or(1).max(1);
    let disk = |line: &str| -> Option<Disk> {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 6 {
            return None;
        }
        Some(Disk { mount: parts[5..].join(" "), used_kb: parts[2].parse().ok()?, total_kb: parts[1].parse().ok()? })
    };
    sample.disks = section(5)
        .lines()
        .filter_map(disk)
        .filter(|d| d.total_kb > 0 && !d.mount.starts_with("/snap") && !d.mount.starts_with("/boot/efi"))
        .collect();
    sample.project_disk = section(6).lines().next().and_then(disk);
    sample.processes = section(7)
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            Some(Process {
                pid: parts.next()?.parse().ok()?,
                cpu: parts.next()?.parse().ok()?,
                mem: parts.next()?.parse().ok()?,
                rss_kb: parts.next()?.parse().ok()?,
                name: parts.collect::<Vec<_>>().join(" "),
            })
        })
        .collect();
    sample.hostname = section(8).trim().to_string();
    sample
}

#[derive(Default)]
pub struct Pulse {
    host_key: String,
    sample: Option<Sample>,
    previous: Option<Sample>,
    pub cpu: VecDeque<f32>,
    pub mem: VecDeque<f32>,
    in_flight: bool,
    last_fetch: Option<Instant>,
    pub error: Option<String>,
    alerted_disk: bool,
    alerted_mem: bool,
}

impl Pulse {
    pub fn cpu_percent(&self) -> Option<f32> {
        self.cpu.back().copied()
    }

    pub fn mem_percent(&self) -> Option<f32> {
        self.mem.back().copied()
    }

    pub fn project_disk(&self) -> Option<&Disk> {
        self.sample.as_ref()?.project_disk.as_ref()
    }

    fn update(&mut self, sample: Sample) {
        if let Some(busy) = sample.cpu_busy {
            push(&mut self.cpu, busy);
        } else if let Some(previous) = &self.sample {
            let total = sample.cpu_total.saturating_sub(previous.cpu_total);
            let idle = sample.cpu_idle.saturating_sub(previous.cpu_idle);
            if total > 0 {
                push(&mut self.cpu, (total - idle.min(total)) as f32 * 100.0 / total as f32);
            }
        }
        if sample.mem_total_kb > 0 {
            push(&mut self.mem, (sample.mem_total_kb - sample.mem_available_kb) as f32 * 100.0 / sample.mem_total_kb as f32);
        }
        self.previous = self.sample.replace(sample);
        self.error = None;
    }

    pub fn json(&self) -> Value {
        let Some(sample) = &self.sample else {
            return json!({ "error": self.error.clone().unwrap_or_else(|| t!("no data").into()) });
        };
        json!({
            "host": sample.hostname,
            "cpu_percent": self.cpu_percent(),
            "cores": sample.cores,
            "load": sample.load,
            "memory": { "used_mb": (sample.mem_total_kb - sample.mem_available_kb) / 1024, "total_mb": sample.mem_total_kb / 1024, "percent": self.mem_percent() },
            "swap": { "used_mb": (sample.swap_total_kb - sample.swap_free_kb) / 1024, "total_mb": sample.swap_total_kb / 1024 },
            "uptime_s": sample.uptime_s,
            "project_disk": sample.project_disk.as_ref().map(|d| json!({ "mount": d.mount, "used_gb": d.used_kb as f64 / 1048576.0, "total_gb": d.total_kb as f64 / 1048576.0, "percent": d.percent() })),
            "disks": sample.disks.iter().map(|d| json!({ "mount": d.mount, "percent": d.percent(), "total_gb": d.total_kb as f64 / 1048576.0 })).collect::<Vec<_>>(),
            "top": sample.processes.iter().map(|p| json!({ "pid": p.pid, "cpu": p.cpu, "mem": p.mem, "name": p.name })).collect::<Vec<_>>(),
        })
    }
}

fn push(history: &mut VecDeque<f32>, value: f32) {
    history.push_back(value.clamp(0.0, 100.0));
    while history.len() > HISTORY {
        history.pop_front();
    }
}

/// Starts a sample when due. Called every frame.
pub fn tick(app: &mut App) {
    if !app.config.pulse {
        return;
    }
    let Some(workspace) = &app.workspace else { return };
    let key = workspace.host.key();
    if app.pulse.host_key != key {
        app.pulse = Pulse { host_key: key, ..Default::default() };
    }
    let due = app.pulse.last_fetch.is_none_or(|t| t.elapsed() >= INTERVAL);
    if app.pulse.in_flight || !due {
        app.ctx.request_repaint_after(INTERVAL);
        return;
    }
    app.pulse.in_flight = true;
    app.pulse.last_fetch = Some(Instant::now());
    let host = workspace.host.clone();
    let root = workspace.root.clone();
    let key = app.pulse.host_key.clone();
    app.bus.spawn(move || {
        let result = crate::host::run_with_input(&mut host.shell(&root, SCRIPT), None).map(|out| parse(&String::from_utf8_lossy(&out)));
        Msg::Run(Box::new(move |app: &mut App| {
            if app.pulse.host_key != key {
                return;
            }
            app.pulse.in_flight = false;
            match result {
                Ok(sample) if sample.cpu_total > 0 => {
                    app.pulse.update(sample);
                    alerts(app);
                }
                Ok(_) => app.pulse.error = Some(t!("the host does not look like Linux: no /proc").into()),
                Err(error) => app.pulse.error = Some(error),
            }
        }))
    });
    app.ctx.request_repaint_after(INTERVAL);
}

fn alerts(app: &mut App) {
    let disk = app.pulse.project_disk().map(Disk::percent).unwrap_or(0.0);
    let mem = app.pulse.mem_percent().unwrap_or(0.0);
    if disk >= 95.0 && !app.pulse.alerted_disk {
        app.pulse.alerted_disk = true;
        let mount = app.pulse.project_disk().map(|d| d.mount.clone()).unwrap_or_default();
        app.notify(t!("Disk {mount} is {disk}% full — builds may fail", mount = mount, disk = format!("{:.0}", disk)), true);
    } else if disk < 90.0 {
        app.pulse.alerted_disk = false;
    }
    if mem >= 95.0 && !app.pulse.alerted_mem {
        app.pulse.alerted_mem = true;
        app.notify(t!("Memory is {mem}% used", mem = format!("{:.0}", mem)), true);
    } else if mem < 90.0 {
        app.pulse.alerted_mem = false;
    }
}

pub fn level_color(percent: f32) -> Color32 {
    if percent >= 95.0 {
        theme::ERROR
    } else if percent >= 85.0 {
        theme::WARNING
    } else {
        theme::MUTED_TEXT
    }
}

fn human_kb(kb: u64) -> String {
    let gb = kb as f64 / 1048576.0;
    if gb >= 10.0 {
        format!("{gb:.0}G")
    } else if gb >= 1.0 {
        format!("{gb:.1}G")
    } else {
        format!("{}M", kb / 1024)
    }
}

fn uptime(seconds: u64) -> String {
    let days = seconds / 86_400;
    let hours = seconds % 86_400 / 3600;
    let minutes = seconds % 3600 / 60;
    if days > 0 {
        t!("{days} d {hours} h", days = days, hours = hours)
    } else {
        t!("{hours} h {minutes} min", hours = hours, minutes = minutes)
    }
}

/// Compact text for the status bar.
pub fn status_text(app: &App) -> Option<(String, Color32)> {
    let pulse = &app.pulse;
    let sample = pulse.sample.as_ref()?;
    let cpu = pulse.cpu_percent().unwrap_or(0.0);
    let mem = pulse.mem_percent().unwrap_or(0.0);
    let disk = sample.project_disk.as_ref().map(Disk::percent).unwrap_or(0.0);
    let worst = cpu.max(mem).max(disk);
    let used = sample.mem_total_kb - sample.mem_available_kb;
    Some((
        t!(
            "CPU {cpu}%  RAM {n1}/{n2}  disk {disk}%",
            cpu = format!("{:.0}", cpu),
            n1 = human_kb(used),
            n2 = human_kb(sample.mem_total_kb),
            disk = format!("{:.0}", disk)
        ),
        level_color(worst),
    ))
}

fn sparkline(ui: &mut egui::Ui, values: &VecDeque<f32>, color: Color32, height: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width().min(520.0), height), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::same(4), theme::EDITOR_BACKGROUND);
    for level in [50.0, 85.0] {
        let y = rect.bottom() - rect.height() * level / 100.0;
        painter.line_segment([pos2(rect.left(), y), pos2(rect.right(), y)], Stroke::new(1.0, theme::BORDER));
    }
    if values.len() < 2 {
        return;
    }
    let step = rect.width() / (HISTORY - 1) as f32;
    let offset = HISTORY - values.len();
    let points: Vec<_> = values
        .iter()
        .enumerate()
        .map(|(i, v)| pos2(rect.left() + (offset + i) as f32 * step, rect.bottom() - rect.height() * v / 100.0))
        .collect();
    let (Some(first), Some(last)) = (points.first().copied(), points.last().copied()) else { return };
    let mut fill = points.clone();
    fill.push(pos2(last.x, rect.bottom()));
    fill.push(pos2(first.x, rect.bottom()));
    painter.add(Shape::convex_polygon(fill, color.gamma_multiply(0.15), Stroke::NONE));
    painter.add(Shape::line(points, Stroke::new(1.5, color)));
}

fn bar(ui: &mut egui::Ui, label: &str, percent: f32, detail: &str) {
    ui.horizontal(|ui| {
        ui.add_sized([120.0, 18.0], egui::Label::new(RichText::new(label).small()).truncate());
        let (rect, _) = ui.allocate_exact_size(vec2(220.0, 10.0), Sense::hover());
        ui.painter().rect_filled(rect, CornerRadius::same(3), theme::SURFACE);
        let mut filled = rect;
        filled.set_width(rect.width() * percent.clamp(0.0, 100.0) / 100.0);
        let color = if percent >= 85.0 { level_color(percent) } else { theme::ACCENT };
        ui.painter().rect_filled(filled, CornerRadius::same(3), color);
        ui.label(RichText::new(format!("{percent:.0}%  {detail}")).small().color(theme::MUTED_TEXT));
    });
}

/// The PULSE tab of the bottom panel.
pub fn show(ui: &mut egui::Ui, app: &mut App) {
    if !app.config.pulse {
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            ui.label(RichText::new(t!("Pulse is off")).color(theme::MUTED_TEXT));
            if ui.button(t!("Turn on")).clicked() {
                app.config.pulse = true;
                app.config.save();
            }
        });
        return;
    }
    let host = app.workspace.as_ref().map(|w| w.host.clone());
    let Some(sample) = app.pulse.sample.clone() else {
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            match &app.pulse.error {
                Some(error) => ui.label(RichText::new(error).color(theme::ERROR)),
                None => ui.spinner(),
            };
        });
        return;
    };
    let mut kill: Option<(u32, String, bool)> = None;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        egui::Frame::new().inner_margin(egui::Margin::symmetric(14, 6)).show(ui, |ui| {
            ui.horizontal(|ui| {
                let where_ = if host.as_ref().is_some_and(Host::is_remote) { t!("over SSH") } else { t!("local") };
                ui.label(RichText::new(&sample.hostname).strong());
                ui.label(RichText::new(where_).small().color(theme::MUTED_TEXT));
                ui.label(RichText::new(t!("· up {n1}", n1 = uptime(sample.uptime_s))).small().color(theme::MUTED_TEXT));
                let load_color = level_color(sample.load[0] / sample.cores as f32 * 100.0);
                ui.label(
                    RichText::new(t!(
                        "· load {n1} {n2} {n3} on {n4} cores",
                        n1 = format!("{:.2}", sample.load[0]),
                        n2 = format!("{:.2}", sample.load[1]),
                        n3 = format!("{:.2}", sample.load[2]),
                        n4 = sample.cores
                    ))
                    .small()
                    .color(load_color),
                );
            });
            ui.add_space(6.0);
            ui.columns(2, |columns| {
                let cpu = app.pulse.cpu_percent().unwrap_or(0.0);
                columns[0].label(RichText::new(format!("CPU {cpu:.0}%")).color(value_color(cpu)));
                sparkline(&mut columns[0], &app.pulse.cpu, theme::ACCENT, 70.0);
                let mem = app.pulse.mem_percent().unwrap_or(0.0);
                let used = sample.mem_total_kb - sample.mem_available_kb;
                let swap = sample.swap_total_kb - sample.swap_free_kb;
                let swap_text = if sample.swap_total_kb > 0 {
                    format!(" · swap {}/{}", human_kb(swap), human_kb(sample.swap_total_kb))
                } else {
                    String::new()
                };
                columns[1].label(
                    RichText::new(t!(
                        "Memory {n1}/{n2} ({mem}%){swap_text}",
                        n1 = human_kb(used),
                        n2 = human_kb(sample.mem_total_kb),
                        mem = format!("{:.0}", mem),
                        swap_text = swap_text
                    ))
                    .color(value_color(mem)),
                );
                sparkline(&mut columns[1], &app.pulse.mem, theme::SUCCESS, 70.0);
            });
            ui.add_space(8.0);
            ui.label(RichText::new(t!("DISKS")).small().strong().color(theme::MUTED_TEXT));
            let project_mount = sample.project_disk.as_ref().map(|d| d.mount.clone());
            for disk in &sample.disks {
                let label = if Some(&disk.mount) == project_mount.as_ref() { format!("{} ◆", disk.mount) } else { disk.mount.clone() };
                bar(ui, &label, disk.percent(), &t!("{n1} of {n2}", n1 = human_kb(disk.used_kb), n2 = human_kb(disk.total_kb)));
            }
            ui.add_space(8.0);
            ui.label(RichText::new(t!("PROCESSES")).small().strong().color(theme::MUTED_TEXT));
            egui::Grid::new("pulse_processes").striped(true).spacing([18.0, 3.0]).show(ui, |ui| {
                for header in ["PID", "CPU", "MEM", "RSS", t!("Process"), ""] {
                    ui.label(RichText::new(header).small().color(theme::FAINT_TEXT));
                }
                ui.end_row();
                for process in &sample.processes {
                    ui.label(RichText::new(process.pid.to_string()).monospace().small());
                    ui.label(
                        RichText::new(format!("{:.1}%", process.cpu))
                            .monospace()
                            .small()
                            .color(level_color(process.cpu / sample.cores as f32)),
                    );
                    ui.label(RichText::new(format!("{:.1}%", process.mem)).monospace().small());
                    ui.label(RichText::new(human_kb(process.rss_kb)).monospace().small());
                    ui.label(&process.name);
                    ui.horizontal(|ui| {
                        if ui.small_button(t!("stop")).on_hover_text("SIGTERM").clicked() {
                            kill = Some((process.pid, process.name.clone(), false));
                        }
                        if ui.small_button("kill -9").clicked() {
                            kill = Some((process.pid, process.name.clone(), true));
                        }
                    });
                    ui.end_row();
                }
            });
        });
    });
    if let (Some((pid, name, force)), Some(host)) = (kill, host) {
        let signal = if force { "-9" } else { "-15" };
        app.dialog = Some(Dialog::Confirm {
            title: t!("Stop process").into(),
            message: t!("Send {n1} to {name} (PID {pid})?", n1 = if force { "SIGKILL" } else { "SIGTERM" }, name = name, pid = pid),
            action: Box::new(move |app: &mut App| {
                let root = app.workspace.as_ref().map(|w| w.root.clone()).unwrap_or_default();
                app.bus.spawn(move || {
                    let result = crate::host::run_with_input(&mut host.shell(&root, &format!("kill {signal} {pid}")), None);
                    match result {
                        Ok(_) => Msg::Notice(t!("{name} (PID {pid}): signal sent", name = name, pid = pid), false),
                        Err(error) => Msg::Notice(format!("{name} (PID {pid}): {error}"), true),
                    }
                });
            }),
        });
    }
}

/// Normal text below 85%, warning colors above.
fn value_color(percent: f32) -> Color32 {
    if percent < 85.0 { theme::TEXT } else { level_color(percent) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sample() {
        let output = "cpu  100 0 50 800 50 0 0 0 0 0\n---\nMemTotal: 16000000 kB\nMemAvailable: 4000000 kB\nSwapTotal: 0 kB\nSwapFree: 0 kB\n---\n0.50 0.40 0.30 1/200 1234\n---\n3600.12\n---\n8\n---\n/dev/sda1 100000 80000 20000 80% /\n---\n/dev/sda1 100000 80000 20000 80% /\n---\n  42 12.5  3.1 51200 cargo\n---\nbox\n";
        let sample = parse(output);
        assert_eq!(sample.cpu_idle, 850);
        assert_eq!(sample.cpu_total, 1000);
        assert_eq!(sample.mem_total_kb, 16_000_000);
        assert_eq!(sample.cores, 8);
        assert_eq!(sample.load, [0.5, 0.4, 0.3]);
        assert_eq!(sample.disks.len(), 1);
        assert_eq!(sample.project_disk.as_ref().unwrap().percent(), 80.0);
        assert_eq!(sample.processes[0].name, "cargo");
        assert_eq!(sample.hostname, "box");
    }

    #[test]
    fn parses_macos_sample() {
        let output = "cpu% 13.5\n---\nMemTotal: 16777216 kB\nMemAvailable: 4194304 kB\nSwapTotal: 2097152 kB\nSwapFree: 1048576 kB\n---\n 1.52 1.81 1.90 \n---\n86400\n---\n10\n---\n/dev/disk3s1s1 482797652 10303580 230047404 5% /\n/dev/disk3s5 482797652 220000000 230047404 49% /System/Volumes/Data\n---\n/dev/disk3s5 482797652 220000000 230047404 49% /System/Volumes/Data\n---\n  612  25.0  1.2 200000 WindowServer\n---\nmac.local\n";
        let sample = parse(output);
        assert_eq!(sample.cpu_busy, Some(13.5));
        assert_eq!(sample.mem_total_kb, 16_777_216);
        assert_eq!(sample.swap_free_kb, 1_048_576);
        assert_eq!(sample.load, [1.52, 1.81, 1.9]);
        assert_eq!(sample.uptime_s, 86400);
        assert_eq!(sample.cores, 10);
        assert_eq!(sample.disks.len(), 2);
        assert_eq!(sample.processes[0].name, "WindowServer");
        assert_eq!(sample.hostname, "mac.local");
    }

    /// The script reads Linux `/proc` (or sysctl on macOS).
    #[test]
    #[cfg(unix)]
    fn parses_real_host() {
        let output = std::process::Command::new("sh").arg("-c").arg(SCRIPT).output().unwrap();
        // The editor treats a failing script as an error, so it must succeed.
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let sample = parse(&String::from_utf8_lossy(&output.stdout));
        assert!((sample.cpu_total > 0 || sample.cpu_busy.is_some()) && sample.mem_total_kb > 0 && sample.cores >= 1);
        assert!(sample.project_disk.is_some());
        assert!(!sample.hostname.is_empty());
    }
}
