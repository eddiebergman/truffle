use super::*;
fn fixture() -> (
    tempfile::TempDir,
    std::sync::Arc<Repository>,
    String,
    String,
    PathBuf,
) {
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().join("project");
    std::fs::create_dir_all(cwd.join("reports")).unwrap();
    std::fs::write(cwd.join("skill.md"), "Fixture skill").unwrap();
    let file = cwd.join("reports/report.html");
    std::fs::write(&file, "<p>Fixture result</p>").unwrap();
    let first = uuid();
    let second = uuid();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    repo.transaction(|state|{state["sessions"]=json!([{"id":first,"cwd":cwd,"items":[],"newsletterId":"first"},{"id":second,"cwd":cwd,"items":[],"newsletterId":"second"}]);state["skillPresets"]=json!([{"id":"first","cwd":cwd,"skill":"skill.md","reports":"reports"},{"id":"second","cwd":cwd,"skill":"skill.md","reports":"reports"}]);Ok(())}).unwrap();
    (temp, repo, first, second, file)
}
#[test]
fn attachment_identity_and_explicit_skill_provenance_survive_reattachment_and_legacy_cli() {
    let (_temp, repo, first, second, file) = fixture();
    let result = attach_to_chat(
        &repo,
        &first,
        &json!({"path":file,"title":"Produced report","skillOutput":true}),
    )
    .unwrap();
    assert_eq!(result["role"], "skill-output");
    assert_eq!(result["presetId"], "first");
    assert_eq!(result["snapshot"], true);
    assert_eq!(result.as_object().unwrap().len(), 8);
    assert!(!result.to_string().contains("sourcePath"));
    assert!(!result.to_string().contains("project/reports"));
    assert_eq!(
        attach_to_chat(&repo, &second, &json!({"path":file,"title":"Reattached"}))
            .unwrap_err()
            .status,
        409
    );
    let again = attach_to_chat(&repo, &second, &json!({"path":file})).unwrap();
    assert_eq!(result["artifactId"], again["artifactId"]);
    assert_eq!(again["presetId"], "first");
    assert_eq!(again["role"], "skill-output");
    let registration = super::super::workflows::register(
        &repo,
        &json!({"operation":"save-artifact","id":second,"path":file}),
    )
    .unwrap();
    let reply = super::super::workflows::registration_reply(&registration);
    assert_eq!(reply["id"], second);
    assert_eq!(reply["artifactReceipt"]["artifactId"], result["artifactId"]);
    assert_eq!(reply["artifactReceipt"]["presetId"], "first");
    let state = repo.state.lock().unwrap();
    assert_eq!(arr(&state["artifacts"]).len(), 1);
    let entry = &state["artifacts"][0];
    assert_eq!(arr(&entry["sessionIds"]).len(), 2);
    assert_eq!(
        std::fs::read(text(entry, "path")).unwrap(),
        b"<p>Fixture result</p>"
    );
}
#[test]
fn attachment_urls_do_not_fetch_and_owner_rejects_invalid_files_targets_and_provenance() {
    let (temp, repo, first, _, file) = fixture();
    let link = attach_to_chat(
        &repo,
        &first,
        &json!({"url":"https://example.invalid/result"}),
    )
    .unwrap();
    assert_eq!(link["format"], "link");
    assert_eq!(link["snapshot"], false);
    assert_eq!(link["presetId"], Value::Null);
    assert_eq!(link["role"], "attachment");
    let outside = temp.path().join("outside.md");
    std::fs::write(&outside, "not in a registered root").unwrap();
    let hidden = file.parent().unwrap().join(".private.md");
    std::fs::write(&hidden, "private").unwrap();
    let oversized = file.parent().unwrap().join("large.md");
    std::fs::File::create(&oversized)
        .unwrap()
        .set_len(10 * 1024 * 1024 + 1)
        .unwrap();
    let plain = uuid();
    repo.transaction(|state| {
        state["sessions"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":plain,"cwd":file.parent().unwrap(),"items":[]}));
        Ok(())
    })
    .unwrap();
    let before = repo.state.lock().unwrap().clone();
    let disk = std::fs::read(&repo.data).unwrap();
    for (id, input) in [
        (
            first.clone(),
            json!({"path":file,"url":"https://example.invalid/result"}),
        ),
        (
            first.clone(),
            json!({"url":"http://example.invalid/result"}),
        ),
        (
            first.clone(),
            json!({"url":"https://user:secret@example.invalid/result"}),
        ),
        (first.clone(), json!({"path":outside})),
        (first.clone(), json!({"path":hidden})),
        (first.clone(), json!({"path":oversized})),
        (first.clone(), json!({"path":file,"skillOutput":"true"})),
        (plain, json!({"path":file,"skillOutput":true})),
        (uuid(), json!({"path":file})),
    ] {
        assert!(attach_to_chat(&repo, &id, &input).is_err());
        assert_eq!(*repo.state.lock().unwrap(), before);
        assert_eq!(std::fs::read(&repo.data).unwrap(), disk);
    }
}
#[test]
fn failed_attachment_metadata_commit_returns_no_saved_receipt_or_state_change() {
    let (_temp, repo, first, _, file) = fixture();
    let before = repo.state.lock().unwrap().clone();
    let disk = std::fs::read(&repo.data).unwrap();
    std::fs::create_dir(repo.data.with_extension("tmp")).unwrap();
    assert!(attach_to_chat(&repo, &first, &json!({"path":file})).is_err());
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), disk);
}
