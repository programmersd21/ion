use std::io;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::buffer::Buffer;
use ratatui::style::Color;
use ratatui::widgets::TableState;
use ratatui::DefaultTerminal;
use tachyonfx::{fx, EffectManager, Motion};

use crate::system::{SystemMonitor, SystemSnapshot};
use crate::ui;

pub struct App {
    system: SystemMonitor,
    history: History,
    cache: ui::UiCache,
    effects: UiEffects,
    anim: AnimatedMetrics,
    process_state: TableState,
    selected: usize,
    nav_mode: bool,
    tick_rate: Duration,
    frame_rate: Duration,
    last_data: Instant,
    last_frame: Instant,
}

impl App {
    pub fn new() -> Self {
        let mut system = SystemMonitor::new();
        system.refresh(Duration::from_millis(1));
        let snapshot = system.snapshot();

        let mut history = History::new(120, 4);
        history.push(snapshot.cpu_total_pct, snapshot.mem_percent);

        let mut cache = ui::UiCache::new();
        cache.update(snapshot, false);

        let mut process_state = TableState::default();
        process_state.select(Some(0));

        let effects = UiEffects::new();
        let anim = AnimatedMetrics::new(snapshot);

        Self {
            system,
            history,
            cache,
            effects,
            anim,
            process_state,
            selected: 0,
            nav_mode: false,
            tick_rate: Duration::from_millis(250),
            frame_rate: Duration::from_millis(16),
            last_data: Instant::now(),
            last_frame: Instant::now(),
        }
    }

    pub async fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        let mut tick = tokio::time::interval(self.tick_rate);
        let mut frame = tokio::time::interval(self.frame_rate);
        loop {
            tokio::select! {
                _ = tick.tick() => {
                    let now = Instant::now();
                    let elapsed = now.saturating_duration_since(self.last_data);
                    self.last_data = now;
                    self.refresh_data(elapsed);
                }
                _ = frame.tick() => {
                    if self.handle_events()? {
                        return Ok(());
                    }
                    let now = Instant::now();
                    let delta = now.saturating_duration_since(self.last_frame);
                    self.last_frame = now;
                    self.anim.update(self.system.snapshot(), delta);
                    terminal.draw(|frame| self.draw(frame, delta))?;
                }
            }
        }
    }

    fn handle_events(&mut self) -> io::Result<bool> {
        while event::poll(Duration::from_millis(0))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    if self.handle_key(key.code) {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
    }

    fn handle_key(&mut self, code: KeyCode) -> bool {
        match code {
            KeyCode::Char('q') => true,
            KeyCode::Char('r') => {
                self.refresh_data(Duration::from_millis(1));
                self.last_data = Instant::now();
                false
            }
            KeyCode::Char('/') => {
                self.nav_mode = !self.nav_mode;
                if self.nav_mode && !self.system.snapshot().processes.is_empty() {
                    self.process_state.select(Some(self.selected));
                    self.effects.on_selection();
                } else {
                    self.process_state.select(None);
                }
                self.cache.update(self.system.snapshot(), self.nav_mode);
                false
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.nav_mode {
                    self.select_prev();
                }
                false
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.nav_mode {
                    self.select_next();
                }
                false
            }
            KeyCode::Esc => {
                self.nav_mode = false;
                self.process_state.select(None);
                self.cache.update(self.system.snapshot(), self.nav_mode);
                false
            }
            _ => false,
        }
    }

    fn select_next(&mut self) {
        let len = self.system.snapshot().processes.len();
        if len == 0 {
            return;
        }
        self.selected = (self.selected + 1) % len;
        self.process_state.select(Some(self.selected));
        self.effects.on_selection();
    }

    fn select_prev(&mut self) {
        let len = self.system.snapshot().processes.len();
        if len == 0 {
            return;
        }
        if self.selected == 0 {
            self.selected = len - 1;
        } else {
            self.selected -= 1;
        }
        self.process_state.select(Some(self.selected));
        self.effects.on_selection();
    }

    fn refresh_data(&mut self, elapsed: Duration) {
        self.system.refresh(elapsed);
        let snapshot = self.system.snapshot();
        self.history.push(snapshot.cpu_total_pct, snapshot.mem_percent);
        self.cache.update(snapshot, self.nav_mode);
        self.effects.on_data();
        let len = snapshot.processes.len();
        if len == 0 {
            self.selected = 0;
            self.process_state.select(None);
        } else if self.selected >= len {
            self.selected = len - 1;
            if self.nav_mode {
                self.process_state.select(Some(self.selected));
            }
        }
    }

    fn draw(&mut self, frame: &mut ratatui::Frame, delta: Duration) {
        self.effects.pulse();
        let layout = ui::draw(
            frame,
            self.system.snapshot(),
            &self.history,
            &self.cache,
            &self.anim,
            self.nav_mode,
            &mut self.process_state,
        );
        self.effects
            .process(delta, frame.buffer_mut(), &layout);
    }
}

