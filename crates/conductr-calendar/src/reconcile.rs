//! Full calendar reconcile: decision slots for open `human`-labelled issues.

use std::collections::{BTreeMap, HashMap};

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use tracing::{debug, info};

use conductr_core::ports::{CalendarPort, ScmHost};
use conductr_core::types::{CalendarEvent, IssueNumber, NewCalendarEvent, RepoSlug};

use crate::parse::{
    extract_conductr_id, extract_originally_scheduled, identity_lines, is_conductr_event,
    overlaps, parse_event_title, window_matches_tag, EventKind,
};

fn slot_duration() -> Duration {
    Duration::minutes(30)
}

#[derive(Debug, Default, Clone)]
pub struct SyncReport {
    pub added: Vec<String>,
    pub kept: Vec<String>,
    pub deleted: Vec<String>,
    pub windows_found: usize,
    pub slots_filled: usize,
    pub unschedulable: Vec<IssueNumber>,
}

impl std::fmt::Display for SyncReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Calendar sync complete")?;
        writeln!(f, "======================")?;
        for msg in &self.added {
            writeln!(f, "Added:   {msg}")?;
        }
        for msg in &self.kept {
            writeln!(f, "Kept:    {msg}")?;
        }
        for msg in &self.deleted {
            writeln!(f, "Deleted: {msg}")?;
        }
        if !self.unschedulable.is_empty() {
            let nums: Vec<String> = self.unschedulable.iter().map(|n| format!("#{n}")).collect();
            writeln!(f, "No slot: {} (no eligible window)", nums.join(", "))?;
        }
        writeln!(
            f,
            "Windows: {} found, {} slot(s) filled",
            self.windows_found, self.slots_filled
        )
    }
}

/// A 30-minute slot carved from a window event.
#[derive(Debug, Clone)]
struct Slot {
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    window_tags: Vec<String>,
}

