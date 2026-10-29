use super::*;
use std::sync::Mutex;
static ENV: Mutex<()> = Mutex::new(());
fn fixture() -> (tempfile::TempDir, std::sync::Arc<Repository>) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("mydash");
    std::fs::create_dir(&root).unwrap();
    let repo = Repository::open(root, None, true).unwrap();
    (temp, repo)
}
#[test]
fn canonical_storage_survives_restart_and_rejects_two_writers() {
    let (temp, repo) = fixture();
    repo.edit(&json!({"action":"save-note","title":"Local project","status":"blocked"}))
        .unwrap();
    let id = repo.snapshot(false)["notes"][0]["id"].clone();
    repo.edit(&json!({"action":"override","id":id,"value":{"hidden":true,"pinned":true}}))
        .unwrap();
    assert!(Repository::open(repo.root.clone(), None, true).is_err());
    let root = repo.root.clone();
    drop(repo);
    let reopened = Repository::open(root, None, true).unwrap();
    assert_eq!(
        reopened.snapshot(false)["notes"][0]["title"],
        "Local project"
    );
    assert_eq!(
        reopened.snapshot(false)["overrides"][id.as_str().unwrap()]["hidden"],
        true
    );
    drop(reopened);
    drop(temp);
}
#[test]
fn rejected_edits_do_not_mutate_persisted_state() {
    let (_temp, repo) = fixture();
    let before = repo.snapshot(false);
    for p in [
        json!({"action":"save-note","title":"Bad","status":"broken"}),
        json!({"action":"save-note","title":"Bad","url":"file:///etc/passwd"}),
        json!({"action":"save-note","title":"Bad","url":"https://secret:token@example.com"}),
    ] {
        assert!(repo.edit(&p).is_err());
        assert_eq!(repo.snapshot(false), before);
    }
}
#[test]
fn projection_does_not_overwrite_unknown_domain_fields() {
    let (_temp, repo) = fixture();
    repo.transaction(|s| {
        s["futureField"] = json!({"preserve":42});
        Ok(())
    })
    .unwrap();
    repo.edit(&json!({"action":"save-note","title":"Project"}))
        .unwrap();
    assert_eq!(repo.snapshot(true)["futureField"]["preserve"], 42);
    assert!(repo.state.lock().unwrap().get("bundles").is_none());
}
#[test]
fn cached_snooze_expires_without_a_source_sync() {
    let (_temp, repo) = fixture();
    repo.transaction(|s| {
        s["items"] = json!([{"id":"github:1","source":"github","status":"todo"}]);
        s["snoozes"]["github:1"] = json!({"until":now()+0.05});
        Ok(())
    })
    .unwrap();
    let first = repo.snapshot(false);
    assert_eq!(first["items"][0]["snoozed"], true);
    std::thread::sleep(std::time::Duration::from_millis(65));
    let expired = repo.snapshot(false);
    assert_eq!(expired["items"][0]["snoozed"], false);
    assert_ne!(first["revision"], expired["revision"]);
}
#[test]
fn concurrent_open_is_rejected_and_failed_open_releases_guard() {
    let (_temp, repo) = fixture();
    let id = uuid();
    repo.opening.lock().unwrap().insert(id.clone());
    assert_eq!(workflows::open(&repo, &id).unwrap()["status"],"unknown");
    repo.opening.lock().unwrap().remove(&id);
    assert!(workflows::open(&repo, &id).is_err());
    assert!(!repo.opening.lock().unwrap().contains(&id));
}
#[test]
fn invalid_collection_shapes_fail_without_modifying_data() {
    for key in ["items", "sessions", "snoozes", "sources"] {
        let mut state = json!({"version":1});
        state[key] = json!("invalid");
        assert!(domain::initialize(&mut state).is_err());
    }
}
#[test]
fn related_work_normalizes_sentry_links_and_pr_body_urls() {
    let mut state = json!({"version":1});
    domain::initialize(&mut state).unwrap();
    state["items"] = json!([
        {"id":"github:1","source":"github","url":"https://github.com/acme/api/pull/1","detail":"Fixes https://sentry.io/organizations/acme/issues/123/)."},
        {"id":"sentry:acme:123","source":"sentry","url":"https://acme.sentry.io/issues/123/"},
        {"id":"linear:2","source":"linear","reference":"ENG-2","url":"https://linear.app/acme/issue/ENG-2","sentryLinks":[{"url":"https://sentry.io/organizations/acme/issues/123/"}]}
    ]);
    let bundles = domain::bundles(&state);
    assert_eq!(arr(&bundles).len(), 1);
    assert_eq!(arr(&bundles[0]["itemIds"]).len(), 3);
}
#[test]
fn schedule_catches_up_once_or_skips_and_never_replays_backlog() {
    let mut s = json!({"version":1});
    domain::initialize(&mut s).unwrap();
    s["skillSchedules"] = json!({"daily":{"enabled":true,"mode":"interval","everyMinutes":60,"missed":"latest","nextRun":1},"skip":{"enabled":true,"mode":"interval","everyMinutes":60,"missed":"skip","nextRun":1}});
    let due = super::scheduler::claim(&mut s, 10000.0).unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].0, "daily");
    assert_eq!(arr(&s["skillRuns"]).len(), 2);
    assert!(super::scheduler::claim(&mut s, 10001.0).unwrap().is_empty());
    assert_eq!(s["skillSchedules"]["daily"]["nextRun"], 13600.0);
}
#[test]
fn registered_tables_are_validated_and_snapshotted() {
    let (temp, repo) = fixture();
    let folder = temp.path().join("work");
    std::fs::create_dir(&folder).unwrap();
    let record = repo
        .transaction(|s| {
            workflows::save_session(
                s,
                &json!({"title":"Table","agent":"claude","cwd":folder,"itemIds":[]}),
            )
        })
        .unwrap();
    let path = folder.join("metrics.json");
    let table =
        json!({"schema":"mydash.table/v1","columns":["metric","value"],"rows":[["latency",12]]});
    std::fs::write(&path, table.to_string()).unwrap();
    let artifact = repo
        .transaction(|s| {
            artifacts::save(&repo, s, &json!({"path":path,"sessionIds":[record["id"]]}))
        })
        .unwrap();
    assert_eq!(artifact["snapshot"], true);
    std::fs::write(&path, "overwritten").unwrap();
    let read = artifacts::read(&repo, text(&artifact, "id")).unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&read.0).unwrap(), table);
    assert!(repo
        .transaction(|s| artifacts::save(&repo, s, &json!({"path":path})))
        .is_err());
}
#[test]
fn hook_events_deduplicate_subagents_and_register_real_conversation() {
    let (_temp, repo) = fixture();
    let record = repo
        .transaction(|s| {
            workflows::save_session(
                s,
                &json!({"agent":"codex","title":"Hook test","cwd":"/tmp","itemIds":[]}),
            )
        })
        .unwrap();
    repo.transaction(|s| {
        s["skillRuns"] = json!([{"id":"test-run","sessionId":record["id"],"outcome":"starting"}]);
        Ok(())
    })
    .unwrap();
    let session = uuid();
    workflows::register(&repo,&json!({"id":record["id"],"operation":"runtime-event","event":"SessionStart","sessionId":session})).unwrap();
    assert_eq!(repo.snapshot(false)["skillRuns"][0]["outcome"], "starting");
    for event in [
        "SessionStart",
        "UserPromptSubmit",
        "SubagentStart",
        "SubagentStart",
        "SubagentStop",
        "Stop",
    ] {
        workflows::register(&repo,&json!({"id":record["id"],"operation":"runtime-event","event":event,"sessionId":session,"subagentId":"child-1"})).unwrap();
    }
    let row = repo.snapshot(false)["sessions"][0].clone();
    assert_eq!(row["sessionId"], session);
    assert_eq!(row["runtime"]["state"], "waiting");
    assert_eq!(row["runtime"]["subagents"], 0);
    assert_eq!(repo.snapshot(false)["skillRuns"][0]["outcome"], "completed");
}
#[test]
fn rename_session_changes_only_its_display_title_and_timestamp() {
    let mut state=json!({"version":1});domain::initialize(&mut state).unwrap();
    let record=workflows::save_session(&mut state,&json!({"agent":"claude","title":"Before","cwd":"/tmp","itemIds":[]})).unwrap();
    domain::edit(&mut state,&json!({"action":"rename-session","id":record["id"],"title":"After"})).unwrap();
    let mut updated=state["sessions"][0].clone();assert_eq!(updated["title"],"After");
    updated["title"]=record["title"].clone();updated["updatedAt"]=record["updatedAt"].clone();assert_eq!(updated,record);
    let before=state.clone();assert!(domain::edit(&mut state,&json!({"action":"rename-session","id":record["id"],"title":""})).is_err());assert_eq!(state,before);
}
#[test]
fn background_snapshots_keep_shells_monitors_and_timers_out_of_attention_state() {
    let (_temp, repo) = fixture();
    let record = repo.transaction(|s| workflows::save_session(s,&json!({"agent":"claude","title":"Background work","cwd":"/tmp","itemIds":[]}))).unwrap();
    let event=|name:&str,extra:Value| {
        let mut p=json!({"id":record["id"],"operation":"runtime-event","event":name});
        for (k,v) in extra.as_object().unwrap(){p[k]=v.clone();}
        workflows::register(&repo,&p)
    };
    repo.transaction(|s|{s["skillRuns"]=json!([{"id":"background-run","sessionId":record["id"],"outcome":"launched"}]);Ok(())}).unwrap();
    let row=event("Stop",json!({"backgroundCounts":{"shell":2,"monitor":1,"custom task":1},"scheduledWakeups":1})).unwrap();
    assert_eq!(row["runtime"]["state"],"background");
    assert_eq!(row["runtime"]["backgroundCounts"]["shell"],2);
    assert_eq!(repo.snapshot(false)["skillRuns"][0]["outcome"],"launched");
    let row=event("PermissionRequest",json!({})).unwrap();
    assert_eq!(row["runtime"]["state"],"needs-input");
    assert_eq!(row["runtime"]["backgroundCounts"]["monitor"],1);
    let row=event("UserPromptSubmit",json!({})).unwrap();
    assert_eq!(row["runtime"]["state"],"working");
    assert_eq!(row["runtime"]["scheduledWakeups"],1);
    let before=repo.snapshot(false);
    assert!(event("Stop",json!({"backgroundCounts":{"shell":-1}})).is_err());
    assert_eq!(repo.snapshot(false),before);
    let row=event("Stop",json!({"backgroundCounts":{},"scheduledWakeups":0})).unwrap();
    assert_eq!(row["runtime"]["state"],"waiting");
    assert_eq!(repo.snapshot(false)["skillRuns"][0]["outcome"],"completed");
    let row=event("SessionStart",json!({})).unwrap();
    assert!(row["runtime"]["backgroundCounts"].is_null());
}
#[test]
fn change_detection_and_snooze_follow_source_changes() {
    let mut s = json!({"version":1});
    domain::initialize(&mut s).unwrap();
    let mut item = json!({"id":"github:1","source":"github","kind":"Pull request","title":"PR","status":"needs-review","sourceStatus":"Awaiting review","relations":["authored"],"ci":{"state":"pending"}});
    s["items"] = json!([item]);
    domain::observe(&mut s, "github", &[item.clone()]);
    assert!(arr(&s["changes"]).is_empty());
    domain::edit(
        &mut s,
        &json!({"action":"snooze","id":"github:1","mode":"change"}),
    )
    .unwrap();
    assert!(domain::snoozed(&s, &item));
    item["ci"]["state"] = json!("failed");
    domain::observe(&mut s, "github", &[item.clone()]);
    assert!(!domain::snoozed(&s, &item));
    assert!(s["changes"][0]["message"]
        .as_str()
        .unwrap()
        .contains("CI failing"));
    assert_eq!(domain::reasons(&item), vec!["Fix failing CI"]);
}
#[test]
fn pr_status_requires_approval_clean_merge_and_passing_ci() {
    let mut item = json!({"id":"x","url":"https://github.com/acme/api/pull/1","title":"PR","number":1,"state":"OPEN","isDraft":false,"reviewDecision":"APPROVED","mergeable":"MERGEABLE","mergeStateStatus":"CLEAN","repository":{"nameWithOwner":"acme/api"},"labels":{"nodes":[]},"commits":{"nodes":[{"commit":{"statusCheckRollup":{"state":"SUCCESS","contexts":{"nodes":[],"totalCount":0}}}}]}});
    assert_eq!(
        providers::github_item(&item).unwrap()["status"],
        "ready-to-merge"
    );
    item["mergeStateStatus"] = json!("BLOCKED");
    assert_eq!(
        providers::github_item(&item).unwrap()["status"],
        "in-progress"
    );
    item["state"] = json!("MERGED");
    assert_eq!(providers::github_item(&item).unwrap()["status"], "done");
}
#[test]
fn linear_without_project_keeps_hierarchy_and_sentry_link() {
    let n = json!({"id":"abc","identifier":"ENG-1","title":"Issue","url":"https://linear.app/acme/issue/ENG-1","state":{"name":"Triage","type":"triage"},"project":null,"team":{"key":"ENG","name":"Engineering"},"labels":{"nodes":[]},"parent":{"id":"parent","identifier":"ENG-0","url":"https://linear.app/acme/issue/ENG-0","title":"Parent"},"children":{"nodes":[]},"attachments":{"nodes":[{"sourceType":"sentry","url":"https://acme.sentry.io/issues/123/","title":"Error"}]}});
    let item = providers::linear_item(&n).unwrap();
    assert_eq!(item["project"], "No project");
    assert_eq!(item["parent"]["reference"], "ENG-0");
    assert_eq!(arr(&item["sentryLinks"]).len(), 1);
}
#[test]
fn quick_launch_attaches_only_clicked_work_and_newsletters_do_not_merge_work_groups() {
    let _env = ENV.lock().unwrap();
    let (temp,repo)=fixture();
    let previous=std::env::var_os("MYDASH_WORKDIR");
    std::env::set_var("MYDASH_WORKDIR",temp.path());
    let folder=temp.path().join("api");std::fs::create_dir(&folder).unwrap();
    let folder=folder.canonicalize().unwrap();
    let a=json!({"id":"github:a","source":"github","kind":"Pull request","title":"Database tests","url":"https://github.com/acme/api/pull/1","reference":"#1","project":"acme/api","status":"draft"});
    let b=json!({"id":"github:b","source":"github","kind":"Pull request","title":"Unrelated GPU change","url":"https://github.com/acme/api/pull/2","reference":"#2","project":"acme/api","status":"draft"});
    repo.transaction(|s|{s["items"]=json!([a,b]);s["sessions"]=json!([{"id":uuid(),"workflow":"newsletter","items":[a,b]}]);assert!(arr(&domain::bundles(s)).is_empty());s["workLinks"]=json!([[a["id"],b["id"]]]);assert_eq!(arr(&domain::bundles(s)).len(),1);Ok(())}).unwrap();
    let record=workflows::quick(&repo,&json!({"agent":"claude","itemId":"github:a","folderPath":folder}),false).unwrap();
    assert_eq!(arr(&record["items"]).len(),1);assert_eq!(record["items"][0]["id"],"github:a");assert_eq!(record["originItemId"],"github:a");
    if let Some(value)=previous { std::env::set_var("MYDASH_WORKDIR",value); } else { std::env::remove_var("MYDASH_WORKDIR"); }
}
#[test]
fn review_launch_reuses_identity_and_actual_worktree() {
    let _env = ENV.lock().unwrap();
    let (temp, repo) = fixture();
    let work = temp.path().join("code");
    let folder = work.join("api");
    std::fs::create_dir_all(&folder).unwrap();
    let previous = std::env::var_os("MYDASH_WORKDIR");
    std::env::set_var("MYDASH_WORKDIR", &work);
    let item = json!({"id":"github:42","source":"github","kind":"Pull request","title":"Fix","reference":"#42","url":"https://github.com/acme/api/pull/42","project":"acme/api","status":"draft"});
    repo.transaction(|s| {
        s["items"] = json!([item]);
        Ok(())
    })
    .unwrap();
    let p = json!({"agent":"claude","itemId":"github:42"});
    repo.settings.apply(&json!({"revision":0,"contentHash":repo.settings.view()["contentHash"],"patch":{"claudeReviewCommand":"/eddie-review"}}),false).unwrap();
    let record = workflows::quick(&repo, &p, true).unwrap();
    assert_eq!(record["workflow"], "pr-review");
    let tree = folder.join(".worktrees/review-pr-42");
    std::fs::create_dir_all(&tree).unwrap();
    workflows::register(
        &repo,
        &json!({"id":record["id"],"agent":"claude","sessionId":record["sessionId"],"cwd":tree}),
    )
    .unwrap();
    let again = workflows::quick(&repo, &p, true).unwrap();
    assert_eq!(again["id"], record["id"]);
    assert_eq!(
        again["cwd"],
        tree.canonicalize().unwrap().to_string_lossy().as_ref()
    );
    assert_eq!(arr(&repo.snapshot(false)["sessions"]).len(), 1);
    let packet = workflows::context(&repo, record["id"].as_str().unwrap()).unwrap();
    assert!(packet.contains("/eddie-review https://github.com/acme/api/pull/42"));
    if let Some(value) = previous {
        std::env::set_var("MYDASH_WORKDIR", value)
    } else {
        std::env::remove_var("MYDASH_WORKDIR")
    }
}
#[test]
fn artifact_paths_cannot_escape_and_links_are_reversible() {
    let (temp, repo) = fixture();
    let folder = temp.path().join("project");
    std::fs::create_dir(&folder).unwrap();
    let record = repo
        .transaction(|s| {
            workflows::save_session(
                s,
                &json!({"agent":"claude","title":"Report","cwd":folder,"itemIds":[]}),
            )
        })
        .unwrap();
    let report = folder.join("report.html");
    std::fs::write(&report, "<h1>Report</h1>").unwrap();
    let entry = repo
        .transaction(|s| {
            artifacts::save(
                &repo,
                s,
                &json!({"path":report,"sessionIds":[record["id"]]}),
            )
        })
        .unwrap();
    assert_eq!(
        artifacts::read(&repo, text(&entry, "id")).unwrap().0,
        b"<h1>Report</h1>"
    );
    let outside = temp.path().join("private.html");
    std::fs::write(&outside, "private").unwrap();
    let symlink = folder.join("escaped.html");
    std::os::unix::fs::symlink(outside, &symlink).unwrap();
    assert!(repo
        .transaction(|s| artifacts::save(&repo, s, &json!({"path":symlink})))
        .is_err());
    let metadata=artifacts::metadata_read(&repo,&json!({"id":entry["id"]})).unwrap();
    artifacts::metadata_update(&repo,&json!({"id":entry["id"],"patch":{"sessionIds":[]},"expected":{"sessionIds":metadata["expected"]["sessionIds"]}})).unwrap();
    assert!(report.exists());
    assert!(arr(&repo.snapshot(false)["artifacts"][0]["sessionIds"]).is_empty());
}
#[test]
fn worktree_registration_tracks_real_branches_and_preserves_other_item_folders() {
    use std::process::Command;
    let (temp, repo) = fixture();
    let main = temp.path().join("main-repo");
    std::fs::create_dir(&main).unwrap();
    let git = |cwd: &std::path::Path, args: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(cwd)
            .args([
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&main, &["init", "--quiet", "-b", "main"]);
    git(
        &main,
        &[
            "-c",
            "user.name=Mydash test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "--quiet",
            "--allow-empty",
            "-m",
            "fixture",
        ],
    );
    let first = temp.path().join("checkout-one");
    let second = temp.path().join("checkout-two");
    git(
        &main,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "feature-one",
            first.to_str().unwrap(),
        ],
    );
    git(
        &main,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "feature-two",
            second.to_str().unwrap(),
        ],
    );
    repo.edit(&json!({"action":"save-note","title":"A","status":"todo"}))
        .unwrap();
    repo.edit(&json!({"action":"save-note","title":"B","status":"todo"}))
        .unwrap();
    let ids = arr(&repo.snapshot(false)["notes"])
        .iter()
        .map(|x| x["id"].clone())
        .collect::<Vec<_>>();
    let session = uuid();
    let record = workflows::register(
        &repo,
        &json!({"agent":"codex","sessionId":session,"cwd":main,"title":"Two tasks","itemIds":ids}),
    )
    .unwrap();
    workflows::register(&repo,&json!({"operation":"worktree","id":record["id"],"path":first,"itemIds":[ids[0]],"primary":false})).unwrap();
    let state = repo.snapshot(false);
    assert_eq!(
        state["sessions"][0]["cwd"],
        main.canonicalize().unwrap().to_str().unwrap()
    );
    assert_eq!(
        state["itemSettings"][ids[0].as_str().unwrap()]["cwd"],
        first.canonicalize().unwrap().to_str().unwrap()
    );
    workflows::register(&repo,&json!({"operation":"worktree","id":record["id"],"path":second,"itemIds":[ids[1]],"primary":true})).unwrap();
    workflows::register(&repo,&json!({"id":record["id"],"agent":"codex","sessionId":session,"cwd":first,"itemIds":[ids[0]]})).unwrap();
    let state = repo.snapshot(false);
    let row = &state["sessions"][0];
    assert_eq!(arr(&row["items"]).len(), 2);
    assert_eq!(row["cwd"], first.canonicalize().unwrap().to_str().unwrap());
    assert_eq!(
        state["itemSettings"][ids[1].as_str().unwrap()]["cwd"],
        second.canonicalize().unwrap().to_str().unwrap()
    );
    let trees = arr(&row["worktrees"]);
    assert_eq!(trees.len(), 3);
    assert_eq!(trees.iter().filter(|x| x["primary"] == true).count(), 1);
    let tree = trees.iter().find(|x| x["primary"] == true).unwrap();
    assert_eq!(tree["branch"], "feature-one");
    assert_eq!(
        tree["repoPath"],
        main.canonicalize().unwrap().to_str().unwrap()
    );
    assert_eq!(tree["itemIds"], json!([ids[0]]));
    let before = repo.state.lock().unwrap().clone();
    assert!(workflows::register(
        &repo,
        &json!({"operation":"worktree","id":record["id"],"path":first,"itemIds":["unrelated"]})
    )
    .is_err());
    assert_eq!(*repo.state.lock().unwrap(), before);
    let packet = workflows::context(&repo, text(&record, "id")).unwrap();
    assert!(packet.contains("Worktree reporting"));
    assert!(packet.contains("runtime-docs/mydash.md"));
    assert!(!packet.contains("register.py"));
    assert!(!packet.contains("uv run"));
    assert!(std::fs::read_to_string(repo.data.parent().unwrap().join("runtime-docs/mydash.md")).unwrap().contains("MYDASH_BIN"));
}