pub struct AnimatedMetrics {
    pub cpu: f32,
    pub mem: f32,
    pub gpu0: f32,
    pub gpu1: f32,
}

impl AnimatedMetrics {
    fn new(snapshot: &SystemSnapshot) -> Self {
        let (gpu0, gpu1) = gpu_targets(snapshot);
        Self {
            cpu: snapshot.cpu_total_pct,
            mem: snapshot.mem_percent,
            gpu0,
            gpu1,
        }
    }

    fn update(&mut self, snapshot: &SystemSnapshot, delta: Duration) {
        let (gpu0, gpu1) = gpu_targets(snapshot);
        let alpha = (delta.as_secs_f32() * 6.0).clamp(0.0, 1.0);
        self.cpu += (snapshot.cpu_total_pct - self.cpu) * alpha;
        self.mem += (snapshot.mem_percent - self.mem) * alpha;
        self.gpu0 += (gpu0 - self.gpu0) * alpha;
        self.gpu1 += (gpu1 - self.gpu1) * alpha;
    }
}

fn gpu_targets(snapshot: &SystemSnapshot) -> (f32, f32) {
    let gpu0 = snapshot
        .gpus
        .get(0)
        .and_then(|g| g.usage)
        .unwrap_or(0.0);
    let gpu1 = snapshot
        .gpus
        .get(1)
        .and_then(|g| g.usage)
        .unwrap_or(0.0);
    (gpu0, gpu1)
}

struct UiEffects {
    banner: EffectManager<()>,
    summary: EffectManager<()>,
    gauges: EffectManager<()>,
    sparklines: EffectManager<()>,
    table: EffectManager<()>,
    side: EffectManager<()>,
    row: EffectManager<()>,
    last_banner: Instant,
    last_gauge: Instant,
    last_spark: Instant,
    last_table: Instant,
    last_side: Instant,
}

impl UiEffects {
    fn new() -> Self {
        let mut banner = EffectManager::default();
        banner.add_effect(fx::sequence(&[
            fx::coalesce(600),
            fx::sweep_in(Motion::LeftToRight, 12, 0, Color::Black, 900),
            fx::fade_to_fg(Color::Cyan, 1200),
        ]));
        Self {
            banner,
            summary: EffectManager::default(),
            gauges: EffectManager::default(),
            sparklines: EffectManager::default(),
            table: EffectManager::default(),
            side: EffectManager::default(),
            row: EffectManager::default(),
            last_banner: Instant::now(),
            last_gauge: Instant::now(),
            last_spark: Instant::now(),
            last_table: Instant::now(),
            last_side: Instant::now(),
        }
    }

    fn pulse(&mut self) {
        let now = Instant::now();
        if now.duration_since(self.last_banner) > Duration::from_secs(2) {
            self.banner
                .add_effect(fx::fade_to_fg(Color::Cyan, 1200));
            self.last_banner = now;
        }
        if now.duration_since(self.last_gauge) > Duration::from_millis(350) {
            self.gauges.add_effect(fx::sweep_in(
                Motion::LeftToRight,
                6,
                0,
                Color::Black,
                400,
            ));
            self.last_gauge = now;
        }
        if now.duration_since(self.last_spark) > Duration::from_millis(320) {
            self.sparklines
                .add_effect(fx::fade_from_fg(Color::Black, 300));
            self.last_spark = now;
        }
        if now.duration_since(self.last_table) > Duration::from_millis(500) {
            self.table.add_effect(fx::dissolve(250));
            self.last_table = now;
        }
        if now.duration_since(self.last_side) > Duration::from_millis(600) {
            self.side.add_effect(fx::coalesce(280));
            self.last_side = now;
        }
    }

    fn on_data(&mut self) {
        self.gauges.add_effect(fx::fade_from_fg(Color::Black, 200));
        self.sparklines.add_effect(fx::fade_to_fg(Color::Cyan, 220));
        self.table.add_effect(fx::fade_from_fg(Color::Black, 200));
        self.side.add_effect(fx::fade_from_fg(Color::Black, 200));
        self.summary.add_effect(fx::fade_to_fg(Color::Cyan, 180));
    }

