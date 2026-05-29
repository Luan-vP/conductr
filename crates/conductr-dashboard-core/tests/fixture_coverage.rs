/// Static fixture-coverage guard.
///
/// Add a new entry to MODELS whenever a new public model type is introduced.
/// Add a corresponding entry to COVERAGE mapping it to a fixture file.
/// CI fails if any entry in MODELS is not covered by at least one fixture.
///
/// Note: this is the simple array-based approach. A build-script `inventory!`
/// trick for auto-discovery of public types is tracked as a follow-up.

/// Every public top-level model type that must appear in at least one fixture.
const MODELS: &[&str] = &[
    "Cycle",
    "PrGrouped",
    "PodSnapshot",
    "FindingEntry",
    "WireFinding",
    "CadenceStaff",
    "DashboardState",
    // Sub-types exercised transitively through the above fixtures:
    "Beat",
    "StaffHit",
    "StaffRow",
    "StaffWindow",
    "RepoEntry",
    "CronEntry",
    "LocalAgentEntry",
];

/// Maps each fixture file to the model types it exercises (directly or transitively).
const COVERAGE: &[(&str, &[&str])] = &[
    (
        "cycle-in-flight.json",
        &["Cycle", "Beat"],
    ),
    (
        "prs-by-repo.json",
        &["PrGrouped"],
    ),
    (
        "pod-health.json",
        &["PodSnapshot"],
    ),
    (
        "idle-findings.json",
        &["FindingEntry", "WireFinding"],
    ),
    (
        "cadence-render.json",
        &["CadenceStaff", "StaffHit", "StaffRow", "StaffWindow"],
    ),
    (
        "state.json",
        &["DashboardState", "RepoEntry", "CronEntry", "LocalAgentEntry"],
    ),
];

#[test]
fn every_model_has_fixture_coverage() {
    let covered: std::collections::HashSet<&str> = COVERAGE
        .iter()
        .flat_map(|(_, types)| types.iter().copied())
        .collect();

    let mut missing = Vec::new();
    for model in MODELS {
        if !covered.contains(*model) {
            missing.push(*model);
        }
    }

    assert!(
        missing.is_empty(),
        "These model types have no fixture coverage — add a fixture and update COVERAGE: {:?}",
        missing
    );
}

#[test]
fn coverage_fixtures_exist() {
    for (fixture, _) in COVERAGE {
        let path = format!("../../docs/dashboard-api/fixtures/{fixture}");
        assert!(
            std::path::Path::new(&path).exists(),
            "Fixture file listed in COVERAGE does not exist: {path}"
        );
    }
}
