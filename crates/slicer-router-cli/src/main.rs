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

    let route = route(&verdict, &slicers, &config);
    let chosen = choose(
        route,
        &mut config,
        &config_path()?,
        dry_run,
        ask_on_terminal,
    )?;
    let (slicer, reason) = match chosen {
        Chosen::Slicer(slicer, reason) => (slicer, reason),
        Chosen::WouldAsk { detected, choices } => {
            println!("{} -> {}", file.display(), would_ask(detected, &choices));
            return Ok(ExitCode::SUCCESS);
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

/// How a route settled.
#[derive(Debug, PartialEq)]
enum Chosen {
    Slicer(Slicer, RouteReason),
    /// A dry run reached a question. It was not asked and nothing was saved.
    WouldAsk {
        detected: Option<Slicer>,
        choices: Vec<Slicer>,
    },
}

/// Settle a route on one slicer. A question goes to `ask`, and the answer is
/// remembered in `config` and saved to `config_path`. A dry run never asks and
/// never saves.
fn choose(
    route: Route,
    config: &mut Config,
    config_path: &Path,
    dry_run: bool,
    ask: impl FnOnce(Option<Slicer>, &[Slicer]) -> Result<Slicer, String>,
) -> Result<Chosen, String> {
    match route {
        Route::Launch { slicer, reason } => Ok(Chosen::Slicer(slicer, reason)),
        Route::NoneInstalled => Err("no supported slicer is installed".into()),
        Route::Ask { choices, detected } if dry_run => Ok(Chosen::WouldAsk { detected, choices }),
        Route::Ask { choices, detected } => {
            let slicer = ask(detected, &choices)?;
            config.remember_choice(detected, slicer);
            config.save(config_path).map_err(|e| e.to_string())?;
            let reason = if detected.is_some() {
                RouteReason::Override
            } else {
                RouteReason::Default
            };
            Ok(Chosen::Slicer(slicer, reason))
        }
        _ => Err("this version cannot act on the routing result".into()),
    }
}

fn describe(verdict: &Verdict, reason: RouteReason) -> String {
    match reason {
        RouteReason::Detected => format!("detected via {}", rule_name(verdict)),
        RouteReason::Override => "config override".into(),
        RouteReason::Default => "default slicer".into(),
        RouteReason::OnlyInstalled => "only installed slicer".into(),
        other => serialized_name(&other),
    }
}

fn rule_name(verdict: &Verdict) -> String {
    serialized_name(&verdict.rule)
}

/// The kebab-case name a unit enum variant serializes to.
fn serialized_name(value: &impl serde::Serialize) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// The question put to the person when no installed slicer was chosen.
fn heading(detected: Option<Slicer>) -> String {
    match detected {
        Some(slicer) => format!("{slicer} isn't installed. Open this file with:"),
        None => "This file has no slicer metadata. Open with:".into(),
    }
}

/// What a dry run reports instead of asking.
fn would_ask(detected: Option<Slicer>, choices: &[Slicer]) -> String {
    let names: Vec<String> = choices.iter().map(Slicer::to_string).collect();
    let why = match detected {
        Some(slicer) => format!("{slicer} isn't installed"),
        None => "no slicer metadata and no default slicer".to_owned(),
    };
    format!(
        "{why}, so it would ask which slicer to use ({})",
        names.join(", ")
    )
}

/// What the answer will be used for, shown in the prompt.
fn remembered_as(detected: Option<Slicer>) -> String {
    match detected {
        Some(slicer) => format!("for {slicer} files"),
        None => "as the default".into(),
    }
}

/// Ask on the terminal which slicer to use. Fails when there is no terminal.
fn ask_on_terminal(detected: Option<Slicer>, choices: &[Slicer]) -> Result<Slicer, String> {
    if !io::stdin().is_terminal() {
        let why = match detected {
            Some(slicer) => format!("{slicer} isn't installed and no default slicer is set"),
            None => "no default slicer is set".to_owned(),
        };
        return Err(format!("{why}; run `slicer-router default <slicer>` first"));
    }
    eprintln!("{}", heading(detected));
    for (i, slicer) in choices.iter().enumerate() {
        eprintln!("  {}) {slicer}", i + 1);
    }
    eprint!("Choice (remembered {}): ", remembered_as(detected));
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

#[cfg(test)]
mod tests {
    use super::*;
    use Slicer::{BambuStudio, PrusaSlicer, SnapmakerOrca};

    fn ask_route(detected: Option<Slicer>) -> Route {
        Route::Ask {
            choices: vec![BambuStudio, PrusaSlicer],
            detected,
        }
    }

    #[test]
    fn a_missing_slicer_is_named_in_the_question() {
        assert_eq!(
            heading(Some(SnapmakerOrca)),
            "Snapmaker Orca isn't installed. Open this file with:"
        );
        assert_eq!(
            remembered_as(Some(SnapmakerOrca)),
            "for Snapmaker Orca files"
        );
        assert!(heading(None).contains("no slicer metadata"));
        assert_eq!(remembered_as(None), "as the default");
    }

    #[test]
    fn answering_for_a_missing_slicer_saves_an_override() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.json");
        let mut config = Config::default();
        let chosen = choose(
            ask_route(Some(SnapmakerOrca)),
            &mut config,
            &path,
            false,
            |detected, choices| {
                assert_eq!(detected, Some(SnapmakerOrca));
                assert_eq!(choices, [BambuStudio, PrusaSlicer]);
                Ok(PrusaSlicer)
            },
        )
        .unwrap();
        assert_eq!(chosen, Chosen::Slicer(PrusaSlicer, RouteReason::Override));

        let saved = Config::load(&path).unwrap();
        assert_eq!(saved.default_slicer, None);
        assert_eq!(saved.overrides.get(&SnapmakerOrca), Some(&PrusaSlicer));
    }

    #[test]
    fn answering_for_a_file_with_no_signal_saves_the_default() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.json");
        let mut config = Config::default();
        let chosen = choose(ask_route(None), &mut config, &path, false, |_, _| {
            Ok(BambuStudio)
        })
        .unwrap();
        assert_eq!(chosen, Chosen::Slicer(BambuStudio, RouteReason::Default));

        let saved = Config::load(&path).unwrap();
        assert_eq!(saved.default_slicer, Some(BambuStudio));
        assert!(saved.overrides.is_empty());
    }

    #[test]
    fn a_dry_run_never_asks_and_never_saves() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("config.json");
        for detected in [None, Some(SnapmakerOrca)] {
            let mut config = Config::default();
            let chosen = choose(ask_route(detected), &mut config, &path, true, |_, _| {
                panic!("a dry run must not ask")
            })
            .unwrap();
            assert_eq!(
                chosen,
                Chosen::WouldAsk {
                    detected,
                    choices: vec![BambuStudio, PrusaSlicer]
                }
            );
            assert_eq!(
                config,
                Config::default(),
                "the in-memory config is untouched"
            );
            assert!(!path.exists(), "a dry run created {}", path.display());
        }
    }

    #[test]
    fn a_dry_run_still_reports_a_direct_launch() {
        let dir = tempfile::TempDir::new().unwrap();
        let route = Route::Launch {
            slicer: PrusaSlicer,
            reason: RouteReason::Detected,
        };
        let chosen = choose(
            route,
            &mut Config::default(),
            &dir.path().join("c.json"),
            true,
            |_, _| panic!("no question expected"),
        )
        .unwrap();
        assert_eq!(chosen, Chosen::Slicer(PrusaSlicer, RouteReason::Detected));
    }

    #[test]
    fn the_dry_run_report_names_the_missing_slicer() {
        let text = would_ask(Some(SnapmakerOrca), &[BambuStudio, PrusaSlicer]);
        assert!(text.contains("Snapmaker Orca isn't installed"), "{text}");
        assert!(text.contains("Bambu Studio, PrusaSlicer"), "{text}");
        assert!(would_ask(None, &[BambuStudio]).contains("no default slicer"));
    }
}
