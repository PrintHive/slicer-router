//! `slicer-router`: detect which slicer a .3mf belongs to and open it there.

use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use slicer_router_core::{
    detect, discover_installed, launch, route, Config, InstalledSlicer, Route, RouteReason, Slicer,
    Verdict,
};

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print which slicer a .3mf belongs to and the evidence behind it.
    Detect {
        file: PathBuf,
        /// Print the verdict as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Detect, pick an installed slicer and open the file in it.
    Open {
        file: PathBuf,
        /// Print what would open without starting a slicer.
        #[arg(long)]
        dry_run: bool,
    },
    /// List installed slicers.
    Slicers {
        /// Print the list as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Show or set the slicer that opens files with no detectable origin.
    Default {
        /// Slicer id, for example `orca-slicer`. Omit to show the current default.
        slicer: Option<Slicer>,
    },
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode, String> {
    match cli.command {
        Command::Detect { file, json } => {
            let verdict = detect(&file).map_err(|e| e.to_string())?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&verdict).map_err(|e| e.to_string())?
                );
            } else {
                print_verdict(&verdict);
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Open { file, dry_run } => open(&file, dry_run),
        Command::Slicers { json } => {
            let installed = discover_installed();
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&installed).map_err(|e| e.to_string())?
                );
            } else if installed.is_empty() {
                println!("No supported slicers found.");
            } else {
                let default = load_config()?.default_slicer;
                for item in &installed {
                    let marker = if default == Some(item.slicer) {
                        " (default)"
                    } else {
                        ""
                    };
                    println!(
                        "{:<16} {}{marker}",
                        item.slicer.display_name(),
                        item.path.display()
                    );
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Default { slicer } => {
            match slicer {
                Some(slicer) => {
                    let mut config = load_config()?;
                    config.default_slicer = Some(slicer);
                    save_config(&config)?;
                    println!("Default slicer: {slicer}");
                }
                None => match load_config()?.default_slicer {
                    Some(slicer) => println!("{slicer}"),
                    None => println!("No default slicer set."),
                },
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn print_verdict(verdict: &Verdict) {
    match verdict.slicer {
        Some(slicer) => println!("slicer:        {} ({})", slicer.id(), slicer),
        None => println!("slicer:        none (no signal)"),
    }
    println!("rule:          {}", rule_name(verdict));
    let evidence = &verdict.evidence;
    let fields = [
        ("application", &evidence.application),
        ("printer model", &evidence.printer.model),
        ("printer preset", &evidence.printer.settings_id),
        ("inherits", &evidence.printer.inherits),
        ("printer vendor", &evidence.printer.vendor),
    ];
    for (label, value) in fields {
        if let Some(value) = value {
            println!("{:<14} {value}", format!("{label}:"));
        }
    }
    if evidence.prusa_config_present {
        println!("Slic3r_PE.config: present");
    }
    for warning in &evidence.warnings {
        println!("warning:       {warning}");
    }
}

fn open(file: &Path, dry_run: bool) -> Result<ExitCode, String> {
    let verdict = detect(file).unwrap_or_else(|err| {
        eprintln!("warning: {err}; opening it as a file with no detectable origin");
        Verdict::no_signal()
    });
    let mut config = load_config()?;
    let installed = discover_installed();
    let slicers: Vec<Slicer> = installed.iter().map(|i| i.slicer).collect();

    let (slicer, reason) = match route(&verdict, &slicers, &config) {
        Route::Launch { slicer, reason } => (slicer, reason),
        Route::NoneInstalled => return Err("no supported slicer is installed".into()),
        Route::Ask { choices } => {
            let slicer = ask(&choices)?;
            config.default_slicer = Some(slicer);
            save_config(&config)?;
            (slicer, RouteReason::Default)
        }
    };
    let target: &InstalledSlicer = installed
        .iter()
        .find(|i| i.slicer == slicer)
        .expect("route only returns installed slicers");

    println!(
        "{} -> {} ({})",
        file.display(),
        slicer,
        describe(&verdict, reason)
    );
    if !dry_run {
        launch(target, file).map_err(|e| e.to_string())?;
    }
    Ok(ExitCode::SUCCESS)
}

fn describe(verdict: &Verdict, reason: RouteReason) -> String {
    match reason {
        RouteReason::Detected => format!("detected via {}", rule_name(verdict)),
        RouteReason::Override => "config override".into(),
        RouteReason::Default => "default slicer".into(),
        RouteReason::OnlyInstalled => "only installed slicer".into(),
    }
}

fn rule_name(verdict: &Verdict) -> String {
    serde_json::to_value(verdict.rule)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// Ask on the terminal which slicer to use. Fails when there is no terminal.
fn ask(choices: &[Slicer]) -> Result<Slicer, String> {
    if !io::stdin().is_terminal() {
        return Err("no default slicer is set; run `slicer-router default <slicer>` first".into());
    }
    eprintln!("This file has no slicer metadata. Open with:");
    for (i, slicer) in choices.iter().enumerate() {
        eprintln!("  {}) {slicer}", i + 1);
    }
    eprint!("Choice (remembered as the default): ");
    io::stderr().flush().ok();
    let mut line = String::new();
    io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    line.trim()
        .parse::<usize>()
        .ok()
        .and_then(|n| choices.get(n.checked_sub(1)?))
        .copied()
        .ok_or_else(|| "not a valid choice".into())
}

fn config_path() -> Result<PathBuf, String> {
    Config::default_path().ok_or_else(|| "cannot determine the config folder".to_owned())
}

fn load_config() -> Result<Config, String> {
    let config = Config::load(&config_path()?).map_err(|e| e.to_string())?;
    for warning in &config.warnings {
        eprintln!("warning: config: {warning}");
    }
    Ok(config)
}

fn save_config(config: &Config) -> Result<(), String> {
    config.save(&config_path()?).map_err(|e| e.to_string())
}
