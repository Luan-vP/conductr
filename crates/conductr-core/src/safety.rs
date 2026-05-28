//! Safety-fader preset enum, config types, and maturity-default resolver.
//!
//! Spec: issue #142. This module is the foundation layer consumed by the
//! enforcement sub-issues (#173, #174, #175).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::maturity::MaturityLevel;

/// Safety fader preset — left → right is more permissive → more restrictive.
///
/// `BUREAUCRATIC` never appears as an auto-default (see [`default_from_maturity`]);
/// it can only be set via an explicit pin. See issue #142 for the full spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SafetyPreset {
    Unhinged,
    Feral,
    Fast,
    Strict,
    Bureaucratic,
}

impl fmt::Display for SafetyPreset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unhinged => write!(f, "unhinged"),
            Self::Feral => write!(f, "feral"),
            Self::Fast => write!(f, "fast"),
            Self::Strict => write!(f, "strict"),
            Self::Bureaucratic => write!(f, "bureaucratic"),
        }
    }
}

impl FromStr for SafetyPreset {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "unhinged" => Ok(Self::Unhinged),
            "feral" => Ok(Self::Feral),
            "fast" => Ok(Self::Fast),
            "strict" => Ok(Self::Strict),
            "bureaucratic" => Ok(Self::Bureaucratic),
            _ => Err(format!("unknown safety preset: {s}")),
        }
    }
}

/// An agent routine slot that can receive a per-routine safety pin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Routine {
    Architect,
    Implementer,
    Reviewer,
    Tester,
    Security,
    DocWriter,
    IdleSweeper,
}

/// Per-routine overrides inside `[safety.overrides]`.
///
/// Unknown keys are rejected. Any field absent means "no pin for this routine".
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SafetyOverrides {
    pub architect: Option<SafetyPreset>,
    pub implementer: Option<SafetyPreset>,
    pub reviewer: Option<SafetyPreset>,
    pub tester: Option<SafetyPreset>,
    pub security: Option<SafetyPreset>,
    #[serde(rename = "doc-writer")]
    pub doc_writer: Option<SafetyPreset>,
    #[serde(rename = "idle-sweeper")]
    pub idle_sweeper: Option<SafetyPreset>,
}

impl SafetyOverrides {
    /// Return the pinned preset for `routine`, or `None` if unpinned.
    pub fn get(&self, routine: Routine) -> Option<SafetyPreset> {
        match routine {
            Routine::Architect => self.architect,
            Routine::Implementer => self.implementer,
            Routine::Reviewer => self.reviewer,
            Routine::Tester => self.tester,
            Routine::Security => self.security,
            Routine::DocWriter => self.doc_writer,
            Routine::IdleSweeper => self.idle_sweeper,
        }
    }
}

/// The `[safety]` block in `.conductr`.
///
/// Missing block → all `None` / empty overrides.
#[derive(Debug, Default, Deserialize)]
pub struct SafetyConfig {
    /// Global preset pin; absence means "derive from maturity".
    pub preset: Option<SafetyPreset>,
    /// Per-routine overrides that take precedence over the global pin.
    #[serde(default)]
    pub overrides: SafetyOverrides,
}

/// Return the effective safety preset for `routine`.
///
/// Look-up order (highest → lowest priority):
/// 1. `cfg.overrides[routine]` — per-routine pin
/// 2. `cfg.preset` — global pin
/// 3. [`default_from_maturity(maturity)`][default_from_maturity]
pub fn resolve_preset(
    maturity: MaturityLevel,
    cfg: &SafetyConfig,
    routine: Routine,
) -> SafetyPreset {
    if let Some(p) = cfg.overrides.get(routine) {
        return p;
    }
    if let Some(p) = cfg.preset {
        return p;
    }
    default_from_maturity(maturity)
}

