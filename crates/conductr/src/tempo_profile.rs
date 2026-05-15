//! Pure computation of per-bucket tempo averages and single-number maturity.

use chrono::{DateTime, Utc};

use crate::config::{PrComplexity, TempoPr};

const WINDOW_DAYS: f64 = 30.0;
const HALF_LIFE_DAYS: f64 = 15.0;

pub struct BucketStats {
    /// Time-decayed weighted average beat duration in seconds.
    pub avg_secs: Option<f64>,
    /// Number of merged PRs in the 30-day window for this bucket.
    pub count: u32,
}

pub struct Profile {
    pub xs: BucketStats,
    pub s: BucketStats,
    pub m: BucketStats,
    pub l: BucketStats,
    /// Weighted median across bucket averages (weight = PR count). None if no data.
    pub maturity_secs: Option<f64>,
}

/// Compute the bucketed tempo profile from merged PRs in the last 30 days.
///
/// Uses exponential time decay with a 15-day half-life so recent PRs weigh
/// more than older ones within the window.
pub fn tempo_profile(prs: &[TempoPr], now: DateTime<Utc>) -> Profile {
    let mut xs_acc = Accumulator::default();
    let mut s_acc = Accumulator::default();
    let mut m_acc = Accumulator::default();
    let mut l_acc = Accumulator::default();

    for pr in prs {
        if !pr.merged {
            continue;
        }
        let closed = match pr.closed {
            Some(c) => c,
            None => continue,
        };
        let age_secs = (now - closed).num_seconds();
        if age_secs < 0 {
            continue;
        }
        let age_days = age_secs as f64 / 86400.0;
        if age_days > WINDOW_DAYS {
            continue;
        }

        let duration_secs = (closed - pr.opened).num_seconds();
        if duration_secs <= 0 {
            continue;
        }

        let weight = (-std::f64::consts::LN_2 * age_days / HALF_LIFE_DAYS).exp();

        match pr.complexity {
            PrComplexity::Xs => xs_acc.add(weight, duration_secs as f64),
            PrComplexity::S => s_acc.add(weight, duration_secs as f64),
            PrComplexity::M => m_acc.add(weight, duration_secs as f64),
            PrComplexity::L => l_acc.add(weight, duration_secs as f64),
        }
    }

    let xs = xs_acc.to_stats();
    let s = s_acc.to_stats();
    let m = m_acc.to_stats();
    let l = l_acc.to_stats();

    let maturity_secs = weighted_median(&xs, &s, &m, &l);

    Profile { xs, s, m, l, maturity_secs }
}

#[derive(Default)]
struct Accumulator {
    weight_sum: f64,
    weighted_duration: f64,
    count: u32,
}

impl Accumulator {
    fn add(&mut self, weight: f64, duration_secs: f64) {
        self.weight_sum += weight;
        self.weighted_duration += weight * duration_secs;
        self.count += 1;
    }

    fn to_stats(&self) -> BucketStats {
        let avg_secs = if self.weight_sum > 0.0 {
            Some(self.weighted_duration / self.weight_sum)
        } else {
            None
        };
        BucketStats { avg_secs, count: self.count }
    }
}

/// Weighted median of bucket averages, weighted by PR count.
///
/// Buckets are sorted by their average; cumulative count is accumulated until
/// it reaches half the total — the bucket that crosses the midpoint is the
/// median bucket.
fn weighted_median(
    xs: &BucketStats,
    s: &BucketStats,
    m: &BucketStats,
    l: &BucketStats,
) -> Option<f64> {
    let mut buckets: Vec<(f64, u32)> = [xs, s, m, l]
        .iter()
        .filter_map(|b| b.avg_secs.map(|a| (a, b.count)))
        .collect();

    if buckets.is_empty() {
        return None;
    }

    buckets.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    let total: u32 = buckets.iter().map(|(_, c)| c).sum();
    if total == 0 {
        return None;
    }

    let half = total as f64 / 2.0;
    let mut cumulative = 0u32;
    for (avg, count) in &buckets {
        cumulative += count;
        if cumulative as f64 >= half {
            return Some(*avg);
        }
    }

    buckets.last().map(|(a, _)| *a)
}

// ── Formatting ────────────────────────────────────────────────────────────────

