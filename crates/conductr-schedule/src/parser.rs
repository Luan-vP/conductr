//! Tiny text DSL for patterns.
//!
//! Example:
//!
//! ```text
//! # conductor life scheduler — one day as 6 bars
//! bar_duration 4h
//!
//! | sleep:w |
//! | sleep:w |
//! | sleep:w[wake:e,sleep:e,wake:e,sleep:e,wake:e,sleep:e,wake:e,sleep:e] |
//! | wake:w |
//! | work:w |
//! | rest:w |
//! ```
//!
//! Grammar (informal):
//!
//! ```text
//! file        := (line NEWLINE)*
//! line        := comment | config | bar | empty
//! comment     := '#' ANY*
//! config      := key SP value
//! key         := 'bar_duration'
//! bar         := '|' (beat '|')+
//! beat        := tag ':' note_value ( '[' beat (',' beat)* ']' )?
//! tag         := [A-Za-z_][A-Za-z0-9_-]*
//! note_value  := 'w' | 'h' | 'q' | 'e' | 's' | 't' | 'x'
//! ```

use std::time::Duration;

use crate::notation::{NoteValue, Tempo, TimeSignature};
use crate::pattern::{Bar, Beat, BeatContent, Pattern, PatternError};

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("line {line}: {msg}")]
    Syntax { line: usize, msg: String },
    #[error("line {line}: unknown directive `{key}`")]
    UnknownDirective { line: usize, key: String },
    #[error("missing required directive: {0}")]
    MissingDirective(&'static str),
    #[error("validation: {0}")]
    Validation(#[from] crate::pattern::PatternError),
}

/// Default bar duration: 4 hours.
const DEFAULT_BAR_SECS: u64 = 4 * 3600;

pub fn parse(src: &str) -> Result<Pattern, ParseError> {
    let mut bar_duration_secs: u64 = DEFAULT_BAR_SECS;
    let mut all_beats: Vec<Vec<Beat>> = Vec::new();

    for (i, raw) in src.lines().enumerate() {
        let line_no = i + 1;
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }

        if line.starts_with('|') {
            let beats = parse_bar_beats(line, line_no)?;
            all_beats.push(beats);
            continue;
        }

        let (key, rest) = line.split_once(char::is_whitespace).ok_or_else(|| ParseError::Syntax {
            line: line_no,
            msg: "expected `key value`".into(),
        })?;
        match key {
            "bar_duration" => {
                bar_duration_secs = parse_duration(rest.trim(), line_no)?.as_secs()
            }
            other => {
                return Err(ParseError::UnknownDirective { line: line_no, key: other.into() })
            }
        }
    }

    // Infer time signature from the first bar's beat structure.
    let first_in_64ths: u32 = all_beats
        .first()
        .map(|beats| beats.iter().map(|b| b.in_64ths()).sum())
        .unwrap_or_else(|| NoteValue::Quarter.in_64ths() * 4); // default 4/4

    // All bars must have the same total 64th count.
    for (j, beats) in all_beats.iter().enumerate() {
        let sum: u32 = beats.iter().map(|b| b.in_64ths()).sum();
        if sum != first_in_64ths {
            return Err(ParseError::Validation(PatternError::BarLengthMismatch {
                bar: j,
                expected: first_in_64ths,
                actual: sum,
            }));
        }
    }

    // Express bar length as N quarter notes (N/4 time signature).
    let beats_per_bar = first_in_64ths / NoteValue::Quarter.in_64ths();
    let time_signature = TimeSignature { beats_per_bar, beat_unit: NoteValue::Quarter };
    let tempo = Tempo::from_bar_secs(bar_duration_secs, first_in_64ths);

    let bars = all_beats
        .into_iter()
        .map(|beats| Bar { time_signature, beats })
        .collect();

    let pattern = Pattern { time_signature, tempo, bars };
    pattern.validate()?;
    Ok(pattern)
}

fn strip_comment(s: &str) -> &str {
    s.split_once('#').map(|(a, _)| a).unwrap_or(s)
}

/// Parse the beats inside a bar line; subdivision validation is done here.
fn parse_bar_beats(line: &str, line_no: usize) -> Result<Vec<Beat>, ParseError> {
    let inner = line.trim_start_matches('|').trim_end_matches('|');
    let beat_sources = split_top_level(inner, '|');
    let mut beats = Vec::with_capacity(beat_sources.len());
    for src in beat_sources {
        let s = src.trim();
        if s.is_empty() {
            continue;
        }
        beats.push(parse_beat(s, line_no)?);
    }
    // Validate subdivision consistency (not bar-level length — deferred to parse()).
    for beat in &beats {
        beat.validate().map_err(ParseError::Validation)?;
    }
    Ok(beats)
}

