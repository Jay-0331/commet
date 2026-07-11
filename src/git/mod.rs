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
pub mod stage_tracker;
pub mod status;
pub mod wrappers;

pub use diff::{
    DiffChunk, filter as filter_diff, header_summary, merge_ignore_globs, parse_chunks,
    truncate as truncate_diff,
};
pub use stage_tracker::StageTracker;
pub use status::{FileEntry, FileStatus, parse_porcelain, status_porcelain};
pub use wrappers::{add, commit, current_branch, diff_staged, repo_root, restore_staged};
