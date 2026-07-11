use std::process::ExitCode;

use clap::Parser;
use commet::cli::{
    Cli, Command, ConfigCmd, ConfigEditArgs, ConfigShowArgs, HistoryArgs, ProvidersArgs,
};
use commet::cmd;
use commet::config::{Layered, Loaded, discover, edit, render_json, render_toml};
use commet::error::Result;
use commet::git;
use commet::log;
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
        None => cmd_generate(&cli),
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
        Some(Command::Providers(args)) => cmd_providers(args),
        Some(Command::History(args)) => cmd_history(args),
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
    let loaded = load_layered_with_set(cli)?;
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

/// Print the provider key + reachability matrix. Reads config from
/// files only (defaults + global + repo) — the default flow's CLI
/// flags don't apply to a subcommand.
fn cmd_providers(args: &ProvidersArgs) -> Result<()> {
    let loaded = load_layered_from_files()?;
    cmd::providers::run(&loaded.config, args)
}

/// Print recorded commit-message history, newest first.
fn cmd_history(args: &HistoryArgs) -> Result<()> {
    let loaded = load_layered_from_files()?;
    let cwd = std::env::current_dir()?;
    let repo_root = git::repo_root(&cwd).ok();
    cmd::history::run(&loaded.config, args, repo_root.as_deref())
}

/// Default flow: generate a commit message from the staged diff and
/// print or commit it. Reads config from files + `--set` (the other
/// CLI overrides — `--provider`, `--model`, `-t`, `-g`, `-p` — are
/// applied per-run inside the generate command).
fn cmd_generate(cli: &Cli) -> Result<()> {
    let loaded = load_layered_with_set(cli)?;
    let cwd = std::env::current_dir()?;
    cmd::generate::run(&loaded.config, &cli.generate, &cwd)
}

/// Load the effective config from defaults + config files, with no
/// CLI-flag or `--set` layer.
fn load_layered_from_files() -> Result<Loaded> {
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

    layered.load()
}

fn load_layered_with_set(cli: &Cli) -> Result<Loaded> {
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
