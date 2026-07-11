# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`commet` — AI-powered git commit message CLI with a `ratatui` TUI.
Binary is `commet` (`src/main.rs`); library is `commet` (`src/lib.rs`).
Rust **edition 2024** (let-chains like `if let Some(x) = .. && x.exists()` are in use).

**Status: early / pre-MVP.** Foundations (CLI parse, config, git plumbing,
provider trait) exist; most subcommands and the default generate→commit flow
are stubbed (`main.rs` prints "not yet implemented"). Provider `generate()`
impls all `unimplemented!()`. Don't assume a feature is wired end-to-end —
check the dispatch in `src/main.rs::run`.

## Commands

Uses a `Justfile` (`cargo install just` or `brew install just`); plain `cargo`
works too.

- Build: `just build` / `cargo build`
- Test all: `just test` / `cargo test` (unit + integration in `tests/` + doc)
- **Single test**: `just test-one <substr>` / `cargo test <substr>`
- Lint (deny warnings): `just lint` → `cargo clippy --all-targets --all-features -- -D warnings`
- Format: `just fmt` (write) / `just fmt-check` (verify, CI-matching)
- Full CI gate: `just ci` (fmt-check → lint → test). Run `just pre-push` before pushing.
- Run binary: `just run -- <args>` / `cargo run -- <args>`

## Architecture

Modules under `src/`, each with a doc-comment header explaining its contract:

- **`cli`** — clap derive tree. `Cli` has an optional `Command` subcommand plus
  a flattened `GenerateOpts` (the default no-subcommand flow's flags). Flags in
  `GenerateOpts` are ignored when a subcommand is present — only read them when
  `cli.command.is_none()`.
- **`config`** — layered TOML config with **per-leaf source tracking**.
  Precedence low→high: defaults → global file → repo file → CLI flags → `--set`
  overrides. `Layered` (builder) → `.load()` → `Loaded { config, sources }`.
  Deep-merge for tables; arrays/scalars replace wholesale. `--set` paths are
  validated against `KNOWN_KEYS` with a Levenshtein "did you mean" hint.
  `schema.rs` = the typed `Config`; `merge.rs` = the merge engine;
  `discover.rs` = file locations (XDG global, `.commet.toml` via
  `git rev-parse --show-toplevel`).
- **`git`** — every git call shells out through `wrappers.rs` (explicit
  `arg()`, never a shell string; errors carry full argv + stderr). `status.rs`
  parses porcelain; `diff.rs` is a **pure** (no-I/O) diff shrink pipeline
  (drop ignored globs → drop largest files → truncate tail with marker);
  `stage_tracker.rs` is an RAII guard that auto-unstages commet-staged paths on
  `Drop` (abort/panic), `release()` after a successful commit.
- **`provider`** — `Provider` trait (`name`, `key_env_var`, `generate`) so the
  app only talks to `Box<dyn Provider>`. `registry()` returns the 4 builtins
  (anthropic, openai, openrouter, ollama; only ollama has no key env var).
  `GenerateRequest` / `ProviderError` are the shared I/O types.
- **`error`** — coarse crate-wide `Error` enum + `Result<T>` alias. Each
  variant maps to a fixed process exit code via `exit_code()`
  (abort=1, git/io=2, provider=3, config=4, doctor=5). `main` is the only place
  that converts error→`ExitCode`. Per-domain errors stringify into a variant
  rather than nesting; only `io::Error` auto-converts (`#[from]`) — everything
  else converts explicitly so the variant (hence exit code) is a deliberate choice.
- **`editor`** / **`log`** — `$EDITOR` spawn; stderr tracing init
  (`COMMITCRAFTER_LOG` env filter).

## Conventions

- New git operations go through `git::wrappers` — never call
  `std::process::Command` for git directly.
- Keep `diff.rs` pure: callers pass in the raw diff + porcelain status; the
  module does no I/O.
- Config keys must be added to both the typed schema and `KNOWN_KEYS` (else
  `--set` rejects them).
- Modules carry substantial `//!` docs describing invariants — read them before
  changing behavior, and keep them in sync.
