use clap::Parser;
use eyre::Result;
use tracing_subscriber::EnvFilter;

mod cli;
mod cmd;

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    // Pick up LANG / HEIMDALL_LANG so log lines emitted by lt!() macros land
    // in the user's locale.
    heimdall_i18n::set_locale(heimdall_i18n::detect_locale());
    let cli = cli::Cli::parse();

    // The TUI takes over the terminal. Stray tracing writes to stderr would
    // appear on top of the rendered UI, so route to a log file for the TUI
    // subcommand. Every other subcommand keeps stderr.
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    if matches!(cli.command, cli::Cmd::Tui(_)) {
        let log_path = tui_log_path();
        match log_path.parent().map(std::fs::create_dir_all) {
            Some(Ok(())) | None => {}
            Some(Err(_)) => {}
        }
        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            Ok(file) => {
                tracing_subscriber::fmt()
                    .with_env_filter(env_filter)
                    .with_target(false)
                    .with_ansi(false)
                    .with_writer(std::sync::Mutex::new(file))
                    .init();
            }
            Err(_) => {
                // Last-resort: silence tracing entirely so we don't corrupt
                // the alternate-screen render. Better quiet than scrambled.
                tracing_subscriber::fmt()
                    .with_env_filter(EnvFilter::new("off"))
                    .with_writer(std::io::sink)
                    .init();
            }
        }
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(env_filter)
            .with_target(false)
            .init();
    }

    match cli.command {
        cli::Cmd::Probe(args) => cmd::probe::run(args, cli.config).await,
        cli::Cmd::Run(args) => cmd::run::run(args, cli.config).await,
        cli::Cmd::Fuzz(args) => cmd::fuzz::run(args, cli.config).await,
        cli::Cmd::Daemon(daemon_cmd) => match daemon_cmd {
            cli::DaemonCmd::Serve(args) => cmd::daemon::serve(args, cli.config).await,
            cli::DaemonCmd::Dump(args) => cmd::daemon::dump(args).await,
            cli::DaemonCmd::Restore(args) => cmd::daemon::restore(args).await,
        },
        cli::Cmd::Campaign(camp) => match camp {
            cli::CampaignCmd::Submit(args) => cmd::campaign::submit(args, &cli.daemon_url).await,
            cli::CampaignCmd::Get(args) => cmd::campaign::get(args, &cli.daemon_url).await,
            cli::CampaignCmd::Report(args) => cmd::campaign::report(args, &cli.daemon_url).await,
        },
        cli::Cmd::Tui(args) => cmd::tui::run(args, &cli.daemon_url).await,
        cli::Cmd::Doctor(args) => cmd::doctor::run(args, &cli.daemon_url).await,
    }
}

/// Where the TUI session writes its tracing output. Resolves via the
/// `directories` crate so each platform uses its native convention:
///   - Linux:   `$XDG_STATE_HOME/heimdall/tui.log` (or `~/.local/state/...`)
///   - macOS:   `~/Library/Application Support/com.Midstall.heimdall/tui.log`
///   - Windows: `%APPDATA%\Midstall\heimdall\state\tui.log`
fn tui_log_path() -> std::path::PathBuf {
    if let Some(proj) = directories::ProjectDirs::from("com", "Midstall", "heimdall") {
        let dir = proj.state_dir().unwrap_or_else(|| proj.cache_dir());
        return dir.join("tui.log");
    }
    std::env::temp_dir().join("heimdall-tui.log")
}
