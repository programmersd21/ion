use ratatui::layout::Constraint;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Cell, Row, Table};

use crate::system::ProcessInfo;

pub fn build<'a>(processes: &'a [ProcessInfo], active: bool) -> Table<'a> {
    let header = Row::new([
        Cell::from("PID"),
        Cell::from("Name"),
        Cell::from("CPU%"),
        Cell::from("MEM%"),
    ])
    .style(Style::default().add_modifier(Modifier::BOLD));

    let rows = processes.iter().map(|p| {
        Row::new([
            Cell::from(p.pid.as_str()),
            Cell::from(p.name.as_str()),
            Cell::from(p.cpu_str.as_str()),
            Cell::from(p.mem_str.as_str()),
        ])
    });

    let widths = [
        Constraint::Length(6),
        Constraint::Percentage(50),
        Constraint::Length(7),
        Constraint::Length(7),
    ];

    let mut table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title("Processes"))
        .column_spacing(1);

    if active {
        table = table.row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    }

    table
}
