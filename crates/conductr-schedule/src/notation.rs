use serde::{Deserialize, Serialize};
use std::time::Duration;

/// A note value, in standard Western musical notation.
///
/// Each variant represents a fraction of a whole note. We keep the durations
/// abstract; concrete wall-clock duration is derived via [`Tempo`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NoteValue {
    Whole,
    Half,
    Quarter,
    Eighth,
    /// Semiquaver.
    Sixteenth,
    /// Demisemiquaver — used for "wake textures" inside a sleep block.
    ThirtySecond,
    /// Hemidemisemiquaver.
    SixtyFourth,
}

impl NoteValue {
    /// Reciprocal denominator. Whole = 1, Quarter = 4, ThirtySecond = 32.
    pub const fn denominator(self) -> u32 {
        match self {
            Self::Whole => 1,
            Self::Half => 2,
            Self::Quarter => 4,
            Self::Eighth => 8,
            Self::Sixteenth => 16,
            Self::ThirtySecond => 32,
            Self::SixtyFourth => 64,
        }
    }

    /// Length of this value expressed in 64th notes (the smallest unit we model).
    pub const fn in_64ths(self) -> u32 {
        64 / self.denominator()
    }

    /// Single-letter shorthand: `w h q e s t x`.
    pub fn from_letter(c: char) -> Option<Self> {
        Some(match c.to_ascii_lowercase() {
            'w' => Self::Whole,
            'h' => Self::Half,
            'q' => Self::Quarter,
            'e' => Self::Eighth,
            's' => Self::Sixteenth,
            't' => Self::ThirtySecond,
            'x' => Self::SixtyFourth,
            _ => return None,
        })
    }

    pub const fn letter(self) -> char {
        match self {
            Self::Whole => 'w',
            Self::Half => 'h',
            Self::Quarter => 'q',
            Self::Eighth => 'e',
            Self::Sixteenth => 's',
            Self::ThirtySecond => 't',
            Self::SixtyFourth => 'x',
        }
    }
}

/// Time signature: beats per bar over the note value that gets one beat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeSignature {
    pub beats_per_bar: u32,
    pub beat_unit: NoteValue,
}

impl TimeSignature {
    pub const COMMON: Self = Self { beats_per_bar: 4, beat_unit: NoteValue::Quarter };
    pub const SIX_FOUR: Self = Self { beats_per_bar: 6, beat_unit: NoteValue::Quarter };

    /// Length of one bar in 64th notes.
    pub const fn bar_in_64ths(self) -> u32 {
        self.beats_per_bar * self.beat_unit.in_64ths()
    }
}

/// Tempo expressed by anchoring the duration of one quarter note.
///
/// For the conductor life scheduler the canonical mapping is
/// `Tempo::from_quarter(Duration::from_secs(4 * 3600))` — one quarter = 4 h,
/// so a 6/4 bar = 24 h and one 32nd-note ≈ 30 min.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tempo {
    /// Duration of a quarter note in seconds.
    pub quarter_seconds: u64,
}

impl Tempo {
    pub fn from_quarter(d: Duration) -> Self {
        Self { quarter_seconds: d.as_secs() }
    }

    pub fn quarter(self) -> Duration {
        Duration::from_secs(self.quarter_seconds)
    }

    /// Duration of a single note of the given value at this tempo.
    pub fn note_duration(self, v: NoteValue) -> Duration {
        // 64ths per quarter = 16. Use float division for sub-second precision,
        // then round to whole seconds.
        let qs = self.quarter_seconds as f64;
        let secs_per_64th = qs / NoteValue::Quarter.in_64ths() as f64;
        let secs = secs_per_64th * v.in_64ths() as f64;
        Duration::from_secs_f64(secs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_value_in_64ths() {
        assert_eq!(NoteValue::Whole.in_64ths(), 64);
        assert_eq!(NoteValue::Quarter.in_64ths(), 16);
        assert_eq!(NoteValue::ThirtySecond.in_64ths(), 2);
        assert_eq!(NoteValue::SixtyFourth.in_64ths(), 1);
    }

    #[test]
    fn six_four_bar_is_six_quarters() {
        assert_eq!(TimeSignature::SIX_FOUR.bar_in_64ths(), 6 * 16);
    }

    #[test]
    fn conductor_life_scheduler_mapping() {
        // 1 quarter = 4 hours; full 6/4 bar = 24 h; 1 demisemiquaver = 30 min.
        let tempo = Tempo::from_quarter(Duration::from_secs(4 * 3600));
        assert_eq!(tempo.note_duration(NoteValue::Quarter), Duration::from_secs(4 * 3600));
        assert_eq!(tempo.note_duration(NoteValue::ThirtySecond), Duration::from_secs(30 * 60));
        let bar_seconds: u64 = (0..6).map(|_| tempo.note_duration(NoteValue::Quarter).as_secs()).sum();
        assert_eq!(bar_seconds, 24 * 3600);
    }

    #[test]
    fn letter_roundtrip() {
        for v in [
            NoteValue::Whole, NoteValue::Half, NoteValue::Quarter, NoteValue::Eighth,
            NoteValue::Sixteenth, NoteValue::ThirtySecond, NoteValue::SixtyFourth,
        ] {
            assert_eq!(NoteValue::from_letter(v.letter()), Some(v));
        }
    }
}
