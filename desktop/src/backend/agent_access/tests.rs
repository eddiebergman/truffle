use super::*;
use crate::backend::uuid;
fn invoke(repo: &std::sync::Arc<Repository>, name: &str, args: Value) -> Result<Value> {
    let mut request = json!({"name":name});
    request["args"] = args;
    super::super::commands::invoke(repo, &request, true)
}
#[test]
fn artifact_pages_use_the_redacted_stream_at_every_cursor() {
    let temp = tempfile::tempdir().unwrap();
    let sentinel = "benign-split-secret-0123456789";
    std::fs::write(
        temp.path().join(".env"),
        format!("ACCESS_TEST_SECRET={sentinel}"),
    )
    .unwrap();
    let project = temp.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let record = uuid();
    repo.transaction(|state| {
        state["sessions"] = json!([{"id":record,"cwd":project,"items":[]}]);
        Ok(())
    })
    .unwrap();
    let path = project.join("secret.md");
    let raw = format!("abc{sentinel}é end");
    std::fs::write(&path, &raw).unwrap();
    repo.edit(&json!({"action":"save-artifact","path":path,"sessionIds":[record]}))
        .unwrap();
    let id = repo.state.lock().unwrap()["artifacts"][0]["id"].clone();
    let expected = "abc[redacted]é end";
    for offset in 0..=raw.len() {
        let page = invoke(
            &repo,
            "artifacts.read-text",
            json!({"id":id,"offset":offset,"maxBytes":4}),
        );
        if offset > expected.len() || !expected.is_char_boundary(offset) {
            assert!(page.is_err());
            continue;
        }
        let page = page.unwrap();
        assert_eq!(page["offsetBasis"], "redacted-utf8");
        let chunk = page["text"].as_str().unwrap();
        assert!(expected[offset..].starts_with(chunk));
        assert!(chunk.len() <= 4);
    }
    let mut combined = String::new();
    let mut offset = 0;
    loop {
        let page = invoke(
            &repo,
            "artifacts.read-text",
            json!({"id":id,"offset":offset,"maxBytes":4}),
        )
        .unwrap();
        combined.push_str(page["text"].as_str().unwrap());
        match page["nextOffset"].as_u64() {
            Some(next) => {
                assert!(next > offset);
                offset = next;
            }
            None => break,
        }
    }
    assert_eq!(combined, expected);
}

#[test]
fn summaries_and_output_tails_redact_before_their_boundaries() {
    let temp = tempfile::tempdir().unwrap();
    let sentinel = "benign-boundary-secret-0123456789";
    std::fs::write(
        temp.path().join(".env"),
        format!("ACCESS_TEST_SECRET={sentinel}"),
    )
    .unwrap();
    let data = temp.path().join("data");
    std::fs::create_dir(&data).unwrap();
    let mut jobs = json!({});
    for suffix in [1980, 1990, 1999, 2000, 2001] {
        jobs[suffix.to_string()] = json!({"id":suffix.to_string(),"actionId":"fixture","title":"Boundary","state":"succeeded","startedAt":suffix,"output":format!("{}{sentinel}{}","p".repeat(3000),"z".repeat(suffix))});
    }
    std::fs::write(
        data.join("quick-actions.json"),
        serde_json::to_vec(&jobs).unwrap(),
    )
    .unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    for suffix in [1980, 1990, 1999, 2000, 2001] {
        let result = invoke(
            &repo,
            "quick-actions.outputs",
            json!({"id":suffix.to_string()}),
        )
        .unwrap();
        let expected = format!("{}[redacted]{}", "p".repeat(3000), "z".repeat(suffix));
        assert_eq!(
            result["jobs"][0]["output"],
            expected
                .chars()
                .skip(expected.chars().count() - OUTPUT_CHARS)
                .collect::<String>()
        );
    }
    let title = format!("{}{sentinel}", "t".repeat(295));
    let message = format!("{}{sentinel}", "m".repeat(595));
    {
        let mut sharing = repo.sharing.lock().unwrap();
        sharing["entries"] =
            json!({"entry":{"id":"entry","title":title,"message":message,"sender":{"name":title}}});
        sharing["jobs"] = json!({"job":{"id":"job","at":1,"summary":title}});
    }
    let inbox = invoke(&repo, "sharing.inbox", json!({})).unwrap();
    let expected_title = format!("{}[reda", "t".repeat(295));
    assert_eq!(inbox["entries"][0]["title"], expected_title);
    assert_eq!(inbox["entries"][0]["sender"]["name"], expected_title);
    assert_eq!(inbox["jobs"][0]["summary"], expected_title);
    assert_eq!(
        inbox["entries"][0]["message"],
        format!("{}[reda", "m".repeat(595))
    );
    repo.transaction(|state| {
            state["items"] = json!([{"id":"linear:secret","source":"linear","title":"Fixture","status":"todo","detail":format!("{}{sentinel}","d".repeat(1995))}]);
            Ok(())
        }).unwrap();
    let result = invoke(
        &repo,
        "work.update",
        json!({"id":"linear:secret","pinned":true}),
    )
    .unwrap();
    assert_eq!(
        result["item"]["detail"],
        format!("{}[reda", "d".repeat(1995))
    );
}

