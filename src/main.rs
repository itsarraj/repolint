use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use repolint::{audit_repo, CheckStatus};

/// Audits a repo directory for real OSS hygiene: a LICENSE file whose
/// actual text matches what the manifest declares, a README with
/// recognizable Install/Usage sections, and a CI config.
#[derive(Parser)]
#[command(name = "repolint", version, about)]
struct Cli {
    /// Directory to audit.
    #[arg(default_value = ".")]
    path: PathBuf,

    /// Print machine-readable JSON instead of the text report.
    #[arg(long)]
    json: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    if !cli.path.is_dir() {
        eprintln!("repolint: {} is not a directory", cli.path.display());
        return ExitCode::from(2);
    }

    let report = audit_repo(&cli.path);

    if cli.json {
        let items: Vec<_> = report
            .checks
            .iter()
            .map(|c| {
                serde_json::json!({
                    "name": c.name,
                    "status": match c.status {
                        CheckStatus::Pass => "pass",
                        CheckStatus::Fail => "fail",
                        CheckStatus::Warn => "warn",
                    },
                    "detail": c.detail,
                })
            })
            .collect();
        let out = serde_json::json!({
            "root": report.root.display().to_string(),
            "passed": report.passed(),
            "checks": items,
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        println!("repolint: {}\n", report.root.display());
        for check in &report.checks {
            let symbol = match check.status {
                CheckStatus::Pass => "PASS",
                CheckStatus::Fail => "FAIL",
                CheckStatus::Warn => "WARN",
            };
            println!("[{symbol}] {:<28} {}", check.name, check.detail);
        }
        println!();
        if report.passed() {
            println!("all checks passed");
        } else {
            println!("{} check(s) failed", report.fail_count());
        }
    }

    if report.passed() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