#[test]
fn python_artifacts_are_immutable_plain_text_without_running_the_script(){
    let(temp,repo)=fixture();let folder=temp.path().join("scripts");std::fs::create_dir(&folder).unwrap();
    let record=repo.transaction(|s|workflows::save_session(s,&json!({"title":"Python artifact","agent":"claude","cwd":folder,"itemIds":[]}))).unwrap();
    let path=folder.join("experiment.py");let bytes=b"from pathlib import Path\n\ndef main():\n    print('<script>alert(1)</script>')\n    Path('MUST_NOT_RUN').write_text('executed')\n\nmain()\n";
    std::fs::write(&path,bytes).unwrap();
    let artifact=repo.transaction(|s|artifacts::save(&repo,s,&json!({"path":path,"sessionIds":[record["id"]]}))).unwrap();
    assert_eq!(artifact["format"],"py");assert_eq!(artifact["sourcePath"],json!(path.canonicalize().unwrap()));assert_eq!(artifact["snapshot"],true);
    std::fs::write(&path,b"changed after attachment").unwrap();let read=artifacts::read(&repo,text(&artifact,"id")).unwrap();assert_eq!(read.0,bytes);assert_eq!(read.1,"text/plain; charset=utf-8");assert!(!folder.join("MUST_NOT_RUN").exists());
}