#[test]
fn work_projection_failure_rolls_back_the_same_transaction() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    repo.transaction(|state| {
        state["notes"] = json!([{"id":"note","source":"notes","title":"Note","status":"todo"}]);
        Ok(())
    })
    .unwrap();
    let before = repo.state.lock().unwrap().clone();
    let saved = std::fs::read(&repo.data).unwrap();
    assert!(change_work(&repo, &json!({"action":"delete-note","id":"note"})).is_err());
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), saved);
}

#[test]
fn validated_artifact_rejects_replacement_symlinks_and_inodes() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let record = uuid();
    repo.transaction(|state| {
        state["sessions"] = json!([{"id":record,"cwd":project,"items":[]}]);
        Ok(())
    })
    .unwrap();
    let path = project.join("report.md");
    std::fs::write(&path, "Registered fixture").unwrap();
    repo.edit(&json!({"action":"save-artifact","path":path,"sessionIds":[record]}))
        .unwrap();
    let id = repo.state.lock().unwrap()["artifacts"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let validated = artifact_file(&repo, &id).unwrap();
    let stored = validated.root.join(&validated.key);
    let outside = temp.path().join("outside.md");
    std::fs::write(&outside, "must-not-be-read").unwrap();
    std::fs::remove_file(&stored).unwrap();
    std::os::unix::fs::symlink(&outside, &stored).unwrap();
    assert!(validated.read_text(&repo).is_err());
    std::fs::remove_file(&stored).unwrap();
    std::fs::write(&stored, "Different inode").unwrap();
    assert!(validated.read_text(&repo).is_err());
}

