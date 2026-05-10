//! Local Claude Code pod management.
//!
//! A "pod" here is the set of `claude` agents running in tmux sessions on a
//! single host. This crate inspects those sessions ([`diagnose`]) and
//! restarts the ones that have crashed back to a shell prompt ([`heal`]).
//!
//! Detection is deliberately heuristic: tmux gives us the rendered pane and
//! some metadata (creation time, last activity, cwd). We classify each session
//! by what's on screen rather than by tracking PIDs, so a session counts as
//! "alive" exactly when a user looking at it would see the Claude Code TUI.

pub mod diagnose;
pub mod free;
pub mod heal;
pub mod session;
pub mod tmux;

pub use conductr_core::types::*;
pub use diagnose::{diagnose_all, diagnose_one};
pub use free::{pick_idle, FreeOpts};
pub use heal::{heal_all, HealOutcome, HealPlan};
pub use session::{ensure_session, SessionState};
pub use tmux::Tmux;