#[test]
fn skill_reports_do_not_promote_supporting_attachments() {
    let (temp,repo)=fixture();let folder=temp.path().join("skill");
    std::fs::create_dir_all(folder.join("reports/2026-10-26")).unwrap();
    std::fs::write(folder.join("SKILL.md"),"Example skill").unwrap();
    let report=folder.join("reports/2026-10-26/report.html");let ledger=folder.join("reports/ledger.md");
    std::fs::write(&report,"<h1>Result</h1>").unwrap();std::fs::write(&ledger,"Supporting notes").unwrap();
    repo.transaction(|s|{s["skillPresets"]=json!([{"id":"example","title":"Example","cwd":folder,"skill":"SKILL.md","reports":"reports","command":"/example","agent":"claude"}]);Ok(())}).unwrap();
    let report_id=repo.transaction(|s|artifacts::save(&repo,s,&json!({"path":report,"presetId":"example"}))).unwrap()["id"].clone();
    let ledger_id=repo.transaction(|s|artifacts::save(&repo,s,&json!({"path":ledger}))).unwrap()["id"].clone();
    let state=repo.state.lock().unwrap();let catalog=artifacts::catalog(&repo,&state).unwrap();let entries=arr(&catalog["artifacts"]);
    let output=entries.iter().find(|e|e["id"]==report_id).unwrap();let supporting=entries.iter().find(|e|e["id"]==ledger_id).unwrap();
    assert_eq!(output["presetId"],"example");assert_eq!(output["role"],"skill-output");assert!(supporting["presetId"].is_null());
}