/// Format a duration in seconds as a concise human-readable string.
///
/// - < 60s  → "Xs"
/// - < 1h   → "Xm" or "X.Xm"
/// - ≥ 1h   → "Xh" or "X.Xh"
pub fn format_duration(secs: f64) -> String {
    if secs < 60.0 {
        format!("{:.0}s", secs)
    } else if secs < 3600.0 {
        fmt_with_one_decimal(secs / 60.0, 'm')
    } else {
        fmt_with_one_decimal(secs / 3600.0, 'h')
    }
}

fn fmt_with_one_decimal(value: f64, unit: char) -> String {
    let rounded = (value * 10.0).round() / 10.0;
    if (rounded - rounded.floor()).abs() < 1e-9 {
        format!("{:.0}{unit}", rounded)
    } else {
        format!("{:.1}{unit}", rounded)
    }
}

fn fmt_bucket(stat: &BucketStats) -> String {
    match stat.avg_secs {
        Some(s) => format_duration(s),
        None => "n/a".to_string(),
    }
}

/// One-line summary for `cadence show`, printed under the staff.
///
/// Returns `None` when no merged PRs exist in the 30-day window.
pub fn format_show_line(profile: &Profile) -> Option<String> {
    if profile.xs.count == 0
        && profile.s.count == 0
        && profile.m.count == 0
        && profile.l.count == 0
    {
        return None;
    }

    let median_str = profile
        .maturity_secs
        .map(|s| format!("  (median: {})", format_duration(s)))
        .unwrap_or_default();

    Some(format!(
        "tempo: XS={}  S={}  M={}  L={}{}",
        fmt_bucket(&profile.xs),
        fmt_bucket(&profile.s),
        fmt_bucket(&profile.m),
        fmt_bucket(&profile.l),
        median_str,
    ))
}