/// Run the full calendar reconcile.
///
/// - `project_tag`: the `project_tag` value from `.conductr` for the current repo.
///   Used to match issues against tagged windows.
/// - `dry_run`: log what would happen without writing to the calendar.
pub async fn reconcile(
    calendar: &dyn CalendarPort,
    scm: &dyn ScmHost,
    repo: &RepoSlug,
    project_tag: Option<&str>,
    dry_run: bool,
) -> Result<SyncReport> {
    let now = Utc::now();
    let mut report = SyncReport::default();

    // ── Step 1: read all upcoming [conductr:*] events ─────────────────────────
    let all_events = calendar.list_upcoming_events(now).await?;
    let conductr_events: Vec<&CalendarEvent> =
        all_events.iter().filter(|e| is_conductr_event(&e.title)).collect();

    let mut windows: Vec<&CalendarEvent> = vec![];
    let mut blockers: Vec<&CalendarEvent> = vec![];
    let mut scheduled_decisions: Vec<&CalendarEvent> = vec![];

    for ev in &conductr_events {
        match parse_event_title(&ev.title) {
            EventKind::Window { .. } => windows.push(ev),
            EventKind::Blocked => blockers.push(ev),
            EventKind::Decision { .. } => scheduled_decisions.push(ev),
            EventKind::Test { .. } | EventKind::Review { .. } => {}
            EventKind::Unknown => {}
        }
    }

    report.windows_found = windows.len();
    debug!("found {} windows, {} blockers, {} scheduled decisions",
        windows.len(), blockers.len(), scheduled_decisions.len());

    // ── Step 2: read open human-labelled issues ───────────────────────────────
    let issues = scm.list_open_issues(repo).await?;
    let human_issues: Vec<_> = issues.iter().filter(|i| i.is_human()).collect();

    // Priority = count of other human issues that depend on this one.
    // Simple approach: parse `depends on #N` patterns in issue bodies.
    let mut depended_on: BTreeMap<IssueNumber, usize> = BTreeMap::new();
    for issue in &human_issues {
        depended_on.entry(issue.number).or_insert(0);
        for dep_num in parse_dep_numbers(&issue.body) {
            *depended_on.entry(dep_num).or_insert(0) += 1;
        }
    }

    // Sort by priority descending (higher = more issues blocked on this)
    let mut sorted_issues = human_issues.clone();
    sorted_issues.sort_by(|a, b| {
        let pa = depended_on.get(&a.number).copied().unwrap_or(0);
        let pb = depended_on.get(&b.number).copied().unwrap_or(0);
        pb.cmp(&pa).then(a.number.cmp(&b.number))
    });

    // ── Step 3: compute available slots (excluding blocked + occupied) ─────────
    // All scheduled events (any kind) count as occupied for slot purposes.
    let all_scheduled: Vec<&CalendarEvent> = conductr_events
        .iter()
        .filter(|e| matches!(parse_event_title(&e.title), EventKind::Decision { .. } | EventKind::Test { .. } | EventKind::Review { .. }))
        .copied()
        .collect();

    // Map conductr-id → event for existing decisions.
    let mut id_to_decision: HashMap<String, &CalendarEvent> = HashMap::new();
    for ev in &scheduled_decisions {
        let desc = ev.description.as_deref().unwrap_or("");
        if let Some(cid) = extract_conductr_id(desc) {
            id_to_decision.insert(cid, ev);
        }
    }

    // ── Step 4: reconcile decision slots ──────────────────────────────────────
    let mut occupied_ids: Vec<String> = vec![];
    for issue in &sorted_issues {
        let issue_id = issue.number.to_string();

        if let Some(existing) = id_to_decision.get(&issue_id).copied() {
            // Check if the slot is still in a valid window
            let desc = existing.description.as_deref().unwrap_or("");
            let originally = extract_originally_scheduled(desc);
            let was_dragged = originally
                .map(|orig| orig != existing.start)
                .unwrap_or(false);

            let in_valid_window = slot_is_in_eligible_window(
                existing.start,
                existing.end,
                &windows,
                &blockers,
                project_tag,
            );

            if in_valid_window || was_dragged {
                let label = if was_dragged { "manually dragged" } else { "in eligible window" };
                info!("keep slot for #{}: {} ({})", issue.number, existing.title, label);
                report.kept.push(format!(
                    "{} (issue #{}, {})",
                    existing.title, issue.number, label
                ));
                occupied_ids.push(existing.id.clone());
                continue;
            }

            // Stale slot — delete it so we can re-schedule
            info!("delete stale slot for #{}: {}", issue.number, existing.title);
            if !dry_run {
                calendar.delete_event(&existing.id).await?;
            }
            report.deleted.push(format!(
                "{} (issue #{} — window gone or blocked)",
                existing.title, issue.number
            ));
        }

        // Find the earliest available slot
        let available = compute_available_slots(
            &windows,
            &blockers,
            &all_scheduled,
            &occupied_ids,
            now,
        );

        let eligible = available
            .iter()
            .find(|s| window_matches_tag(&s.window_tags, project_tag));

        match eligible {
            Some(slot) => {
                let tag_prefix = project_tag
                    .map(|t| format!("[conductr:{t}] "))
                    .unwrap_or_else(|| "[conductr:*] ".to_string());
                let title = format!("{tag_prefix}decision: {}", issue.title);
                let description = identity_lines(&issue_id, slot.start);
                let new_ev = NewCalendarEvent {
                    title: title.clone(),
                    start: slot.start,
                    end: slot.end,
                    description,
                };

                if dry_run {
                    info!("dry-run: would create '{title}' at {}", slot.start.to_rfc3339());
                    occupied_ids.push(format!("dry-{}", issue.number));
                } else {
                    let created = calendar.create_event(new_ev).await?;
                    occupied_ids.push(created.id);
                }

                report.added.push(format!(
                    "{title}  (issue #{}, slot {})",
                    issue.number,
                    slot.start.format("%Y-%m-%d %H:%M")
                ));
                report.slots_filled += 1;
            }
            None => {
                info!("no eligible slot for #{}", issue.number);
                report.unschedulable.push(issue.number);
            }
        }
    }

    // ── Step 5: remove orphaned decision slots ────────────────────────────────
    let open_ids: std::collections::BTreeSet<String> =
        sorted_issues.iter().map(|i| i.number.to_string()).collect();

    for ev in &scheduled_decisions {
        let desc = ev.description.as_deref().unwrap_or("");
        if let Some(cid) = extract_conductr_id(desc) {
            if !open_ids.contains(&cid) && !occupied_ids.contains(&ev.id) {
                info!("delete orphaned decision slot: {}", ev.title);
                if !dry_run {
                    calendar.delete_event(&ev.id).await?;
                }
                report.deleted.push(format!(
                    "{} (issue #{cid} — closed or re-labelled)",
                    ev.title
                ));
            }
        }
    }

    Ok(report)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn compute_available_slots(
    windows: &[&CalendarEvent],
    blockers: &[&CalendarEvent],
    scheduled: &[&CalendarEvent],
    occupied_event_ids: &[String],
    now: DateTime<Utc>,
) -> Vec<Slot> {
    let mut slots: Vec<Slot> = vec![];

    for window in windows {
        let window_tags = match parse_event_title(&window.title) {
            EventKind::Window { tags } => tags,
            _ => continue,
        };

        let mut slot_start = window.start;
        while slot_start + slot_duration() <= window.end {
            let slot_end = slot_start + slot_duration();

            let blocked = blockers
                .iter()
                .any(|b| overlaps(slot_start, slot_end, b.start, b.end));

            let occupied_by_scheduled = scheduled.iter().any(|s| {
                !occupied_event_ids.contains(&s.id)
                    && overlaps(slot_start, slot_end, s.start, s.end)
            });

            if !blocked && !occupied_by_scheduled && slot_start >= now {
                slots.push(Slot { start: slot_start, end: slot_end, window_tags: window_tags.clone() });
            }

            slot_start = slot_end;
        }
    }

    slots.sort_by_key(|s| s.start);
    slots
}

fn slot_is_in_eligible_window(
    slot_start: DateTime<Utc>,
    slot_end: DateTime<Utc>,
    windows: &[&CalendarEvent],
    blockers: &[&CalendarEvent],
    issue_tag: Option<&str>,
) -> bool {
    for window in windows {
        let tags = match parse_event_title(&window.title) {
            EventKind::Window { tags } => tags,
            _ => continue,
        };
        if !window_matches_tag(&tags, issue_tag) {
            continue;
        }
        // Slot must be fully within the window
        if slot_start >= window.start && slot_end <= window.end {
            // Must not be blocked
            let is_blocked = blockers
                .iter()
                .any(|b| overlaps(slot_start, slot_end, b.start, b.end));
            if !is_blocked {
                return true;
            }
        }
    }
    false
}

fn parse_dep_numbers(body: &str) -> Vec<IssueNumber> {
    // Simple regex-free parser for `depends on #N`, `blocked by #N`, `after #N`
    let mut deps = vec![];
    let lower = body.to_lowercase();
    let keywords = ["depends on", "blocked by", "after", "requires"];
    for kw in keywords {
        let mut search = lower.as_str();
        while let Some(pos) = search.find(kw) {
            let rest = &search[pos + kw.len()..];
            let rest_trim = rest.trim_start();
            // Extract all #N references in the remaining fragment (up to end of "sentence")
            let fragment: String = rest_trim.chars().take_while(|c| *c != '.' && *c != '\n').collect();
            let mut scan = fragment.as_str();
            while let Some(hash_pos) = scan.find('#') {
                let after_hash = &scan[hash_pos + 1..];
                let digits: String = after_hash.chars().take_while(|c| c.is_ascii_digit()).collect();
                if !digits.is_empty() {
                    if let Ok(n) = digits.parse::<IssueNumber>() {
                        deps.push(n);
                    }
                }
                scan = &scan[hash_pos + 1..];
                if scan.is_empty() { break; }
            }
            search = &search[pos + kw.len()..];
            if search.is_empty() { break; }
        }
    }
    deps.sort();
    deps.dedup();
    deps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_deps_simple() {
        let deps = parse_dep_numbers("This depends on #42 and #43.");
        assert_eq!(deps, vec![42, 43]);
    }

    #[test]
    fn parse_deps_blocked_by() {
        let deps = parse_dep_numbers("Blocked by #5.");
        assert_eq!(deps, vec![5]);
    }

    #[test]
    fn parse_deps_empty() {
        let deps = parse_dep_numbers("No dependencies here. See #99 for context.");
        assert!(deps.is_empty());
    }
}
