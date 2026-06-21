use clap::{ArgGroup, Args, Parser, Subcommand, ValueEnum};

/// AI-powered git commit message CLI with a ratatui TUI.
#[derive(Debug, Parser)]
#[command(
    name = "cc",
    bin_name = "cc",
    version,
    about,
    long_about = None,
    propagate_version = true,
    after_help = "Environment:\n  \
        ANTHROPIC_API_KEY, OPENAI_API_KEY, OPENROUTER_API_KEY  read at runtime\n  \
        COMMITCRAFTER_LOG=info,reqwest=warn                    log filter\n  \
        NO_COLOR=1                                             disable ANSI color"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Flags that apply only when no subcommand is given (the default
    /// "stage → generate → commit" flow).
    #[command(flatten)]
    pub generate: GenerateOpts,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// First-run bootstrap: pick a provider, write config, run doctor.
    Setup(SetupArgs),

    /// Write a starter config file without the TUI wizard.
    Init(InitArgs),

    /// Run health checks against the environment + config.
    Doctor(DoctorArgs),

    /// Inspect or edit configuration.
    #[command(subcommand)]
    Config(ConfigCmd),

    /// List registered providers and report status.
    Providers(ProvidersArgs),

    /// Show recent accepted commit messages from the learning store.
    History(HistoryArgs),

    /// Wipe entries from the learning store.
    Forget(ForgetArgs),
}

/// Options for the default "stage → generate → commit" flow.
///
/// These flags are silently ignored when a subcommand is provided; callers
/// should only consult them when `Cli::command.is_none()`.
#[derive(Debug, Args, Default)]
#[command(group(ArgGroup::new("commit_action").args(["yes", "clipboard", "print"]).multiple(false)))]
pub struct GenerateOpts {
    /// Skip preview; commit the first generated message.
    #[arg(short = 'y', long)]
    pub yes: bool,

    /// Copy the selected message to the clipboard instead of committing.
    #[arg(short = 'c', long)]
    pub clipboard: bool,

    /// Number of candidate messages to generate (1..=5).
    #[arg(
        short = 'g',
        long = "generate",
        value_name = "N",
        value_parser = clap::value_parser!(u32).range(1..=5)
    )]
    pub count: Option<u32>,

    /// Exclude paths from the diff sent to the LLM (repeatable; globs).
    #[arg(short = 'x', long = "exclude", value_name = "GLOB")]
    pub exclude: Vec<String>,

    /// Commit message format for this run.
    #[arg(short = 't', long = "type", value_enum, value_name = "FORMAT")]
    pub format: Option<MessageFormat>,

    /// Extra instructions appended to the system prompt.
    #[arg(short = 'p', long = "prompt", value_name = "TEXT")]
    pub prompt: Option<String>,

    /// Bypass pre-commit / commit-msg hooks (passes --no-verify to git commit).
    #[arg(short = 'n', long = "no-verify")]
    pub no_verify: bool,

    /// Print the generated message(s) to stdout; no commit, no TUI.
    #[arg(long)]
    pub print: bool,

    /// Override the configured provider for this run.
    #[arg(long, value_name = "NAME")]
    pub provider: Option<String>,

    /// Override the model id for this run.
    #[arg(long, value_name = "ID")]
    pub model: Option<String>,

    /// Auto-stage all tracked changes before generating.
    #[arg(long)]
    pub all: bool,

    /// Disable ANSI color (also honors NO_COLOR).
    #[arg(long = "no-color")]
    pub no_color: bool,

    /// One-shot config override; repeatable, e.g. --set style.subject_max_len=50
    #[arg(long = "set", value_name = "KEY=VAL")]
    pub set: Vec<String>,
}

/// Commit message format selectable via `-t / --type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum MessageFormat {
    Plain,
    Conventional,
    #[value(name = "conventional+body")]
    ConventionalBody,
    Gitmoji,
    #[value(name = "subject+body")]
    SubjectBody,
    Custom,
}

#[derive(Debug, Args)]
pub struct SetupArgs {
    /// Overwrite an existing config file.
    #[arg(long)]
    pub force: bool,

    /// Skip the TUI wizard; write defaults non-interactively.
    #[arg(long = "noninteractive")]
    pub noninteractive: bool,
}

#[derive(Debug, Args)]
pub struct InitArgs {
    /// Overwrite an existing config file.
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Args)]
pub struct DoctorArgs {
    /// Also issue a tiny smoke completion against the selected provider.
    #[arg(long)]
    pub full: bool,

    /// Emit machine-readable JSON instead of the human-friendly table.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCmd {
    /// Print the effective merged config with source per key.
    Show(ConfigShowArgs),

    /// Open the config file in $EDITOR.
    Edit(ConfigEditArgs),
}

#[derive(Debug, Args)]
pub struct ConfigShowArgs {
    /// Emit JSON `{ value, source }` per leaf instead of TOML.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
#[command(group(ArgGroup::new("scope").args(["global", "repo"]).multiple(false)))]
pub struct ConfigEditArgs {
    /// Edit the global config file even when inside a repo.
    #[arg(long)]
    pub global: bool,

    /// Edit the per-repo config file (default when inside a repo).
    #[arg(long)]
    pub repo: bool,
}

#[derive(Debug, Args)]
pub struct ProvidersArgs {
    /// JSON output.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct HistoryArgs {
    /// Show only the most recent N entries.
    #[arg(long, value_name = "N", default_value_t = 20)]
    pub last: usize,

