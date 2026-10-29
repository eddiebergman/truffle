use super::*;
use crate::backend::workflows;
#[test]
fn resources_keep_open_metadata_and_update_without_erasing_other_resources() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let record = repo
        .transaction(|s| {
            workflows::save_session(
                s,
                &json!({"agent":"claude","title":"Resource test","cwd":temp.path(),"itemIds":[]}),
            )
        })
        .unwrap();
    let report = json!({"resources":[{"provider":"gcp","id":"gpu-1","kind":"gpu","state":"running","purpose":"Bench","uris":{"console":["https://example.com/gpu"]},"data":{"arbitrary":{"gpuCount":1}},"cleanup":"Ask first"},{"provider":"local","identifier":"monitor-1","kind":"monitor","state":"running"}]});
    repo.transaction(|s| save(s, text(&record, "id"), &report))
        .unwrap();
    let updated=repo.transaction(|s|save(s,text(&record,"id"),&json!({"resources":[{"provider":"gcp","id":"gpu-1","kind":"gpu","state":"stopped"}]}))).unwrap();
    assert_eq!(updated["resources"][0]["state"], "stopped");
    assert_eq!(updated["resources"][0]["purpose"], "Bench");
    assert_eq!(updated["resources"][0]["data"]["arbitrary"]["gpuCount"], 1);
    assert_eq!(arr(&updated["resources"]).len(), 2);
    repo.transaction(|s| save(s, text(&record, "id"), &json!({"resources":[]})))
        .unwrap();
    assert_eq!(
        arr(&repo.snapshot(false)["sessions"][0]["resources"]).len(),
        2
    );
}
#[test]
fn bad_resource_reports_do_not_partially_modify_sessions() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let record = repo
        .transaction(|s| {
            workflows::save_session(
                s,
                &json!({"agent":"codex","title":"Resources","cwd":temp.path()}),
            )
        })
        .unwrap();
    assert!(repo.transaction(|s|save(s,text(&record,"id"),&json!({"resources":[{"provider":"gcp","id":"ok","kind":"vm"},{"provider":"gcp","kind":"vm"}]}))).is_err());
    assert!(arr(&repo.snapshot(false)["sessions"][0]["resources"]).is_empty());
}
fn replacement_fixture() -> (tempfile::TempDir, std::sync::Arc<Repository>, String) {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let record=repo.transaction(|s|workflows::save_session(s,&json!({"agent":"claude","title":"Replacement fixture","cwd":temp.path(),"itemIds":[]}))).unwrap();
    let id = text(&record, "id").to_owned();
    apply(&repo,&id,&json!({"resources":[{"id":"A","provider":"local","kind":"monitor","purpose":"Keep history"}]})).unwrap();
    (temp, repo, id)
}
#[test]
fn resource_context_undo_preserves_exact_captured_report_time_without_clock_assumptions() {
    let (_temp, repo, id) = replacement_fixture();
    let old = "2001-02-03T04:05:06+00:00";
    repo.transaction(|s| {
        s["sessions"][0]["resources"][0]["reportedAt"] = json!(old);
        s["sessions"][0]["resources"][0]["privateExtension"] = json!({"keep":true});
        Ok(())
    })
    .unwrap();
    let before = super::super::session_context::read(&repo, &id).unwrap()["context"].clone();
    let cleared = super::super::session_context::replace_receipt(
        &repo,
        &id,
        &json!({"revision":before["revision"],"resources":[]}),
    )
    .unwrap();
    assert!(
        arr(&super::super::session_context::read(&repo, &id).unwrap()["context"]["resources"])
            .is_empty()
    );
    let restored = super::super::session_context::replace_receipt(
        &repo,
        &id,
        &json!({"revision":cleared["revision"],"resources":before["resources"]}),
    )
    .unwrap();
    let after = super::super::session_context::read(&repo, &id).unwrap()["context"].clone();
    assert_eq!(after["resources"], before["resources"]);
    assert_eq!(after["revision"], before["revision"]);
    assert_eq!(restored["revision"], after["revision"]);
    apply(
        &repo,
        &id,
        &json!({"resources":[{"id":"A","provider":"local","kind":"monitor","reportedAt":old}]}),
    )
    .unwrap();
    assert_ne!(
        super::super::session_context::read(&repo, &id).unwrap()["context"]["resources"][0]
            ["reportedAt"],
        old
    );
}
#[test]
fn resource_replacement_preserves_only_valid_bounded_rfc3339_report_times() {
    let (_temp, repo, id) = replacement_fixture();
    for time in [
        Value::Null,
        json!(7),
        json!("not-a-time"),
        json!("2001-02-30T04:05:06Z"),
        json!("2001-02-03"),
        json!("x".repeat(101)),
        json!("2001-02-03T04:05:06.123+02:00"),
    ] {
        let before = super::super::session_context::read(&repo, &id).unwrap()["context"].clone();
        let valid = time.as_str().is_some_and(|value| {
            value.len() <= 100 && chrono::DateTime::parse_from_rfc3339(value).is_ok()
        });
        apply(&repo,&id,&json!({"replace":true,"revision":before["revision"],"resources":[{"id":"A","provider":"local","kind":"monitor","reportedAt":time}]})).unwrap();
        let current = super::super::session_context::read(&repo, &id).unwrap()["context"]
            ["resources"][0]["reportedAt"]
            .clone();
        if valid {
            assert_eq!(current, time);
        } else {
            assert_ne!(current, time);
            assert!(chrono::DateTime::parse_from_rfc3339(current.as_str().unwrap()).is_ok());
        }
    }
}
#[test]
fn destructive_reports_share_context_cas_and_transaction_local_save_cannot_bypass_it() {
    let (_temp, repo, id) = replacement_fixture();
    let captured =
        super::super::session_context::read(&repo, &id).unwrap()["context"]["revision"].clone();
    apply(
        &repo,
        &id,
        &json!({"resources":[{"id":"B","provider":"local","kind":"monitor"}]}),
    )
    .unwrap();
    let bytes = std::fs::read(&repo.data).unwrap();
    let state = repo.state.lock().unwrap().clone();
    for revision in [None, Some(json!(false)), Some(json!("bad")), Some(captured)] {
        let mut report = json!({"replace":true,"resources":[]});
        if let Some(revision) = revision {
            report["revision"] = revision;
        }
        assert_eq!(
            workflows::register(
                &repo,
                &json!({"operation":"resources","id":id,"report":report})
            )
            .unwrap_err()
            .status,
            409
        );
        assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
        assert_eq!(*repo.state.lock().unwrap(), state);
    }
    assert!(repo
        .transaction(|s| save(s, &id, &json!({"replace":true,"resources":[]})))
        .is_err());
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    let fresh =
        super::super::session_context::read(&repo, &id).unwrap()["context"]["revision"].clone();
    let record=workflows::register(&repo,&json!({"operation":"resources","id":id,"report":{"replace":true,"revision":fresh,"resources":[{"id":"A","provider":"local","kind":"monitor","state":"completed"}]}})).unwrap();
    assert_eq!(arr(&record["resources"]).len(), 1);
    assert_eq!(record["resources"][0]["id"], "A");
    assert_eq!(record["resources"][0]["purpose"], "");
    assert!(super::super::workflows::context(&repo, &id)
        .unwrap()
        .contains("revision is required"));
}
#[test]
fn complete_replacements_can_detach_from_stored_128_capacity_without_widening_incremental_reports()
{
    let (_temp, repo, id) = replacement_fixture();
    let entries=(0..65).map(|index|json!({"id":format!("resource-{index}"),"provider":"local","kind":"monitor","purpose":format!("Purpose {index}"),"extension":{"keep":index}})).collect::<Vec<_>>();
    for chunk in entries.chunks(64) {
        apply(&repo, &id, &json!({"resources":chunk})).unwrap();
    }
    let context = super::super::session_context::read(&repo, &id).unwrap()["context"].clone();
    assert_eq!(arr(&context["resources"]).len(), 66);
    let retained = arr(&context["resources"])
        .into_iter()
        .skip(1)
        .collect::<Vec<_>>();
    let receipt = super::super::session_context::replace_receipt(
        &repo,
        &id,
        &json!({"revision":context["revision"],"resources":retained}),
    )
    .unwrap();
    let current = super::super::session_context::read(&repo, &id).unwrap()["context"].clone();
    assert_eq!(arr(&current["resources"]).len(), 65);
    assert_eq!(receipt["revision"], current["revision"]);
    for (index, row) in arr(&current["resources"]).iter().enumerate() {
        assert_eq!(row["purpose"], format!("Purpose {index}"));
        assert_eq!(row["extension"]["keep"], index);
    }
    let bytes = std::fs::read(&repo.data).unwrap();
    assert!(apply(&repo, &id, &json!({"resources":entries})).is_err());
    let over = (0..129)
        .map(|index| json!({"id":format!("too-many-{index}"),"provider":"local","kind":"monitor"}))
        .collect::<Vec<_>>();
    assert!(super::super::session_context::replace_receipt(
        &repo,
        &id,
        &json!({"revision":current["revision"],"resources":over})
    )
    .is_err());
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
}
#[test]
fn pending_destructive_reports_keep_bytes_and_pending_identity_until_valid_revision_is_supplied() {
    let (temp, repo, id) = replacement_fixture();
    let folder = temp.path().join("data/resource-reports-pending");
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("replace.json");
    let state = repo.state.lock().unwrap().clone();
    let disk = std::fs::read(&repo.data).unwrap();
    for revision in [None, Some(json!("0".repeat(64)))] {
        let mut report = json!({"replace":true,"resources":[]});
        if let Some(revision) = revision {
            report["revision"] = revision;
        }
        let bytes = serde_json::to_vec(&json!({"recordId":id,"report":report})).unwrap();
        std::fs::write(&file, &bytes).unwrap();
        import_pending(&repo);
        assert_eq!(std::fs::read(&file).unwrap(), bytes);
        assert!(!file.with_extension("imported").exists());
        assert_eq!(std::fs::read(&repo.data).unwrap(), disk);
        assert_eq!(*repo.state.lock().unwrap(), state);
    }
    let revision =
        super::super::session_context::read(&repo, &id).unwrap()["context"]["revision"].clone();
    std::fs::write(
        &file,
        serde_json::to_vec(
            &json!({"recordId":id,"report":{"replace":true,"revision":revision,"resources":[]}}),
        )
        .unwrap(),
    )
    .unwrap();
    import_pending(&repo);
    assert!(!file.exists());
    assert!(file.with_extension("imported").exists());
    assert!(arr(&repo.state.lock().unwrap()["sessions"][0]["resources"]).is_empty());
}
#[test]
fn staged_reports_import_once_and_new_state_does_not_inherit_old_observation() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let record = repo
        .transaction(|s| {
            workflows::save_session(
                s,
                &json!({"agent":"claude","title":"Imported resources","cwd":temp.path()}),
            )
        })
        .unwrap();
    let id = text(&record, "id").to_string();
    drop(repo);
    let folder = temp.path().join("data/resource-reports-pending");
    std::fs::create_dir_all(&folder).unwrap();
    let file = folder.join("report.json");
    std::fs::write(&file,serde_json::to_vec(&json!({"recordId":id,"report":{"resources":[{"id":"vm-1","name":"Friendly name","provider":"gcp","kind":"vm","state":"running","verification":"checked","observedAt":"2026-10-23T10:00:00Z"}]}})).unwrap()).unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    assert!(!file.exists());
    assert!(file.with_extension("imported").exists());
    let updated=workflows::register(&repo,&json!({"id":id,"operation":"resources","report":{"resources":[{"id":"vm-1","provider":"gcp","kind":"vm","state":"stopped"}]}})).unwrap();
    assert_eq!(updated["resources"][0]["name"], "Friendly name");
    assert_eq!(updated["resources"][0]["verification"], "reported");
    assert!(updated["resources"][0]["observedAt"].is_null());
    drop(repo);
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    assert_eq!(
        repo.snapshot(false)["sessions"][0]["resources"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        repo.snapshot(false)["sessions"][0]["resources"][0]["state"],
        "stopped"
    );
}
