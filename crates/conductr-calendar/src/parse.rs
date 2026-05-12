//! Title grammar parser for `[conductr:*]` calendar events.
//!
//! See `docs/calendar.md` for the full grammar specification.

use chrono::{DateTime, Utc};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventKind {
    /// `[conductr:window]`, `[conductr:window:<tag>]`, `[conductr:window:<t1>,<t2>]`
    Window { tags: Vec<String> },
    /// `[conductr:blocked]`
    Blocked,
    /// `[conductr:<tag>] decision: <subject>`
    Decision { tag: Option<String>, subject: String },
    /// `[conductr:<tag>] test: <subject>`
    Test { tag: Option<String>, subject: String },
    /// `[conductr:<tag>] review: <subject>`
    Review { tag: Option<String>, subject: String },
    /// Not a conductr-managed event or unrecognised format.
    Unknown,
}

impl EventKind {
    pub fn is_scheduled_item(&self) -> bool {
        matches!(self, EventKind::Decision { .. } | EventKind::Test { .. } | EventKind::Review { .. })
    }

    pub fn is_decision(&self) -> bool {
        matches!(self, EventKind::Decision { .. })
    }
}

/// Returns `true` if the title begins with `[conductr:`.
pub fn is_conductr_event(title: &str) -> bool {
    title.starts_with("[conductr:")
}

/// Parse a calendar event title into its structured kind.
pub fn parse_event_title(title: &str) -> EventKind {
    if !title.starts_with("[conductr:") {
        return EventKind::Unknown;
    }

    let close = match title.find(']') {
        Some(i) => i,
        None => return EventKind::Unknown,
    };

    // Content between '[' and ']', e.g. "conductr:window:auth,api" or "conductr:blocked"
    let bracket_content = &title[1..close];
    let after_bracket = title[close + 1..].trim();

    let inner = match bracket_content.strip_prefix("conductr:") {
        Some(s) => s,
        None => return EventKind::Unknown,
    };

    // Window events
    if inner == "window" {
        return EventKind::Window { tags: vec![] };
    }
    if let Some(rest) = inner.strip_prefix("window:") {
        let tags = rest
            .split(',')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect();
        return EventKind::Window { tags };
    }

    // Blocker events
    if inner == "blocked" {
        return EventKind::Blocked;
    }

    // Scheduled items: [conductr:<tag>] <kind>: <subject>
    // `*` tag means "no specific tag"
    let tag = if inner == "*" { None } else { Some(inner.to_string()) };

    if let Some(subject) = after_bracket.strip_prefix("decision:") {
        return EventKind::Decision { tag, subject: subject.trim().to_string() };
    }
    if let Some(subject) = after_bracket.strip_prefix("test:") {
        return EventKind::Test { tag, subject: subject.trim().to_string() };
    }
    if let Some(subject) = after_bracket.strip_prefix("review:") {
        return EventKind::Review { tag, subject: subject.trim().to_string() };
    }

    EventKind::Unknown
}

/// Extract `conductr-id: <value>` from an event description.
pub fn extract_conductr_id(description: &str) -> Option<String> {
    for line in description.lines() {
        if let Some(val) = line.strip_prefix("conductr-id:") {
            return Some(val.trim().to_string());
        }
    }
    None
}

/// Extract `originally-scheduled: <ISO-8601>` from an event description.
pub fn extract_originally_scheduled(description: &str) -> Option<DateTime<Utc>> {
    for line in description.lines() {
        if let Some(val) = line.strip_prefix("originally-scheduled:") {
            return DateTime::parse_from_rfc3339(val.trim())
                .ok()
                .map(|dt| dt.with_timezone(&Utc));
        }
    }
    None
}

/// Build the standard two-line identity footer for a conductr-managed event.
pub fn identity_lines(conductr_id: &str, originally_scheduled: DateTime<Utc>) -> String {
    format!(
        "conductr-id: {}\noriginally-scheduled: {}",
        conductr_id,
        originally_scheduled.to_rfc3339(),
    )
}

