//! Shell-out wrappers around `git`.
//!
//! Every git invocation in the crate funnels through this module so:
//!
//! - Arguments are passed as explicit `arg()` calls (never a shell
//!   string) — no quoting bugs, no injection risk.
//! - Failures include the full argv and stderr in the error message
//!   so users can reproduce the exact command we ran.
//! - The rest of the codebase touches only typed helpers like
//!   [`status_porcelain`] and [`commit`], not `std::process::Command`
//!   directly.
//!
//! Porcelain parsing lives in [`status`]; the bare process plumbing
//! lives in [`wrappers`].

pub mod diff;
pub mod status;
pub mod wrappers;

pub use diff::{DiffChunk, header_summary, parse_chunks, truncate as truncate_diff};
pub use status::{FileEntry, FileStatus, parse_porcelain, status_porcelain};
pub use wrappers::{add, commit, diff_staged, repo_root, restore_staged};
