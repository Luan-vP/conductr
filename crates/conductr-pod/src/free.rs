//! Pick a free (idle, unattached) Claude Code session from the pod.

use conductr_core::types::{Diagnosis, Health};

pub struct FreeOpts {
    /// Allow picking a session that already has an attached client.
    pub include_attached: bool,
}

/// Return the best idle session from `diagnoses`, or `None` if none qualify.
///
/// "Best" means largest `idle_seconds` (longest untouched), so repeated calls
/// are deterministic rather than bouncing between panes.
pub fn pick_idle<'a>(diagnoses: &'a [Diagnosis], opts: &FreeOpts) -> Option<&'a Diagnosis> {
    diagnoses
        .iter()
        .filter(|d| matches!(d.health, Health::Idle { .. }))
        .filter(|d| opts.include_attached || !d.session.attached)
        .max_by_key(|d| d.idle_seconds)
}

#[cfg(test)]
mod tests {
    use super::*;
    use conductr_core::types::TmuxSession;
    use chrono::Utc;

    fn make(name: &str, health: Health, idle_seconds: i64, attached: bool) -> Diagnosis {
        Diagnosis {
            session: TmuxSession {
                name: name.to_string(),
                created: Utc::now(),
                last_activity: Utc::now(),
                windows: 1,
                attached,
                cwd: None,
            },
            health,
            idle_seconds,
            tail: vec![],
        }
    }

    fn idle() -> Health {
        Health::Idle { last_message: None, tokens: None }
    }

    #[test]
    fn picks_idle_unattached() {
        let diagnoses = vec![
            make("claude-1", Health::Working { activity: "busy".into() }, 5, false),
            make("claude-2", idle(), 10, false),
            make("claude-3", idle(), 30, false),
        ];
        let opts = FreeOpts { include_attached: false };
        assert_eq!(pick_idle(&diagnoses, &opts).unwrap().session.name, "claude-3");
    }

    #[test]
    fn skips_attached_by_default() {
        let diagnoses = vec![
            make("claude-1", idle(), 100, true),
            make("claude-2", idle(), 20, false),
        ];
        let opts = FreeOpts { include_attached: false };
        assert_eq!(pick_idle(&diagnoses, &opts).unwrap().session.name, "claude-2");
    }

    #[test]
    fn includes_attached_when_flag_set() {
        let diagnoses = vec![
            make("claude-1", idle(), 100, true),
            make("claude-2", idle(), 20, false),
        ];
        let opts = FreeOpts { include_attached: true };
        assert_eq!(pick_idle(&diagnoses, &opts).unwrap().session.name, "claude-1");
    }

    #[test]
    fn returns_none_when_no_idle() {
        let diagnoses = vec![
            make("claude-1", Health::Working { activity: "busy".into() }, 5, false),
            make("claude-2", Health::Crashed { last_shell_line: None }, 0, false),
        ];
        let opts = FreeOpts { include_attached: false };
        assert!(pick_idle(&diagnoses, &opts).is_none());
    }

    #[test]
    fn returns_none_when_empty() {
        let opts = FreeOpts { include_attached: false };
        assert!(pick_idle(&[], &opts).is_none());
    }

    #[test]
    fn tie_break_largest_idle_seconds() {
        let diagnoses = vec![
            make("claude-1", idle(), 50, false),
            make("claude-2", idle(), 200, false),
            make("claude-3", idle(), 100, false),
        ];
        let opts = FreeOpts { include_attached: false };
        assert_eq!(pick_idle(&diagnoses, &opts).unwrap().session.name, "claude-2");
    }
}
