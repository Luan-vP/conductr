//! Time patterns described in musical notation.
//!
//! Seed concept (from `Conductor life scheduler`):
//!   - Use **6 bars** for a full day: `bar_duration 4h`, 6 bars = 24 h.
//!   - Encode sleep vs wake by tag (low register = sleep, high = wake).
//!   - Subdivide a bar (whole note) into eighth notes for 30-min textures
//!     inside a sleep block. With bar=4h, 1 eighth = 30 min.

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
