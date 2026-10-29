use super::*;

fn chat(s: &mut Value, dir: &tempfile::TempDir, title: &str) -> Value {
    save(
        s,
        &json!({"agent":"codex","title":title,"cwd":dir.path(),"itemIds":[]}),
    )
    .unwrap()
}
#[test]
fn lookup_rejects_malformed_and_unknown_ids_with_one_message() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = json!({"sessions":[]});
    let saved = chat(&mut s, &dir, "Known");
    for id in ["not-a-uuid", "", &uuid()] {
        assert_eq!(get(&s, id).unwrap_err().message, "Chat unavailable.");
        assert_eq!(
            get_mut(&mut s, id).unwrap_err().message,
            "Chat unavailable."
        );
    }
    assert_eq!(get(&s, text(&saved, "id")).unwrap(), &saved);
}
#[test]
fn save_returns_the_stored_record_with_its_extension_and_appends_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = json!({"sessions":[]});
    let first = chat(&mut s, &dir, "First");
    let second = save_with(
        &mut s,
        &json!({"agent":"claude","title":"Second","cwd":dir.path(),"itemIds":[]}),
        |record| record["workflow"] = json!("work-review"),
    )
    .unwrap();
    assert_eq!(second["workflow"], "work-review");
    assert_eq!(all(&s), [first.clone(), second.clone()]);
    let renamed = save(&mut s, &json!({"id":first["id"],"title":"Renamed"})).unwrap();
    assert_eq!(renamed["title"], "Renamed");
    assert_eq!(all(&s), [second, renamed]);
}
#[test]
fn saving_an_unknown_id_creates_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = json!({"sessions":[]});
    let error = save(
        &mut s,
        &json!({"id":uuid(),"agent":"codex","title":"Gone","cwd":dir.path()}),
    )
    .unwrap_err();
    assert_eq!(error.message, "Chat unavailable.");
    assert_eq!(all(&s), [] as [Value; 0]);
}
#[test]
fn unknown_fields_survive_save_and_get_mut() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = json!({"sessions":[]});
    let id = chat(&mut s, &dir, "Fields")["id"].clone();
    get_mut(&mut s, id.as_str().unwrap()).unwrap()["futureNs"] = json!({"version":3});
    save(&mut s, &json!({"id":id,"title":"Renamed"})).unwrap();
    let record = get(&s, id.as_str().unwrap()).unwrap();
    assert_eq!(record["futureNs"], json!({"version":3}));
    assert_eq!(record["title"], "Renamed");
}
#[test]
fn find_and_latest_search_from_opposite_ends() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = json!({"sessions":[]});
    let first = chat(&mut s, &dir, "Same");
    let second = chat(&mut s, &dir, "Same");
    assert_eq!(find(&s, |r| r["title"] == "Same"), Some(&first));
    assert_eq!(latest(&s, |r| r["title"] == "Same"), Some(&second));
    assert_eq!(find(&s, |r| r["title"] == "Other"), None);
}
#[test]
fn public_projection_removes_only_private_fields() {
    let mut record = json!({"id":"x","title":"T","reviewDraft":{},"reviewHistory":{},"preparationRequests":{},"linkExtension":1});
    public(&mut record);
    assert_eq!(record, json!({"id":"x","title":"T","linkExtension":1}));
}
