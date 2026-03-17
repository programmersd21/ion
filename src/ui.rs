use std::fmt::Write;

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Paragraph, TableState};
use ratatui::Frame;

use crate::app::{AnimatedMetrics, History};
use crate::components::{gauges, process_table, sparkline};
use crate::system::SystemSnapshot;

const ACCENT: Color = Color::Rgb(0, 220, 255);
const ACCENT_ALT: Color = Color::Rgb(0, 255, 170);
const WARN: Color = Color::Rgb(255, 200, 0);
const DIM: Color = Color::Rgb(90, 90, 90);

const BANNER: [&str; 5] = [
    "IIII  OOOO  NN   N",
    " II  O    O N N  N",
    " II  O    O N  N N",
    " II  O    O N   NN",
    "IIII  OOOO  N    N",
];

pub struct UiCache {
    banner_text: String,
    summary_text: String,
    per_core_text: String,
    net_text: String,
    conn_text: String,
}

impl UiCache {
    pub fn new() -> Self {
        Self {
            banner_text: BANNER.join("\n"),
            summary_text: String::new(),
            per_core_text: String::new(),
            net_text: String::new(),
            conn_text: String::new(),
        }
    }

    pub fn update(&mut self, snapshot: &SystemSnapshot, nav_mode: bool) {
        self.summary_text.clear();
        let mode = if nav_mode { "NAV" } else { "VIEW" };
        let _ = write!(
            self.summary_text,
            "MODE {}   CPU {}   RAM {}   DISK {}   NET D:{} U:{}",
            mode,
            snapshot.cpu_total_str,
            snapshot.mem_label,
            snapshot.disk_label,
            snapshot.net_rx_rate,
            snapshot.net_tx_rate
        );

        self.per_core_text.clear();
        for (i, line) in snapshot.cpu_per_core_str.iter().enumerate() {
            if i > 0 {
                self.per_core_text.push('\n');
            }
            self.per_core_text.push_str(line);
        }

        self.net_text.clear();
        let _ = write!(
            self.net_text,
            "Down {}\nUp   {}",
            snapshot.net_rx_rate, snapshot.net_tx_rate
        );

        self.conn_text.clear();
        let _ = write!(
            self.conn_text,
            "WiFi {}\nSSID {}\nSignal {} {}\nBT {}\nDevices {}",
            snapshot.wifi.state,
            snapshot.wifi.ssid,
            snapshot.wifi.signal_label,
            snapshot.wifi.signal_dbm_label,
            snapshot.bluetooth.state,
            snapshot.bluetooth.devices_label
        );
    }
}

pub struct UiLayout {
    pub banner: Rect,
    pub summary: Rect,
    pub gauges_area: Rect,
    pub sparklines_area: Rect,
    pub table: Rect,
    pub side: Rect,
    pub selected_row: Option<Rect>,
}

pub fn draw(
    frame: &mut Frame,
    snapshot: &SystemSnapshot,
    history: &History,
    cache: &UiCache,
    anim: &AnimatedMetrics,
    nav_mode: bool,
    process_state: &mut TableState,
) -> UiLayout {
    let area = frame.area();
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(BANNER.len() as u16),
            Constraint::Length(2),
            Constraint::Length(7),
            Constraint::Min(0),
        ])
        .split(area);

    let banner_area = layout[0];
    let summary_area = layout[1];
    let middle_area = layout[2];
    let bottom_area = layout[3];

    draw_banner(frame, banner_area, cache);
    draw_summary(frame, summary_area, cache, nav_mode);

    let middle_split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(middle_area);
    let gauges_area = middle_split[0];
    let sparklines_area = middle_split[1];

    draw_gauges(frame, gauges_area, snapshot, anim);
    draw_sparklines(frame, sparklines_area, history);

    let (table_area, side_area) = if bottom_area.width < 80 {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(bottom_area);
        (rows[0], rows[1])
    } else {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(68), Constraint::Percentage(32)])
            .split(bottom_area);
        (cols[0], cols[1])
    };

    let table = process_table::build(snapshot.processes.as_slice(), nav_mode);
    frame.render_stateful_widget(table, table_area, process_state);

    let side_split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(45),
            Constraint::Percentage(30),
            Constraint::Percentage(25),
        ])
        .split(side_area);

    draw_per_core(frame, side_split[0], cache);
    draw_disk_net(frame, side_split[1], snapshot, cache);
    draw_connectivity(frame, side_split[2], cache);

    let selected_row = if nav_mode {
        selected_row_rect(table_area, process_state.selected())
    } else {
        None
    };

    UiLayout {
        banner: banner_area,
        summary: summary_area,
        gauges_area,
        sparklines_area,
        table: table_area,
        side: side_area,
        selected_row,
    }
}

