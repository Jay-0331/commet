use std::process::ExitCode;

use clap::Parser;
use commitcrafter::cli::{Cli, Command};
use commitcrafter::error::Result;

fn main() -> ExitCode {
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
    match cli.command {
        None => println!("(default) generate + commit — not yet implemented"),
        Some(Command::Setup(_)) => println!("setup — not yet implemented"),
        Some(Command::Init(_)) => println!("init — not yet implemented"),
        Some(Command::Doctor(_)) => println!("doctor — not yet implemented"),
        Some(Command::Config(_)) => println!("config — not yet implemented"),
        Some(Command::Providers(_)) => println!("providers — not yet implemented"),
        Some(Command::History(_)) => println!("history — not yet implemented"),
        Some(Command::Forget(_)) => println!("forget — not yet implemented"),
    }
    Ok(())
}
