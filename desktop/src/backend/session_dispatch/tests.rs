use super::super::terminals::fake::Fake;
use super::*;
use crate::backend::settings::Approval;
use std::path::PathBuf;
use std::sync::Arc;
fn fixture() -> (tempfile::TempDir, Arc<Repository>, Arc<Fake>, Value, String) {
    let temp = tempfile::tempdir().unwrap();
    let (repo, fake) = reopen(&temp);
    let id = uuid();
    let generation = uuid();
    repo.transaction(|state|{state["sessions"]=json!([{"id":id,"agent":"codex","title":"Review fixture","cwd":temp.path(),"items":[{"id":"github:pr"}],"futureNs":{"preserve":true},"reviewDraft":{"revision":3,"note":"Captured private note","comments":[{"id":"captured","text":"Captured private comment","author":"Reviewer","path":"file.rs","root":temp.path(),"side":"additions","start":1,"end":1,"excerpt":"Captured private excerpt","createdAt":1}],"assessments":[{"id":"assessment","pass":"correctness","author":"Reviewer","text":"Captured assessment","createdAt":1}]}}]);state["items"]=json!([{"id":"github:pr","source":"github","kind":"Pull request","title":"Broken checks","url":"https://github.com/example/project/pull/1","ci":{"state":"failed","checks":[{"name":"unit","state":"failed","result":"FAIL","url":"https://example.invalid/log"}]}}]);Ok(())}).unwrap();
    let args = json!({"recordId":id,"operationId":uuid(),"revision":3});
    fake.live(&args["recordId"], &generation);
    (temp, repo, fake, args, generation)
}
/// Opens the repository in `temp` with fake agent terminals.
fn reopen(temp: &tempfile::TempDir) -> (Arc<Repository>, Arc<Fake>) {
    let mut repo = Repository::open(temp.path().into(), None, true).unwrap();
    let fake = Fake::install(&mut repo);
    (repo, fake)
}
/// Runs `call` and checks that it wrote no terminal input.
fn unsent<T>(fake: &Fake, call: impl FnOnce() -> T) -> T {
    let sent = fake.sent().len();
    let result = call();
    assert_eq!(fake.sent().len(), sent, "unexpected terminal input");
    result
}
/// Runs `call` and checks that it neither read the terminal inventory nor wrote input.
fn untouched<T>(fake: &Fake, call: impl FnOnce() -> T) -> T {
    let reads = fake.reads();
    let result = unsent(fake, call);
    assert_eq!(fake.reads(), reads, "unexpected inventory read");
    result
}
fn matching(
    entries: &[Value],
    id: &str,
    operation: &str,
    kind: Kind,
    item: &str,
) -> Result<Option<Value>> {
    if let Some(entry) = entries
        .iter()
        .find(|entry| entry["operationId"] == operation)
    {
        if entry["recordId"] != id || entry["kind"] != kind.name() || text(entry, "itemId") != item
        {
            return Err(Error::new(
                "This dispatch operation ID belongs to a different feature or target.",
            ));
        }
        return Ok(Some(entry.clone()));
    }
    Ok(entries
        .iter()
        .rev()
        .find(|entry| {
            entry["recordId"] == id
                && entry["kind"] == kind.name()
                && text(entry, "itemId") == item
                && entry["finalized"] != true
                && entry["resolution"] != "user-abandoned"
        })
        .cloned())
}
/// Prepares or reuses the operation in `args`, then submits it.
fn dispatch_after(
    repo: &Repository,
    kind: Kind,
    args: &Value,
    after_written: impl FnOnce() -> Result<()>,
) -> Result<Value> {
    let previous = {
        let state = repo.state.lock().unwrap();
        matching(
            &entries(&state)?,
            text(args, "recordId"),
            text(args, "operationId"),
            kind,
            text(args, "itemId"),
        )?
    };
    let receipt = if let Some(entry) = previous {
        public(&entry)
    } else {
        prepare_with(repo, kind, args, || text(args, "operationId").to_owned())?
    };
    submit_with(
        repo,
        &json!({"recordId":args["recordId"],"operationId":receipt["operationId"]}),
        after_written,
    )
}
fn dispatch(repo: &Repository, kind: Kind, args: &Value) -> Result<Value> {
    dispatch_after(repo, kind, args, || Ok(()))
}
fn feedback_send(repo: &Repository, args: &Value) -> Result<Value> {
    dispatch(repo, Kind::ReviewFeedback, args)
}
#[test]
fn dispatch_once_preserves_later_feedback_and_reused_comment_ids() {
    let (_temp, repo, fake, args, _generation) = fixture();
    let during = repo.clone();
    fake.on_send(move |payload|{assert!(payload.starts_with("\u{1b}[200~"));assert!(payload.ends_with("\u{1b}[201~\r"));assert_eq!(during.state.lock().unwrap()[JOURNAL]["entries"][0]["status"],"sending");
            during.transaction(|s|{let draft=&mut s["sessions"][0]["reviewDraft"];draft["comments"]=json!([{"id":"captured","text":"Reused ID with newer content"},{"id":"later","text":"Later feedback"}]);draft["assessments"].as_array_mut().unwrap().push(json!({"id":"later-assessment","text":"Later assessment"}));draft["note"]=json!("Later note");draft["revision"]=json!(4);Ok(())})?;Ok(())});
    let receipt = feedback_send(&repo, &args).unwrap();
    assert_eq!(receipt["status"], "written");
    assert_eq!(receipt["finalized"], true);
    assert_eq!(receipt["transport"], "legacy-unfenced");
    assert_eq!(fake.sent().len(), 1);
    let state = repo.state.lock().unwrap().clone();
    let draft = &state["sessions"][0]["reviewDraft"];
    assert_eq!(arr(&draft["comments"]).len(), 2);
    assert_eq!(draft["note"], "Later note");
    assert!(draft["assessments"][0]["sentAt"].is_number());
    assert!(draft["assessments"][1]["sentAt"].is_null());
    assert_eq!(state["sessions"][0]["futureNs"]["preserve"], true);
    assert!(state[JOURNAL]["entries"][0].get("payload").is_none());
    let mut retry = args.clone();
    retry["revision"] = json!(999);
    let again = unsent(&fake, || feedback_send(&repo, &retry)).unwrap();
    assert_eq!(again["operationId"], receipt["operationId"]);
    assert_eq!(again["finalized"], true);
    assert!(repo.snapshot(false).get(JOURNAL).is_none());
    assert!(repo.snapshot(true).get(JOURNAL).is_none());
    let public = read(&repo, &args).unwrap().to_string();
    assert!(!public.contains("Captured private"));
    assert!(!public.contains("payload\""));
}
#[test]
fn dispatch_uncertainty_and_failed_written_marker_survive_restart_without_replay() {
    for marker_failure in [false, true] {
        let (temp, repo, fake, args, generation) = fixture();
        let path = repo.data.clone();
        let marker = path.clone();
        fake.on_send(move |_| {
            if marker_failure {
                std::fs::create_dir(marker.with_extension("tmp"))?;
                Ok(())
            } else {
                Err(Error::new("Partial write or lost daemon acknowledgement"))
            }
        });
        let receipt = feedback_send(&repo, &args).unwrap();
        assert_eq!(receipt["status"], "uncertain");
        assert_eq!(receipt["finalized"], false);
        if marker_failure {
            std::fs::remove_dir(path.with_extension("tmp")).unwrap();
        }
        drop(repo);
        let (repo, fake) = reopen(&temp);
        fake.live(&args["recordId"], &generation);
        let mut retry = args.clone();
        retry["operationId"] = json!(uuid());
        let receipt = unsent(&fake, || feedback_send(&repo, &retry)).unwrap();
        assert_eq!(receipt["operationId"], args["operationId"]);
        assert_eq!(receipt["status"], "uncertain");
        assert_eq!(finalize(&repo, &args).unwrap()["status"], "uncertain");
        assert_eq!(
            repo.state.lock().unwrap()["sessions"][0]["reviewDraft"]["note"],
            "Captured private note"
        );
    }
}
#[test]
fn dispatch_written_marker_allows_only_ack_recovery_after_failed_save_and_restart() {
    let (temp, repo, _fake, args, generation) = fixture();
    let path = repo.data.clone();
    let receipt = dispatch_after(&repo, Kind::ReviewFeedback, &args, || {
        std::fs::create_dir(path.with_extension("tmp"))?;
        Ok(())
    })
    .unwrap();
    assert_eq!(receipt["status"], "written");
    assert_eq!(receipt["finalized"], false);
    assert_eq!(
        repo.state.lock().unwrap()["sessions"][0]["reviewDraft"]["note"],
        "Captured private note"
    );
    std::fs::remove_dir(path.with_extension("tmp")).unwrap();
    drop(repo);
    let (repo, fake) = reopen(&temp);
    fake.live(&args["recordId"], &generation);
    let receipt = unsent(&fake, || feedback_send(&repo, &args)).unwrap();
    assert_eq!(receipt["finalized"], true);
    assert_eq!(
        repo.state.lock().unwrap()["sessions"][0]["reviewDraft"]["note"],
        ""
    );
    assert_eq!(
        arr(&repo.state.lock().unwrap()["sessions"][0]["reviewDraft"]["comments"]).len(),
        0
    );
}
#[test]
fn dispatch_reports_the_daemon_error_when_terminal_state_is_unknown() {
    let (_temp, repo, fake, args, generation) = fixture();
    let daemon = "Terminal daemon did not respond. Retry; do not start a duplicate agent.";
    fake.fail(daemon);
    let error = prepare(&repo, Kind::ReviewFeedback, &args).unwrap_err();
    assert_eq!(error.message, daemon);
    fake.rows(vec![]);
    let error = prepare(&repo, Kind::ReviewFeedback, &args).unwrap_err();
    assert!(error.message.contains("Resume this chat"));
    fake.live(&args["recordId"], &generation);
    let prepared = prepare(&repo, Kind::ReviewFeedback, &args).unwrap();
    fake.fail(daemon);
    let error = unsent(&fake, || submit(&repo, &prepared)).unwrap_err();
    assert_eq!(error.message, daemon);
    assert_eq!(
        locate(
            &repo,
            text(&args, "recordId"),
            text(&prepared, "operationId")
        )
        .unwrap()["status"],
        "prepared"
    );
}
#[test]
fn dispatch_writes_to_the_terminal_that_runs_the_chat_conversation() {
    let (_temp, repo, fake, args, generation) = fixture();
    let (conversation, terminal) = (uuid(), uuid());
    repo.transaction(|s| {
        s["sessions"][0]["sessionId"] = json!(conversation);
        Ok(())
    })
    .unwrap();
    fake.rows(vec![
        json!({"recordId":terminal,"sessionId":conversation,"agent":"codex","generation":generation,"live":true}),
    ]);
    assert_eq!(feedback_send(&repo, &args).unwrap()["finalized"], true);
    assert_eq!(fake.sent()[0].0, terminal);
}
#[test]
fn dispatch_prepared_retry_uses_frozen_payload_and_reservation_failure_never_queues() {
    let (_temp, repo, fake, args, _generation) = fixture();
    let path = repo.data.clone();
    let marker = path.clone();
    fake.before_read(|| {});
    fake.before_read(move || std::fs::create_dir(marker.with_extension("tmp")).unwrap());
    let first = unsent(&fake, || feedback_send(&repo, &args)).unwrap();
    assert_eq!(first["status"], "prepared");
    std::fs::remove_dir(path.with_extension("tmp")).unwrap();
    repo.transaction(|s| {
        s["sessions"][0]["title"] = json!("New title");
        s["sessions"][0]["reviewDraft"]["note"] = json!("New note");
        s["sessions"][0]["reviewDraft"]["revision"] = json!(4);
        Ok(())
    })
    .unwrap();
    fake.on_send(|payload| {
        assert!(payload.contains("Captured private note"));
        assert!(!payload.contains("New note"));
        Ok(())
    });
    let receipt = feedback_send(&repo, &args).unwrap();
    assert_eq!(receipt["payloadHash"], first["payloadHash"]);
    assert_eq!(
        repo.state.lock().unwrap()["sessions"][0]["reviewDraft"]["note"],
        "New note"
    );
    let (_temp, repo, fake, mut args, _generation) = fixture();
    args["itemId"] = json!("github:pr");
    std::fs::create_dir(repo.data.with_extension("tmp")).unwrap();
    let before = repo.state.lock().unwrap().clone();
    assert!(unsent(&fake, || dispatch(&repo, Kind::CiInvestigation, &args)).is_err());
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert!(repo.state.lock().unwrap()["sessions"][0]["ciInvestigations"].is_null());
}
#[test]
fn dispatch_concurrent_same_operation_has_only_one_terminal_write() {
    let (_temp, repo, fake, args, _generation) = fixture();
    let (started, ready) = std::sync::mpsc::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let wait = std::sync::Mutex::new(wait);
    let worker_repo = repo.clone();
    let worker_args = args.clone();
    fake.on_send(move |_| {
        started.send(()).unwrap();
        wait.lock().unwrap().recv().unwrap();
        Ok(())
    });
    let worker = std::thread::spawn(move || feedback_send(&worker_repo, &worker_args));
    ready.recv().unwrap();
    let receipt = unsent(&fake, || feedback_send(&repo, &args)).unwrap();
    assert_eq!(receipt["status"], "uncertain");
    release.send(()).unwrap();
    assert_eq!(worker.join().unwrap().unwrap()["finalized"], true);
}
#[test]
fn dispatch_resolution_is_explicit_retains_uncertainty_and_never_replays_old_ids() {
    for decision in ["submitted", "abandon"] {
        let (_temp, repo, fake, args, _generation) = fixture();
        fake.on_send(|_| Err(Error::new("Unknown delivery")));
        let receipt = feedback_send(&repo, &args).unwrap();
        assert_eq!(receipt["status"], "uncertain");
        let result=resolve(&repo,&json!({"recordId":args["recordId"],"operationId":args["operationId"],"decision":decision})).unwrap();
        assert_eq!(result["status"], "uncertain");
        assert_eq!(result["resolved"], true);
        assert_eq!(result["finalized"], decision == "submitted");
        assert_eq!(
            unsent(&fake, || feedback_send(&repo, &args)).unwrap()["resolved"],
            true
        );
        if decision == "abandon" {
            let mut next = args.clone();
            next["operationId"] = json!(uuid());
            let receipt = feedback_send(&repo, &next).unwrap();
            assert_eq!(receipt["status"], "written");
        }
    }
}
#[test]
fn dispatch_identity_revision_capacity_and_private_extensions_are_guarded() {
    let (_temp, repo, fake, args, _generation) = fixture();
    let mut bad = args.clone();
    bad.as_object_mut().unwrap().remove("revision");
    assert!(unsent(&fake, || feedback_send(&repo, &bad)).is_err());
    bad["revision"] = json!(2);
    assert!(unsent(&fake, || feedback_send(&repo, &bad)).is_err());
    assert_eq!(
        entries(&repo.state.lock().unwrap()).unwrap(),
        [] as [serde_json::Value; 0]
    );
    fake.on_send(|_| Err(Error::new("Unknown")));
    feedback_send(&repo, &args).unwrap();
    let mut collision = args.clone();
    collision["itemId"] = json!("github:pr");
    assert!(untouched(&fake, || dispatch(&repo, Kind::CiInvestigation, &collision)).is_err());
    repo.transaction(|s| {
        s[JOURNAL]["privateNs"] = json!({"secret":"private-journal-extension"});
        s[JOURNAL]["entries"][0]["futureNs"] = json!({"keep":true});
        Ok(())
    })
    .unwrap();
    resolve(&repo,&json!({"recordId":args["recordId"],"operationId":args["operationId"],"decision":"submitted"})).unwrap();
    let state = repo.state.lock().unwrap().clone();
    assert_eq!(state[JOURNAL]["entries"][0]["futureNs"]["keep"], true);
    assert_eq!(
        state[JOURNAL]["privateNs"]["secret"],
        "private-journal-extension"
    );
    assert!(!read(&repo, &args)
        .unwrap()
        .to_string()
        .contains("private-journal-extension"));
    assert!(repo.snapshot(true).get(JOURNAL).is_none());
    repo.transaction(|s| {
        let template = s[JOURNAL]["entries"][0].clone();
        let mut all = vec![];
        for index in 0..MAX_UNRESOLVED {
            let mut entry = template.clone();
            entry["operationId"] = json!(uuid());
            entry["recordId"] = json!(uuid());
            entry["ticket"] = json!((index + 1).to_string());
            entry["finalized"] = json!(false);
            entry["status"] = json!("uncertain");
            entry.as_object_mut().unwrap().remove("resolved");
            entry.as_object_mut().unwrap().remove("resolution");
            entry["payload"] = json!("frozen");
            all.push(entry);
        }
        s[JOURNAL]["nextTicket"] = json!((MAX_UNRESOLVED + 1).to_string());
        store(s, all)
    })
    .unwrap();
    let before = repo.state.lock().unwrap().clone();
    let mut next = args;
    next["operationId"] = json!(uuid());
    assert!(unsent(&fake, || feedback_send(&repo, &next)).is_err());
    assert_eq!(*repo.state.lock().unwrap(), before);
}
#[test]
fn dispatch_prompts_preserve_anchors_freshness_and_strip_terminal_controls() {
    let record = json!({"id":uuid(),"cwd":"/project","title":"Goal","items":[]});
    let comment = json!({"id":"c","author":"Reviewer","text":"Fix\r\u{1b}[31m\u{7}this\nnext\tcolumn","root":"/other","path":"file.rs","side":"deletions","start":2,"end":3,"baseCommit":"base123","fingerprint":"fingerprint123","excerpt":"old code","freshness":"outdated"});
    let assessment = json!({"id":"a","author":"Performance","pass":"performance","text":"Slow path","root":"/other","baseCommit":"otherbase","freshness":"unverified"});
    let draft =
        json!({"comments":[comment],"assessments":[assessment],"note":"Summary","revision":1});
    let (prompt, ack) = feedback(&record, &draft, &draft).unwrap();
    for fragment in [
        "/other/file.rs freshness: outdated",
        "assessment freshness: unverified",
        "Worktree: /other · base: otherbase",
        "old lines 2–3",
        "Base: base123 · diff fingerprint: fingerprint123",
        "Code excerpt:\nold code",
        "Fix[31mthis\nnext\tcolumn",
    ] {
        assert!(prompt.contains(fragment), "missing {fragment}");
    }
    assert!(!prompt.contains('\r'));
    assert!(!prompt.contains('\u{1b}'));
    assert_eq!(ack["commentIds"], json!(["c"]));
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let item = json!({"id":"github:pr","reference":"#7","title":"CI","url":"https://github.com/example/repo/pull/7","headRefName":"feature","ci":{"state":"failed","checks":[{"name":"unit","state":"failed","result":"FAIL","url":"https://example.invalid/log"}]}});
    let prompt = ci_prompt(&repo, &record, &item, "operation");
    for fragment in [
        "unit · FAIL · https://example.invalid/log",
        "cached snapshot may be stale",
        "Investigate only",
        "do not change code, rerun workflows, push, or post externally",
        "--request operation",
    ] {
        assert!(prompt.contains(fragment));
    }
    let settings = json!({"values":{"codexReviewCommand":"$custom"},"workflows":{"agent-review":"{{REVIEW_COMMAND}} {{CONTEXT_JSON}} {{RECORD_ID}} {{REQUEST_ID}} {{HELPER}}"}});
    let mut record = record;
    record["agent"] = json!("codex");
    record["title"] = json!("{{HELPER}} is reference data");
    let run = json!({"id":uuid(),"versions":{"/another":"stamp"}});
    let prompt = request_prompt(&repo, &record, &run, &settings).unwrap();
    assert!(prompt.contains("$custom"));
    assert!(prompt.contains("{{HELPER}} is reference data"));
    assert!(prompt.contains("/another"));
    assert!(prompt.contains(text(&run, "id")));
}
#[test]
fn dispatch_request_and_ci_queue_are_atomic_and_resolution_cancels_only_matching_progress() {
    let (_temp, repo, fake, args, _generation) = fixture();
    let settings = repo.settings.view();
    let command = text(&settings["reviewSkill"]["commands"], "codex");
    assert_ne!(command, "");
    repo.settings.apply(&json!({"revision":settings["revision"],"contentHash":settings["contentHash"],"patch":{"codexReviewCommand":command}}),Approval::User).unwrap();
    let (during, operation) = (repo.clone(), text(&args, "operationId").to_owned());
    fake.on_send(move |payload| {
        assert!(payload.contains("truffle-review"));
        assert!(payload.contains(&operation));
        let state = during.state.lock().unwrap();
        assert_eq!(
            state["sessions"][0]["reviewDraft"]["run"]["state"],
            "requested"
        );
        assert_eq!(state[JOURNAL]["entries"][0]["status"], "sending");
        Err(Error::new("Lost acknowledgement"))
    });
    let result = dispatch(&repo, Kind::ReviewRequest, &args).unwrap();
    assert_eq!(result["status"], "uncertain");
    resolve(&repo,&json!({"recordId":args["recordId"],"operationId":args["operationId"],"decision":"abandon"})).unwrap();
    assert_eq!(
        repo.state.lock().unwrap()["sessions"][0]["reviewDraft"]["run"]["state"],
        "cancelled"
    );
    let (_temp, repo, fake, mut args, _generation) = fixture();
    args["itemId"] = json!("github:pr");
    let during = repo.clone();
    fake.on_send(move |payload| {
        assert!(payload.contains("--request "));
        assert!(payload.contains("Investigate only"));
        assert_eq!(
            during.state.lock().unwrap()["sessions"][0]["ciInvestigations"]["github:pr"]["state"],
            "queued"
        );
        Err(Error::new("Unknown delivery"))
    });
    let result = dispatch(&repo, Kind::CiInvestigation, &args).unwrap();
    assert_eq!(result["status"], "uncertain");
    resolve(&repo,&json!({"recordId":args["recordId"],"operationId":args["operationId"],"decision":"abandon"})).unwrap();
    assert_eq!(
        repo.state.lock().unwrap()["sessions"][0]["ciInvestigations"]["github:pr"]["state"],
        "cancelled"
    );
    let mut next = args.clone();
    next["operationId"] = json!(uuid());
    assert_eq!(
        dispatch(&repo, Kind::CiInvestigation, &next).unwrap()["status"],
        "written"
    );
    assert_eq!(
        untouched(&fake, || dispatch(&repo, Kind::CiInvestigation, &args)).unwrap()["resolution"],
        "user-abandoned"
    );
}
#[test]
fn dispatch_abandoned_prepared_generation_change_and_new_review_round_never_replay_or_ack_new_data()
{
    let (_temp, repo, fake, args, _generation) = fixture();
    replace_generation_after_prepare(&fake, &args, &uuid());
    assert!(unsent(&fake, || feedback_send(&repo, &args)).is_err());
    let resolved=resolve(&repo,&json!({"recordId":args["recordId"],"operationId":args["operationId"],"decision":"abandon"})).unwrap();
    assert_eq!(resolved["status"], "prepared");
    assert_eq!(resolved["resolution"], "user-abandoned");
    assert!(!text(&resolved, "message").contains("Retry this same"));
    assert_eq!(
        unsent(&fake, || feedback_send(&repo, &args)).unwrap()["resolved"],
        true
    );
    let (_temp, repo, fake, args, _generation) = fixture();
    let during = repo.clone();
    fake.on_send(move |_| {
        during.transaction(|s| {
            s["sessions"][0]["reviewHistory"] = json!({"version":1,"entries":[{"id":uuid()}]});
            Ok(())
        })?;
        Ok(())
    });
    let result = feedback_send(&repo, &args).unwrap();
    assert_eq!(result["finalized"], true);
    assert_eq!(
        repo.state.lock().unwrap()["sessions"][0]["reviewDraft"]["note"],
        "Captured private note"
    );
}
#[test]
fn dispatch_pending_filters_and_both_agent_skill_paths_preserve_protocol_context() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let id = uuid();
    let record = json!({"id":id,"cwd":"/checkout","title":"Goal","items":[{"reference":"#5","title":"Task","url":"https://example.invalid/work"}],"worktrees":[{"path":"/linked"}]});
    let draft = json!({"revision":1,"note":"Already reviewed summary","reviewedNote":"Already reviewed summary","comments":[{"id":"reviewed","text":"Consumed comment","reviewedAt":5},{"id":"pending","text":"Pending comment","path":"p","start":1,"end":1}],"assessments":[{"id":"sent","text":"Sent assessment","sentAt":5},{"id":"reviewed-assessment","text":"Consumed assessment","reviewedAt":5},{"id":"pending-assessment","text":"Pending assessment"}]});
    let (prompt, ack) = feedback(&record, &draft, &draft).unwrap();
    assert!(prompt.contains("Pending comment"));
    assert!(prompt.contains("Pending assessment"));
    for omitted in [
        "Consumed comment",
        "Sent assessment",
        "Consumed assessment",
        "Already reviewed summary",
    ] {
        assert!(!prompt.contains(omitted));
    }
    assert_eq!(ack["commentIds"], json!(["pending"]));
    assert_eq!(ack["assessmentIds"], json!(["pending-assessment"]));
    assert_eq!(ack["note"], "");
    let old = json!({"note":"","run":{"state":"complete","completedAt":10},"comments":[{"id":"old","createdAt":5,"text":"Old round"},{"id":"new","createdAt":11,"text":"After round"}]});
    let (prompt, _) = feedback(&record, &old, &old).unwrap();
    assert!(!prompt.contains("Old round"));
    assert!(prompt.contains("After round"));
    for (agent, command) in [("codex", "$portable"), ("claude", "/portable")] {
        let mut record = record.clone();
        record["agent"] = json!(agent);
        let run = json!({"id":uuid(),"versions":{"/extra":"stamp"}});
        let settings = json!({"values":{if agent=="codex"{"codexReviewCommand"}else{"claudeReviewCommand"}:command},"reviewSkill":{"available":true,"path":"/editable/config/review/SKILL.md","commands":{agent:command}}});
        let prompt = request_prompt(&repo, &record, &run, &settings).unwrap();
        for fragment in [
            "/editable/config/review/SKILL.md",
            "/linked",
            "/extra",
            "https://example.invalid/work",
            text(&run, "id"),
            "Report all four passes",
            "do not send or apply them",
        ] {
            assert!(prompt.contains(fragment), "missing {fragment}");
        }
    }
}
#[test]
fn dispatch_hash_target_and_malformed_journal_cannot_admit_terminal_input() {
    let (_temp, repo, fake, args, _generation) = fixture();
    replace_generation_after_prepare(&fake, &args, "00000000-0000-0000-0000-000000000000");
    assert!(unsent(&fake, || feedback_send(&repo, &args)).is_err());
    repo.transaction(|s| {
        s[JOURNAL]["entries"][0]["payload"] = json!("tampered bytes");
        Ok(())
    })
    .unwrap();
    assert!(unsent(&fake, || feedback_send(&repo, &args))
        .unwrap_err()
        .message
        .contains("integrity"));
    let other = uuid();
    repo.transaction(|s| {
        s["sessions"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":other,"items":[]}));
        Ok(())
    })
    .unwrap();
    let mut collision = args.clone();
    collision["recordId"] = json!(other);
    assert!(unsent(&fake, || feedback_send(&repo, &collision)).is_err());
    repo.transaction(|s| {
        s[JOURNAL]["entries"][0]["resolved"] = json!({"private":"not a boolean"});
        Ok(())
    })
    .unwrap();
    assert!(read(&repo, &args).is_err());
    assert!(unsent(&fake, || feedback_send(&repo, &args)).is_err());
}
#[test]
fn dispatch_concurrent_prepare_and_lost_response_reserve_one_server_identity_without_input() {
    let (_temp, repo, fake, args, _generation) = fixture();
    let (started, ready) = std::sync::mpsc::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let wait = std::sync::Mutex::new(wait);
    let worker_repo = repo.clone();
    let worker_args = args.clone();
    fake.before_read(move || {
        started.send(()).unwrap();
        wait.lock().unwrap().recv().unwrap();
    });
    let worker =
        std::thread::spawn(move || prepare(&worker_repo, Kind::ReviewFeedback, &worker_args));
    ready.recv().unwrap();
    assert_eq!(read(&repo, &args).unwrap()["operations"], json!([]));
    let result = prepare(&repo, Kind::ReviewFeedback, &args).unwrap();
    assert_ne!(result["operationId"], args["operationId"]);
    release.send(()).unwrap();
    let recovered = worker.join().unwrap().unwrap();
    assert_eq!(result, recovered);
    assert_eq!(result["status"], "prepared");
    assert_eq!(result["ticket"], "1");
    assert_eq!(repo.state.lock().unwrap()[JOURNAL]["nextTicket"], "2");
    let again = untouched(&fake, || prepare(&repo, Kind::ReviewFeedback, &args)).unwrap();
    assert_eq!(again, result);
}

#[test]
fn dispatch_v2_compacts_seeded_history_and_continues_durable_cycles() {
    let (temp, repo, fake, args, generation) = fixture();
    let unresolved = prepare(&repo, Kind::ReviewFeedback, &args).unwrap();
    let other = uuid();
    repo.transaction(|state| {
        let mut record = state["sessions"][0].clone();
        record["id"] = json!(other);
        state["sessions"].as_array_mut().unwrap().push(record);
        Ok(())
    })
    .unwrap();
    let prepare_args = json!({"recordId":other,"itemId":"github:pr"});
    fake.rows(vec![
        json!({"recordId":args["recordId"],"generation":generation,"live":true}),
        json!({"recordId":other,"generation":uuid(),"live":true}),
    ]);
    let first = prepare(&repo, Kind::CiInvestigation, &prepare_args).unwrap();
    let completed = complete(&repo, &first, &prepare_args);
    assert_eq!(completed["ticket"], "2");
    let seeded_completed = 1030u32;
    seed_completed_history(&repo, &first, seeded_completed);
    for cycle in 0..3 {
        let prepared = prepare(&repo, Kind::CiInvestigation, &prepare_args).unwrap();
        assert_eq!(
            prepared["ticket"],
            (seeded_completed + 2 + cycle).to_string()
        );
        complete(&repo, &prepared, &prepare_args);
        assert_eq!(
            locate(&repo, &other, text(&prepared, "operationId")).unwrap()["finalized"],
            true
        );
    }
    assert_eq!(fake.sent().len(), 4);
    let state = repo.state.lock().unwrap().clone();
    assert_eq!(entries(&state).unwrap().len(), RECENT_COMPLETED + 1);
    assert_eq!(
        decimal(&state[JOURNAL]["retiredThrough"]).unwrap(),
        u64::from(seeded_completed + 4) - RECENT_COMPLETED as u64
    );
    assert_eq!(
        state[JOURNAL]["nextTicket"],
        (seeded_completed + 5).to_string()
    );
    assert_eq!(
        locate(
            &repo,
            text(&args, "recordId"),
            text(&unresolved, "operationId")
        )
        .unwrap()["ticket"],
        "1"
    );
    drop(repo);
    let (repo, fake) = reopen(&temp);
    fake.live(&args["recordId"], &generation);
    assert_eq!(repo.state.lock().unwrap()[JOURNAL], state[JOURNAL]);
    let bytes = std::fs::read(&repo.data).unwrap();
    let retired = untouched(&fake, || submit(&repo, &first)).unwrap();
    assert_eq!(retired["type"], "unavailable");
    assert!(retired.get("status").is_none());
    assert!(text(&retired, "message").contains("Prior delivery may have occurred"));
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    assert_unresolved_becomes_uncertain(&repo, &fake, &unresolved);
    assert_eq!(fake.sent().len(), 1);
}
/// Lets the next inventory read pass, then replaces the terminal generation.
fn replace_generation_after_prepare(fake: &Arc<Fake>, args: &Value, generation: &str) {
    let (replaced, record, generation) = (
        fake.clone(),
        args["recordId"].clone(),
        generation.to_owned(),
    );
    fake.before_read(|| {});
    fake.before_read(move || replaced.live(&record, &generation));
}
fn seed_completed_history(repo: &Repository, first: &Value, seeded_completed: u32) {
    repo.transaction(|state| {
        let mut all = entries(state)?;
        let template = all
            .iter()
            .find(|entry| entry["operationId"] == first["operationId"])
            .unwrap()
            .clone();
        let at = template["finalizedAt"].as_f64().unwrap();
        all.retain(|entry| entry["operationId"] != first["operationId"]);
        for ticket in 2..=seeded_completed + 1 {
            let mut entry = template.clone();
            if ticket != 2 {
                entry["operationId"] = json!(uuid());
            }
            entry["ticket"] = json!(ticket.to_string());
            entry["finalizedAt"] = json!(at - f64::from(seeded_completed + 2 - ticket));
            all.push(entry);
        }
        state[JOURNAL]["nextTicket"] = json!((seeded_completed + 2).to_string());
        store(state, all)
    })
    .unwrap();
}
fn assert_unresolved_becomes_uncertain(repo: &Repository, fake: &Fake, unresolved: &Value) {
    fake.on_send(|_| Err(Error::new("Unknown acknowledgement")));
    let still_live = submit(repo, unresolved).unwrap();
    assert_eq!(still_live["status"], "uncertain");
    assert_eq!(still_live["operationId"], unresolved["operationId"]);
    assert_eq!(
        untouched(fake, || submit(repo, unresolved)).unwrap(),
        still_live
    );
}
/// Submits a prepared CI investigation and marks the investigation complete.
fn complete(repo: &Repository, prepared: &Value, args: &Value) -> Value {
    let done = submit(repo, prepared).unwrap();
    assert_eq!(done["finalized"], true);
    super::super::session_ci::investigation(repo,&json!({"id":args["recordId"],"itemId":"github:pr","requestId":prepared["operationId"],"state":"complete"})).unwrap();
    done
}
fn legacy_fixture() -> (
    tempfile::TempDir,
    Arc<Repository>,
    Arc<Fake>,
    Value,
    String,
    Value,
) {
    let (temp, repo, fake, args, generation) = fixture();
    let prepared = prepare(&repo, Kind::ReviewFeedback, &args).unwrap();
    repo.transaction(|s| {
        s[JOURNAL]["version"] = json!(1);
        for key in ["nextTicket", "retiredThrough"] {
            s[JOURNAL].as_object_mut().unwrap().remove(key);
        }
        s[JOURNAL]["rootNs"] = json!({"keep":"private extension"});
        for entry in s[JOURNAL]["entries"].as_array_mut().unwrap() {
            entry.as_object_mut().unwrap().remove("ticket");
            entry["unknownNs"] = json!({"keep":true});
        }
        Ok(())
    })
    .unwrap();
    (temp, repo, fake, args, generation, prepared)
}
#[test]
fn dispatch_v2_migration_is_atomic_preserves_legacy_fields_and_recovers_existing_uuid() {
    let (_temp, repo, fake, args, _generation, prepared) = legacy_fixture();
    let original = repo.state.lock().unwrap().clone();
    let bytes = std::fs::read(&repo.data).unwrap();
    let public_before = read(&repo, &args).unwrap();
    assert_eq!(
        public_before["operations"][0]["operationId"],
        prepared["operationId"]
    );
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    std::fs::create_dir(repo.data.with_extension("tmp")).unwrap();
    assert!(untouched(&fake, || submit(&repo, &prepared)).is_err());
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    assert_eq!(*repo.state.lock().unwrap(), original);
    std::fs::remove_dir(repo.data.with_extension("tmp")).unwrap();
    ensure_migrated(&repo).unwrap();
    let migrated = repo.state.lock().unwrap().clone();
    let mut expected = original[JOURNAL]["entries"][0].clone();
    expected["ticket"] = json!("1");
    assert_eq!(migrated[JOURNAL]["entries"][0], expected);
    assert_eq!(migrated[JOURNAL]["rootNs"], original[JOURNAL]["rootNs"]);
    assert_eq!(migrated[JOURNAL]["version"], 2);
    assert_eq!(migrated[JOURNAL]["nextTicket"], "2");
    fake.on_send(|payload| {
        assert!(payload.contains("Captured private note"));
        Ok(())
    });
    let done = submit(&repo, &prepared).unwrap();
    assert_eq!(done["finalized"], true);
    assert_eq!(
        repo.state.lock().unwrap()[JOURNAL]["entries"][0]["unknownNs"]["keep"],
        true
    );
}
#[test]
fn dispatch_v2_ticket_metadata_overflow_and_unknown_identity_fail_closed() {
    let (_temp, repo, fake, args, _generation) = fixture();
    let unknown = untouched(&fake, || submit(&repo, &args)).unwrap();
    assert_eq!(unknown["type"], "unavailable");
    let clean = repo.state.lock().unwrap()[JOURNAL].clone();
    for next in [
        json!(0),
        json!("0"),
        json!("01"),
        json!("18446744073709551616"),
        json!("18446744073709551615"),
    ] {
        repo.transaction(|s| {
            s[JOURNAL] = clean.clone();
            s[JOURNAL]["nextTicket"] = next;
            Ok(())
        })
        .unwrap();
        let before = repo.state.lock().unwrap().clone();
        assert!(prepare(&repo, Kind::ReviewFeedback, &args).is_err());
        assert_eq!(*repo.state.lock().unwrap(), before);
    }
    repo.transaction(|s| {
        s[JOURNAL] = clean.clone();
        s[JOURNAL]["nextTicket"] = json!("9007199254740993");
        Ok(())
    })
    .unwrap();
    let operation = prepare(&repo, Kind::ReviewFeedback, &args).unwrap();
    assert_eq!(operation["ticket"], "9007199254740993");
    assert_eq!(
        repo.state.lock().unwrap()[JOURNAL]["nextTicket"],
        "9007199254740994"
    );
    let good = repo.state.lock().unwrap()[JOURNAL].clone();
    for mutation in [
        "floor",
        "duplicate-ticket",
        "duplicate-id",
        "ticket-next",
        "ticket-number",
    ] {
        repo.transaction(|s| {
            s[JOURNAL] = good.clone();
            match mutation {
                "floor" => s[JOURNAL]["retiredThrough"] = s[JOURNAL]["nextTicket"].clone(),
                "ticket-next" => {
                    s[JOURNAL]["entries"][0]["ticket"] = s[JOURNAL]["nextTicket"].clone();
                }
                "ticket-number" => s[JOURNAL]["entries"][0]["ticket"] = json!(9),
                _ => {
                    let mut entry = s[JOURNAL]["entries"][0].clone();
                    if mutation == "duplicate-ticket" {
                        entry["operationId"] = json!(uuid());
                    } else {
                        entry["ticket"] = json!("1");
                    }
                    s[JOURNAL]["entries"].as_array_mut().unwrap().push(entry);
                }
            }
            Ok(())
        })
        .unwrap();
        assert!(read(&repo, &args).is_err(), "{mutation}");
    }
}
#[test]
fn dispatch_v2_abandon_releases_owned_bytes_and_preserves_unknown_extensions() {
    let (_temp, repo, fake, args, _generation) = fixture();
    let prepared = prepare(&repo, Kind::ReviewFeedback, &args).unwrap();
    repo.transaction(|s| {
        s[JOURNAL]["entries"][0]["extension"] = json!({"keep":"unknown"});
        Ok(())
    })
    .unwrap();
    let resolved=resolve(&repo,&json!({"recordId":args["recordId"],"operationId":prepared["operationId"],"decision":"abandon"})).unwrap();
    assert_eq!(resolved["status"], "prepared");
    let entry = repo.state.lock().unwrap()[JOURNAL]["entries"][0].clone();
    assert!(entry.get("payload").is_none());
    assert!(entry.get("ack").is_none());
    assert_eq!(entry["extension"]["keep"], "unknown");
    assert_eq!(
        untouched(&fake, || submit(&repo, &prepared)).unwrap()["resolution"],
        "user-abandoned"
    );
}

#[test]
fn dispatch_v2_legacy_prepare_migrates_without_reallocation_and_extension_collisions_preserve_bytes(
) {
    for status in ["prepared", "uncertain"] {
        let (_temp, repo, fake, args, _generation, old) = legacy_fixture();
        repo.transaction(|s| {
            s[JOURNAL]["entries"][0]["status"] = json!(status);
            Ok(())
        })
        .unwrap();
        let prepared = untouched(&fake, || {
            prepare_with(&repo, Kind::ReviewFeedback, &args, || {
                panic!("existing legacy operation must not reallocate")
            })
        })
        .unwrap();
        assert_eq!(prepared["type"], "operation");
        assert_eq!(prepared["ticket"], "1");
        assert_eq!(prepared["status"], status);
        assert_eq!(prepared["operationId"], old["operationId"]);
        assert_eq!(repo.state.lock().unwrap()[JOURNAL]["nextTicket"], "2");
    }
    for key in ["nextTicket", "retiredThrough", "entry-ticket"] {
        let (_temp, repo, _fake, _args, _generation, _old) = legacy_fixture();
        repo.transaction(|s| {
            if key == "entry-ticket" {
                s[JOURNAL]["entries"][0]["ticket"] = json!({"legacy":"keep"});
            } else {
                s[JOURNAL][key] = json!({"legacy":"keep"});
            }
            Ok(())
        })
        .unwrap();
        let before = repo.state.lock().unwrap().clone();
        let bytes = std::fs::read(&repo.data).unwrap();
        assert!(ensure_migrated(&repo).is_err());
        assert_eq!(*repo.state.lock().unwrap(), before);
        assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    }
}

#[test]
fn dispatch_v2_small_compaction_keeps_old_live_and_newly_completed_receipts() {
    let (_temp, repo, _fake, args, _generation) = fixture();
    let live = prepare(&repo, Kind::ReviewFeedback, &args).unwrap();
    repo.transaction(|s| {
        let template = s[JOURNAL]["entries"][0].clone();
        let mut all = vec![template.clone()];
        for index in 2..=RECENT_COMPLETED + 2 {
            let mut entry = template.clone();
            entry["operationId"] = json!(uuid());
            entry["ticket"] = json!(index.to_string());
            entry["status"] = json!("written");
            entry["finalized"] = json!(true);
            entry["finalizedAt"] = json!(f64::from(u32::try_from(index).unwrap()));
            all.push(entry);
        }
        s[JOURNAL]["nextTicket"] = json!((RECENT_COMPLETED + 3).to_string());
        store(s, all)
    })
    .unwrap();
    let state = repo.state.lock().unwrap().clone();
    assert_eq!(entries(&state).unwrap().len(), RECENT_COMPLETED + 1);
    assert!(decimal(&state[JOURNAL]["retiredThrough"]).unwrap() > 1);
    assert_eq!(
        locate(&repo, text(&args, "recordId"), text(&live, "operationId")).unwrap()["status"],
        "prepared"
    );
    let done = submit(&repo, &live).unwrap();
    assert_eq!(done["finalized"], true);
    assert_eq!(
        entries(&repo.state.lock().unwrap()).unwrap().len(),
        RECENT_COMPLETED
    );
    assert_eq!(
        locate(&repo, text(&args, "recordId"), text(&live, "operationId")).unwrap()["finalized"],
        true
    );
}
#[test]
fn dispatch_v2_initial_migration_preserves_all_legacy_receipts_before_later_compaction() {
    let (_temp, repo, _fake, args, _generation, _old) = legacy_fixture();
    let original = repo
        .transaction(|s| {
            let template = s[JOURNAL]["entries"][0].clone();
            let mut all = vec![];
            for _ in 0..RECENT_COMPLETED + 2 {
                let mut entry = template.clone();
                entry["operationId"] = json!(uuid());
                entry["finalized"] = json!(true);
                entry["status"] = json!("written");
                all.push(entry);
            }
            s[JOURNAL]["entries"] = json!(all);
            Ok(s[JOURNAL]["entries"].clone())
        })
        .unwrap();
    let prepared = prepare(&repo, Kind::ReviewFeedback, &args).unwrap();
    let state = repo.state.lock().unwrap().clone();
    assert_eq!(entries(&state).unwrap().len(), RECENT_COMPLETED + 3);
    for (index, old) in arr(&original).iter().enumerate() {
        let mut expected = old.clone();
        expected["ticket"] = json!((index + 1).to_string());
        assert_eq!(state[JOURNAL]["entries"][index], expected);
    }
    let done = submit(&repo, &prepared).unwrap();
    assert_eq!(done["finalized"], true);
    assert_eq!(
        entries(&repo.state.lock().unwrap()).unwrap().len(),
        RECENT_COMPLETED
    );
}
#[test]
fn dispatch_v2_legacy_bounds_are_not_relaxed_by_the_migration_allowance() {
    let (_temp, repo, _fake, _args, _generation, _old) = legacy_fixture();
    let original = repo.state.lock().unwrap().clone();
    let mut excessive = original.clone();
    excessive[JOURNAL]["largeNs"] = json!("x".repeat(4 * 1024 * 1024));
    let before = excessive.clone();
    assert!(migrate(&mut excessive).is_err());
    assert_eq!(excessive, before);
    let mut excessive = original;
    let entry = excessive[JOURNAL]["entries"][0].clone();
    excessive[JOURNAL]["entries"] = json!(vec![entry; LEGACY_MAX_ENTRIES + 1]);
    let before = excessive.clone();
    assert!(migrate(&mut excessive).is_err());
    assert_eq!(excessive, before);
}

fn backup_path(repo: &Repository) -> PathBuf {
    repo.data
        .with_extension("session-dispatch-v1-recovery.json")
}
#[test]
fn dispatch_v2_recovery_backup_is_exact_private_and_retry_safe_after_canonical_failure() {
    use std::os::unix::fs::PermissionsExt;
    let (temp, repo, _fake, _args, _generation, _old) = legacy_fixture();
    let original = repo.state.lock().unwrap().clone();
    let bytes = std::fs::read(&repo.data).unwrap();
    let path = backup_path(&repo);
    std::fs::create_dir(repo.data.with_extension("tmp")).unwrap();
    assert!(ensure_migrated(&repo).is_err());
    assert_eq!(*repo.state.lock().unwrap(), original);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        serde_json::to_vec(&original[JOURNAL]).unwrap()
    );
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(!path.with_extension("tmp").exists());
    std::fs::remove_dir(repo.data.with_extension("tmp")).unwrap();
    drop(repo);
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    ensure_migrated(&repo).unwrap();
    assert_eq!(repo.state.lock().unwrap()[JOURNAL]["version"], 2);
    assert_eq!(
        serde_json::from_slice::<Value>(&std::fs::read(&path).unwrap()).unwrap(),
        original[JOURNAL]
    );
    let before = std::fs::read(&path).unwrap();
    ensure_migrated(&repo).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert!(repo.snapshot(true).get(JOURNAL).is_none());
}
#[test]
fn dispatch_v2_recovery_backup_failures_preserve_both_canonical_and_existing_artifact() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    for failure in ["conflict", "symlink", "directory", "non-private"] {
        let (_temp, repo, _fake, _args, _generation, _old) = legacy_fixture();
        let path = backup_path(&repo);
        let target = path.with_extension("symlink-target");
        match failure {
            "directory" => std::fs::create_dir(&path).unwrap(),
            "symlink" => {
                std::fs::write(&target, b"unchanged symlink target").unwrap();
                symlink(&target, &path).unwrap();
            }
            _ => {
                let content = if failure == "conflict" {
                    b"different legacy backup".to_vec()
                } else {
                    serde_json::to_vec(&repo.state.lock().unwrap()[JOURNAL]).unwrap()
                };
                std::fs::write(&path, content).unwrap();
                std::fs::set_permissions(
                    &path,
                    std::fs::Permissions::from_mode(if failure == "non-private" {
                        0o644
                    } else {
                        0o600
                    }),
                )
                .unwrap();
            }
        }
        let before = repo.state.lock().unwrap().clone();
        let canonical = std::fs::read(&repo.data).unwrap();
        let old_backup = std::fs::read(&path).ok();
        assert!(ensure_migrated(&repo).is_err(), "{failure}");
        assert_eq!(*repo.state.lock().unwrap(), before);
        assert_eq!(std::fs::read(&repo.data).unwrap(), canonical);
        assert_eq!(std::fs::read(&path).ok(), old_backup);
        if failure == "symlink" {
            assert_eq!(std::fs::read(&target).unwrap(), b"unchanged symlink target");
        }
        assert!(!path.with_extension("tmp").exists());
    }
}
#[test]
fn dispatch_v2_recovery_backup_includes_explicit_empty_legacy_extensions_but_skips_absent_and_v2() {
    let (_temp, repo, _fake, _args, _generation) = fixture();
    ensure_migrated(&repo).unwrap();
    let path = backup_path(&repo);
    assert!(!path.exists());
    ensure_migrated(&repo).unwrap();
    assert!(!path.exists());
    repo.transaction(|s| {
        s[JOURNAL] =
            json!({"version":1,"entries":[],"extension":{"keep":"empty journal metadata"}});
        Ok(())
    })
    .unwrap();
    let expected = repo.state.lock().unwrap()[JOURNAL].clone();
    ensure_migrated(&repo).unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&std::fs::read(&path).unwrap()).unwrap(),
        expected
    );
}