    fn on_selection(&mut self) {
        self.row.add_effect(fx::fade_to_fg(Color::Cyan, 240));
    }

    fn process(&mut self, delta: Duration, buf: &mut Buffer, layout: &ui::UiLayout) {
        let dt = delta.into();
        self.banner.process_effects(dt, buf, layout.banner);
        self.summary.process_effects(dt, buf, layout.summary);
        self.gauges.process_effects(dt, buf, layout.gauges_area);
        self.sparklines.process_effects(dt, buf, layout.sparklines_area);
        self.table.process_effects(dt, buf, layout.table);
        self.side.process_effects(dt, buf, layout.side);
        if let Some(row) = layout.selected_row {
            self.row.process_effects(dt, buf, row);
        }
    }
}

pub struct History {
    cpu: RingBuffer,
    ram: RingBuffer,
    cpu_ordered: Vec<u64>,
    ram_ordered: Vec<u64>,
    cpu_spaced: Vec<u64>,
    ram_spaced: Vec<u64>,
    cpu_max: u64,
    ram_max: u64,
    gap: usize,
}

impl History {
    pub fn new(size: usize, gap: usize) -> Self {
        let cpu = RingBuffer::new(size);
        let ram = RingBuffer::new(size);
        let mut cpu_ordered = Vec::with_capacity(size);
        let mut ram_ordered = Vec::with_capacity(size);
        let spaced_cap = size + (size / gap.max(1)) + 1;
        let cpu_spaced = Vec::with_capacity(spaced_cap);
        let ram_spaced = Vec::with_capacity(spaced_cap);
        cpu.write_ordered(&mut cpu_ordered);
        ram.write_ordered(&mut ram_ordered);
        Self {
            cpu,
            ram,
            cpu_ordered,
            ram_ordered,
            cpu_spaced,
            ram_spaced,
            cpu_max: 1,
            ram_max: 1,
            gap: gap.max(1),
        }
    }

    pub fn push(&mut self, cpu_pct: f32, ram_pct: f32) {
        self.cpu.push(cpu_pct.round().clamp(0.0, 100.0) as u64);
        self.ram.push(ram_pct.round().clamp(0.0, 100.0) as u64);
        self.cpu.write_ordered(&mut self.cpu_ordered);
        self.ram.write_ordered(&mut self.ram_ordered);
        self.cpu_max = max_value(&self.cpu_ordered);
        self.ram_max = max_value(&self.ram_ordered);
        self.cpu_spaced.clear();
        self.ram_spaced.clear();
        for (i, value) in self.cpu_ordered.iter().enumerate() {
            self.cpu_spaced.push(*value);
            if (i + 1) % self.gap == 0 {
                self.cpu_spaced.push(0);
            }
        }
        for (i, value) in self.ram_ordered.iter().enumerate() {
            self.ram_spaced.push(*value);
            if (i + 1) % self.gap == 0 {
                self.ram_spaced.push(0);
            }
        }
    }

    pub fn cpu(&self) -> &[u64] {
        &self.cpu_spaced
    }

    pub fn ram(&self) -> &[u64] {
        &self.ram_spaced
    }

    pub fn cpu_max(&self) -> u64 {
        self.cpu_max.max(1)
    }

    pub fn ram_max(&self) -> u64 {
        self.ram_max.max(1)
    }
}

fn max_value(values: &[u64]) -> u64 {
    values.iter().copied().max().unwrap_or(1)
}

struct RingBuffer {
    data: Vec<u64>,
    index: usize,
    filled: bool,
}

impl RingBuffer {
    fn new(size: usize) -> Self {
        Self {
            data: vec![0; size],
            index: 0,
            filled: false,
        }
    }

    fn push(&mut self, value: u64) {
        if self.data.is_empty() {
            return;
        }
        self.data[self.index] = value;
        self.index = (self.index + 1) % self.data.len();
        if self.index == 0 {
            self.filled = true;
        }
    }

    fn write_ordered(&self, out: &mut Vec<u64>) {
        out.clear();
        if self.data.is_empty() {
            return;
        }
        if self.filled {
            out.extend_from_slice(&self.data[self.index..]);
            out.extend_from_slice(&self.data[..self.index]);
        } else {
            out.extend_from_slice(&self.data);
        }
    }
}