/// Parse a duration like `4h`, `30m`, `90s`, `1h30m`.
fn parse_duration(s: &str, line: usize) -> Result<Duration, ParseError> {
    let mut total: u64 = 0;
    let mut digits = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
            continue;
        }
        if digits.is_empty() {
            return Err(ParseError::Syntax {
                line,
                msg: format!("expected digits before `{c}` in `{s}`"),
            });
        }
        let n: u64 = digits.parse().unwrap();
        digits.clear();
        let mult = match c {
            's' => 1,
            'm' => 60,
            'h' => 3600,
            'd' => 86400,
            other => {
                return Err(ParseError::Syntax {
                    line,
                    msg: format!("unknown duration unit `{other}`"),
                })
            }
        };
        total += n * mult;
    }
    if !digits.is_empty() {
        return Err(ParseError::Syntax {
            line,
            msg: format!("trailing digits `{digits}` without a unit in `{s}`"),
        });
    }
    Ok(Duration::from_secs(total))
}

fn parse_beat(src: &str, line: usize) -> Result<Beat, ParseError> {
    let (tag, rest) = src.split_once(':').ok_or_else(|| ParseError::Syntax {
        line,
        msg: format!("beat `{src}` missing `:value`"),
    })?;
    let tag = tag.trim();
    if tag.is_empty()
        || !tag.chars().next().map(|c| c.is_ascii_alphabetic() || c == '_').unwrap_or(false)
    {
        return Err(ParseError::Syntax { line, msg: format!("invalid tag `{tag}`") });
    }
    let rest = rest.trim();
    let (value_letter, subdiv) = match rest.find('[') {
        Some(i) => {
            if !rest.ends_with(']') {
                return Err(ParseError::Syntax {
                    line,
                    msg: format!("unmatched `[` in beat `{src}`"),
                });
            }
            let value = &rest[..i];
            let children = &rest[i + 1..rest.len() - 1];
            (value.trim(), Some(children))
        }
        None => (rest, None),
    };
    if value_letter.len() != 1 {
        return Err(ParseError::Syntax {
            line,
            msg: format!("expected single-letter note value, got `{value_letter}`"),
        });
    }
    let value = NoteValue::from_letter(value_letter.chars().next().unwrap()).ok_or_else(|| {
        ParseError::Syntax { line, msg: format!("unknown note value `{value_letter}`") }
    })?;

    let content = match subdiv {
        None => BeatContent::Tag(tag.to_string()),
        Some(children_src) => {
            let children = split_top_level(children_src, ',')
                .into_iter()
                .map(|s| parse_beat(s.trim(), line))
                .collect::<Result<Vec<_>, _>>()?;
            BeatContent::Subdivided(children)
        }
    };
    Ok(Beat { value, content })
}

/// Split on a separator that appears at bracket-depth 0.
fn split_top_level(src: &str, sep: char) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth: i32 = 0;
    let mut start = 0;
    for (i, c) in src.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => depth -= 1,
            x if x == sep && depth == 0 => {
                out.push(&src[start..i]);
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    out.push(&src[start..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONDUCTOR_LIFE_DAY: &str = r#"
        # one day as 6 bars
        bar_duration 4h

        | sleep:w |
        | sleep:w |
        | sleep:w[wake:e,sleep:e,wake:e,sleep:e,wake:e,sleep:e,wake:e,sleep:e] |
        | wake:w |
        | work:w |
        | rest:w |
    "#;

    #[test]
    fn parses_canonical_day() {
        let p = parse(CONDUCTOR_LIFE_DAY).unwrap();
        // 6 bars × 4 h = 24 h.
        assert_eq!(p.bars.len(), 6);
        assert_eq!(p.total_duration(), Duration::from_secs(24 * 3600));
        // bar_duration 4h → whole note = 4h, eighth = 30 min.
        assert_eq!(p.tempo.note_duration(NoteValue::Whole), Duration::from_secs(4 * 3600));
        assert_eq!(p.tempo.note_duration(NoteValue::Eighth), Duration::from_secs(30 * 60));
        // 5 whole-note beats + 8 eighth-note subdivisions = 13 leaves.
        let leaves = p.leaves();
        assert_eq!(leaves.len(), 13);
    }

    #[test]
    fn rejects_inconsistent_bars() {
        // First bar is a whole note (64 64ths), second is a quarter (16 64ths).
        let src = "bar_duration 4h\n| x:w |\n| x:q |";
        assert!(parse(src).is_err());
    }

    #[test]
    fn parses_compound_duration() {
        // bar_duration 6h with a 4-quarter bar → quarter = 6h/4 = 1h30m.
        let src = "bar_duration 6h\n| x:q | x:q | x:q | x:q |";
        let p = parse(src).unwrap();
        assert_eq!(p.tempo.quarter(), Duration::from_secs(5400));
    }

    #[test]
    fn default_bar_duration_is_4h() {
        // Omitting bar_duration should default to 4h.
        let src = "| x:w |";
        let p = parse(src).unwrap();
        assert_eq!(p.tempo.note_duration(NoteValue::Whole), Duration::from_secs(4 * 3600));
    }
}
