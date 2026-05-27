use clap::Args as ClapArgs;
use eyre::Result;

#[derive(Debug, ClapArgs)]
pub struct TuiArgs {}

pub async fn run(_args: TuiArgs, daemon_url: &str) -> Result<()> {
    heimdall::tui::run_app(daemon_url.to_string())
        .await
        .map_err(eyre::Report::from)?;
    Ok(())
}
