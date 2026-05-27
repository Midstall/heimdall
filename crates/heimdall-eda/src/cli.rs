use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "heimdall",
    version = heimdall::core::VERSION,
    about = "Physical hardware verification suite"
)]
pub struct Cli {
    /// Path to heimdall.toml. If unset, looks at $HEIMDALL_CONFIG, then ./heimdall.toml.
    #[arg(short = 'c', long, global = true)]
    pub config: Option<PathBuf>,

    /// Daemon URL for subcommands that need to talk to a running daemon.
    #[arg(long, global = true, default_value = "http://127.0.0.1:7777")]
    pub daemon_url: String,

    #[command(subcommand)]
    pub command: Cmd,
}

#[derive(Debug, Subcommand)]
pub enum Cmd {
    /// Verify every configured DUT is reachable and its IDCODE matches.
    Probe(crate::cmd::probe::Args),
    /// Run a built-in test against a configured DUT.
    Run(crate::cmd::run::Args),
    /// Fuzz a target with random generated inputs against the golden model.
    Fuzz(crate::cmd::fuzz::Args),
    /// Daemon mode: long-running rig host (HTTP + WS API).
    #[command(subcommand)]
    Daemon(DaemonCmd),
    /// Campaigns: production-pipeline pipelines of jobs against one DUT.
    #[command(subcommand)]
    Campaign(CampaignCmd),
    /// TUI for monitoring a running daemon.
    Tui(crate::cmd::tui::TuiArgs),
    /// Diagnose the local environment: presence of required external tools,
    /// daemon reachability, GPIO access.
    Doctor(crate::cmd::doctor::DoctorArgs),
}

#[derive(Debug, Subcommand)]
pub enum DaemonCmd {
    /// Start the daemon server.
    Serve(crate::cmd::daemon::ServeArgs),
    /// Dump the current JobStore + BlobStore as a tar snapshot.
    Dump(crate::cmd::daemon::DumpArgs),
    /// Restore a previously-dumped tar snapshot into the configured stores.
    Restore(crate::cmd::daemon::RestoreArgs),
}

#[derive(Debug, Subcommand)]
pub enum CampaignCmd {
    /// Submit a new campaign and print its id + initial state.
    Submit(crate::cmd::campaign::SubmitArgs),
    /// Fetch a campaign by id and print the current state.
    Get(crate::cmd::campaign::GetArgs),
    /// Fetch the JSON acceptance report for a campaign.
    Report(crate::cmd::campaign::ReportArgs),
}
