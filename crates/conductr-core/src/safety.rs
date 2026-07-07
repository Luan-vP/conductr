use crate::maturity::MaturityLevel;
use crate::types::SafetyPreset;

/// Resolve the effective [`SafetyPreset`] for a routine.
///
/// Resolution order (highest precedence first):
/// 1. Per-routine override (`routine`) — derived from a `safety:<preset>` label on the issue
/// 2. Orchestrator-level config override (`cfg`)
/// 3. Repo maturity-derived default
///
/// Maturity-derived defaults top out at [`SafetyPreset::Fast`]. `Strict` and
/// `Bureaucratic` are pin-only — reachable only via `cfg` or `routine` — so
/// the heaviest process overhead is never imposed without an explicit choice.
pub fn resolve_preset(
    maturity: MaturityLevel,
    cfg: Option<SafetyPreset>,
    routine: Option<SafetyPreset>,
) -> SafetyPreset {
    if let Some(p) = routine {
        return p;
    }
    if let Some(p) = cfg {
        return p;
    }
    match maturity {
        MaturityLevel::L0Bootstrap | MaturityLevel::L1Tested => SafetyPreset::Unhinged,
        MaturityLevel::L2GitFlow => SafetyPreset::Feral,
        MaturityLevel::L3Architected | MaturityLevel::L4Skilled | MaturityLevel::L5Orchestrated => {
            SafetyPreset::Fast
        }
    }
}

/// Extract a per-issue safety preset from its labels (`safety:<preset>`).
/// Returns `None` if no such label is present or the value is unrecognised.
pub fn preset_from_labels(labels: &[String]) -> Option<SafetyPreset> {
    for label in labels {
        let l = label.trim().to_ascii_lowercase();
        if let Some(name) = l.strip_prefix("safety:") {
            match name.trim() {
                "unhinged" => return Some(SafetyPreset::Unhinged),
                "feral" => return Some(SafetyPreset::Feral),
                "fast" => return Some(SafetyPreset::Fast),
                "strict" => return Some(SafetyPreset::Strict),
                "bureaucratic" => return Some(SafetyPreset::Bureaucratic),
                _ => {}
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::maturity::MaturityLevel::*;
    use crate::types::SafetyPreset::*;

    #[test]
    fn routine_override_wins() {
        assert_eq!(
            resolve_preset(L5Orchestrated, Some(Feral), Some(Unhinged)),
            Unhinged
        );
    }

    #[test]
    fn cfg_wins_over_maturity() {
        assert_eq!(resolve_preset(L0Bootstrap, Some(Bureaucratic), None), Bureaucratic);
    }

    #[test]
    fn maturity_defaults() {
        assert_eq!(resolve_preset(L0Bootstrap, None, None), Unhinged);
        assert_eq!(resolve_preset(L1Tested, None, None), Unhinged);
        assert_eq!(resolve_preset(L2GitFlow, None, None), Feral);
        assert_eq!(resolve_preset(L3Architected, None, None), Fast);
        assert_eq!(resolve_preset(L4Skilled, None, None), Fast);
        assert_eq!(resolve_preset(L5Orchestrated, None, None), Fast);
    }

    #[test]
    fn maturity_defaults_never_reach_strict_or_bureaucratic() {
        for m in [
            L0Bootstrap,
            L1Tested,
            L2GitFlow,
            L3Architected,
            L4Skilled,
            L5Orchestrated,
        ] {
            let preset = resolve_preset(m, None, None);
            assert!(
                preset != Strict && preset != Bureaucratic,
                "maturity {m:?} auto-defaulted to {preset:?}, but Strict/Bureaucratic must be pin-only"
            );
        }
    }

    #[test]
    fn label_parsing() {
        let labels: Vec<String> = vec!["safety:strict".into(), "other".into()];
        assert_eq!(preset_from_labels(&labels), Some(Strict));
    }

    #[test]
    fn label_parsing_case_insensitive() {
        let labels: Vec<String> = vec!["Safety:BUREAUCRATIC".into()];
        assert_eq!(preset_from_labels(&labels), Some(Bureaucratic));
    }

    #[test]
    fn label_parsing_none_on_missing() {
        let labels: Vec<String> = vec!["runner:tmux".into()];
        assert_eq!(preset_from_labels(&labels), None);
    }
}
