mod app;
mod components;
mod system;
mod ui;

use std::io;

use app::App;

fn main() -> io::Result<()> {
    let mut terminal = ratatui::init();
    let mut app = App::new();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_time()
        .build()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    let res = rt.block_on(app.run(&mut terminal));
    ratatui::restore();
    res
}
