use super::*;
use serde_json::json;

#[test]
fn recursive_redaction_uses_private_profile_credentials() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join(".env"),
        "TRUFFLE_TEST_SECRET=benign-private-command-sentinel",
    )
    .unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let mut value =
        json!({"rows":[{"message":"benign-private-command-sentinel","token":"hidden"}]});
    redact(&repo, &mut value);
    assert_eq!(value, json!({"rows":[{"message":"[redacted]"}]}));
}
