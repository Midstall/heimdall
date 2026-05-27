use std::path::PathBuf;
use std::process::Stdio;

use clap::Args as ClapArgs;
use eyre::Result;
use tokio::process::Command;

#[derive(Debug, ClapArgs)]
pub struct DoctorArgs {
    /// Run checks intended for Pi-as-host deployments (GPIO access).
    #[arg(long)]
    pub pi: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Ok,
    Warn,
    Error,
}

impl Severity {
    fn label(self) -> &'static str {
        match self {
            Self::Ok => "OK",
            Self::Warn => "WARN",
            Self::Error => "ERR",
        }
    }

    fn ansi(self) -> &'static str {
        match self {
            Self::Ok => "\x1b[32m",    // green
            Self::Warn => "\x1b[33m",  // yellow
            Self::Error => "\x1b[31m", // red
        }
    }
}

#[derive(Debug, Clone)]
pub struct Check {
    pub name: String,
    pub severity: Severity,
    pub message: String,
}

pub async fn run(args: DoctorArgs, daemon_url: &str) -> Result<()> {
    let mut checks: Vec<Check> = Vec::new();

    // External tools.
    for (tool, importance) in [
        ("clang", Severity::Warn),
        ("spike", Severity::Warn),
        ("openocd", Severity::Warn),
        ("dart", Severity::Warn),
        ("yosys", Severity::Warn),
    ] {
        checks.push(check_tool(tool, importance).await);
    }

    // Daemon reachability.
    checks.push(check_daemon(daemon_url).await);

    // Pi-bound checks.
    if args.pi {
        checks.push(check_path_exists(
            "/dev/gpiochip0",
            "gpiochip0",
            Severity::Error,
        ));
        checks.push(check_user_group("gpio"));
    }

    let mut any_error = false;
    let reset = "\x1b[0m";
    let name_width = checks.iter().map(|c| c.name.len()).max().unwrap_or(8);
    for check in &checks {
        if check.severity == Severity::Error {
            any_error = true;
        }
        println!(
            "  {color}{:width$}{reset}  [{color}{:>4}{reset}]  {}",
            check.name,
            check.severity.label(),
            check.message,
            color = check.severity.ansi(),
            reset = reset,
            width = name_width,
        );
    }

    if any_error {
        Err(eyre::eyre!("one or more doctor checks failed"))
    } else {
        Ok(())
    }
}

async fn check_tool(name: &str, severity_on_missing: Severity) -> Check {
    match Command::new(name)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
    {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let first = stdout.lines().next().unwrap_or("").trim().to_string();
            Check {
                name: name.into(),
                severity: Severity::Ok,
                message: if first.is_empty() {
                    "on PATH".into()
                } else {
                    first
                },
            }
        }
        Ok(_) => Check {
            name: name.into(),
            severity: severity_on_missing,
            message: "on PATH but `--version` failed".into(),
        },
        Err(_) => Check {
            name: name.into(),
            severity: severity_on_missing,
            message: "not found on PATH".into(),
        },
    }
}

async fn check_daemon(url: &str) -> Check {
    let health = format!("{url}/health");
    match reqwest::Client::new()
        .get(&health)
        .timeout(std::time::Duration::from_millis(500))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => Check {
            name: "daemon".into(),
            severity: Severity::Ok,
            message: format!("reachable at {url}"),
        },
        Ok(resp) => Check {
            name: "daemon".into(),
            severity: Severity::Warn,
            message: format!("{url} returned {}", resp.status()),
        },
        Err(_) => Check {
            name: "daemon".into(),
            severity: Severity::Warn,
            message: format!("not reachable at {url} (ok if not running)"),
        },
    }
}

fn check_path_exists(path: &str, name: &str, sev_on_missing: Severity) -> Check {
    let p = PathBuf::from(path);
    if p.exists() {
        Check {
            name: name.into(),
            severity: Severity::Ok,
            message: format!("{path} present"),
        }
    } else {
        Check {
            name: name.into(),
            severity: sev_on_missing,
            message: format!("{path} not found"),
        }
    }
}

fn check_user_group(group: &str) -> Check {
    let user = std::env::var("USER").unwrap_or_default();
    match std::process::Command::new("groups").arg(&user).output() {
        Ok(out) => {
            let s = String::from_utf8_lossy(&out.stdout);
            if s.split_whitespace().any(|g| g == group) {
                Check {
                    name: format!("group:{group}"),
                    severity: Severity::Ok,
                    message: format!("user {user} is in {group}"),
                }
            } else {
                Check {
                    name: format!("group:{group}"),
                    severity: Severity::Warn,
                    message: format!("user {user} NOT in {group}"),
                }
            }
        }
        Err(_) => Check {
            name: format!("group:{group}"),
            severity: Severity::Warn,
            message: "could not run `groups`".into(),
        },
    }
}
