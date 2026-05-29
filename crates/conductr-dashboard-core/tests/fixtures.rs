use conductr_dashboard_core::*;

macro_rules! fixture {
    ($name:literal, $ty:ty) => {{
        let path = concat!("../../docs/dashboard-api/fixtures/", $name);
        let raw = std::fs::read_to_string(path).expect(path);
        let parsed: Envelope<$ty> = serde_json::from_str(&raw).expect(path);
        let reserialized = serde_json::to_value(&parsed).expect(path);
        let original: serde_json::Value = serde_json::from_str(&raw).expect(path);
        assert_eq!(reserialized, original, "round-trip mismatch in {}", $name);
    }};
}

#[test]
fn cycle_fixture() {
    fixture!("cycle-in-flight.json", Cycle);
}

#[test]
fn prs_fixture() {
    fixture!("prs-by-repo.json", PrGrouped);
}

#[test]
fn pod_fixture() {
    fixture!("pod-health.json", PodSnapshot);
}

#[test]
fn findings_fixture() {
    fixture!("idle-findings.json", Vec<WireFinding>);
}

#[test]
fn cadence_fixture() {
    fixture!("cadence-render.json", CadenceStaff);
}

#[test]
fn cadence_all_glyphs_validate() {
    let path = "../../docs/dashboard-api/fixtures/cadence-render.json";
    let raw = std::fs::read_to_string(path).expect(path);
    let parsed: Envelope<CadenceStaff> = serde_json::from_str(&raw).expect(path);

    let mut seen_glyphs = std::collections::HashSet::new();
    for row in &parsed.data.rows {
        for hit in &row.hits {
            hit.validate().unwrap_or_else(|e| panic!("StaffHit::validate() failed: {e}"));
            seen_glyphs.insert(format!("{:?}", hit.glyph));
        }
    }

    for glyph in ["Head", "Rest", "Hit", "Tied"] {
        assert!(
            seen_glyphs.contains(glyph),
            "cadence-render.json is missing glyph '{glyph}'"
        );
    }
}

// `state.json` is tested via serde_json::Value to avoid coupling to DashboardState layout.
#[test]
fn state_fixture_parses_as_value() {
    let path = "../../docs/dashboard-api/fixtures/state.json";
    let raw = std::fs::read_to_string(path).expect(path);
    let parsed: Envelope<serde_json::Value> = serde_json::from_str(&raw).expect(path);
    let reserialized = serde_json::to_value(&parsed).expect(path);
    let original: serde_json::Value = serde_json::from_str(&raw).expect(path);
    assert_eq!(reserialized, original, "round-trip mismatch in state.json");
}