#[test]
fn skill_reports_shared_folders_preserve_explicit_ownership_and_attachments() {
    let (temp, repo) = fixture();
    let folder = temp.path().join("shared-skills");
    for day in ["2026-10-25", "2026-10-26", "2026-10-27"] {
        std::fs::create_dir_all(folder.join("reports").join(day)).unwrap();
        std::fs::write(folder.join("reports").join(day).join("report.html"), day).unwrap();
    }
    std::fs::write(folder.join("SKILL.md"), "Example skill").unwrap();
    let presets = json!([
        {"id":"first","title":"First","cwd":folder,"skill":"SKILL.md","reports":"reports"},
        {"id":"second","title":"Second","cwd":folder,"skill":"SKILL.md","reports":"reports"}
    ]);
    repo.transaction(|s| { s["skillPresets"] = presets.clone(); Ok(()) }).unwrap();
    let output_path = folder.join("reports/2026-10-25/report.html");
    let output = repo.transaction(|s| artifacts::save(&repo, s, &json!({"path":output_path,"presetId":"first"}))).unwrap();
    let attachment = repo.transaction(|s| artifacts::save(&repo, s, &json!({"path":folder.join("reports/2026-10-26/report.html")}))).unwrap();
    // Linking the exact same snapshot again must not erase its explicit owner.
    let reattached = repo.transaction(|s| artifacts::save(&repo, s, &json!({"path":output_path}))).unwrap();
    assert_eq!(reattached["id"], output["id"]);
    assert_eq!(reattached["presetId"], "first");
    assert_eq!(reattached["role"], "skill-output");
    let original = repo.state.lock().unwrap().clone();
    for reversed in [false, true] {
        let mut state = original.clone();
        if reversed { state["skillPresets"].as_array_mut().unwrap().reverse(); }
        let catalog = artifacts::catalog(&repo, &state).unwrap();
        let entries = arr(&catalog["artifacts"]);
        assert_eq!(entries.len(), 2, "ambiguous unregistered report must not be assigned");
        let registered = entries.iter().find(|e| e["id"] == output["id"]).unwrap();
        assert_eq!(registered["presetId"], "first");
        assert_eq!(registered["role"], "skill-output");
        let supporting = entries.iter().find(|e| e["id"] == attachment["id"]).unwrap();
        assert!(supporting["presetId"].is_null());
        assert!(supporting["role"].is_null());
    }
    assert_eq!(*repo.state.lock().unwrap(), original, "catalog must remain read-only");
}

