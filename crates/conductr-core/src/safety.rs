//! Safety preset model — schema, resolver, and taglines.
//!
//! This is the **single source of truth** for safety logic.  Dashboards and CLIs
//! must call [`safety_default_for_maturity`] and [`resolve_project_safety`] rather
//! than duplicating the mapping.

use serde::{Deserialize, Serialize};

use crate::maturity::MaturityLevel;

// ── SafetyPreset ──────────────────────────────────────────────────────────────

/// The five safety detents, ordered from least to most cautious (left → right
/// on the fader).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SafetyPreset {
    Unhinged,
    Feral,
    Fast,
    Strict,
    Bureaucratic,
}

impl SafetyPreset {
    /// Serialisation key (lowercase, matches `.conductr` TOML values).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unhinged => "unhinged",
            Self::Feral => "feral",
            Self::Fast => "fast",
            Self::Strict => "strict",
            Self::Bureaucratic => "bureaucratic",
        }
    }

    /// Display name shown on the fader detent.
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Unhinged => "UNHINGED",
            Self::Feral => "FERAL",
            Self::Fast => "FAST",
            Self::Strict => "STRICT",
            Self::Bureaucratic => "BUREAUCRATIC",
        }
    }

    /// Doctrine tagline rendered under the active detent.
    pub fn tagline(self) -> &'static str {
        match self {
            Self::Unhinged => "Ship it; we'll debug in prod",
            Self::Feral => "First to green wins",
            Self::Fast => "Trust the process; keep moving",
            Self::Strict => "Measure twice, cut once",
            Self::Bureaucratic => "If it's worth shipping, it's worth a paper trail",
        }
    }

    /// All detents in left-to-right fader order.
    pub fn all() -> [SafetyPreset; 5] {
        [Self::Unhinged, Self::Feral, Self::Fast, Self::Strict, Self::Bureaucratic]
    }
}

impl std::fmt::Display for SafetyPreset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.display_name())
    }
}

// ── SafetyRole ────────────────────────────────────────────────────────────────

/// Band roles that may carry per-routine safety overrides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SafetyRole {
    Architect,
    Implementer,
    Reviewer,
    Tester,
    Security,
    DocWriter,
    IdleSweeper,
}

impl SafetyRole {
    /// All roles in the order they appear in the override drawer.
    pub fn all() -> [SafetyRole; 7] {
        [
            Self::Architect,
            Self::Implementer,
            Self::Reviewer,
            Self::Tester,
            Self::Security,
            Self::DocWriter,
            Self::IdleSweeper,
        ]
    }

    /// The TOML key used in `[safety.overrides]`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Architect => "architect",
            Self::Implementer => "implementer",
            Self::Reviewer => "reviewer",
            Self::Tester => "tester",
            Self::Security => "security",
            Self::DocWriter => "doc-writer",
            Self::IdleSweeper => "idle-sweeper",
        }
    }
}

impl std::fmt::Display for SafetyRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ── ResolvedSafety ────────────────────────────────────────────────────────────

/// The fully-resolved safety configuration for the project or a single role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSafety {
    /// The effective preset after applying the pin and any role override.
    pub effective: SafetyPreset,
    /// The maturity-derived suggestion (always populated).
    pub maturity_default: SafetyPreset,
    /// Whether the effective value was set by a user pin (not just the maturity default).
    pub pinned: bool,
}

impl ResolvedSafety {
    /// Returns the fader subtitle when the active preset diverges from the
    /// maturity-derived default (e.g. `"maturity suggests FAST; pinned to FERAL"`).
    pub fn subtitle(&self) -> Option<String> {
        if self.pinned && self.effective != self.maturity_default {
            Some(format!(
                "maturity suggests {}; pinned to {}",
                self.maturity_default.display_name(),
                self.effective.display_name(),
            ))
        } else {
            None
        }
    }
}

// ── Resolver ──────────────────────────────────────────────────────────────────

/// Derive the default [`SafetyPreset`] from the repository's maturity level.
///
/// This is the **single source of truth** for the maturity → safety mapping.
/// Dashboards and other consumers must call this function; do not copy-paste
/// the `match` arm elsewhere.
pub fn safety_default_for_maturity(level: MaturityLevel) -> SafetyPreset {
    match level {
        MaturityLevel::L0Bootstrap => SafetyPreset::Feral,
        MaturityLevel::L1Tested => SafetyPreset::Feral,
        MaturityLevel::L2GitFlow => SafetyPreset::Fast,
        MaturityLevel::L3Architected => SafetyPreset::Fast,
        MaturityLevel::L4Skilled => SafetyPreset::Strict,
        MaturityLevel::L5Orchestrated => SafetyPreset::Strict,
    }
}