/// Multi-line section for `cadence status`, showing per-bucket averages and counts.
pub fn format_status_lines(profile: &Profile) -> String {
    let mut lines = Vec::new();
    lines.push(String::new());
    lines.push("tempo profile (30-day time-decayed):".to_string());

    let fmt_row = |label: &str, stat: &BucketStats| -> String {
        let avg_str = match stat.avg_secs {
            Some(s) => format_duration(s),
            None => "n/a".to_string(),
        };
        format!("  {:>2}: {:<8}  (n={})", label, avg_str, stat.count)
    };

    lines.push(fmt_row("XS", &profile.xs));
    lines.push(fmt_row("S", &profile.s));
    lines.push(fmt_row("M", &profile.m));
    lines.push(fmt_row("L", &profile.l));

    if let Some(m) = profile.maturity_secs {
        lines.push(format!("  maturity: {}", format_duration(m)));
    }

    lines.join("\n")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 15, 12, 0, 0).unwrap()
    }

    /// Build a synthetic TempoPr.
    ///
    /// `closed_ago_days`: how many days before `now` the PR was closed.
    /// `duration_days`: how long the PR was open.
    fn make_pr(
        number: u64,
        complexity: PrComplexity,
        closed_ago_days: f64,
        duration_days: f64,
        merged: bool,
    ) -> TempoPr {
        let n = now();
        let closed = n - chrono::Duration::seconds((closed_ago_days * 86400.0) as i64);
        let opened = closed - chrono::Duration::seconds((duration_days * 86400.0) as i64);
        TempoPr {
            number,
            phrase: None,
            chord: None,
            complexity,
            opened,
            closed: if merged { Some(closed) } else { None },
            merged,
        }
    }

    #[test]
    fn empty_prs_all_none() {
        let profile = tempo_profile(&[], now());
        assert!(profile.xs.avg_secs.is_none());
        assert!(profile.s.avg_secs.is_none());
        assert!(profile.m.avg_secs.is_none());
        assert!(profile.l.avg_secs.is_none());
        assert!(profile.maturity_secs.is_none());
        assert!(format_show_line(&profile).is_none());
    }

    #[test]
    fn single_xs_pr_computes_average() {
        let pr = make_pr(1, PrComplexity::Xs, 1.0, 0.5, true);
        let profile = tempo_profile(&[pr], now());
        let avg = profile.xs.avg_secs.expect("xs avg should be Some");
        // duration = 0.5 days = 43200s; weight factor is close to 1 near day 1
        assert!((avg - 43200.0).abs() < 1.0, "expected ~43200s, got {avg}");
        assert_eq!(profile.xs.count, 1);
        assert_eq!(profile.maturity_secs, profile.xs.avg_secs);
    }

    #[test]
    fn unmerged_pr_excluded() {
        let pr = make_pr(1, PrComplexity::Xs, 1.0, 0.5, false);
        let profile = tempo_profile(&[pr], now());
        assert!(profile.xs.avg_secs.is_none());
    }

    #[test]
    fn pr_outside_30_day_window_excluded() {
        let pr = make_pr(1, PrComplexity::Xs, 31.0, 0.5, true);
        let profile = tempo_profile(&[pr], now());
        assert!(profile.xs.avg_secs.is_none());
        assert_eq!(profile.xs.count, 0);
    }

    #[test]
    fn xs_dominated_repo_gives_small_maturity() {
        let mut prs = Vec::new();
        // 10 XS PRs ~1h each, all closed 1 day ago
        for i in 0..10u64 {
            prs.push(make_pr(i, PrComplexity::Xs, 1.0, 1.0 / 24.0, true));
        }
        // 1 L PR ~24h, closed 1 day ago
        prs.push(make_pr(100, PrComplexity::L, 1.0, 1.0, true));

        let profile = tempo_profile(&prs, now());
        let maturity = profile.maturity_secs.unwrap();
        // Median weight: XS has count=10, L has count=1, total=11, half=5.5
        // Sorted: XS(~3600, 10) → cumulative 10 ≥ 5.5 → median = XS avg ≈ 3600s
        assert!(maturity < 7200.0, "expected <2h, got {maturity}s");
    }

    #[test]
    fn l_dominated_repo_gives_large_maturity() {
        let mut prs = Vec::new();
        prs.push(make_pr(1, PrComplexity::M, 1.0, 0.5, true)); // ~12h
        // 10 L PRs ~48h each
        for i in 0..10u64 {
            prs.push(make_pr(i + 10, PrComplexity::L, 1.0, 2.0, true));
        }

        let profile = tempo_profile(&prs, now());
        let maturity = profile.maturity_secs.unwrap();
        // L avg ≈ 172800s (48h); M avg ≈ 43200s (12h)
        // Sorted: M(43200, 1), L(172800, 10); total=11, half=5.5
        // M cumulative=1 < 5.5; L cumulative=11 ≥ 5.5 → median = L avg
        assert!(maturity > 100_000.0, "expected >27h, got {maturity}s");
    }

    #[test]
    fn recent_prs_weighted_higher_than_old() {
        // Two XS PRs with same duration but one is recent and one is near the boundary.
        let recent = make_pr(1, PrComplexity::Xs, 1.0, 1.0, true); // 1 day ago → weight ≈ 0.955
        let old = make_pr(2, PrComplexity::Xs, 29.0, 5.0, true); // 29 days ago → weight ≈ 0.129

        // recent PR: 1 day duration; old PR: 5 day duration
        // weighted avg = (0.955*86400 + 0.129*432000) / (0.955+0.129)
        //              = (82512 + 55728) / 1.084 ≈ 127485s
        // Without weighting avg would be (86400+432000)/2 = 259200s
        let profile = tempo_profile(&[recent, old], now());
        let avg = profile.xs.avg_secs.unwrap();
        assert!(avg < 200_000.0, "recent PR should outweigh old one; avg={avg}");
        assert!(avg > 50_000.0, "old PR should still contribute; avg={avg}");
    }

    #[test]
    fn format_duration_examples() {
        assert_eq!(format_duration(30.0), "30s");
        assert_eq!(format_duration(720.0), "12m");
        assert_eq!(format_duration(4320.0), "1.2h");
        assert_eq!(format_duration(64800.0), "18h");
        assert_eq!(format_duration(7560.0), "2.1h");
    }

    #[test]
    fn format_show_line_includes_all_buckets() {
        let pr = make_pr(1, PrComplexity::M, 1.0, 0.5, true);
        let profile = tempo_profile(&[pr], now());
        let line = format_show_line(&profile).expect("should return Some");
        assert!(line.starts_with("tempo:"), "got: {line}");
        assert!(line.contains("XS="), "got: {line}");
        assert!(line.contains("M="), "got: {line}");
        assert!(line.contains("median"), "got: {line}");
    }

    #[test]
    fn format_status_lines_shows_counts() {
        let pr = make_pr(1, PrComplexity::S, 2.0, 0.25, true);
        let profile = tempo_profile(&[pr], now());
        let status = format_status_lines(&profile);
        assert!(status.contains("tempo profile"), "got: {status}");
        assert!(status.contains("n=1"), "got: {status}");
        assert!(status.contains("maturity"), "got: {status}");
    }
}
