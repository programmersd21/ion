use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, Sparkline};

pub fn sparkline<'a>(title: &'a str, data: &'a [u64], max: u64, color: Color) -> Sparkline<'a> {
    Sparkline::default()
        .block(Block::default().borders(Borders::ALL).title(title))
        .data(data)
        .max(max)
        .style(Style::default().fg(color))
}
