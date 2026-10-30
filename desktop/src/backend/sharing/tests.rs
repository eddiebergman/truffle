use super::*;
use crate::backend::sessions;
#[test]
fn saved_chat_open_import_receipt_projection_is_bounded_and_mismatch_stays_unknown() {
    let (_temp, repo) = fixture();
    let session = uuid();
    let delivery = uuid();
    let task = uuid();
    let mut receipt = workflows::open_receipt(&session, "embedded", "accepted", true);
    receipt["message"] = json!("private-open-message");
    receipt["terminal"]["private"] = json!("private-terminal-canary");
    receipt["private"] = json!("private-extension-canary");
    change(&repo, |s| {
        s["entries"][&delivery] = json!({"id":delivery,"sessionId":session,"openReceipt":receipt});
        s["jobs"][&task] = json!({"id":task,"sessionId":session,"openReceipt":receipt});
        Ok(())
    })
    .unwrap();
    let public = view(&repo);
    assert!(!public.to_string().contains("private-"));
    let job = action(&repo, Action::Job, &json!({"id":task})).unwrap();
    assert!(!job.to_string().contains("private-"));
    let repeated = import(&repo, &task, &json!({"deliveryId":delivery})).unwrap();
    assert!(!repeated.to_string().contains("private-"));
    assert_eq!(repeated["openReceipt"]["status"], "accepted");
    for bad in [
        json!({"recordId":"different","status":"accepted"}),
        Value::Null,
        json!({"recordId":session,"status":"accepted","target":"embedded","localSaved":true,"terminal":{"state":"unconfirmed"}}),
    ] {
        let safe = imported_open_receipt(&bad, &session);
        assert_eq!(safe["status"], "unknown");
        assert_eq!(safe["recordId"], session);
        assert_eq!(safe.as_object().unwrap().len(), 6);
    }
}
#[test]
fn saved_chat_open_import_marker_is_persisted_before_open_and_survives_lost_receipt() {
    let (_temp, repo) = fixture();
    let delivery = uuid();
    let task = uuid();
    let session = uuid();
    change(&repo, |s| {
        s["entries"][&delivery] = json!({"id":delivery});
        s["jobs"][&task] = json!({"id":task,"kind":"import","status":"working"});
        Ok(())
    })
    .unwrap();
    let mut calls = 0;
    let receipt = finish_import_open(&repo, &task, &delivery, &session, |repo, id| {
        calls += 1;
        assert_eq!(id, session);
        let saved = load(repo.data.parent().unwrap()).unwrap();
        assert_eq!(
            saved["entries"][&delivery]["openReceipt"]["status"],
            "unknown"
        );
        assert_eq!(saved["jobs"][&task]["openReceipt"]["recordId"], session);
        let file = base(repo).join("state.json");
        fs::remove_file(&file).unwrap();
        fs::create_dir(file).unwrap();
        Ok(workflows::open_receipt(id, "embedded", "accepted", true))
    })
    .unwrap();
    assert_eq!(calls, 1);
    assert_eq!(receipt["status"], "unknown");
    let again = import(&repo, &task, &json!({"deliveryId":delivery})).unwrap();
    assert_eq!(again["sessionId"], session);
    assert_eq!(again["openReceipt"]["status"], "unknown");
    assert_eq!(calls, 1);
    let projected = view(&repo);
    assert_eq!(projected["entries"][0]["openReceipt"]["recordId"], session);
    assert_eq!(projected["jobs"][0]["openReceipt"]["status"], "unknown");
}
#[test]
fn saved_chat_open_import_failed_admission_never_opens_and_preeffect_error_clears_only_marker() {
    for fail in [false, true] {
        let (_temp, repo) = fixture();
        let delivery = uuid();
        let task = uuid();
        let session = uuid();
        change(&repo, |s| {
            s["entries"][&delivery] = json!({"id":delivery,"privateNs":"keep"});
            s["jobs"][&task] = json!({"id":task,"kind":"import"});
            Ok(())
        })
        .unwrap();
        if fail {
            let file = base(&repo).join("state.json");
            fs::remove_file(&file).unwrap();
            fs::create_dir(file).unwrap();
        }
        let mut calls = 0;
        assert!(
            finish_import_open(&repo, &task, &delivery, &session, |_, _| {
                calls += 1;
                Err(Error::new("Invalid saved folder"))
            })
            .is_err()
        );
        assert_eq!(calls, i32::from(!fail));
        let s = repo.sharing.lock().unwrap();
        assert!(s["entries"][&delivery]["openReceipt"].is_null());
        assert_eq!(s["entries"][&delivery]["privateNs"], "keep");
        if !fail {
            assert_eq!(s["entries"][&delivery]["sessionId"], session);
            assert_eq!(s["entries"][&delivery]["imported"], true);
            assert!(s["jobs"][&task]["openReceipt"].is_null());
        }
        drop(s);
    }
}
#[test]
fn sharing_state_write_limit_matches_load_and_rejects_without_losing_import_identity() {
    let (_temp, repo) = fixture();
    change(&repo, |state| {
        state["importKeys"]["verified-bundle"] = json!("existing-chat");
        state["jobs"]["large"] = json!({"status":"ready","output":""});
        Ok(())
    })
    .unwrap();
    let baseline = repo.sharing.lock().unwrap().clone();
    let mut persisted = baseline;
    if config_path(&repo).is_file() {
        persisted.as_object_mut().unwrap().remove("config");
    }
    let overhead = serde_json::to_vec_pretty(&persisted).unwrap().len();
    let padding = "x".repeat(usize::try_from(MAX_STATE).unwrap() - overhead);
    change(&repo, |state| {
        state["jobs"]["large"]["output"] = json!(padding);
        Ok(())
    })
    .unwrap();
    let path = base(&repo).join("state.json");
    let bytes = fs::read(&path).unwrap();
    assert_eq!(bytes.len() as u64, MAX_STATE);
    let saved = repo.sharing.lock().unwrap().clone();
    assert_eq!(load(repo.data.parent().unwrap()).unwrap(), saved);
    let error = change(&repo, |state| {
        let mut output = text(&state["jobs"]["large"], "output").to_owned();
        output.push('x');
        state["jobs"]["large"]["output"] = json!(output);
        state["importKeys"]["verified-bundle"] = json!("replacement-chat");
        Ok(())
    })
    .unwrap_err();
    assert!(error.message.contains("16 MiB"));
    assert_eq!(*repo.sharing.lock().unwrap(), saved);
    assert_eq!(fs::read(path).unwrap(), bytes);
    assert_eq!(
        load(repo.data.parent().unwrap()).unwrap()["importKeys"]["verified-bundle"],
        "existing-chat"
    );
}

