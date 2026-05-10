//! Convert an implementation plan (markdown) into a schedule `Pattern`.
//!
//! Mapping:
//! - One bar per topological batch (items that can run in parallel share a bar).
//! - One beat per item; note value encodes the estimate (`w`≥1d, `h`~½d, `q`~2h, …).
//! - Time signature: N/4 where N = max beats in any bar; shorter bars padded with `rest` notes.
//! - Default tempo: quarter = 2 h.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::notation::{NoteValue, Tempo, TimeSignature};
use crate::pattern::{Bar, Beat, BeatContent, Pattern};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    #[error("duplicate ticket id: {0}")]
    DuplicateId(String),
    #[error("unknown dependency `{dep}` referenced by `{from}`")]
    UnknownDep { from: String, dep: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanItem {
    pub id: String,
    pub title: String,
    pub deps: Vec<String>,
    pub estimate: NoteValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub items: Vec<PlanItem>,
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Parse a markdown plan document into a `Plan`.
///
/// Recognises sections of the form `### T<n> — <title>` (em-dash or ASCII dash).
/// Within each section it reads:
/// - `**Depends on:** T1, T2, ...`  (optional)
/// - `**Estimate:** q`              (optional; defaults to `q`)
pub fn parse_plan(markdown: &str) -> Result<Plan, PlanError> {
    let mut items: Vec<PlanItem> = Vec::new();
    // (id, title, deps, estimate)
    let mut current: Option<(String, String, Vec<String>, NoteValue)> = None;

    for line in markdown.lines() {
        let trimmed = line.trim();

        if let Some(rest) = trimmed.strip_prefix("### ") {
            if let Some(new_item) = try_parse_ticket_heading(rest) {
                if let Some((id, title, deps, est)) = current.take() {
                    items.push(PlanItem { id, title, deps, estimate: est });
                }
                current = Some(new_item);
            }
            continue;
        }

        if let Some(ref mut cur) = current {
            if let Some(rest) = trimmed.strip_prefix("**Depends on:**") {
                collect_deps(rest, &mut cur.2);
            } else if let Some(rest) = trimmed.strip_prefix("**Estimate:**") {
                if let Some(c) = rest.trim().chars().next() {
                    if let Some(nv) = NoteValue::from_letter(c) {
                        cur.3 = nv;
                    }
                }
            }
        }
    }

    if let Some((id, title, deps, est)) = current {
        items.push(PlanItem { id, title, deps, estimate: est });
    }

    validate_plan(&items)?;
    Ok(Plan { items })
}

/// Returns `Some((id, title, deps=[], estimate=Quarter))` for a `T<n>` heading.
fn try_parse_ticket_heading(rest: &str) -> Option<(String, String, Vec<String>, NoteValue)> {
    let rest = rest.trim();
    let em_dash = " \u{2014} "; // " — "
    let (id_part, title_part) = if let Some(i) = rest.find(em_dash) {
        (&rest[..i], &rest[i + em_dash.len()..])
    } else if let Some(i) = rest.find(" - ") {
        (&rest[..i], &rest[i + 3..])
    } else {
        return None;
    };
    let id = id_part.trim().to_string();
    if is_ticket_id(&id) {
        Some((id, title_part.trim().to_string(), Vec::new(), NoteValue::Quarter))
    } else {
        None
    }
}

fn is_ticket_id(s: &str) -> bool {
    s.starts_with('T') && s.len() > 1 && s[1..].chars().all(|c| c.is_ascii_digit())
}

fn collect_deps(rest: &str, deps: &mut Vec<String>) {
    let text = rest.trim().trim_end_matches('.');
    for seg in text.split(',') {
        let seg = seg.trim();
        // Allow trailing parentheticals: "T1 (uses core types)" → "T1"
        let token = seg.split_whitespace().next().unwrap_or("").trim_end_matches('.');
        if is_ticket_id(token) {
            deps.push(token.to_string());
        }
    }
}

fn validate_plan(items: &[PlanItem]) -> Result<(), PlanError> {
    let mut seen: HashSet<&str> = HashSet::new();
    for item in items {
        if !seen.insert(item.id.as_str()) {
            return Err(PlanError::DuplicateId(item.id.clone()));
        }
    }
    for item in items {
        for dep in &item.deps {
            if !seen.contains(dep.as_str()) {
                return Err(PlanError::UnknownDep { from: item.id.clone(), dep: dep.clone() });
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Plan → Pattern
// ---------------------------------------------------------------------------

/// Default tempo: 1 quarter note = 2 hours.
const DEFAULT_TEMPO: Tempo = Tempo { quarter_seconds: 2 * 3600 };

/// Convert a `Plan` into a `Pattern` using topological batch ordering.
pub fn plan_to_pattern(plan: &Plan) -> Pattern {
    let batches = topo_batches(plan);
    if batches.is_empty() {
        return Pattern { time_signature: TimeSignature::COMMON, tempo: DEFAULT_TEMPO, bars: vec![] };
    }

    // Total 64ths contributed by each bar's beats.
    let bar_64ths: Vec<u32> = batches.iter()
        .map(|b| b.iter().map(|item| item.estimate.in_64ths()).sum())
        .collect();
    let max_64ths = *bar_64ths.iter().max().unwrap_or(&NoteValue::Quarter.in_64ths());

    // Time sig: express bar size as N quarter notes.
    let q64 = NoteValue::Quarter.in_64ths();
    let beats_per_bar = (max_64ths + q64 - 1) / q64;
    let time_sig = TimeSignature { beats_per_bar, beat_unit: NoteValue::Quarter };

    let bars = batches.iter().zip(bar_64ths.iter()).map(|(batch, &used)| {
        let mut beats: Vec<Beat> = batch.iter()
            .map(|item| Beat::tag(item.estimate, item.id.clone()))
            .collect();
        let mut remaining = max_64ths - used;
        while remaining > 0 {
            let nv = largest_note_fitting(remaining);
            beats.push(Beat::tag(nv, "rest"));
            remaining -= nv.in_64ths();
        }
        Bar { time_signature: time_sig, beats }
    }).collect();

    Pattern { time_signature: time_sig, tempo: DEFAULT_TEMPO, bars }
}

fn largest_note_fitting(available: u32) -> NoteValue {
    for &nv in &[
        NoteValue::Whole, NoteValue::Half, NoteValue::Quarter,
        NoteValue::Eighth, NoteValue::Sixteenth, NoteValue::ThirtySecond,
        NoteValue::SixtyFourth,
    ] {
        if nv.in_64ths() <= available {
            return nv;
        }
    }
    NoteValue::SixtyFourth
}

/// Kahn's topological sort returning items grouped into parallel batches.
/// Items within a batch maintain their original order from the plan.
fn topo_batches(plan: &Plan) -> Vec<Vec<&PlanItem>> {
    let id_to_idx: HashMap<&str, usize> = plan.items.iter().enumerate()
        .map(|(i, item)| (item.id.as_str(), i))
        .collect();

    let n = plan.items.len();
    let mut in_degree = vec![0usize; n];
    let mut successors: Vec<Vec<usize>> = vec![vec![]; n];

    for (i, item) in plan.items.iter().enumerate() {
        for dep in &item.deps {
            if let Some(&j) = id_to_idx.get(dep.as_str()) {
                in_degree[i] += 1;
                successors[j].push(i);
            }
        }
    }

    let mut queue: VecDeque<usize> = (0..n).filter(|&i| in_degree[i] == 0).collect();
    let mut batches: Vec<Vec<&PlanItem>> = Vec::new();

    while !queue.is_empty() {
        let batch_size = queue.len();
        let mut batch = Vec::with_capacity(batch_size);
        for _ in 0..batch_size {
            let i = queue.pop_front().unwrap();
            batch.push(&plan.items[i]);
            for &j in &successors[i] {
                in_degree[j] -= 1;
                if in_degree[j] == 0 {
                    queue.push_back(j);
                }
            }
        }
        batches.push(batch);
    }

    batches
}

// ---------------------------------------------------------------------------
// Pattern → DSL
// ---------------------------------------------------------------------------

/// Render a `Pattern` as the text DSL that `parse()` can re-read.
pub fn pattern_to_dsl(pattern: &Pattern) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let ts = pattern.time_signature;
    let _ = writeln!(out, "time_signature {}/{}", ts.beats_per_bar, ts.beat_unit.denominator());
    let secs = pattern.tempo.quarter().as_secs();
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    if m == 0 {
        let _ = writeln!(out, "quarter_duration {}h", h);
    } else if h == 0 {
        let _ = writeln!(out, "quarter_duration {}m", m);
    } else {
        let _ = writeln!(out, "quarter_duration {}h{}m", h, m);
    }
    out.push('\n');
    for bar in &pattern.bars {
        out.push_str("| ");
        for beat in &bar.beats {
            out.push_str(&beat_to_dsl(beat));
            out.push_str(" | ");
        }
        out.push('\n');
    }
    out
}

fn beat_to_dsl(beat: &Beat) -> String {
    match &beat.content {
        BeatContent::Tag(tag) => format!("{}:{}", tag, beat.value.letter()),
        BeatContent::Subdivided(children) => {
            let inner: Vec<String> = children.iter().map(beat_to_dsl).collect();
            // The outer tag is not stored in the data model for subdivided beats;
            // use the first child's tag as a best-effort label.
            let outer_tag = match children.first().map(|b| &b.content) {
                Some(BeatContent::Tag(t)) => t.as_str(),
                _ => "beat",
            };
            format!("{}:{}[{}]", outer_tag, beat.value.letter(), inner.join(","))
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    const FIXTURE: &str = r#"# Simple Plan

### T1 — Foundation
Core work with no dependencies.

### T2 — Feature A
**Depends on:** T1.
**Estimate:** h

### T3 — Feature B
**Depends on:** T1.

### T4 — Integration
**Depends on:** T2, T3.
"#;

    #[test]
    fn parse_fixture_items() {
        let plan = parse_plan(FIXTURE).unwrap();
        assert_eq!(plan.items.len(), 4);

        assert_eq!(plan.items[0].id, "T1");
        assert!(plan.items[0].deps.is_empty());
        assert_eq!(plan.items[0].estimate, NoteValue::Quarter);

        assert_eq!(plan.items[1].id, "T2");
        assert_eq!(plan.items[1].deps, ["T1"]);
        assert_eq!(plan.items[1].estimate, NoteValue::Half);

        assert_eq!(plan.items[2].id, "T3");
        assert_eq!(plan.items[2].deps, ["T1"]);
        assert_eq!(plan.items[2].estimate, NoteValue::Quarter);

        assert_eq!(plan.items[3].id, "T4");
        assert_eq!(plan.items[3].deps, ["T2", "T3"]);
    }

    #[test]
    fn topo_batches_ordering() {
        let plan = parse_plan(FIXTURE).unwrap();
        let batches = topo_batches(&plan);
        assert_eq!(batches.len(), 3);
        assert_eq!(batches[0].iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["T1"]);
        let b1: HashSet<_> = batches[1].iter().map(|i| i.id.as_str()).collect();
        assert!(b1.contains("T2") && b1.contains("T3"));
        assert_eq!(batches[2].iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["T4"]);
    }

    #[test]
    fn plan_to_pattern_validates() {
        let plan = parse_plan(FIXTURE).unwrap();
        let pattern = plan_to_pattern(&plan);
        pattern.validate().unwrap();
    }

    #[test]
    fn snapshot_bar_structure() {
        let plan = parse_plan(FIXTURE).unwrap();
        let pattern = plan_to_pattern(&plan);
        // Batches: [T1], [T2(h)+T3(q)], [T4]
        // max_64ths = max(q=16, h+q=48, q=16) = 48 → beats_per_bar = 3
        assert_eq!(pattern.bars.len(), 3);
        assert_eq!(pattern.time_signature.beats_per_bar, 3);
        // Bar 0: T1(q) + rest(h) = 16+32=48 ✓
        assert_eq!(pattern.bars[0].beats.len(), 2);
        assert_eq!(pattern.bars[0].beats[0].value, NoteValue::Quarter);
        assert_eq!(pattern.bars[0].beats[1].value, NoteValue::Half);
        // Bar 1: T2(h) + T3(q) = 32+16=48 ✓, no padding needed
        assert_eq!(pattern.bars[1].beats.len(), 2);
        // Bar 2: T4(q) + rest(h) = 16+32=48 ✓
        assert_eq!(pattern.bars[2].beats.len(), 2);
    }

    #[test]
    fn round_trip_dsl() {
        let plan = parse_plan(FIXTURE).unwrap();
        let original = plan_to_pattern(&plan);
        let dsl = pattern_to_dsl(&original);
        let reparsed = parse(&dsl).unwrap();
        assert_eq!(reparsed.time_signature, original.time_signature);
        assert_eq!(reparsed.tempo, original.tempo);
        assert_eq!(reparsed.bars.len(), original.bars.len());
        for (ob, rb) in original.bars.iter().zip(reparsed.bars.iter()) {
            assert_eq!(ob.beats.len(), rb.beats.len());
            for (obx, rbx) in ob.beats.iter().zip(rb.beats.iter()) {
                assert_eq!(obx.value, rbx.value);
                assert_eq!(obx.content, rbx.content);
            }
        }
    }

    #[test]
    fn deps_with_parenthetical_notes() {
        let md = "### T1 — Base\n\n### T2 — Extended\n**Depends on:** T1 (uses core types).\n";
        let plan = parse_plan(md).unwrap();
        assert_eq!(plan.items[1].deps, ["T1"]);
    }

    #[test]
    fn hex_refactor_plan_parses_and_validates() {
        let src = include_str!("../../../docs/hex-refactor-plan.md");
        let plan = parse_plan(src).unwrap();
        assert!(plan.items.len() >= 11, "expected T1-T11, got {}", plan.items.len());

        let pattern = plan_to_pattern(&plan);
        pattern.validate().unwrap();

        // T3-T6 must be in the same bar (same topo batch).
        let batches = topo_batches(&plan);
        let t3_batch = batches.iter().position(|b| b.iter().any(|i| i.id == "T3")).unwrap();
        for tid in ["T4", "T5", "T6"] {
            let batch = batches.iter().position(|b| b.iter().any(|i| i.id == tid)).unwrap();
            assert_eq!(batch, t3_batch, "{tid} should share a bar with T3");
        }
        let t7_batch = batches.iter().position(|b| b.iter().any(|i| i.id == "T7")).unwrap();
        assert_eq!(t7_batch, t3_batch + 1, "T7 should follow T3-T6");

        // DSL round-trip
        let dsl = pattern_to_dsl(&pattern);
        let reparsed = crate::parser::parse(&dsl).unwrap();
        assert_eq!(reparsed.bars.len(), pattern.bars.len());
    }
}
