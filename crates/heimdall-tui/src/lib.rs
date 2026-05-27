//! Heimdall TUI. Ratatui front-end over the daemon's HTTP + WS API.

pub mod client;
pub mod error;

pub mod app;
pub mod event;
pub mod ui;
pub mod views;

pub use app::{App, ConnectionState, ConnectionStatus, DutRow, View};
pub use client::DaemonClient;
pub use error::{Result, TuiError};
pub use event::{AppEvent, backoff_delay, handle_event, spawn_event_pump, ws_reconnect_loop};

use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::io;

/// Drive the TUI against a daemon. Sets up the terminal, runs the render
/// loop, and tears down on exit.
///
/// The TUI tolerates a daemon that's down at startup or goes away mid-session:
/// the WS subscription self-heals (capped exponential backoff to 30s), HTTP
/// poll failures flip the status bar to "disconnected, retrying..." instead
/// of crashing the app, and the next successful reconnect refreshes state.
pub async fn run_app(daemon_url: String) -> Result<()> {
    heimdall_i18n::set_locale(heimdall_i18n::detect_locale());
    let client = DaemonClient::new(daemon_url.clone());
    let mut app = App::new();
    app.status = heimdall_i18n::t!("tui.connecting_to", url = daemon_url);

    // Optimistic initial fetch. Failure is fine: the reconnect loop will
    // surface a disconnected state and retry shortly.
    if let Ok(jobs) = client.list_jobs().await {
        app.jobs = jobs;
        app.mark_connected(&daemon_url);
    }

    let mut events = spawn_event_pump(&client).await?;

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = run_loop(&mut terminal, &mut app, &client, &mut events).await;

    // Restore terminal even on error.
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    res
}

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    client: &DaemonClient,
    events: &mut tokio::sync::mpsc::UnboundedReceiver<AppEvent>,
) -> Result<()> {
    loop {
        terminal.draw(|f| ui::render(f, app))?;
        if app.should_quit {
            return Ok(());
        }
        match events.recv().await {
            Some(ev) => handle_event(app, client, ev).await?,
            None => return Ok(()),
        }
    }
}
