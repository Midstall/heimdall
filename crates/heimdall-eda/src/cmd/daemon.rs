use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use clap::Args as ClapArgs;
use eyre::Result;
use heimdall::daemon::{LocalFsBlobStore, SqliteJobStore};
use heimdall_daemon::dump as snapshot;

#[derive(Debug, ClapArgs)]
pub struct ServeArgs {
    /// Address to bind. Non-loopback addresses emit a startup warning.
    #[arg(long, default_value = "127.0.0.1:7777")]
    pub bind: SocketAddr,

    /// Path to the sqlite database file. Created if missing.
    #[arg(long, default_value = "heimdall.db")]
    pub store_path: PathBuf,

    /// Path to the content-addressed blob store root. Created if missing.
    #[arg(long, default_value = "objects")]
    pub blob_path: PathBuf,
}

#[derive(Debug, ClapArgs)]
pub struct DumpArgs {
    /// Path to the sqlite database file to read from.
    #[arg(long, default_value = "heimdall.db")]
    pub store_path: PathBuf,

    /// Path to the content-addressed blob store root to read from.
    #[arg(long, default_value = "objects")]
    pub blob_path: PathBuf,

    /// Output snapshot path. Use `-` to write to stdout.
    #[arg(long)]
    pub to: PathBuf,
}

#[derive(Debug, ClapArgs)]
pub struct RestoreArgs {
    /// Path to the sqlite database file to populate. Created if missing.
    #[arg(long, default_value = "heimdall.db")]
    pub store_path: PathBuf,

    /// Path to the content-addressed blob store root to populate.
    #[arg(long, default_value = "objects")]
    pub blob_path: PathBuf,

    /// Input snapshot path. Use `-` to read from stdin.
    #[arg(long)]
    pub from: PathBuf,
}

pub async fn dump(args: DumpArgs) -> Result<()> {
    let store = SqliteJobStore::open(&args.store_path)
        .await
        .map_err(eyre::Report::from)?;
    let blobs = LocalFsBlobStore::open(&args.blob_path)
        .await
        .map_err(eyre::Report::from)?;

    let writer: Box<dyn std::io::Write + Send> = if args.to.as_os_str() == "-" {
        Box::new(std::io::stdout())
    } else {
        Box::new(std::fs::File::create(&args.to)?)
    };

    let manifest = snapshot::dump(&store, &blobs, writer)
        .await
        .map_err(eyre::Report::from)?;

    tracing::info!(
        jobs = manifest.job_count,
        campaigns = manifest.campaign_count,
        events = manifest.event_count,
        blobs = manifest.blob_count,
        "dump complete"
    );
    println!(
        "wrote snapshot: {} jobs, {} campaigns, {} events, {} blobs",
        manifest.job_count, manifest.campaign_count, manifest.event_count, manifest.blob_count
    );
    Ok(())
}

pub async fn restore(args: RestoreArgs) -> Result<()> {
    let store = SqliteJobStore::open(&args.store_path)
        .await
        .map_err(eyre::Report::from)?;
    let blobs = LocalFsBlobStore::open(&args.blob_path)
        .await
        .map_err(eyre::Report::from)?;

    let reader: Box<dyn std::io::Read + Send> = if args.from.as_os_str() == "-" {
        Box::new(std::io::stdin())
    } else {
        Box::new(std::fs::File::open(&args.from)?)
    };

    let stats = snapshot::restore(&store, &blobs, reader)
        .await
        .map_err(eyre::Report::from)?;

    tracing::info!(
        jobs = stats.jobs_restored,
        campaigns = stats.campaigns_restored,
        events = stats.events_restored,
        blobs = stats.blobs_restored,
        "restore complete"
    );
    println!(
        "restored snapshot: {} jobs, {} campaigns, {} events, {} blobs",
        stats.jobs_restored, stats.campaigns_restored, stats.events_restored, stats.blobs_restored
    );
    Ok(())
}

pub async fn serve(args: ServeArgs, cfg_path: Option<PathBuf>) -> Result<()> {
    heimdall_i18n::linfo!("log.daemon.starting", bind = args.bind);

    let store = SqliteJobStore::open(&args.store_path)
        .await
        .map_err(eyre::Report::from)?;
    let blobs = LocalFsBlobStore::open(&args.blob_path)
        .await
        .map_err(eyre::Report::from)?;

    let handles = if let Some(path) = cfg_path.as_ref() {
        let config = heimdall::config::load_from_path(path).map_err(eyre::Report::from)?;
        heimdall::config::validate::validate(&config).map_err(eyre::Report::from)?;
        heimdall_i18n::linfo!(
            "log.daemon.loaded_config",
            host = config.host.name,
            duts = config.duts.len(),
        );
        heimdall::daemon::start_with_config(args.bind, Arc::new(store), Arc::new(blobs), &config)
            .await
            .map_err(eyre::Report::from)?
    } else {
        heimdall_i18n::linfo!("log.daemon.no_config");
        heimdall::daemon::start(args.bind, Arc::new(store), Arc::new(blobs))
            .await
            .map_err(eyre::Report::from)?
    };

    tokio::signal::ctrl_c().await.map_err(eyre::Report::from)?;
    heimdall_i18n::linfo!("log.daemon.shutting_down");
    handles.server_task.abort();
    handles.worker_task.abort();
    Ok(())
}