#[test]
fn skill_reports_unique_folder_discovers_only_unregistered_outputs() {
    let (temp, repo) = fixture();
    let folder = temp.path().join("unique-skill");
    std::fs::create_dir_all(folder.join("reports/2026-10-26")).unwrap();
    std::fs::write(folder.join("SKILL.md"), "Example").unwrap();
    std::fs::write(folder.join("reports/2026-10-26/report.html"), "Attachment").unwrap();
    std::fs::write(folder.join("reports/2026-10-26/report.md"), "Unregistered legacy output").unwrap();
    repo.transaction(|s| { s["skillPresets"] = json!([{"id":"unique","title":"Unique","cwd":folder,"skill":"SKILL.md","reports":"reports"}]); Ok(()) }).unwrap();
    let attachment = repo.transaction(|s| artifacts::save(&repo, s, &json!({"path":folder.join("reports/2026-10-26/report.html")}))).unwrap();
    let state = repo.state.lock().unwrap();
    let catalog = artifacts::catalog(&repo, &state).unwrap();
    let entries = arr(&catalog["artifacts"]);
    assert_eq!(entries.len(), 2);
    let registered = entries.iter().find(|e| e["id"] == attachment["id"]).unwrap();
    assert!(registered["presetId"].is_null());
    let discovered = entries.iter().find(|e| e["discovered"] == true).unwrap();
    assert_eq!(discovered["presetId"], "unique");
    assert_eq!(discovered["role"], "skill-output");
}

#[test]
fn skill_reports_symlink_alias_is_still_a_shared_folder() {
    let (temp, repo) = fixture();
    let folder = temp.path().join("aliased-skills");
    std::fs::create_dir_all(folder.join("reports/2026-10-26")).unwrap();
    std::fs::write(folder.join("SKILL.md"), "Example").unwrap();
    std::fs::write(folder.join("reports/2026-10-26/report.html"), "Ambiguous output").unwrap();
    std::os::unix::fs::symlink(folder.join("reports"), folder.join("reports-alias")).unwrap();
    repo.transaction(|s| { s["skillPresets"] = json!([
        {"id":"first","title":"First","cwd":folder,"skill":"SKILL.md","reports":"reports"},
        {"id":"second","title":"Second","cwd":folder,"skill":"SKILL.md","reports":"reports-alias"}
    ]); Ok(()) }).unwrap();
    let state = repo.state.lock().unwrap();
    assert!(arr(&artifacts::catalog(&repo, &state).unwrap()["artifacts"]).is_empty());
}
