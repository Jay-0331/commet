use std::process::ExitCode;

use clap::Parser;
use commitcrafter::cli::{Cli, Command, ConfigCmd, ConfigEditArgs, ConfigShowArgs};
use commitcrafter::config::{Layered, Loaded, discover, edit, render_json, render_toml};
use commitcrafter::error::Result;
use commitcrafter::log;
use tracing::{debug, info};

fn main() -> ExitCode {
    log::init_stderr();

    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::from(err.exit_code())
        }
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    debug!(?cli, "parsed CLI arguments");

    match &cli.command {
        Some(Command::Config(ConfigCmd::Show(args))) => cmd_config_show(&cli, args),
        Some(Command::Config(ConfigCmd::Edit(args))) => cmd_config_edit(args),
        None => {
            info!("(default) generate + commit — not yet implemented");
            println!("commitcrafter: (no behavior yet — see issues #11–#70)");
            Ok(())
        }
        Some(Command::Setup(_)) => {
            info!("setup — not yet implemented");
            Ok(())
        }
        Some(Command::Init(_)) => {
            info!("init — not yet implemented");
            Ok(())
        }
        Some(Command::Doctor(_)) => {
            info!("doctor — not yet implemented");
            Ok(())
        }
        Some(Command::Providers(_)) => {
            info!("providers — not yet implemented");
            Ok(())
        }
        Some(Command::History(_)) => {
            info!("history — not yet implemented");
            Ok(())
        }
        Some(Command::Forget(_)) => {
            info!("forget — not yet implemented");
            Ok(())
        }
    }
}

/// Load the effective layered config (defaults + global + repo + `--set`)
/// and render it to stdout, either as annotated TOML or as JSON.
///
/// CLI flag-layer translation (`--provider`, `--model`, `--no-color`,
/// `--type`) is intentionally deferred until the dispatch for the
/// default command lands; until then, only `--set` overrides feed in
/// from the CLI for `cc config show`.
fn cmd_config_show(cli: &Cli, args: &ConfigShowArgs) -> Result<()> {
    let loaded = load_layered_for_show(cli)?;
    let text = if args.json {
        render_json(&loaded)?
    } else {
        render_toml(&loaded)?
    };
    print!("{text}");
    if !text.ends_with('\n') {
        println!();
    }
    Ok(())
}

/// Open the relevant config file (per-repo or global) in `$EDITOR`,
/// scaffolding a starter template first if it doesn't exist yet.
fn cmd_config_edit(args: &ConfigEditArgs) -> Result<()> {
    edit::run(args)
}

fn load_layered_for_show(cli: &Cli) -> Result<Loaded> {
    let mut layered = Layered::new();

    if let Some(path) = discover::global_config_path()
        && path.exists()
    {
        layered = layered.with_global_file(path)?;
    }
    if let Some(path) = discover::repo_config_path()
        && path.exists()
    {
        layered = layered.with_repo_file(path)?;
    }
    for arg in &cli.generate.set {
        layered = layered.with_set_arg(arg)?;
    }

    layered.load()
}
