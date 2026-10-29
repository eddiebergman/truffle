use super::*;
use crate::backend::{resources, uuid};
use std::fs;
use std::path::PathBuf;
fn fixture() -> (
    tempfile::TempDir,
    std::sync::Arc<Repository>,
    String,
    PathBuf,
) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("checkout");
    std::fs::create_dir(&root).unwrap();
    let git = |args: &[&str]| {
        let result = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(args)
            .output()
            .unwrap();
        assert!(result.status.success());
    };
    git(&["init", "-b", "main"]);
    std::fs::write(root.join("code.txt"), "base\n").unwrap();
    git(&["add", "."]);
    git(&[
        "-c",
        "user.name=Test",
        "-c",
        "user.email=test@example.invalid",
        "commit",
        "-m",
        "fixture",
    ]);
    let next = temp.path().join("next");
    git(&["worktree", "add", "-b", "next", next.to_str().unwrap()]);
    let repo = Repository::open(temp.path().to_path_buf(), None, true).unwrap();
    let record = repo
        .transaction(|s| {
            workflows::save_session(
                s,
                &json!({"agent":"claude","title":"Context fixture","cwd":root,"itemIds":[]}),
            )
        })
        .unwrap();
    let id = text(&record, "id").to_owned();
    repo.transaction(|s|{
            s["sessions"][0]["items"]=json!([{"id":"one"},{"id":"two"}]);
            s["sessions"][0]["worktrees"]=json!([{"path":root,"primary":true,"itemIds":["one","two"]}]);
            s["artifacts"]=json!([{"id":"artifact-a","sessionIds":[id,"other"]},{"id":"artifact-b","sessionIds":[id]}]);
            resources::save(s,&id,&json!({"resources":[{"provider":"local","kind":"monitor","id":"old","state":"running","purpose":"Old monitor"}]}))?;Ok(())
        }).unwrap();
    (temp, repo, id, next.canonicalize().unwrap())
}
#[test]
fn replacement_changes_only_supplied_fields_and_preserves_other_sessions_artifacts() {
    let (_temp, repo, id, _) = fixture();
    replace(
            &repo,
            &id,
            &json!({"revision":read(&repo,&id).unwrap()["context"]["revision"],"resources":[],"artifactIds":["artifact-a"]}),
        )
        .unwrap();
    let state = repo.state.lock().unwrap();
    assert!(arr(&state["sessions"][0]["resources"]).is_empty());
    assert_eq!(arr(&state["sessions"][0]["worktrees"]).len(), 1);
    assert_eq!(arr(&state["sessions"][0]["items"]).len(), 2);
    assert_eq!(state["artifacts"][0]["sessionIds"], json!(["other", id]));
    assert_eq!(state["artifacts"][1]["sessionIds"], json!([]));
}
#[test]
fn worktree_replacement_updates_resume_path_and_prunes_removed_work_links() {
    let (_temp, repo, id, next) = fixture();
    replace(&repo,&id,&json!({"revision":read(&repo,&id).unwrap()["context"]["revision"],"worktrees":[{"path":next,"primary":true,"itemIds":["two"]}],"itemIds":["two"]})).unwrap();
    let state = repo.state.lock().unwrap();
    let record = &state["sessions"][0];
    assert_eq!(record["cwd"], json!(next));
    assert_eq!(arr(&record["worktrees"]).len(), 1);
    assert_eq!(record["worktrees"][0]["itemIds"], json!(["two"]));
    assert_eq!(record["items"], json!([{"id":"two"}]));
    assert_eq!(state["itemSettings"]["two"]["cwd"], json!(next));
    drop(state);
    replace(
        &repo,
        &id,
        &json!({"revision":read(&repo,&id).unwrap()["context"]["revision"],"itemIds":[]}),
    )
    .unwrap();
    assert_eq!(
        repo.state.lock().unwrap()["sessions"][0]["worktrees"][0]["itemIds"],
        json!([])
    );
}
#[test]
fn invalid_or_stale_replacement_rolls_back_every_change() {
    let (_temp, repo, id, next) = fixture();
    let before = repo.state.lock().unwrap().clone();
    for mut input in [
        json!({"resources":[],"worktrees":[{"path":next,"itemIds":["unknown"]}]}),
        json!({"resources":null}),
        json!({"worktrees":[{"path":next,"primary":true},{"path":next,"primary":true}]}),
        json!({"artifactIds":["missing"],"resources":[]}),
    ] {
        input["revision"] = read(&repo, &id).unwrap()["context"]["revision"].clone();
        let error = replace(&repo, &id, &input).unwrap_err();
        assert_ne!(
            error.status, 409,
            "Malformed input must reach its own validation"
        );
        assert_eq!(*repo.state.lock().unwrap(), before);
    }
    let old = read(&repo, &id).unwrap()["context"].clone();
    replace(
        &repo,
        &id,
        &json!({"revision":old["revision"],"itemIds":["one"]}),
    )
    .unwrap();
    assert!(replace(&repo, &id, &old)
        .unwrap_err()
        .message
        .contains("changed"));
}
#[test]
fn replacement_requires_captured_revision_and_preserves_state_on_missing_invalid_or_stale_input() {
    let (_temp, repo, id, _) = fixture();
    let bytes = std::fs::read(&repo.data).unwrap();
    let state = repo.state.lock().unwrap().clone();
    for revision in [
        None,
        Some(Value::Null),
        Some(json!(1)),
        Some(json!("")),
        Some(json!("a".repeat(63))),
        Some(json!("g".repeat(64))),
        Some(json!("0".repeat(64))),
    ] {
        let mut input = json!({"resources":[]});
        if let Some(revision) = revision {
            input["revision"] = revision;
        }
        assert_eq!(replace_receipt(&repo, &id, &input).unwrap_err().status, 409);
        assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
        assert_eq!(*repo.state.lock().unwrap(), state);
    }
    assert_eq!(
        workflows::register(
            &repo,
            &json!({"operation":"context-replace","id":id,"context":{"resources":[]}})
        )
        .unwrap_err()
        .status,
        409
    );
    let captured = read(&repo, &id).unwrap()["context"].clone();
    let result=workflows::register(&repo,&json!({"operation":"context-replace","id":id,"context":{"revision":captured["revision"],"resources":[]}})).unwrap();
    assert_eq!(result["id"], id);
    assert!(arr(&result["resources"]).is_empty());
}
#[test]
fn mutation_receipt_binds_undo_to_exact_commit_and_excludes_private_record_data() {
    let (_temp, repo, id, _) = fixture();
    repo.transaction(|s| {
        s["sessions"][0]["privateExtension"] = json!("private-receipt-marker");
        Ok(())
    })
    .unwrap();
    let before = read(&repo, &id).unwrap()["context"].clone();
    let receipt = replace_receipt(
        &repo,
        &id,
        &json!({"revision":before["revision"],"resources":[]}),
    )
    .unwrap();
    assert_eq!(receipt.as_object().unwrap().len(), 2);
    assert_eq!(receipt["id"], id);
    assert_eq!(
        receipt["revision"],
        read(&repo, &id).unwrap()["context"]["revision"]
    );
    assert!(!receipt.to_string().contains("private-receipt-marker"));
    repo.transaction(|s| resources::save(s,&id,&json!({"resources":[{"provider":"local","kind":"monitor","id":"concurrent","state":"running"}]}))).unwrap();
    let bytes = std::fs::read(&repo.data).unwrap();
    let error = replace_receipt(
        &repo,
        &id,
        &json!({"revision":receipt["revision"],"resources":before["resources"]}),
    )
    .unwrap_err();
    assert_eq!(error.status, 409);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    assert_eq!(
        read(&repo, &id).unwrap()["context"]["resources"][0]["id"],
        "concurrent"
    );
    let fresh = read(&repo, &id).unwrap()["context"].clone();
    let restored = replace_receipt(
        &repo,
        &id,
        &json!({"revision":fresh["revision"],"resources":before["resources"]}),
    )
    .unwrap();
    assert_eq!(
        restored["revision"],
        read(&repo, &id).unwrap()["context"]["revision"]
    );
    assert_eq!(
        read(&repo, &id).unwrap()["context"]["resources"],
        before["resources"]
    );
}
#[test]
fn resource_replacement_drops_stale_metadata_instead_of_merging_it() {
    let (_temp, repo, id, _) = fixture();
    resources::apply(&repo,&id,&json!({"replace":true,"revision":read(&repo,&id).unwrap()["context"]["revision"],"resources":[{"provider":"local","kind":"monitor","id":"old","state":"completed"}]})).unwrap();
    let record = read(&repo, &id).unwrap();
    assert_eq!(record["context"]["resources"][0]["purpose"], "");
    assert_eq!(record["context"]["resources"][0]["state"], "completed");
    let reply = workflows::registration_reply(&record);
    assert_eq!(reply["context"], record["context"]);
}
#[test]
fn link_work_adds_latest_relationships_only_with_exact_receipt_and_atomic_rollback() {
    let (_temp, repo, id, _) = fixture();
    repo.transaction(|s| {
        s["items"] =
            json!([{"id":"three","source":"linear","title":"Third","detail":"private-detail"}]);
        s["itemSettings"]["three"] = json!({"cwd":"unchanged"});
        s["sessions"][0]["title"] = json!("Newer title");
        s["sessions"][0]["instruction"] = json!("private-instructions");
        Ok(())
    })
    .unwrap();
    let before = repo.state.lock().unwrap().clone();
    let receipt = link_work(&repo, &id, &json!(["three", "three", "one"])).unwrap();
    let after = repo.state.lock().unwrap().clone();
    let mut expected = before["sessions"][0].clone();
    expected["items"] = after["sessions"][0]["items"].clone();
    expected["updatedAt"] = after["sessions"][0]["updatedAt"].clone();
    assert_eq!(expected, after["sessions"][0]);
    assert_eq!(before["itemSettings"], after["itemSettings"]);
    assert_eq!(before["items"], after["items"]);
    assert_eq!(after["sessions"][0]["items"].as_array().unwrap().len(), 3);
    assert_eq!(receipt["addedItemIds"], json!(["three"]));
    assert_eq!(receipt["itemIds"], json!(["three", "one"]));
    assert_eq!(
        receipt["revision"],
        read(&repo, &id).unwrap()["context"]["revision"]
    );
    assert_eq!(receipt.as_object().unwrap().len(), 6);
    assert!(!receipt.to_string().contains("private"));
    assert_eq!(
        link_work(&repo, &id, &json!(["three"])).unwrap()["addedItemIds"],
        json!([])
    );
    for ids in [
        json!([]),
        json!(["unknown"]),
        json!([3]),
        json!([" "]),
        json!(["one\n"]),
        json!(vec!["one"; 101]),
    ] {
        let before = repo.state.lock().unwrap().clone();
        let disk = fs::read(&repo.data).unwrap();
        assert!(link_work(&repo, &id, &ids).is_err());
        assert_eq!(*repo.state.lock().unwrap(), before);
        assert_eq!(fs::read(&repo.data).unwrap(), disk);
    }
    let before = repo.state.lock().unwrap().clone();
    fs::create_dir(repo.data.with_extension("tmp")).unwrap();
    assert!(link_work(&repo, &id, &json!(["one"])).is_err());
    assert_eq!(*repo.state.lock().unwrap(), before);
}
#[test]
fn link_work_url_reads_outside_lock_merges_latest_and_rejects_before_resolution() {
    let (_temp, repo, id, _) = fixture();
    let url = "https://github.com/example/project/issues/17";
    let receipt=link_work_url_with(&repo,&id,url,|_|{assert!(repo.state.try_lock().is_ok());repo.transaction(|s|{s["sessions"][0]["title"]=json!("Changed during provider read");s["sessions"][0]["items"].as_array_mut().unwrap().push(json!({"id":"concurrent"}));Ok(())})?;Ok(json!({"id":"github:17","url":url,"source":"github","title":"Imported","detail":"private-provider-detail"}))}).unwrap();
    assert_eq!(receipt["addedItemIds"], json!(["github:17"]));
    let state = repo.state.lock().unwrap().clone();
    assert_eq!(
        state["sessions"][0]["title"],
        "Changed during provider read"
    );
    assert_eq!(arr(&state["sessions"][0]["items"]).len(), 4);
    assert_eq!(
        receipt["revision"],
        read(&repo, &id).unwrap()["context"]["revision"]
    );
    assert_eq!(
        link_work_url_with(&repo, &id, url, |_| panic!("cached URL must not resolve")).unwrap()
            ["addedItemIds"],
        json!([])
    );
    for url in [
        "http://github.com/example/project/issues/17",
        "https://example.invalid/17",
        "https://user:password@github.com/example/project/issues/17",
        "https://github.com/example/project/issues/",
        "https://github.com:444/example/project/issues/17",
    ] {
        assert!(
            link_work_url_with(&repo, &id, url, |_| panic!("invalid URL must not resolve"))
                .is_err()
        );
    }
    assert!(link_work_url_with(&repo, &uuid(), url, |_| panic!(
        "unknown chat must not resolve"
    ))
    .is_err());
    let before = repo.state.lock().unwrap().clone();
    assert!(link_work_url_with(
        &repo,
        &id,
        "https://linear.app/team/issue/TEAM-12",
        |_| Err(Error::new("Fixture read failed"))
    )
    .is_err());
    assert_eq!(*repo.state.lock().unwrap(), before);
    repo.transaction(|s| {
        s["sessions"][0]["items"] = json!((0..100)
            .map(|n| json!({"id":format!("cached:{n}")}))
            .collect::<Vec<_>>());
        Ok(())
    })
    .unwrap();
    let before = repo.state.lock().unwrap().clone();
    assert!(
        link_work_url_with(&repo, &id, "https://linear.app/team/issue/TEAM-12", |_| Ok(
            json!({"id":"new","url":"https://linear.app/team/issue/TEAM-12"})
        ))
        .is_err()
    );
    assert_eq!(*repo.state.lock().unwrap(), before);
}