/// Resolve the effective safety preset for the whole project.
///
/// - `maturity_level`: from `conductr-core`'s maturity scanner.
/// - `pinned`: what the user wrote to `[safety] preset = "..."` in `.conductr`
///   (absent = follow maturity default).
pub fn resolve_project_safety(
    maturity_level: MaturityLevel,
    pinned: Option<SafetyPreset>,
) -> ResolvedSafety {
    let maturity_default = safety_default_for_maturity(maturity_level);
    let effective = pinned.unwrap_or(maturity_default);
    ResolvedSafety { effective, maturity_default, pinned: pinned.is_some() }
}

/// Resolve the effective safety preset for a specific role.
///
/// Override chain: `role_override` > `project.effective` > maturity default.
pub fn resolve_role_safety(
    project: &ResolvedSafety,
    role_override: Option<SafetyPreset>,
) -> ResolvedSafety {
    let effective = role_override.unwrap_or(project.effective);
    ResolvedSafety {
        effective,
        maturity_default: project.maturity_default,
        pinned: role_override.is_some() || project.pinned,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use MaturityLevel::*;

    #[test]
    fn maturity_defaults_cover_all_levels() {
        assert_eq!(safety_default_for_maturity(L0Bootstrap), SafetyPreset::Feral);
        assert_eq!(safety_default_for_maturity(L1Tested), SafetyPreset::Feral);
        assert_eq!(safety_default_for_maturity(L2GitFlow), SafetyPreset::Fast);
        assert_eq!(safety_default_for_maturity(L3Architected), SafetyPreset::Fast);
        assert_eq!(safety_default_for_maturity(L4Skilled), SafetyPreset::Strict);
        assert_eq!(safety_default_for_maturity(L5Orchestrated), SafetyPreset::Strict);
    }

    #[test]
    fn resolve_project_no_pin_follows_maturity() {
        let resolved = resolve_project_safety(L2GitFlow, None);
        assert_eq!(resolved.effective, SafetyPreset::Fast);
        assert_eq!(resolved.maturity_default, SafetyPreset::Fast);
        assert!(!resolved.pinned);
        assert!(resolved.subtitle().is_none());
    }

    #[test]
    fn resolve_project_pin_overrides_maturity() {
        let resolved = resolve_project_safety(L2GitFlow, Some(SafetyPreset::Strict));
        assert_eq!(resolved.effective, SafetyPreset::Strict);
        assert_eq!(resolved.maturity_default, SafetyPreset::Fast);
        assert!(resolved.pinned);
        assert_eq!(
            resolved.subtitle().as_deref(),
            Some("maturity suggests FAST; pinned to STRICT")
        );
    }

    #[test]
    fn resolve_project_pin_matches_maturity_no_subtitle() {
        let resolved = resolve_project_safety(L2GitFlow, Some(SafetyPreset::Fast));
        assert!(resolved.pinned);
        assert!(resolved.subtitle().is_none(), "subtitle should be None when pin == maturity default");
    }

    #[test]
    fn resolve_role_inherits_project() {
        let project = resolve_project_safety(L3Architected, None);
        let role = resolve_role_safety(&project, None);
        assert_eq!(role.effective, SafetyPreset::Fast);
        assert!(!role.pinned);
    }

    #[test]
    fn resolve_role_override_takes_precedence() {
        let project = resolve_project_safety(L3Architected, None);
        let role = resolve_role_safety(&project, Some(SafetyPreset::Strict));
        assert_eq!(role.effective, SafetyPreset::Strict);
        assert!(role.pinned);
    }

    #[test]
    fn all_presets_have_taglines() {
        for preset in SafetyPreset::all() {
            assert!(!preset.tagline().is_empty());
        }
    }

    #[test]
    fn preset_serialises_to_lowercase() {
        let s = serde_json::to_string(&SafetyPreset::Unhinged).unwrap();
        assert_eq!(s, "\"unhinged\"");
        let s = serde_json::to_string(&SafetyPreset::Bureaucratic).unwrap();
        assert_eq!(s, "\"bureaucratic\"");
    }

    #[test]
    fn preset_deserialises_from_lowercase() {
        let p: SafetyPreset = serde_json::from_str("\"fast\"").unwrap();
        assert_eq!(p, SafetyPreset::Fast);
    }

    #[test]
    fn all_roles_have_str_keys() {
        for role in SafetyRole::all() {
            assert!(!role.as_str().is_empty());
        }
    }
}
