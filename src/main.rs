use clap::Parser;
use commitcrafter::cli::{Cli, Command};

fn main() {
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
}