#[test]
fn assistant_changes_only_local_work_state_and_can_restore_it() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    repo.transaction(|state| {
        state["items"] =
            json!([{"id":"linear:fixture","source":"linear","title":"Fixture","status":"todo"}]);
        Ok(())
    })
    .unwrap();
    let sources = repo.state.lock().unwrap()["sources"].clone();
    let updated = invoke(
        &repo,
        "work.update",
        json!({"id":"linear:fixture","pinned":true,"hidden":true,"status":"done"}),
    )
    .unwrap();
    assert_eq!(updated["scope"], "local");
    assert_eq!(updated["item"]["status"], "done");
    assert_eq!(updated["item"]["sourceStatusCode"], "todo");
    assert_eq!(updated["item"]["pinned"], true);
    assert_eq!(updated["item"]["hidden"], true);
    assert_eq!(repo.state.lock().unwrap()["items"][0]["status"], "todo");
    assert_eq!(invoke(&repo, "work.query", json!({})).unwrap()["total"], 0);
    let restored = invoke(
        &repo,
        "work.update",
        json!({"id":"linear:fixture","status":null,"pinned":false,"hidden":false}),
    )
    .unwrap();
    assert_eq!(restored["item"]["status"], "todo");
    let until = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600;
    assert_eq!(
        invoke(
            &repo,
            "work.snooze",
            json!({"id":"linear:fixture","mode":"until","until":until})
        )
        .unwrap()["item"]["snoozed"],
        true
    );
    assert_eq!(
        invoke(
            &repo,
            "work.snooze",
            json!({"id":"linear:fixture","mode":"clear"})
        )
        .unwrap()["item"]["snoozed"],
        false
    );
    assert_eq!(
        invoke(
            &repo,
            "work.snooze",
            json!({"id":"linear:fixture","mode":"change"})
        )
        .unwrap()["item"]["snoozed"],
        true
    );
    let before = repo.state.lock().unwrap().clone();
    for (name, args) in [
        ("work.update", json!({"id":"missing","pinned":true})),
        ("work.update", json!({"id":"linear:fixture"})),
        ("work.snooze", json!({"id":"linear:fixture","mode":"until"})),
        (
            "work.snooze",
            json!({"id":"linear:fixture","mode":"clear","until":until}),
        ),
        (
            "work.snooze",
            json!({"id":"linear:fixture","mode":"until","until":1}),
        ),
    ] {
        assert!(invoke(&repo, name, args).is_err());
    }
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(repo.state.lock().unwrap()["sources"], sources);
}

#[test]
fn artifact_reads_use_registered_snapshots_bounds_and_utf8_cursors() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let record = uuid();
    repo.transaction(|state| {
        state["sessions"] = json!([{"id":record,"cwd":project,"items":[]}]);
        Ok(())
    })
    .unwrap();
    let path = project.join("report.md");
    std::fs::write(&path, format!("abcé\n{}", "x".repeat(70000))).unwrap();
    repo.edit(&json!({"action":"save-artifact","title":"Fixture report","path":path,"sessionIds":[record]})).unwrap();
    let id = repo.state.lock().unwrap()["artifacts"][0]["id"].clone();
    std::fs::write(&path, "Changed source").unwrap();
    let before = repo.state.lock().unwrap().clone();
    let listed = invoke(
        &repo,
        "artifacts.list",
        json!({"recordId":record,"limit":1}),
    )
    .unwrap();
    assert_eq!(listed["total"], 1);
    assert_eq!(listed["artifacts"][0]["id"], id);
    assert!(listed["artifacts"][0].get("path").is_none());
    assert!(listed["artifacts"][0].get("sourcePath").is_none());
    let first = invoke(&repo, "artifacts.read-text", json!({"id":id,"maxBytes":4})).unwrap();
    assert_eq!(first["text"], "abc");
    assert_eq!(first["bytes"], 3);
    assert_eq!(first["truncated"], true);
    assert_eq!(first["nextOffset"], 3);
    let next = invoke(
        &repo,
        "artifacts.read-text",
        json!({"id":id,"offset":first["nextOffset"],"maxBytes":4}),
    )
    .unwrap();
    assert_eq!(next["text"], "é\nx");
    assert_eq!(next["nextOffset"], 7);
    let bounded = invoke(
        &repo,
        "artifacts.read-text",
        json!({"id":id,"maxBytes":65536}),
    )
    .unwrap();
    assert!(bounded["bytes"].as_u64().unwrap() <= 65536);
    assert_eq!(bounded["truncated"], true);
    assert!(invoke(&repo, "artifacts.read-text", json!({"id":id,"offset":4})).is_err());
    assert!(invoke(
        &repo,
        "artifacts.read-text",
        json!({"id":"000000000000000000000000"})
    )
    .is_err());
    assert_eq!(*repo.state.lock().unwrap(), before);
    let outside = temp.path().join("outside.md");
    std::fs::write(&outside, "Outside registered roots").unwrap();
    let outside_id = "111111111111111111111111";
    repo.transaction(|state| {
        state["artifacts"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":outside_id,"path":outside,"title":"Stale path","at":1}));
        Ok(())
    })
    .unwrap();
    assert!(invoke(&repo, "artifacts.read-text", json!({"id":outside_id})).is_err());
    repo.edit(
        &json!({"action":"save-artifact","title":"Remote link","url":"https://example.com/report"}),
    )
    .unwrap();
    let remote_id = repo.state.lock().unwrap()["artifacts"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()["id"]
        .clone();
    assert!(invoke(&repo, "artifacts.read-text", json!({"id":remote_id})).is_err());
}

#[test]
fn cached_inbox_reads_are_bounded_and_exclude_internal_payloads() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    {
        let mut sharing = repo.sharing.lock().unwrap();
        sharing["entries"] = json!({
            "one":{"id":"one","title":"Shared fixture","created_at":"2026-01-02","sender":{"id":"sender","name":"Sender","private":"not-shared"},"message":"m".repeat(1000),"bundle":{"uri":"private-locator"}},
            "two":{"id":"two","title":"Dismissed","created_at":"2026-01-01","dismissed":true}
        });
        sharing["jobs"] = json!({"job":{"id":"job","kind":"prepare","status":"ready","at":1,"payload":{"recordId":"record","private":"not-shared"},"bundlePath":"private-path","preview":{"body":"not-shared"},"summary":"Cached job"}});
    }
    let before = repo.sharing.lock().unwrap().clone();
    let result = invoke(&repo, "sharing.inbox", json!({"limit":1})).unwrap();
    assert_eq!(result["cached"], true);
    assert_eq!(result["total"], 1);
    assert_eq!(result["entries"][0]["sender"]["name"], "Sender");
    assert_eq!(result["entries"][0]["message"].as_str().unwrap().len(), 600);
    assert!(result["entries"][0].get("bundle").is_none());
    assert!(!result.to_string().contains("not-shared"));
    assert!(!result.to_string().contains("private-path"));
    assert_eq!(
        invoke(&repo, "sharing.inbox", json!({"includeDismissed":true})).unwrap()["total"],
        2
    );
    assert_eq!(*repo.sharing.lock().unwrap(), before);
    assert!(repo.sharing_busy.lock().unwrap().is_empty());
}