#[test]
fn preparation_carries_attached_results_and_keeps_large_previews_off_the_poll_path() {
    let (_temp, repo) = fixture();
    let kit_root = repo.root.parent().unwrap().join("context-kit");
    write(
        &kit_root.join("skill/handoff-context/SKILL.md"),
        b"Test fixture instructions",
    )
    .unwrap();
    write(
        &kit_root.join("protocol/FORMAT.md"),
        b"Context Bundle 1.0 test fixture",
    )
    .unwrap();
    let record=repo.transaction(|s|sessions::save(s,&json!({"agent":"claude","cwd":repo.root,"title":"Attached results","destination":"embedded","itemIds":[]}))).unwrap();
    let csv = repo.root.join("measurements.csv");
    write(&csv, b"metric,value\np50,12\n").unwrap();
    repo.transaction(|s| {
        artifacts::save(
            &repo,
            s,
            &json!({"path":csv,"title":"Latency table","sessionIds":[record["id"]]}),
        )
    })
    .unwrap();
    let id = uuid();
    let ready = prepare(
        &repo,
        &id,
        &json!({"recordId":record["id"],"recentConversation":false}),
    )
    .unwrap();
    assert_eq!(ready["resourceCount"], 1);
    let bundle = Bundle::load(Path::new(text(&ready, "bundlePath"))).unwrap();
    assert_eq!(bundle.raw()["resources"][0]["media_type"], "text/csv");
    job(&repo, &id, ready).unwrap();
    assert!(repo.sharing.lock().unwrap()["jobs"][&id]["preview"].is_null());
    assert!(view(&repo)["jobs"][0]["preview"].is_null());
    let details = action(&repo, Action::Job, &json!({"id":id})).unwrap();
    assert_eq!(details["preview"]["resourceCount"], 1);
    let script = repo.root.join("reproduce.py");
    write(&script, b"def main():\n    print('attached source')\n").unwrap();
    repo.transaction(|s| {
        artifacts::save(
            &repo,
            s,
            &json!({"path":script,"title":"Reproduction script","sessionIds":[record["id"]]}),
        )
    })
    .unwrap();
    let ready = prepare(
        &repo,
        &uuid(),
        &json!({"recordId":record["id"],"recentConversation":false}),
    )
    .unwrap();
    let bundle = Bundle::load(Path::new(text(&ready, "bundlePath"))).unwrap();
    assert_eq!(ready["resourceCount"], 2);
    assert!(arr(&bundle.raw()["resources"])
        .iter()
        .any(|r| r["media_type"] == "text/x-python"
            && Path::new(text(r, "name"))
                .extension()
                .is_some_and(|extension| extension == "py")));
}
#[test]
fn preparation_view_exposes_captured_conversation_option_without_payload() {
    let (_temp, repo) = fixture();
    let mut expected = Vec::new();
    for recent in [Some(true), Some(false), None] {
        let id = uuid();
        let mut payload =
            json!({"recordId":uuid(),"privateExtension":"private-prepare-payload-sentinel"});
        if let Some(recent) = recent {
            payload["recentConversation"] = json!(recent);
        }
        job(
            &repo,
            &id,
            json!({"id":id,"kind":"prepare","status":"working","payload":payload}),
        )
        .unwrap();
        expected.push((id, recent.unwrap_or(true)));
    }
    let before = fs::read(base(&repo).join("state.json")).unwrap();
    let projected = view(&repo);
    for (id, recent) in expected {
        let public = arr(&projected["jobs"])
            .into_iter()
            .find(|job| job["id"] == id)
            .unwrap();
        assert_eq!(public["recentConversation"], recent);
        assert!(public.get("payload").is_none());
    }
    assert!(!projected
        .to_string()
        .contains("private-prepare-payload-sentinel"));
    assert_eq!(fs::read(base(&repo).join("state.json")).unwrap(), before);
}
#[test]
fn preparation_reuses_only_matching_conversation_options_without_starting_work() {
    for prepared_recent in [true, false] {
        for requested_recent in [Some(true), Some(false), None] {
            // Independent fixtures avoid the owner's deliberate per-action cooldown.
            let (_temp, repo) = fixture();
            let record = repo.transaction(|state| {
                    let record = sessions::save(state, &json!({"agent":"claude","cwd":repo.root,"title":"Registered conversation","destination":"embedded","itemIds":[]}))?;
                    let saved = state["sessions"].as_array_mut().unwrap().iter_mut().find(|saved| saved["id"] == record["id"]).unwrap();
                    saved["sessionId"] = json!(uuid());
                    Ok(saved.clone())
                }).unwrap();
            let job_id = uuid();
            job(&repo, &job_id, json!({"id":job_id,"kind":"prepare","status":"working","payload":{"recordId":record["id"],"recentConversation":prepared_recent}})).unwrap();
            let state_before = repo.sharing.lock().unwrap().clone();
            let sharing_before = fs::read(base(&repo).join("state.json")).unwrap();
            let core_before = fs::read(&repo.data).unwrap();
            let mut args = json!({"recordId":record["id"]});
            if let Some(recent) = requested_recent {
                args["recentConversation"] = json!(recent);
            }
            let result = action(&repo, Action::Prepare, &args);
            if prepared_recent == requested_recent.unwrap_or(true) {
                assert_eq!(result.unwrap()["jobId"], job_id);
            } else {
                let error = result.unwrap_err();
                assert!(error.message.contains("different conversation option"));
                assert!(error.message.contains("Wait for it to finish"));
            }
            assert_eq!(*repo.sharing.lock().unwrap(), state_before);
            assert_eq!(
                fs::read(base(&repo).join("state.json")).unwrap(),
                sharing_before
            );
            assert_eq!(fs::read(&repo.data).unwrap(), core_before);
            assert!(repo.sharing_busy.lock().unwrap().is_empty());
        }
    }
}
#[test]
fn oversized_log_events_do_not_swallow_the_next_conversation_turn() {
    let t = tempfile::tempdir().unwrap();
    let path = t.path().join("chat.jsonl");
    let huge = json!({"type":"user","message":{"content":[{"type":"tool_result","content":"x".repeat(4*1024*1024+100)}]}});
    let next = json!({"type":"assistant","message":{"content":"The useful answer"}});
    fs::write(&path, format!("{huge}\n{next}\n")).unwrap();
    let messages = recent_messages(&path).unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["text"], "The useful answer");
}
fn fixture() -> (tempfile::TempDir, Arc<Repository>) {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("truffle");
    fs::create_dir(&root).unwrap();
    let repo = Repository::open(root, None, true).unwrap();
    change(&repo, |s| {
        s["config"] = json!({"handle":"eddie","root":"gs://misc/context-bundles/v1"});
        Ok(())
    })
    .unwrap();
    (t, repo)
}
fn draft(repo: &Repository) -> String {
    let id = uuid();
    let dir = base(repo).join("drafts").join(&id);
    let bytes = b"name,value\nlatency,12\n";
    write(&dir.join("files/measurements.csv"), bytes).unwrap();
    let digest = context_kit::sha256(bytes);
    let raw = json!({"format":"context-bundle","version":"1.0","title":"Shared test","summary":"Measure latency; no new task is assigned.","authors":[{"name":"Eddie","handle":"eddie"}],"items":[{"id":"table","kind":"table","title":"Measurements","resources":["csv"]},{"id":"conversation","kind":"conversation","data":{"messages":[{"role":"user","text":"Can we compare latency?"},{"role":"assistant","text":"The measurement is 12."}]}}],"resources":[{"id":"csv","name":"measurements.csv","href":"files/measurements.csv","media_type":"text/csv","size":bytes.len(),"sha256":digest}],"uris":{"anything":["gs://another-bucket/huge-object"]}});
    write(
        &dir.join("context.json"),
        &serde_json::to_vec_pretty(&raw).unwrap(),
    )
    .unwrap();
    change(repo,|s|{s["jobs"][&id]=json!({"id":id,"kind":"prepare","status":"ready","title":"Shared test","bundlePath":dir.join("context.json")});Ok(())}).unwrap();
    id
}
fn send_payload(repo: &Repository, draft: &str) -> Value {
    json!({"draftId":draft,"config":configuration(repo).unwrap(),"shareId":uuid(),"created_at":"2026-10-23T12:00:00Z","recipients":["eddie"],"deliveryIds":{"eddie":uuid()},"message":"Review this context."})
}
#[test]
fn configuration_commit_serializes_file_and_cache_reset_with_job_startup() {
    use std::sync::{mpsc, TryLockError};
    let (_temp, repo) = fixture();
    let payload = send_payload(&repo, "draft");
    change(&repo, |s| {
        s["entries"]["old"] = json!({"id":"old"});
        Ok(())
    })
    .unwrap();
    let (written_tx, written_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let writer_repo = repo.clone();
    let writer = std::thread::spawn(move || {
        with_idle_configuration(&writer_repo, || {
            let config = SharingConfig {
                version: 1,
                revision: 1,
                handle: "sam".into(),
                transport: Transport::Gcs {
                    root: "gs://new-team/context".into(),
                },
            };
            write(&config_path(&writer_repo), &serde_json::to_vec(&config)?)?;
            // Pause in the exact gap that previously admitted a job against stale caches.
            written_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            change(&writer_repo, |s| {
                s["config"] = serde_json::to_value(&config)?;
                s["entries"] = json!({});
                Ok(())
            })
        })
        .unwrap();
    });
    written_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(matches!(
        repo.sharing_busy.try_lock(),
        Err(TryLockError::WouldBlock)
    ));
    let starter_repo = repo.clone();
    let starter = std::thread::spawn(move || reserve_job(&starter_repo, "send", "retry", &payload));
    release_tx.send(()).unwrap();
    writer.join().unwrap();
    let error = starter.join().unwrap().unwrap_err();
    assert!(error.message.contains("Sharing destination changed"));
    assert!(repo.sharing_busy.lock().unwrap().is_empty());
    assert!(repo.sharing.lock().unwrap()["entries"]
        .as_object()
        .unwrap()
        .is_empty());
}
#[test]
fn failed_send_retry_refuses_changed_transport_root_or_identity_but_keeps_stable_ids() {
    let (temp, repo) = fixture();
    let draft = draft(&repo);
    let payload = send_payload(&repo, &draft);
    let id = uuid();
    job(
        &repo,
        &id,
        json!({"id":id,"kind":"send","status":"failed","payload":payload}),
    )
    .unwrap();
    let original = configuration(&repo).unwrap();
    for (name, transport) in [
        (
            "eddie",
            json!({"kind":"gcs","root":"gs://another-team/context"}),
        ),
        ("sam", original["transport"].clone()),
        (
            "eddie",
            json!({"kind":"folder","root":temp.path().canonicalize().unwrap()}),
        ),
    ] {
        let current = config_view(&repo).unwrap();
        action(&repo, Action::Configure, &json!({"revision":current["revision"],"contentHash":current["contentHash"],"handle":name,"transport":transport})).unwrap();
        let error = action(
            &repo,
            Action::Send,
            &json!({"draftId":draft,"recipients":["eddie"],"retryId":id}),
        )
        .unwrap_err();
        assert!(error.message.contains("Prepare a new share"));
        assert_eq!(
            repo.sharing.lock().unwrap()["jobs"][&id]["status"],
            "failed"
        );
        assert!(repo.sharing_busy.lock().unwrap().is_empty());
    }
    let current = config_view(&repo).unwrap();
    action(&repo, Action::Configure, &json!({"revision":current["revision"],"contentHash":current["contentHash"],"handle":"eddie","transport":original["transport"]})).unwrap();
    // Revision/hash changes alone do not invalidate the immutable send identity.
    assert!(reserve_job(&repo, "send", &id, &payload).unwrap());
    assert!(!reserve_job(&repo, "send", &id, &payload).unwrap());
    let current = config_view(&repo).unwrap();
    assert!(action(&repo, Action::Configure, &json!({"revision":current["revision"],"contentHash":current["contentHash"],"handle":"eddie","transport":original["transport"]})).unwrap_err().message.contains("Wait for the current sharing operation"));
    repo.sharing_busy
        .lock()
        .unwrap()
        .remove(&format!("send:{id}"));
    let first = send(&repo, &id, &payload).unwrap();
    let retry = send(&repo, &id, &payload).unwrap();
    assert_eq!(first["uri"], retry["uri"]);
    assert_eq!(
        repo.sharing.lock().unwrap()["jobs"][&id]["payload"],
        payload
    );
}
#[test]
fn direct_handles_need_no_directory_and_preserve_names() {
    assert_eq!(handle(" /Eddie ").unwrap(), "eddie");
    for s in ["../eddie", "/", "//eddie", "first last", "a/b"] {
        assert!(handle(s).is_err());
    }
}
#[test]
fn recipient_discovery_uses_only_immediate_names_and_optional_display_names() {
    let (_t, repo) = fixture();
    let root = base(&repo).join("demo-team");
    for id in ["alex", "eddie", "sam", "Bad Name", "UPPER", "hidden"] {
        write(
            &root.join(format!("inboxes/{id}/deliveries/not-a-delivery.json")),
            b"foreign private content must not be parsed",
        )
        .unwrap();
    }
    write(&root.join("inboxes/stray-file"), b"not a directory").unwrap();
    refresh_once(&repo).unwrap();
    let people = arr(&view(&repo)["people"]);
    assert_eq!(
        people.iter().map(|p| text(p, "id")).collect::<Vec<_>>(),
        ["alex", "eddie", "hidden", "sam"]
    );
    assert_eq!(arr(&view(&repo)["entries"]), [] as [serde_json::Value; 0]);
    write(&root.join("directory.json"), &serde_json::to_vec(&json!({"format":"context-directory","version":"1.0","people":[{"id":"alex","name":"Alex Example","aliases":["secret"]},{"id":"hidden","active":false},{"id":"sam","name":"One"},{"id":"sam","name":"Two"},{"id":"new","name":"New Person"}]})).unwrap()).unwrap();
    refresh_once(&repo).unwrap();
    let people = arr(&view(&repo)["people"]);
    assert_eq!(
        people.iter().map(|p| text(p, "id")).collect::<Vec<_>>(),
        ["alex", "eddie", "new", "sam"]
    );
    assert_eq!(people[0]["name"], "Alex Example");
    assert_eq!(people[3]["name"], "sam"); // ambiguous metadata cannot rename the discovered handle
    assert!(!serde_json::to_string(&people).unwrap().contains("secret"));
    fs::remove_dir_all(root.join("inboxes/alex")).unwrap();
    fs::remove_file(root.join("directory.json")).unwrap();
    refresh_once(&repo).unwrap();
    assert!(!arr(&view(&repo)["people"])
        .iter()
        .any(|p| p["id"] == "alex" || p["id"] == "new"));
}
#[test]
fn recipient_discovery_resumes_pages_and_preserves_cached_names_until_complete() {
    let (_t, repo) = fixture();
    for n in 0..451 {
        write(
            &base(&repo).join(format!("demo-team/inboxes/person-{n:04}/deliveries/opaque")),
            b"not read",
        )
        .unwrap();
    }
    refresh_once(&repo).unwrap();
    assert_eq!(view(&repo)["peopleHasMore"], true);
    assert_eq!(arr(&view(&repo)["people"]).len(), 400);
    let persisted = load(repo.data.parent().unwrap()).unwrap();
    assert_eq!(persisted["peoplePageToken"], "400");
    refresh_once(&repo).unwrap();
    assert_eq!(view(&repo)["peopleHasMore"], false);
    assert_eq!(arr(&view(&repo)["people"]).len(), 451);
    refresh_once(&repo).unwrap();
    assert_eq!(arr(&view(&repo)["people"]).len(), 451);
    refresh_once(&repo).unwrap();
    assert_eq!(arr(&view(&repo)["people"]).len(), 451);
}
#[test]
fn import_retry_recovers_core_session_after_actual_sharing_reference_write_failure() {
    let (_temp, repo) = fixture();
    let draft = draft(&repo);
    let payload = send_payload(&repo, &draft);
    let send_id = uuid();
    job(&repo, &send_id, json!({"id":send_id,"kind":"send"})).unwrap();
    send(&repo, &send_id, &payload).unwrap();
    refresh_once(&repo).unwrap();
    let delivery = text(&payload["deliveryIds"], "eddie");
    let import_id = uuid();
    job(&repo, &import_id, json!({"id":import_id,"kind":"import"})).unwrap();
    let args = json!({"deliveryId":delivery,"agent":"claude","cwd":repo.root});
    // Only sharing-state persistence fails; the independent core session transaction remains writable.
    let sharing_file = base(&repo).join("state.json");
    let saved = fs::read(&sharing_file).unwrap();
    fs::remove_file(&sharing_file).unwrap();
    fs::create_dir(&sharing_file).unwrap();
    assert!(import(&repo, &import_id, &args).is_err());
    let sessions = arr(&repo.state.lock().unwrap()["sessions"]);
    assert_eq!(sessions.len(), 1);
    let session_id = sessions[0]["id"].clone();
    assert_eq!(sessions[0]["sharingImportJobId"], import_id);
    assert!(repo.sharing.lock().unwrap()["jobs"][&import_id]["sessionId"].is_null());
    fs::remove_dir(&sharing_file).unwrap();
    fs::write(&sharing_file, saved).unwrap();
    let imported = import(&repo, &import_id, &args).unwrap();
    assert_eq!(imported["sessionId"], session_id);
    assert_eq!(arr(&repo.state.lock().unwrap()["sessions"]).len(), 1);
    assert_eq!(
        repo.sharing.lock().unwrap()["jobs"][&import_id]["sessionId"],
        session_id
    );
    let core: Value = serde_json::from_slice(&fs::read(&repo.data).unwrap()).unwrap();
    assert_eq!(core["sessions"][0]["sharingImportJobId"], import_id);
}
#[test]
fn import_cache_identity_and_digest_keep_reused_share_ids_immutable() {
    let (temp, repo) = fixture();
    let folder = temp.path().join("company");
    fs::create_dir(&folder).unwrap();
    change(&repo,|state| {state["config"]=json!({"version":1,"revision":0,"handle":"eddie","transport":{"kind":"folder","root":folder.canonicalize().unwrap()}});Ok(())}).unwrap();
    let share = uuid();
    let first = uuid();
    let second = uuid();
    let store = Store::new(&repo, &configuration(&repo).unwrap()).unwrap();
    let mut manifests = vec![];
    for (delivery, title) in [
        (&first, "First preserved handover"),
        (&second, "Second different handover"),
    ] {
        let manifest = json!({"format":"context-bundle","version":"1.0","title":title,"summary":"Verified local fixture.","authors":[],"items":[],"resources":[]});
        let bytes = serde_json::to_vec_pretty(&manifest).unwrap();
        let digest = context_kit::sha256(&bytes);
        let key = format!("manifests/sha256/{digest}/context.json");
        store.put(&key, &bytes).unwrap();
        change(&repo,|state| {state["entries"][delivery]=json!({"id":delivery,"shareId":share,"bundle":{"uri":format!("{}/{}",store.root,key),"sha256":digest,"size":bytes.len()}});Ok(())}).unwrap();
        manifests.push(bytes);
    }
    let old = base(&repo)
        .join("imports")
        .join(&share)
        .join("original-context.json");
    write(&old, b"Previous chat path must remain unchanged.").unwrap();
    let first_job = uuid();
    job(&repo, &first_job, json!({"id":first_job,"kind":"import"})).unwrap();
    let imported = import(
        &repo,
        &first_job,
        &json!({"deliveryId":first,"agent":"claude","cwd":repo.root}),
    )
    .unwrap();
    let (_, first_dir, _) = fetched_manifest(&repo, &first).unwrap();
    let original = fs::read(first_dir.join("original-context.json")).unwrap();
    let materialized = fs::read(first_dir.join("context.json")).unwrap();
    let handover = fs::read(first_dir.join("handover.txt")).unwrap();
    preview(&repo, "preview-second", &json!({"deliveryId":second})).unwrap();
    let (_, second_dir, _) = fetched_manifest(&repo, &second).unwrap();
    assert_ne!(first_dir, second_dir);
    assert_eq!(
        fs::read(second_dir.join("original-context.json")).unwrap(),
        manifests[1]
    );
    assert_eq!(
        fs::read(first_dir.join("original-context.json")).unwrap(),
        original
    );
    assert_eq!(
        fs::read(first_dir.join("context.json")).unwrap(),
        materialized
    );
    assert_eq!(fs::read(first_dir.join("handover.txt")).unwrap(), handover);
    let instruction = arr(&repo.state.lock().unwrap()["sessions"])
        .into_iter()
        .find(|record| record["id"] == imported["sessionId"])
        .unwrap()["instruction"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(instruction.contains(first_dir.to_str().unwrap()));
    change(&repo, |state| {
        state["config"]["handle"] = json!("sam");
        Ok(())
    })
    .unwrap();
    let (_, different_identity, _) = fetched_manifest(&repo, &first).unwrap();
    assert_ne!(different_identity, first_dir);
    assert_eq!(
        fs::read(different_identity.join("original-context.json")).unwrap(),
        manifests[0]
    );
    assert_eq!(
        fs::read(&old).unwrap(),
        b"Previous chat path must remain unchanged."
    );
    assert_eq!(
        fs::read(first_dir.join("original-context.json")).unwrap(),
        original
    );
}

#[test]
fn local_send_retry_discovery_preview_and_import_are_usable() {
    let (_t, repo) = fixture();
    let draft = draft(&repo);
    let p = send_payload(&repo, &draft);
    let id = uuid();
    job(&repo, &id, json!({"id":id,"kind":"send"})).unwrap();
    let a = send(&repo, &id, &p).unwrap();
    let b = send(&repo, &id, &p).unwrap();
    assert_eq!(a["uri"], b["uri"]);
    assert!(!base(&repo).join("demo-team/directory.json").exists());
    refresh_once(&repo).unwrap();
    let v = view(&repo);
    assert_eq!(arr(&v["entries"]).len(), 1);
    assert_eq!(v["unread"], 1);
    let delivery = text(&p["deliveryIds"], "eddie");
    let preview = preview(&repo, "unused", &json!({"deliveryId":delivery})).unwrap();
    assert_eq!(preview["preview"]["title"], "Shared test");
    let job_id = uuid();
    job(&repo, &job_id, json!({"id":job_id,"kind":"import"})).unwrap();
    let payload = json!({"deliveryId":delivery,"agent":"claude","cwd":repo.root});
    let imported = import(&repo, &job_id, &payload).unwrap();
    let again = import(&repo, &job_id, &payload).unwrap();
    assert_eq!(imported["sessionId"], again["sessionId"]);
    assert_eq!(arr(&repo.state.lock().unwrap()["sessions"]).len(), 1);
    assert_eq!(arr(&repo.state.lock().unwrap()["artifacts"]).len(), 1);
    assert_eq!(view(&repo)["entries"][0]["imported"], true);
}
#[test]
fn one_bad_delivery_does_not_hide_good_context() {
    let (_t, repo) = fixture();
    let draft = draft(&repo);
    let p = send_payload(&repo, &draft);
    let id = uuid();
    job(&repo, &id, json!({"id":id})).unwrap();
    send(&repo, &id, &p).unwrap();
    let bad = uuid();
    write(
        &base(&repo)
            .join("demo-team/inboxes/eddie/deliveries")
            .join(format!("{bad}.json")),
        b"{invalid",
    )
    .unwrap();
    refresh_once(&repo).unwrap();
    let entries = arr(&view(&repo)["entries"]);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries.iter().filter(|e| !e["error"].is_null()).count(), 1);
}
#[test]
fn conversation_excerpt_excludes_tools_and_machine_notifications() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("session.jsonl");
    fs::write(&path,[json!({"type":"user","uuid":"1","message":{"content":[{"type":"text","text":"Question"},{"type":"tool_result","content":"SECRET TOOL OUTPUT"}]}}),json!({"type":"assistant","uuid":"2","message":{"content":[{"type":"text","text":"Answer"},{"type":"tool_use","input":{"secret":"not chat"}}]}}),json!({"type":"user","uuid":"3","message":{"content":"<task-notification>not user conversation"}})].iter().map(|v|v.to_string()+"\n").collect::<String>()).unwrap();
    let messages = recent_messages(&path).unwrap();
    assert_eq!(messages.len(), 2);
    let serialized = serde_json::to_string(&messages).unwrap();
    assert!(!serialized.contains("SECRET"));
    assert!(!serialized.contains("task-notification"));
}
#[test]
fn immutable_conflicts_and_unsafe_uris_fail_without_overwrite() {
    let (_t, repo) = fixture();
    let store = Store::new(&repo, &configuration(&repo).unwrap()).unwrap();
    store.put("test/value", b"first").unwrap();
    assert!(store.put("test/value", b"second").is_err());
    assert_eq!(
        store.read("test/value", 100, None).unwrap().unwrap(),
        b"first"
    );
    for uri in [
        "https://example.org/token",
        "gs://other-bucket/x",
        "gs://misc/context-bundles/v1/../escape",
    ] {
        assert!(store.object(uri).is_err());
    }
}
#[test]
fn folder_objects_are_atomic_immutable_bounded_and_do_not_follow_symlinks() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().canonicalize().unwrap();
    let store = Store {
        root: Url::from_directory_path(&directory)
            .unwrap()
            .as_str()
            .trim_end_matches('/')
            .into(),
        backend: StoreBackend::Folder {
            directory: directory.clone(),
        },
    };
    store
        .put("inboxes/alex/deliveries/object.json", b"first")
        .unwrap();
    store
        .put("inboxes/alex/deliveries/object.json", b"first")
        .unwrap();
    assert!(store
        .put("inboxes/alex/deliveries/object.json", b"second")
        .is_err());
    let listing = store.list("inboxes/alex/deliveries/", "").unwrap();
    let generation = text(&listing["items"][0], "generation");
    assert_eq!(
        store
            .read("inboxes/alex/deliveries/object.json", 5, Some(generation))
            .unwrap()
            .unwrap(),
        b"first"
    );
    assert!(store
        .read("inboxes/alex/deliveries/object.json", 4, None)
        .is_err());
    fs::write(
        directory.join("inboxes/alex/deliveries/object.json"),
        b"changed",
    )
    .unwrap();
    assert!(store
        .read("inboxes/alex/deliveries/object.json", 100, Some(generation))
        .is_err());
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret"), b"outside").unwrap();
    symlink(outside.path(), directory.join("escape")).unwrap();
    symlink(outside.path().join("secret"), directory.join("leaf")).unwrap();
    assert!(store.read("escape/secret", 100, None).is_err());
    assert!(store.read("leaf", 100, None).is_err());
    assert!(store.put("escape/new", b"no").is_err());
    assert!(!outside.path().join("new").exists());
    assert!(store.object("file:///outside/context.json").is_err());
    assert!(store.object("https://example.org/context.json").is_err());
    assert!(store.object(&(store.root.clone() + "/../outside")).is_err());
    assert!(store.read("../outside", 100, None).is_err());
    assert_eq!(
        fs::read_dir(directory.join("inboxes/alex/deliveries"))
            .unwrap()
            .count(),
        1
    );
}
#[test]
fn folder_send_preview_and_import_preserve_portable_bundle_and_retry_identity() {
    let (temp, repo) = fixture();
    let folder = temp.path().join("company folder");
    fs::create_dir(&folder).unwrap();
    let folder = folder.canonicalize().unwrap();
    change(&repo,|state|{state["config"]=json!({"version":1,"revision":0,"handle":"eddie","transport":{"kind":"folder","root":folder}});Ok(())}).unwrap();
    let draft = draft(&repo);
    let draft_path = PathBuf::from(text(
        &repo.sharing.lock().unwrap()["jobs"][&draft],
        "bundlePath",
    ));
    let mut raw: Value = serde_json::from_slice(&fs::read(&draft_path).unwrap()).unwrap();
    raw["items"].as_array_mut().unwrap().push(json!({"id":"unknown","kind":"example.org/extension","title":"Unknown fallback","text":"Keep this context","data":{"original":true}}));
    fs::write(&draft_path, serde_json::to_vec_pretty(&raw).unwrap()).unwrap();
    let payload = send_payload(&repo, &draft);
    let id = uuid();
    job(&repo, &id, json!({"id":id,"kind":"send"})).unwrap();
    let first = send(&repo, &id, &payload).unwrap();
    let retry = send(&repo, &id, &payload).unwrap();
    assert_eq!(first["uri"], retry["uri"]);
    assert!(text(&first, "uri").starts_with("file://"));
    let store = Store::new(&repo, &configuration(&repo).unwrap()).unwrap();
    let StoreBackend::Folder { directory } = &store.backend else {
        panic!()
    };
    let manifest = directory.join(store.object(text(&first, "uri")).unwrap());
    let bundle = Bundle::load(&manifest).unwrap();
    assert!(bundle
        .inspect(manifest.parent().unwrap())
        .resources
        .iter()
        .all(|resource| resource.status == "verified"));
    assert_eq!(bundle.raw()["items"][2]["data"]["original"], true);
    assert_eq!(
        bundle.raw()["uris"]["anything"][0],
        "gs://another-bucket/huge-object"
    );
    let delivery = text(&payload["deliveryIds"], "eddie");
    let bytes = store
        .read(
            &format!("inboxes/eddie/deliveries/{delivery}.json"),
            MAX_RECORD,
            None,
        )
        .unwrap()
        .unwrap();
    let record: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        record["extensions"]["org.truffle/folder-storage"]["version"],
        1
    );
    refresh_once(&repo).unwrap();
    preview(&repo, "preview", &json!({"deliveryId":delivery})).unwrap();
    let (_, imported_dir, _) = fetched_manifest(&repo, delivery).unwrap();
    assert!(!imported_dir.join("objects").exists());
    let job_id = uuid();
    job(&repo, &job_id, json!({"id":job_id,"kind":"import"})).unwrap();
    let args = json!({"deliveryId":delivery,"agent":"claude","cwd":repo.root});
    let imported = import(&repo, &job_id, &args).unwrap();
    let again = import(&repo, &job_id, &args).unwrap();
    assert_eq!(imported["sessionId"], again["sessionId"]);
    refresh_once(&repo).unwrap();
    assert_eq!(arr(&repo.state.lock().unwrap()["sessions"]).len(), 1);
    assert_eq!(arr(&repo.state.lock().unwrap()["artifacts"]).len(), 1);
    assert_eq!(
        load(repo.data.parent().unwrap()).unwrap()["entries"][delivery]["imported"],
        true
    );
}
#[test]
fn folder_configuration_cas_and_profiles_preserve_existing_gcs_configuration() {
    let (temp, repo) = fixture();
    let previous = view(&repo)["config"].clone();
    assert_eq!(previous["transport"]["kind"], "gcs");
    assert_eq!(previous["root"], "gs://misc/context-bundles/v1");
    let folder = temp.path().canonicalize().unwrap();
    let payload = json!({"revision":previous["revision"],"contentHash":previous["contentHash"],"handle":"sam","transport":{"kind":"folder","root":folder}});
    let result = action(&repo, Action::Configure, &payload).unwrap();
    assert_eq!(result["config"]["revision"], 1);
    assert_eq!(result["config"]["transport"]["kind"], "folder");
    assert_eq!(
        action(&repo, Action::Configure, &payload)
            .unwrap_err()
            .status,
        409
    );
    let persisted: Value =
        serde_json::from_slice(&fs::read(base(&repo).join("state.json")).unwrap()).unwrap();
    assert!(persisted.get("config").is_none());
    assert_eq!(
        load(repo.data.parent().unwrap()).unwrap()["config"]["handle"],
        "sam"
    );
    let stale = json!({"revision":result["config"]["revision"],"contentHash":result["config"]["contentHash"],"handle":"sam","transport":{"kind":"folder","root":folder}});
    let mut external: Value =
        serde_json::from_slice(&fs::read(config_path(&repo)).unwrap()).unwrap();
    external["handle"] = json!("alex");
    fs::write(config_path(&repo), serde_json::to_vec(&external).unwrap()).unwrap();
    assert_eq!(
        action(&repo, Action::Configure, &stale).unwrap_err().status,
        409
    );
    external["version"] = json!(2);
    assert!(parse_config(&external).is_err());
}
