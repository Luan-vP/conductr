//! Manual test-slot creation: `conductr sync schedule-test`.

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use tracing::info;

use conductr_core::ports::CalendarPort;
use conductr_core::types::{CalendarEvent, NewCalendarEvent};

use crate::parse::{
    identity_lines, is_conductr_event, overlaps, parse_event_title, window_matches_tag, EventKind,
};

#[derive(Debug)]
pub struct ScheduleTestReport {
    pub title: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub event_id: Option<String>,
    pub dry_run: bool,
}

impl std::fmt::Display for ScheduleTestReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let prefix = if self.dry_run { "dry-run: would create" } else { "Created" };
        write!(
            f,
            "{prefix} '{}' at {} – {}",
            self.title,
            self.start.format("%Y-%m-%d %H:%M"),
            self.end.format("%H:%M"),
        )
    }
}

/// Place a single test slot in the next available 30-minute window.
///
/// - `tag`: the tag for the test slot (e.g. `"auth"`). `None` → uses `*`.
/// - `subject`: description of the test being scheduled.
/// - `dry_run`: log the plan without writing to the calendar.
pub async fn schedule_test_slot(
    calendar: &dyn CalendarPort,
    tag: Option<&str>,
    subject: &str,
    dry_run: bool,
) -> Result<ScheduleTestReport> {
    let now = Utc::now();

    // Fetch all upcoming events
    let all_events = calendar.list_upcoming_events(now).await?;
    let conductr_events: Vec<&CalendarEvent> =
        all_events.iter().filter(|e| is_conductr_event(&e.title)).collect();

    let mut windows: Vec<&CalendarEvent> = vec![];
    let mut blockers: Vec<&CalendarEvent> = vec![];
    let mut scheduled: Vec<&CalendarEvent> = vec![];

    for ev in &conductr_events {
        match parse_event_title(&ev.title) {
            EventKind::Window { .. } => windows.push(ev),
            EventKind::Blocked => blockers.push(ev),
            EventKind::Decision { .. } | EventKind::Test { .. } | EventKind::Review { .. } => {
                scheduled.push(ev)
            }
            EventKind::Unknown => {}
        }
    }

    // Find the earliest available slot matching the requested tag
    let slot = find_eligible_slot(&windows, &blockers, &scheduled, tag, now)
        .ok_or_else(|| anyhow::anyhow!("no eligible 30-minute slot found in any window"))?;

    let tag_str = tag.unwrap_or("*");
    let title = format!("[conductr:{tag_str}] test: {subject}");
    let test_id = gen_test_id();
    let description = identity_lines(&test_id, slot.start);

    info!(
        "schedule-test: '{}' at {} (id: {})",
        title,
        slot.start.to_rfc3339(),
        test_id
    );

    let event_id = if dry_run {
        None
    } else {
        let created = calendar
            .create_event(NewCalendarEvent {
                title: title.clone(),
                start: slot.start,
                end: slot.end,
                description,
            })
            .await?;
        Some(created.id)
    };

    Ok(ScheduleTestReport {
        title,
        start: slot.start,
        end: slot.end,
        event_id,
        dry_run,
    })
}

struct Slot {
    start: DateTime<Utc>,
    end: DateTime<Utc>,
}

fn find_eligible_slot(
    windows: &[&CalendarEvent],
    blockers: &[&CalendarEvent],
    scheduled: &[&CalendarEvent],
    tag: Option<&str>,
    now: DateTime<Utc>,
) -> Option<Slot> {
    let duration = Duration::minutes(30);
    let mut candidates: Vec<Slot> = vec![];

    for window in windows {
        let window_tags = match parse_event_title(&window.title) {
            EventKind::Window { tags } => tags,
            _ => continue,
        };
        if !window_matches_tag(&window_tags, tag) {
            continue;
        }

        let mut slot_start = window.start;
        while slot_start + duration <= window.end {
            let slot_end = slot_start + duration;

            let blocked = blockers
                .iter()
                .any(|b| overlaps(slot_start, slot_end, b.start, b.end));

            let occupied = scheduled
                .iter()
                .any(|s| overlaps(slot_start, slot_end, s.start, s.end));

            if !blocked && !occupied && slot_start >= now {
                candidates.push(Slot { start: slot_start, end: slot_end });
                break; // one per window is enough; we take the earliest overall
            }

            slot_start = slot_end;
        }
    }

    candidates.into_iter().min_by_key(|s| s.start)
}

fn gen_test_id() -> String {
    let ts = Utc::now().timestamp();
    let sub = Utc::now().timestamp_subsec_nanos();
    format!("test-{ts:x}-{sub:x}")
}

