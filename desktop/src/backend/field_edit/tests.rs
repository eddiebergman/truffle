use super::*;
use std::sync::Arc;

struct Note {
    id: String,
    title: &'static str,
    body: &'static str,
}
impl Record for Note {
    const NAME: &'static str = "note";
    const FIELDS: &'static [&'static str] = &["title", "body"];
    fn id(&self) -> &str {
        &self.id
    }
    fn projection(&self, field: &str) -> Result<Value> {
        Ok(json!(if field == "title" {
            self.title
        } else {
            self.body
        }))
    }
    fn normalize(_: &Repository, _: &str, value: &Value) -> Result<Value> {
        value
            .as_str()
            .map(|value| json!(value.trim()))
            .ok_or_else(|| Error::new("Note fields are text."))
    }
}
fn note(id: &str) -> Note {
    Note {
        id: id.into(),
        title: "Title",
        body: "Body",
    }
}
fn fixture(secret: &str) -> (tempfile::TempDir, Arc<Repository>) {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join(".env"),
        format!("TRUFFLE_TEST_SECRET={secret}\n"),
    )
    .unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    (temp, repo)
}
fn edit(repo: &Repository, patch: &Value, expected: &Value) -> Result<Edit<Note>> {
    Edit::new(repo, patch, expected)
}

#[test]
fn apply_writes_patched_fields_with_current_tokens_and_reports_changes() {
    let (_temp, repo) = fixture("unused-field-edit-secret");
    let record = note("note-1");
    let expected = tokens(&repo, &record).unwrap();
    let edit = edit(
        &repo,
        &json!({"title":" New ","body":"Body"}),
        &json!({"title":expected["title"],"body":expected["body"]}),
    )
    .unwrap();
    let mut written = vec![];
    let changed = edit
        .apply(&record, |field, value| {
            written.push((field.to_owned(), value.clone()));
            Ok(*value != record.projection(field)?)
        })
        .unwrap();
    assert_eq!(changed, ["title"]);
    assert_eq!(
        written,
        [
            ("body".to_owned(), json!("Body")),
            ("title".to_owned(), json!("New"))
        ]
    );
}

#[test]
fn a_stale_token_is_refused_with_409_before_any_write() {
    let (_temp, repo) = fixture("unused-field-edit-secret");
    let expected = tokens(&repo, &note("note-1")).unwrap();
    let mut current = note("note-1");
    current.title = "Edited elsewhere";
    let edit = edit(
        &repo,
        &json!({"body":"Mine","title":"Mine"}),
        &json!({"body":expected["body"],"title":expected["title"]}),
    )
    .unwrap();
    let error = edit
        .apply(&current, |_, _| panic!("A stale edit must not write."))
        .unwrap_err();
    assert_eq!(error.status, 409);
}

#[test]
fn tokens_cover_the_record_id_and_field_name() {
    let (_temp, repo) = fixture("unused-field-edit-secret");
    let mut same = note("note-1");
    same.body = same.title;
    let first = tokens(&repo, &same).unwrap();
    assert_ne!(first["title"], first["body"]);
    let mut other = note("note-2");
    other.body = other.title;
    assert_ne!(tokens(&repo, &other).unwrap()["title"], first["title"]);
    let edit = edit(
        &repo,
        &json!({"title":"New"}),
        &json!({"title":first["title"]}),
    )
    .unwrap();
    assert_eq!(edit.check(&other).unwrap_err().status, 409);
}

#[test]
fn the_patch_must_be_nonempty_known_and_match_the_expected_keys() {
    let (_temp, repo) = fixture("unused-field-edit-secret");
    let expected = tokens(&repo, &note("note-1")).unwrap();
    let title = &expected["title"];
    for (patch, captured) in [
        (json!({}), json!({})),
        (json!({"title":"New"}), json!({})),
        (json!({"title":"New"}), expected.clone()),
        (json!({"title":"New"}), json!({"body":title})),
        (json!({"title":"New","body":"New"}), json!({"title":title})),
        (json!({"owner":"New"}), json!({"owner":title})),
        (json!({"title":7}), json!({"title":title})),
        (json!(["title"]), json!({"title":title})),
        (json!({"title":"New"}), json!(["title"])),
    ] {
        assert_eq!(edit(&repo, &patch, &captured).err().unwrap().status, 400);
    }
}

#[test]
fn a_token_must_be_64_hex_characters() {
    let (_temp, repo) = fixture("unused-field-edit-secret");
    for token in [
        json!("0".repeat(63)),
        json!("0".repeat(65)),
        json!("g".repeat(64)),
        json!(7),
        json!(null),
    ] {
        assert!(edit(&repo, &json!({"title":"New"}), &json!({"title":token})).is_err());
    }
    assert!(edit(
        &repo,
        &json!({"title":"New"}),
        &json!({"title":"aB".repeat(32)})
    )
    .is_ok());
}

#[test]
fn values_changed_by_redaction_or_with_control_characters_are_refused() {
    let token = "c".repeat(64);
    let (_temp, repo) = fixture(&token);
    let error = edit(&repo, &json!({"title":"New"}), &json!({"title":token}))
        .err()
        .unwrap();
    assert!(!error.message.contains(&token));
    let (_temp, repo) = fixture("private-note-identity");
    assert!(tokens(&repo, &note("private-note-identity")).is_err());
    assert!(tokens(&repo, &note("note\n1")).is_err());
    assert!(tokens(&repo, &note("note-1")).is_ok());
    assert!(exact(&repo, "tab\tseparated").is_err());
}