    /// Restrict to the current repo's learning store.
    #[arg(long)]
    pub repo: bool,

    /// JSON output.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
#[command(group(ArgGroup::new("target").args(["all", "last", "repo"]).required(true).multiple(false)))]
pub struct ForgetArgs {
    /// Truncate the entire learning history (both global and per-repo).
    #[arg(long)]
    pub all: bool,

    /// Drop only the most recent entry.
    #[arg(long)]
    pub last: bool,

    /// Truncate only the per-repo learning history.
    #[arg(long)]
    pub repo: bool,

    /// Skip the confirmation prompt.
    #[arg(short = 'y', long = "yes")]
    pub yes: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn default_invocation_parses() {
        let cli = Cli::try_parse_from(["cc"]).expect("bare `cc` should parse");
        assert!(cli.command.is_none());
        assert!(!cli.generate.yes);
        assert_eq!(cli.generate.count, None);
    }

    #[test]
    fn short_flags_parse() {
        let cli = Cli::try_parse_from([
            "cc",
            "-y",
            "-g",
            "3",
            "-x",
            "*.lock",
            "-p",
            "in Spanish",
            "-n",
        ])
        .expect("short flags should parse");
        assert!(cli.generate.yes);
        assert_eq!(cli.generate.count, Some(3));
        assert_eq!(cli.generate.exclude, vec!["*.lock".to_string()]);
        assert_eq!(cli.generate.prompt.as_deref(), Some("in Spanish"));
        assert!(cli.generate.no_verify);
    }

    #[test]
    fn yes_conflicts_with_clipboard() {
        let err = Cli::try_parse_from(["cc", "-y", "-c"]).unwrap_err();
        assert!(
            err.kind() == clap::error::ErrorKind::ArgumentConflict,
            "expected ArgumentConflict, got {:?}",
            err.kind()
        );
    }

    #[test]
    fn yes_conflicts_with_print() {
        let err = Cli::try_parse_from(["cc", "-y", "--print"]).unwrap_err();
        assert!(
            err.kind() == clap::error::ErrorKind::ArgumentConflict,
            "expected ArgumentConflict, got {:?}",
            err.kind()
        );
    }

    #[test]
    fn clipboard_and_print_also_conflict() {
        let err = Cli::try_parse_from(["cc", "-c", "--print"]).unwrap_err();
        assert!(
            err.kind() == clap::error::ErrorKind::ArgumentConflict,
            "expected ArgumentConflict, got {:?}",
            err.kind()
        );
    }

    #[test]
    fn generate_range_enforced() {
        let too_many = Cli::try_parse_from(["cc", "-g", "6"]).unwrap_err();
        assert_eq!(too_many.kind(), clap::error::ErrorKind::ValueValidation);

        let zero = Cli::try_parse_from(["cc", "-g", "0"]).unwrap_err();
        assert_eq!(zero.kind(), clap::error::ErrorKind::ValueValidation);
    }

    #[test]
    fn message_format_accepts_dotted_values() {
        let cli = Cli::try_parse_from(["cc", "-t", "conventional+body"]).unwrap();
        assert_eq!(cli.generate.format, Some(MessageFormat::ConventionalBody));

        let cli = Cli::try_parse_from(["cc", "-t", "gitmoji"]).unwrap();
        assert_eq!(cli.generate.format, Some(MessageFormat::Gitmoji));
    }

    #[test]
    fn set_overrides_are_repeatable() {
        let cli = Cli::try_parse_from([
            "cc",
            "--set",
            "style.subject_max_len=50",
            "--set",
            "providers.openrouter.model=meta-llama/llama-3.1-70b-instruct",
        ])
        .unwrap();
        assert_eq!(cli.generate.set.len(), 2);
    }

    #[test]
    fn doctor_subcommand_parses() {
        let cli = Cli::try_parse_from(["cc", "doctor", "--full"]).unwrap();
        match cli.command {
            Some(Command::Doctor(args)) => assert!(args.full),
            other => panic!("expected Doctor, got {other:?}"),
        }
    }

    #[test]
    fn config_subcommands_parse() {
        let cli = Cli::try_parse_from(["cc", "config", "show", "--json"]).unwrap();
        match cli.command {
            Some(Command::Config(ConfigCmd::Show(args))) => assert!(args.json),
            other => panic!("expected config show, got {other:?}"),
        }

        let cli = Cli::try_parse_from(["cc", "config", "edit", "--global"]).unwrap();
        match cli.command {
            Some(Command::Config(ConfigCmd::Edit(args))) => assert!(args.global),
            other => panic!("expected config edit, got {other:?}"),
        }
    }

    #[test]
    fn forget_requires_a_target() {
        let err = Cli::try_parse_from(["cc", "forget"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn forget_targets_are_mutually_exclusive() {
        let err = Cli::try_parse_from(["cc", "forget", "--all", "--last"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn history_defaults_to_last_20() {
        let cli = Cli::try_parse_from(["cc", "history"]).unwrap();
        match cli.command {
            Some(Command::History(args)) => {
                assert_eq!(args.last, 20);
                assert!(!args.repo);
            }
            other => panic!("expected History, got {other:?}"),
        }
    }
}