fn draw_banner(frame: &mut Frame, area: Rect, cache: &UiCache) {
    let banner = Paragraph::new(cache.banner_text.as_str())
        .style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD))
        .alignment(Alignment::Center);
    frame.render_widget(banner, area);
}

fn draw_summary(frame: &mut Frame, area: Rect, cache: &UiCache, nav_mode: bool) {
    let style = if nav_mode {
        Style::default().fg(WARN)
    } else {
        Style::default().fg(ACCENT_ALT)
    };
    let summary = Paragraph::new(cache.summary_text.as_str()).style(style);
    frame.render_widget(summary, area);
}

fn draw_gauges(frame: &mut Frame, area: Rect, snapshot: &SystemSnapshot, anim: &AnimatedMetrics) {
    let slots = gauge_slots(area);
    let cpu = gauges::percent_gauge(
        "CPU",
        anim.cpu,
        snapshot.cpu_total_str.as_str(),
        ACCENT,
    );
    let mem = gauges::percent_gauge(
        "RAM",
        anim.mem,
        snapshot.mem_label.as_str(),
        ACCENT_ALT,
    );
    let (gpu0_label, gpu0_usage) = snapshot
        .gpus
        .get(0)
        .map(|g| (g.label.as_str(), g.usage_label.as_str()))
        .unwrap_or(("GPU 0", "N/A"));
    let (gpu1_label, gpu1_usage) = snapshot
        .gpus
        .get(1)
        .map(|g| (g.label.as_str(), g.usage_label.as_str()))
        .unwrap_or(("GPU 1", "N/A"));

    let gpu_a = gauges::percent_gauge(gpu0_label, anim.gpu0, gpu0_usage, ACCENT);
    let gpu_b = gauges::percent_gauge(gpu1_label, anim.gpu1, gpu1_usage, ACCENT_ALT);

    frame.render_widget(cpu, slots[0]);
    frame.render_widget(mem, slots[1]);
    frame.render_widget(gpu_a, slots[2]);
    frame.render_widget(gpu_b, slots[3]);
}

fn draw_sparklines(frame: &mut Frame, area: Rect, history: &History) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let cpu = sparkline::sparkline("CPU History", history.cpu(), history.cpu_max(), ACCENT);
    let ram = sparkline::sparkline("RAM History", history.ram(), history.ram_max(), ACCENT_ALT);
    frame.render_widget(cpu, chunks[0]);
    frame.render_widget(ram, chunks[1]);
}

fn draw_per_core(frame: &mut Frame, area: Rect, cache: &UiCache) {
    let block = Block::default().borders(Borders::ALL).title("Per-Core");
    let paragraph = Paragraph::new(cache.per_core_text.as_str())
        .block(block)
        .style(Style::default().fg(DIM));
    frame.render_widget(paragraph, area);
}

fn draw_disk_net(frame: &mut Frame, area: Rect, snapshot: &SystemSnapshot, cache: &UiCache) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    let disk = gauges::percent_gauge(
        "Disk",
        snapshot.disk_percent,
        snapshot.disk_label.as_str(),
        ACCENT_ALT,
    );
    frame.render_widget(disk, chunks[0]);

    let block = Block::default().borders(Borders::ALL).title("Network");
    let paragraph = Paragraph::new(cache.net_text.as_str())
        .block(block)
        .style(Style::default().fg(DIM));
    frame.render_widget(paragraph, chunks[1]);
}

fn draw_connectivity(frame: &mut Frame, area: Rect, cache: &UiCache) {
    let block = Block::default().borders(Borders::ALL).title("Connectivity");
    let paragraph = Paragraph::new(cache.conn_text.as_str())
        .block(block)
        .style(Style::default().fg(DIM));
    frame.render_widget(paragraph, area);
}

fn gauge_slots(area: Rect) -> [Rect; 4] {
    if area.width < 80 {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);
        let top = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(rows[0]);
        let bottom = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(rows[1]);
        [top[0], top[1], bottom[0], bottom[1]]
    } else {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(25),
                Constraint::Percentage(25),
                Constraint::Percentage(25),
                Constraint::Percentage(25),
            ])
            .split(area);
        [cols[0], cols[1], cols[2], cols[3]]
    }
}

fn selected_row_rect(table_area: Rect, selected: Option<usize>) -> Option<Rect> {
    let inner = Rect {
        x: table_area.x + 1,
        y: table_area.y + 1,
        width: table_area.width.saturating_sub(2),
        height: table_area.height.saturating_sub(2),
    };
    if inner.height <= 1 {
        return None;
    }
    let idx = selected? as u16;
    let row_y = inner.y + 1 + idx;
    if row_y >= inner.y + inner.height {
        None
    } else {
        Some(Rect {
            x: inner.x,
            y: row_y,
            width: inner.width,
            height: 1,
        })
    }
}
