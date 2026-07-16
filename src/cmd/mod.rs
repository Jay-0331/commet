//! Subcommand implementations.
//!
//! Each module owns the logic for one `commet` subcommand, kept out of
//! `main.rs` so the binary stays a thin dispatch layer over the
//! library.

pub mod doctor;
pub mod forget;
pub mod generate;
pub mod history;
pub mod providers;
pub mod setup;
