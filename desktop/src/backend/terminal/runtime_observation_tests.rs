use super::*;
#[test]
fn null_id_runtime_observation_requires_exact_live_identity_and_canonical_generation() {
    let id = uuid();
    let generation = uuid();
    let record = json!({"id":id,"agent":"codex","sessionId":null});
    let live = json!({"host":"embedded","recordId":id,"agent":"codex","sessionId":null,"live":true,"generation":generation});
    assert_eq!(
        existing_codex_runtime(&record, &[live.clone()], false),
        RuntimeObservation::Observed(generation)
    );
    assert_eq!(
        existing_codex_runtime(&record, &[live.clone()], true),
        RuntimeObservation::Unavailable
    );
    for (key, value) in [("recordId", json!(uuid())), ("live", json!(false))] {
        let mut row = live.clone();
        row[key] = value;
        assert_eq!(
            existing_codex_runtime(&record, &[row], false),
            RuntimeObservation::Missing
        );
    }
    for (key, value) in [
        ("agent", json!("claude")),
        ("host", json!("other")),
        ("sessionId", json!(uuid())),
        ("generation", json!("stale-not-a-generation")),
    ] {
        let mut row = live.clone();
        row[key] = value;
        assert_eq!(
            existing_codex_runtime(&record, &[row], false),
            RuntimeObservation::Unavailable
        );
    }
    assert_eq!(
        existing_codex_runtime(&record, &[live.clone(), live.clone()], false),
        RuntimeObservation::Unavailable
    );
    let next = uuid();
    let mut replaced = live;
    replaced["generation"] = json!(next);
    assert_eq!(
        existing_codex_runtime(&record, &[replaced], false),
        RuntimeObservation::Observed(next)
    );
}