/// Returns `true` if a slot `[a_start, a_end)` overlaps `[b_start, b_end)`.
pub fn overlaps(
    a_start: DateTime<Utc>,
    a_end: DateTime<Utc>,
    b_start: DateTime<Utc>,
    b_end: DateTime<Utc>,
) -> bool {
    a_start < b_end && b_start < a_end
}

/// Returns `true` if a window with the given tags accepts an issue with `issue_tag`.
///
/// - Empty tags (generic window) → accepts any tag.
/// - `*` in window tags → accepts any tag.
/// - Otherwise exact match.
pub fn window_matches_tag(window_tags: &[String], issue_tag: Option<&str>) -> bool {
    if window_tags.is_empty() {
        return true;
    }
    match issue_tag {
        None => false,
        Some(tag) => window_tags.iter().any(|t| t == tag || t == "*"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_generic_window() {
        assert_eq!(
            parse_event_title("[conductr:window]"),
            EventKind::Window { tags: vec![] }
        );
    }

    #[test]
    fn parse_tagged_window() {
        assert_eq!(
            parse_event_title("[conductr:window:auth]"),
            EventKind::Window { tags: vec!["auth".to_string()] }
        );
    }

    #[test]
    fn parse_multi_tag_window() {
        let kind = parse_event_title("[conductr:window:auth,api]");
        assert_eq!(
            kind,
            EventKind::Window { tags: vec!["auth".to_string(), "api".to_string()] }
        );
    }

    #[test]
    fn parse_blocked() {
        assert_eq!(parse_event_title("[conductr:blocked]"), EventKind::Blocked);
    }

    #[test]
    fn parse_decision() {
        let kind = parse_event_title("[conductr:auth] decision: Fix login timeout");
        assert_eq!(
            kind,
            EventKind::Decision {
                tag: Some("auth".to_string()),
                subject: "Fix login timeout".to_string()
            }
        );
    }

    #[test]
    fn parse_test_slot() {
        let kind = parse_event_title("[conductr:*] test: auth integration suite");
        assert_eq!(
            kind,
            EventKind::Test { tag: None, subject: "auth integration suite".to_string() }
        );
    }

    #[test]
    fn parse_review_slot() {
        let kind = parse_event_title("[conductr:api] review: PR #99");
        assert_eq!(
            kind,
            EventKind::Review { tag: Some("api".to_string()), subject: "PR #99".to_string() }
        );
    }

    #[test]
    fn parse_unknown() {
        assert_eq!(parse_event_title("Team standup"), EventKind::Unknown);
        assert_eq!(parse_event_title("[conductr:window"), EventKind::Unknown);
    }

    #[test]
    fn extract_id_and_scheduled() {
        let desc = "conductr-id: 42\noriginally-scheduled: 2026-05-15T09:00:00Z\nExtra line";
        assert_eq!(extract_conductr_id(desc), Some("42".to_string()));
        let ts = extract_originally_scheduled(desc).unwrap();
        assert_eq!(ts.to_rfc3339(), "2026-05-15T09:00:00+00:00");
    }

    #[test]
    fn window_matches_any_when_no_tags() {
        assert!(window_matches_tag(&[], Some("auth")));
        assert!(window_matches_tag(&[], None));
    }

    #[test]
    fn window_tag_exact_match() {
        let tags = vec!["auth".to_string()];
        assert!(window_matches_tag(&tags, Some("auth")));
        assert!(!window_matches_tag(&tags, Some("api")));
        assert!(!window_matches_tag(&tags, None));
    }

    #[test]
    fn overlaps_basic() {
        use chrono::TimeZone;
        let t = |h: u32| Utc.with_ymd_and_hms(2026, 5, 15, h, 0, 0).unwrap();
        assert!(overlaps(t(9), t(10), t(9), t(10)));
        assert!(overlaps(t(9), t(10), t(9), t(9) + chrono::Duration::minutes(30)));
        assert!(!overlaps(t(9), t(10), t(10), t(11)));
        assert!(!overlaps(t(10), t(11), t(9), t(10)));
    }
}
