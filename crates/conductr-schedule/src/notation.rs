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

/// Tempo anchored to the wall-clock duration of one bar.
///
/// Internally stored as `quarter_seconds` (duration of a quarter note).
/// Use [`from_bar_secs`](Tempo::from_bar_secs) when constructing from a
/// `bar_duration` directive: a bar = one whole note, so a quarter = bar/4.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tempo {
    /// Duration of a quarter note in seconds.
    pub quarter_seconds: u64,
}

impl Tempo {
    /// Construct from the duration of a single quarter note.
    pub fn from_quarter(d: Duration) -> Self {
        Self { quarter_seconds: d.as_secs() }
    }

    /// Construct from a bar's wall-clock duration and its length in 64th notes.
    ///
    /// `bar_in_64ths` is the sum of all beat 64ths in one bar.  For a
    /// whole-note bar (`bar_duration 4h`), `bar_in_64ths = 64` and each
    /// eighth note resolves to 30 min.
    pub fn from_bar_secs(bar_secs: u64, bar_in_64ths: u32) -> Self {
        let q64 = NoteValue::Quarter.in_64ths() as u64;
        Self { quarter_seconds: bar_secs * q64 / bar_in_64ths as u64 }
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
        // bar_duration 4h; whole note = 1 bar = 4h; eighth note = 30 min.
        let tempo = Tempo::from_bar_secs(4 * 3600, NoteValue::Whole.in_64ths());
        assert_eq!(tempo.note_duration(NoteValue::Whole), Duration::from_secs(4 * 3600));
        assert_eq!(tempo.note_duration(NoteValue::Eighth), Duration::from_secs(30 * 60));
        // 6 bars × 4 h = 24 h day.
        assert_eq!(6 * 4 * 3600u64, 24 * 3600);
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
