//! Time patterns described in musical notation.
//!
//! Seed concept (from `Conductor life scheduler`):
//!   - Use **6/4** for a full day: 1 quarter note = 4 hours, 6 beats = 24 h.
//!   - Encode sleep vs wake by tag (low register = sleep, high = wake).
//!   - Subdivide a beat into demisemiquavers (32nds) to show wake textures
//!     inside a sleep block. With q=4h, 1 thirty-second ≈ 30 min.

pub mod notation;
pub mod pattern;
pub mod parser;
pub mod render;
pub mod from_plan;

pub use notation::{NoteValue, TimeSignature, Tempo};
pub use pattern::{Beat, BeatContent, Bar, Pattern, PatternError};
pub use parser::{parse, ParseError};
pub use render::render_ascii;
pub use from_plan::{parse_plan, plan_to_pattern, pattern_to_dsl, Plan, PlanItem, PlanError};
