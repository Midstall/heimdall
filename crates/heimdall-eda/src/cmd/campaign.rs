use clap::Args as ClapArgs;
use eyre::Result;
use heimdall::core::DutId;
use heimdall::daemon::CampaignTemplate;
use serde::Serialize;

#[derive(Debug, ClapArgs)]
pub struct SubmitArgs {
    /// Template to run. Choices: bring-up, characterization, release.
    pub template: String,
    /// DUT id.
    #[arg(long)]
    pub dut: String,
    /// Optional chip serial number.
    #[arg(long)]
    pub chip_serial: Option<String>,
}

#[derive(Debug, ClapArgs)]
pub struct GetArgs {
    /// Campaign id (UUID).
    pub id: String,
}

#[derive(Debug, ClapArgs)]
pub struct ReportArgs {
    /// Campaign id (UUID).
    pub id: String,
}

#[derive(Serialize)]
struct CreateCampaign {
    dut: DutId,
    template: CampaignTemplate,
    #[serde(skip_serializing_if = "Option::is_none")]
    chip_serial: Option<String>,
}

pub async fn submit(args: SubmitArgs, daemon_url: &str) -> Result<()> {
    let template = match args.template.as_str() {
        "bring-up" | "bringup" => CampaignTemplate::BringUp,
        "characterization" => CampaignTemplate::Characterization,
        "release" => CampaignTemplate::Release,
        other => CampaignTemplate::Custom { name: other.into() },
    };
    let body = CreateCampaign {
        dut: DutId::new(args.dut),
        template,
        chip_serial: args.chip_serial,
    };
    let url = format!("{daemon_url}/campaigns");
    let resp = reqwest::Client::new()
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(eyre::Report::from)?;
    if !resp.status().is_success() {
        return Err(eyre::eyre!(
            "daemon returned {}: {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        ));
    }
    let value: serde_json::Value = resp.json().await.map_err(eyre::Report::from)?;
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

pub async fn get(args: GetArgs, daemon_url: &str) -> Result<()> {
    let url = format!("{daemon_url}/campaigns/{}", args.id);
    let resp = reqwest::get(&url).await.map_err(eyre::Report::from)?;
    if !resp.status().is_success() {
        return Err(eyre::eyre!(
            "daemon returned {}: {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        ));
    }
    let value: serde_json::Value = resp.json().await.map_err(eyre::Report::from)?;
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

pub async fn report(args: ReportArgs, daemon_url: &str) -> Result<()> {
    let url = format!("{daemon_url}/campaigns/{}/report.json", args.id);
    let resp = reqwest::get(&url).await.map_err(eyre::Report::from)?;
    if !resp.status().is_success() {
        return Err(eyre::eyre!(
            "daemon returned {}: {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        ));
    }
    let value: serde_json::Value = resp.json().await.map_err(eyre::Report::from)?;
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}
