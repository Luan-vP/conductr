//! Tiny text DSL for patterns.
//!
//! Example:
//!
//! ```text
//! # conductor life scheduler — one day in 6/4
//! time_signature 6/4
//! quarter_duration 4h
//!
//! | sleep:q | sleep:q | sleep:q[wake:t,sleep:t,wake:t,sleep:t,wake:t,sleep:t,wake:t,sleep:t] | wake:q | work:q | rest:q |
//! ```
//!
//! Grammar (informal):
//!
//! ```text
//! file        := (line NEWLINE)*
//! line        := comment | config | bar | empty
//! comment     := '#' ANY*
//! config      := key SP value
//! key         := 'time_signature' | 'quarter_duration'
//! bar         := '|' (beat '|')+
//! beat        := tag ':' note_value ( '[' beat (',' beat)* ']' )?
//! tag         := [A-Za-z_][A-Za-z0-9_-]*
//! note_value  := 'w' | 'h' | 'q' | 'e' | 's' | 't' | 'x'
//! ```

use std::time::Duration;

use crate::notation::{NoteValue, Tempo, TimeSignature};
use crate::pattern::{Bar, Beat, BeatContent, Pattern};

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

pub fn parse(src: &str) -> Result<Pattern, ParseError> {
    let mut time_signature: Option<TimeSignature> = None;
    let mut tempo: Option<Tempo> = None;
    let mut bars: Vec<Bar> = Vec::new();

    for (i, raw) in src.lines().enumerate() {
        let line_no = i + 1;
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }

        if line.starts_with('|') {
            let ts = time_signature.ok_or(ParseError::MissingDirective("time_signature"))?;
            let bar = parse_bar(line, line_no, ts)?;
            bars.push(bar);
            continue;
        }

        let (key, rest) = line.split_once(char::is_whitespace).ok_or_else(|| ParseError::Syntax {
            line: line_no,
            msg: "expected `key value`".into(),
        })?;
        match key {
            "time_signature" => time_signature = Some(parse_time_sig(rest.trim(), line_no)?),
            "quarter_duration" => tempo = Some(Tempo::from_quarter(parse_duration(rest.trim(), line_no)?)),
            other => {
                return Err(ParseError::UnknownDirective { line: line_no, key: other.into() })
            }
        }
    }

    let time_signature = time_signature.ok_or(ParseError::MissingDirective("time_signature"))?;
    let tempo = tempo.ok_or(ParseError::MissingDirective("quarter_duration"))?;

    let pattern = Pattern { time_signature, tempo, bars };
    pattern.validate()?;
    Ok(pattern)
}

fn strip_comment(s: &str) -> &str {
    s.split_once('#').map(|(a, _)| a).unwrap_or(s)
}

fn parse_time_sig(s: &str, line: usize) -> Result<TimeSignature, ParseError> {
    let (top, bottom) = s.split_once('/').ok_or_else(|| ParseError::Syntax {
        line,
        msg: format!("invalid time signature `{s}` (expected N/D)"),
    })?;
    let beats_per_bar: u32 = top.trim().parse().map_err(|_| ParseError::Syntax {
        line,
        msg: format!("non-numeric beats `{top}`"),
    })?;
    let denom: u32 = bottom.trim().parse().map_err(|_| ParseError::Syntax {
        line,
        msg: format!("non-numeric denominator `{bottom}`"),
    })?;
    let beat_unit = match denom {
        1 => NoteValue::Whole,
        2 => NoteValue::Half,
        4 => NoteValue::Quarter,
        8 => NoteValue::Eighth,
        16 => NoteValue::Sixteenth,
        32 => NoteValue::ThirtySecond,
        64 => NoteValue::SixtyFourth,
        _ => {
            return Err(ParseError::Syntax {
                line,
                msg: format!("unsupported denominator {denom}"),
            })
        }
    };
    Ok(TimeSignature { beats_per_bar, beat_unit })
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

fn parse_bar(line: &str, line_no: usize, ts: TimeSignature) -> Result<Bar, ParseError> {
    // Strip leading and trailing `|`, then split on top-level `|`.
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
    let bar = Bar { time_signature: ts, beats };
    // Bar-level validation surfaces with the actual line index (best-effort).
    bar.validate_at(0).map_err(ParseError::Validation)?;
    Ok(bar)
}

fn parse_beat(src: &str, line: usize) -> Result<Beat, ParseError> {
    let (tag, rest) = src.split_once(':').ok_or_else(|| ParseError::Syntax {
        line,
        msg: format!("beat `{src}` missing `:value`"),
    })?;
    let tag = tag.trim();
    if tag.is_empty() || !tag.chars().next().map(|c| c.is_ascii_alphabetic() || c == '_').unwrap_or(false) {
        return Err(ParseError::Syntax { line, msg: format!("invalid tag `{tag}`") });
    }
    let rest = rest.trim();
    // Either `x` or `x[children]`.
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
        # one day in 6/4
        time_signature 6/4
        quarter_duration 4h

        | sleep:q | sleep:q | sleep:q[wake:t,sleep:t,wake:t,sleep:t,wake:t,sleep:t,wake:t,sleep:t] | wake:q | work:q | rest:q |
    "#;

    #[test]
    fn parses_canonical_day() {
        let p = parse(CONDUCTOR_LIFE_DAY).unwrap();
        assert_eq!(p.time_signature, TimeSignature::SIX_FOUR);
        assert_eq!(p.bars.len(), 1);
        assert_eq!(p.bars[0].beats.len(), 6);
        assert_eq!(p.total_duration(), Duration::from_secs(24 * 3600));
        let leaves = p.leaves();
        assert_eq!(leaves.len(), 13);
    }

    #[test]
    fn rejects_short_bar() {
        let src = "time_signature 4/4\nquarter_duration 1h\n| x:q | x:q | x:q |";
        assert!(parse(src).is_err());
    }

    #[test]
    fn parses_compound_duration() {
        let src = "time_signature 4/4\nquarter_duration 1h30m\n| x:q | x:q | x:q | x:q |";
        let p = parse(src).unwrap();
        assert_eq!(p.tempo.quarter(), Duration::from_secs(5400));
    }
}