/// Map a maturity level to its auto-default safety preset.
///
/// `BUREAUCRATIC` never appears here.
pub fn default_from_maturity(maturity: MaturityLevel) -> SafetyPreset {
    match maturity {
        MaturityLevel::L0Bootstrap => SafetyPreset::Unhinged,
        MaturityLevel::L1Tested => SafetyPreset::Feral,
        MaturityLevel::L2GitFlow => SafetyPreset::Fast,
        MaturityLevel::L3Architected => SafetyPreset::Fast,
        MaturityLevel::L4Skilled => SafetyPreset::Strict,
        MaturityLevel::L5Orchestrated => SafetyPreset::Strict,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── SafetyPreset round-trip ───────────────────────────────────────────────

    #[test]
    fn preset_display_round_trip() {
        for preset in [
            SafetyPreset::Unhinged,
            SafetyPreset::Feral,
            SafetyPreset::Fast,
            SafetyPreset::Strict,
            SafetyPreset::Bureaucratic,
        ] {
            let s = preset.to_string();
            let parsed: SafetyPreset = s.parse().unwrap();
            assert_eq!(parsed, preset);
        }
    }

    #[test]
    fn preset_serde_round_trip() {
        let json = serde_json::to_string(&SafetyPreset::Strict).unwrap();
        assert_eq!(json, "\"strict\"");
        let back: SafetyPreset = serde_json::from_str(&json).unwrap();
        assert_eq!(back, SafetyPreset::Strict);
    }

    #[test]
    fn all_presets_serde() {
        for preset in [
            SafetyPreset::Unhinged,
            SafetyPreset::Feral,
            SafetyPreset::Fast,
            SafetyPreset::Strict,
            SafetyPreset::Bureaucratic,
        ] {
            let json = serde_json::to_string(&preset).unwrap();
            let back: SafetyPreset = serde_json::from_str(&json).unwrap();
            assert_eq!(back, preset);
        }
    }

    #[test]
    fn preset_from_str_unknown_is_error() {
        assert!("unknown".parse::<SafetyPreset>().is_err());
        assert!("Fast".parse::<SafetyPreset>().is_err()); // case-sensitive
    }

    #[test]
    fn preset_ordering() {
        assert!(SafetyPreset::Unhinged < SafetyPreset::Feral);
        assert!(SafetyPreset::Feral < SafetyPreset::Fast);
        assert!(SafetyPreset::Fast < SafetyPreset::Strict);
        assert!(SafetyPreset::Strict < SafetyPreset::Bureaucratic);
    }

    // ── default_from_maturity ─────────────────────────────────────────────────

    #[test]
    fn maturity_defaults_all_levels() {
        use MaturityLevel::*;
        assert_eq!(default_from_maturity(L0Bootstrap), SafetyPreset::Unhinged);
        assert_eq!(default_from_maturity(L1Tested), SafetyPreset::Feral);
        assert_eq!(default_from_maturity(L2GitFlow), SafetyPreset::Fast);
        assert_eq!(default_from_maturity(L3Architected), SafetyPreset::Fast);
        assert_eq!(default_from_maturity(L4Skilled), SafetyPreset::Strict);
        assert_eq!(default_from_maturity(L5Orchestrated), SafetyPreset::Strict);
    }

    #[test]
    fn bureaucratic_never_a_maturity_default() {
        use MaturityLevel::*;
        for lvl in [L0Bootstrap, L1Tested, L2GitFlow, L3Architected, L4Skilled, L5Orchestrated] {
            assert_ne!(default_from_maturity(lvl), SafetyPreset::Bureaucratic);
        }
    }

    // ── resolve_preset ────────────────────────────────────────────────────────

    fn empty_cfg() -> SafetyConfig {
        SafetyConfig::default()
    }

    fn global_cfg(preset: SafetyPreset) -> SafetyConfig {
        SafetyConfig { preset: Some(preset), ..Default::default() }
    }

    #[test]
    fn resolve_no_override_uses_maturity_default() {
        use MaturityLevel::*;
        let cfg = empty_cfg();
        assert_eq!(resolve_preset(L0Bootstrap, &cfg, Routine::Implementer), SafetyPreset::Unhinged);
        assert_eq!(resolve_preset(L1Tested, &cfg, Routine::Implementer), SafetyPreset::Feral);
        assert_eq!(resolve_preset(L2GitFlow, &cfg, Routine::Implementer), SafetyPreset::Fast);
        assert_eq!(resolve_preset(L3Architected, &cfg, Routine::Implementer), SafetyPreset::Fast);
        assert_eq!(resolve_preset(L4Skilled, &cfg, Routine::Implementer), SafetyPreset::Strict);
        assert_eq!(resolve_preset(L5Orchestrated, &cfg, Routine::Implementer), SafetyPreset::Strict);
    }

    #[test]
    fn resolve_global_pin_overrides_all_maturity_levels() {
        use MaturityLevel::*;
        let cfg = global_cfg(SafetyPreset::Bureaucratic);
        for lvl in [L0Bootstrap, L1Tested, L2GitFlow, L3Architected, L4Skilled, L5Orchestrated] {
            assert_eq!(resolve_preset(lvl, &cfg, Routine::Implementer), SafetyPreset::Bureaucratic);
        }
    }

    #[test]
    fn resolve_per_routine_pin_overrides_global_pin() {
        use MaturityLevel::*;
        let mut cfg = global_cfg(SafetyPreset::Strict);
        cfg.overrides.implementer = Some(SafetyPreset::Feral);

        // pinned routine gets its own value
        assert_eq!(resolve_preset(L2GitFlow, &cfg, Routine::Implementer), SafetyPreset::Feral);
        // other routines fall through to global pin
        assert_eq!(resolve_preset(L2GitFlow, &cfg, Routine::Reviewer), SafetyPreset::Strict);
        assert_eq!(resolve_preset(L2GitFlow, &cfg, Routine::Security), SafetyPreset::Strict);
    }

    #[test]
    fn resolve_per_routine_pin_without_global_pin() {
        use MaturityLevel::*;
        let mut cfg = empty_cfg();
        cfg.overrides.security = Some(SafetyPreset::Unhinged);

        // security is pinned even at L4
        assert_eq!(resolve_preset(L4Skilled, &cfg, Routine::Security), SafetyPreset::Unhinged);
        // others fall through to maturity default
        assert_eq!(resolve_preset(L4Skilled, &cfg, Routine::Implementer), SafetyPreset::Strict);
    }

    #[test]
    fn resolve_all_routines_use_maturity_default_when_no_pin() {
        use MaturityLevel::*;
        let cfg = empty_cfg();
        let routines = [
            Routine::Architect,
            Routine::Implementer,
            Routine::Reviewer,
            Routine::Tester,
            Routine::Security,
            Routine::DocWriter,
            Routine::IdleSweeper,
        ];
        for r in routines {
            assert_eq!(resolve_preset(L2GitFlow, &cfg, r), SafetyPreset::Fast);
        }
    }

    #[test]
    fn resolve_every_maturity_x_no_override() {
        use MaturityLevel::*;
        let cfg = empty_cfg();
        let cases = [
            (L0Bootstrap, SafetyPreset::Unhinged),
            (L1Tested, SafetyPreset::Feral),
            (L2GitFlow, SafetyPreset::Fast),
            (L3Architected, SafetyPreset::Fast),
            (L4Skilled, SafetyPreset::Strict),
            (L5Orchestrated, SafetyPreset::Strict),
        ];
        for (lvl, expected) in cases {
            assert_eq!(resolve_preset(lvl, &cfg, Routine::Architect), expected);
        }
    }
}
