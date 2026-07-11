//! Subcommand implementations.
//!
//! Each module owns the logic for one `cc` subcommand, kept out of
//! `main.rs` so the binary stays a thin dispatch layer over the
//! library.

pub mod history;
pub mod providers;
