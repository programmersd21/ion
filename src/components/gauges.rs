use ratatui::style::{Color, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, Gauge};

pub fn percent_gauge<'a>(title: &'a str, percent: f32, label: &'a str, color: Color) -> Gauge<'a> {
    let pct = percent.clamp(0.0, 100.0).round() as u16;
    Gauge::default()
        .block(Block::default().borders(Borders::ALL).title(title))
        .gauge_style(Style::default().fg(color))
        .label(Span::raw(label))
        .percent(pct)
}
