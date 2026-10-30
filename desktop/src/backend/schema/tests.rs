use super::*;
use serde_json::json;

#[test]
fn one_of_requires_exactly_one_match_and_rejects_empty_alternatives() {
    let ambiguous = json!({"type":"string","oneOf":[{"type":"string"},{"type":"string"}]});
    assert!(validate(&ambiguous, &json!("twice"), "args").is_err());
    assert!(validate(&json!({"type":"object","oneOf":[]}), &json!({}), "args").is_err());
}
