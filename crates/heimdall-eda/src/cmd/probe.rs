use clap::Args as ClapArgs;
use eyre::Result;

#[derive(Debug, ClapArgs)]
pub struct Args {
    /// Specific DUT id. If omitted, probes all configured DUTs.
    #[arg(long)]
    pub dut: Option<String>,
}

pub async fn run(_args: Args, cfg_path: Option<std::path::PathBuf>) -> Result<()> {
    let path = cfg_path
        .or_else(|| std::env::var_os("HEIMDALL_CONFIG").map(Into::into))
        .unwrap_or_else(|| std::path::PathBuf::from("heimdall.toml"));
    tracing::info!(?path, "loading config");
    let cfg = heimdall::config::load_from_path(&path).map_err(eyre::Report::from)?;
    heimdall::config::validate::validate(&cfg).map_err(eyre::Report::from)?;
    tracing::info!(host = %cfg.host.name, dut_count = cfg.duts.len(), "config ok");
    for d in &cfg.duts {
        tracing::info!(id = %d.id, kind = ?d.kind, "probe (placeholder; real IDCODE check lives in driver layer)");
    }
    Ok(())
}
