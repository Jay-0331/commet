pub mod cli;
pub mod clipboard;
pub mod config;
pub mod doctor;
pub mod editor;
pub mod error;
pub mod git;
pub mod learning;
pub mod prompt;
pub mod provider;
pub mod tui;

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