#[test]
fn dispatch_v2_stale_recovery_temporary_artifacts_do_not_block_restart_migration() {
    for directory in [false, true] {
        let (temp, repo, _fake, _args, _generation, _old) = legacy_fixture();
        let expected = repo.state.lock().unwrap()[JOURNAL].clone();
        let path = backup_path(&repo);
        let stale = path.with_extension("tmp");
        let stale_uuid = path.with_extension(format!("tmp-{}", uuid()));
        if directory {
            std::fs::create_dir(&stale).unwrap();
            std::fs::create_dir(&stale_uuid).unwrap();
        } else {
            std::fs::write(&stale, b"incomplete interrupted backup").unwrap();
            std::fs::write(&stale_uuid, b"interrupted UUID attempt").unwrap();
        }
        drop(repo);
        let repo = Repository::open(temp.path().into(), None, true).unwrap();
        ensure_migrated(&repo).unwrap();
        assert_eq!(repo.state.lock().unwrap()[JOURNAL]["version"], 2);
        assert_eq!(
            serde_json::from_slice::<Value>(&std::fs::read(&path).unwrap()).unwrap(),
            expected
        );
        if directory {
            assert!(stale.is_dir());
            assert!(stale_uuid.is_dir());
        } else {
            assert_eq!(
                std::fs::read(&stale).unwrap(),
                b"incomplete interrupted backup"
            );
            assert_eq!(
                std::fs::read(&stale_uuid).unwrap(),
                b"interrupted UUID attempt"
            );
        }
        let prefix = path.file_stem().unwrap().to_string_lossy().to_string() + ".tmp-";
        let attempts = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(std::result::Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
            .count();
        assert_eq!(attempts, 1, "only the unrelated stale attempt may remain");
    }
}
