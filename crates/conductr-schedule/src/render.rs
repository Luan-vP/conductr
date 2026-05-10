use std::time::Duration;

use crate::pattern::Pattern;

/// Render a pattern as an ASCII timeline. One row per leaf interval.
///
/// Output columns: `start_offset | duration | note_value | tag`.
pub fn render_ascii(p: &Pattern) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Pattern: {}/{} time, q={}\n",
        p.time_signature.beats_per_bar,
        p.time_signature.beat_unit.denominator(),
        format_duration(p.tempo.quarter()),
    ));
    out.push_str("──────────────────────────────────────────────\n");
    let secs_per_64th = p.tempo.quarter().as_secs_f64() / 16.0;
    for leaf in p.leaves() {
        let start = Duration::from_secs_f64(leaf.offset_64ths as f64 * secs_per_64th);
        let dur = Duration::from_secs_f64(leaf.length_64ths as f64 * secs_per_64th);
        out.push_str(&format!(
            "{:>10}  +{:<10}  {}  {}\n",
            format_duration(start),
            format_duration(dur),
            leaf.value.letter(),
            leaf.tag,
        ));
    }
    out
}

fn format_duration(d: Duration) -> String {
    let total = d.as_secs();
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 && (m > 0 || s > 0) {
        format!("{h}h{m:02}m")
    } else if h > 0 {
        format!("{h}h")
    } else if m > 0 && s > 0 {
        format!("{m}m{s:02}s")
    } else if m > 0 {
        format!("{m}m")
    } else {
        format!("{s}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    #[test]
    fn render_does_not_panic() {
        let p = parse(
            "time_signature 6/4\nquarter_duration 4h\n| sleep:q | sleep:q | sleep:q | wake:q | work:q | rest:q |",
        )
        .unwrap();
        let s = render_ascii(&p);
        assert!(s.contains("Pattern:"));
        assert!(s.contains("sleep"));
    }
}
