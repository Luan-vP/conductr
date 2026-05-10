use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::notation::{NoteValue, Tempo, TimeSignature};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Beat {
    pub value: NoteValue,
    pub content: BeatContent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BeatContent {
    /// Plain labelled beat: "sleep", "wake", "work", "rest", ...
    Tag(String),
    /// Beat split into smaller beats; durations must sum to the parent.
    Subdivided(Vec<Beat>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bar {
    pub time_signature: TimeSignature,
    pub beats: Vec<Beat>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pattern {
    pub time_signature: TimeSignature,
    pub tempo: Tempo,
    pub bars: Vec<Bar>,
}

#[derive(Debug, thiserror::Error)]
pub enum PatternError {
    #[error("bar {bar} length mismatch: expected {expected} 64ths, got {actual}")]
    BarLengthMismatch { bar: usize, expected: u32, actual: u32 },
    #[error(
        "subdivision of {parent:?} expected {expected} 64ths, children sum to {actual}"
    )]
    SubdivisionMismatch { parent: NoteValue, expected: u32, actual: u32 },
}

impl Beat {
    pub fn tag(value: NoteValue, tag: impl Into<String>) -> Self {
        Self { value, content: BeatContent::Tag(tag.into()) }
    }

    pub fn subdivided(value: NoteValue, children: Vec<Beat>) -> Self {
        Self { value, content: BeatContent::Subdivided(children) }
    }

    pub fn in_64ths(&self) -> u32 {
        self.value.in_64ths()
    }

    pub fn duration(&self, tempo: Tempo) -> Duration {
        tempo.note_duration(self.value)
    }

    pub fn validate(&self) -> Result<(), PatternError> {
        if let BeatContent::Subdivided(children) = &self.content {
            let sum: u32 = children.iter().map(|b| b.in_64ths()).sum();
            if sum != self.in_64ths() {
                return Err(PatternError::SubdivisionMismatch {
                    parent: self.value,
                    expected: self.in_64ths(),
                    actual: sum,
                });
            }
            for c in children {
                c.validate()?;
            }
        }
        Ok(())
    }

    /// Walk leaves in order (depth-first), yielding `(absolute_offset, leaf_value, tag)`.
    pub fn walk_leaves<'a>(&'a self, out: &mut Vec<LeafInterval<'a>>, offset_64ths: u32) {
        match &self.content {
            BeatContent::Tag(t) => out.push(LeafInterval {
                offset_64ths,
                length_64ths: self.in_64ths(),
                value: self.value,
                tag: t.as_str(),
            }),
            BeatContent::Subdivided(children) => {
                let mut o = offset_64ths;
                for c in children {
                    c.walk_leaves(out, o);
                    o += c.in_64ths();
                }
            }
        }
    }
}

/// A flattened leaf of a beat tree at a fixed time offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeafInterval<'a> {
    pub offset_64ths: u32,
    pub length_64ths: u32,
    pub value: NoteValue,
    pub tag: &'a str,
}

impl Bar {
    pub fn validate_at(&self, index: usize) -> Result<(), PatternError> {
        let sum: u32 = self.beats.iter().map(|b| b.in_64ths()).sum();
        let expected = self.time_signature.bar_in_64ths();
        if sum != expected {
            return Err(PatternError::BarLengthMismatch { bar: index, expected, actual: sum });
        }
        for b in &self.beats {
            b.validate()?;
        }
        Ok(())
    }
}

impl Pattern {
    pub fn validate(&self) -> Result<(), PatternError> {
        for (i, bar) in self.bars.iter().enumerate() {
            bar.validate_at(i)?;
        }
        Ok(())
    }

    pub fn total_duration(&self) -> Duration {
        self.bars
            .iter()
            .flat_map(|b| &b.beats)
            .map(|b| b.duration(self.tempo))
            .sum()
    }

    /// Flatten the pattern into a sequence of leaf intervals across all bars.
    pub fn leaves(&self) -> Vec<OwnedLeafInterval> {
        let mut out = Vec::new();
        let mut bar_offset = 0u32;
        for bar in &self.bars {
            let mut beat_offset = 0u32;
            for beat in &bar.beats {
                let mut tmp = Vec::new();
                beat.walk_leaves(&mut tmp, beat_offset);
                for li in tmp {
                    out.push(OwnedLeafInterval {
                        offset_64ths: bar_offset + li.offset_64ths,
                        length_64ths: li.length_64ths,
                        value: li.value,
                        tag: li.tag.to_string(),
                    });
                }
                beat_offset += beat.in_64ths();
            }
            bar_offset += bar.time_signature.bar_in_64ths();
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedLeafInterval {
    pub offset_64ths: u32,
    pub length_64ths: u32,
    pub value: NoteValue,
    pub tag: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quarter_subdivided_into_eight_thirty_seconds() {
        let beat = Beat::subdivided(
            NoteValue::Quarter,
            (0..8)
                .map(|i| Beat::tag(NoteValue::ThirtySecond, if i % 2 == 0 { "sleep" } else { "wake" }))
                .collect(),
        );
        beat.validate().unwrap();
    }

    #[test]
    fn subdivision_mismatch_is_caught() {
        let bad = Beat::subdivided(
            NoteValue::Quarter,
            vec![Beat::tag(NoteValue::ThirtySecond, "wake")],
        );
        assert!(matches!(bad.validate(), Err(PatternError::SubdivisionMismatch { .. })));
    }

    #[test]
    fn six_four_bar_validates() {
        let bar = Bar {
            time_signature: TimeSignature::SIX_FOUR,
            beats: (0..6).map(|_| Beat::tag(NoteValue::Quarter, "sleep")).collect(),
        };
        bar.validate_at(0).unwrap();
    }

    #[test]
    fn bar_length_mismatch_is_caught() {
        let bar = Bar {
            time_signature: TimeSignature::SIX_FOUR,
            beats: vec![Beat::tag(NoteValue::Quarter, "x")],
        };
        assert!(matches!(bar.validate_at(0), Err(PatternError::BarLengthMismatch { .. })));
    }

    #[test]
    fn leaves_flatten_with_offsets() {
        let pattern = Pattern {
            time_signature: TimeSignature::SIX_FOUR,
            tempo: Tempo::from_quarter(Duration::from_secs(4 * 3600)),
            bars: vec![Bar {
                time_signature: TimeSignature::SIX_FOUR,
                beats: vec![
                    Beat::tag(NoteValue::Quarter, "sleep"),
                    Beat::tag(NoteValue::Quarter, "sleep"),
                    Beat::subdivided(
                        NoteValue::Quarter,
                        (0..8)
                            .map(|i| Beat::tag(NoteValue::ThirtySecond, if i % 2 == 0 { "wake" } else { "sleep" }))
                            .collect(),
                    ),
                    Beat::tag(NoteValue::Quarter, "wake"),
                    Beat::tag(NoteValue::Quarter, "work"),
                    Beat::tag(NoteValue::Quarter, "rest"),
                ],
            }],
        };
        pattern.validate().unwrap();
        let leaves = pattern.leaves();
        // 2 quarters + 8 32nds + 3 quarters = 13 leaves
        assert_eq!(leaves.len(), 13);
        // Total length = bar of 6/4 = 6 quarters = 96 64ths
        assert_eq!(
            leaves.iter().map(|l| l.length_64ths).sum::<u32>(),
            TimeSignature::SIX_FOUR.bar_in_64ths()
        );
    }
}