#[test]
fn existing_action_outputs_are_redacted_without_exposing_or_running_commands() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    std::fs::create_dir(&data).unwrap();
    std::fs::write(
        temp.path().join(".env"),
        "ACCESS_TEST_SECRET=benign-private-output-sentinel",
    )
    .unwrap();
    let forbidden = temp.path().join("must-not-exist");
    let jobs = json!({"fixture":{"id":"fixture","actionId":"action","title":"Fixture action","state":"succeeded","startedAt":1,"command":format!("touch {}",forbidden.display()),"cwd":"not-projected","output":format!("{} benign-private-output-sentinel","o".repeat(3000))}});
    std::fs::write(
        data.join("quick-actions.json"),
        serde_json::to_vec(&jobs).unwrap(),
    )
    .unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let before = std::fs::read(data.join("quick-actions.json")).unwrap();
    let result = invoke(&repo, "quick-actions.outputs", json!({"id":"fixture"})).unwrap();
    assert_eq!(result["total"], 1);
    assert_eq!(result["jobs"][0]["state"], "succeeded");
    assert_eq!(result["jobs"][0]["outputTruncated"], true);
    assert!(result["jobs"][0]["output"]
        .as_str()
        .unwrap()
        .contains("[redacted]"));
    assert!(!result
        .to_string()
        .contains("benign-private-output-sentinel"));
    assert!(result["jobs"][0].get("command").is_none());
    assert!(result["jobs"][0].get("cwd").is_none());
    assert!(!forbidden.exists());
    assert_eq!(
        std::fs::read(data.join("quick-actions.json")).unwrap(),
        before
    );
    assert!(invoke(
        &repo,
        "quick-actions.run",
        json!({"actionId":"action","revision":0})
    )
    .is_err());
}
