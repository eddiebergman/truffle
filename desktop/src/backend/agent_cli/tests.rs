use super::*;
use crate::backend::{arr, workflows, Repository};
fn options(args: &[&str]) -> Options {
    Options::parse(
        &args
            .iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>(),
    )
    .unwrap()
}
#[test]
fn replacement_payloads_require_and_preserve_the_captured_revision_without_new_flags() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("context.json");
    let path = file.to_str().unwrap();
    let id = uuid();
    let revision = "aB".repeat(32);
    let context = json!({"revision":revision,"resources":[],"itemIds":[],"artifactIds":[]});
    fs::write(&file, serde_json::to_vec(&context).unwrap()).unwrap();
    let request = payload("context", &options(&["--record", &id, "--file", path])).unwrap();
    assert_eq!(request["context"], context);
    assert_eq!(request["operation"], "context-replace");
    let request = payload(
        "resource",
        &options(&["--record", &id, "--file", path, "--replace"]),
    )
    .unwrap();
    assert_eq!(request["report"]["revision"], revision);
    assert_eq!(request["report"]["resources"], json!([]));
    assert_eq!(request["report"]["replace"], true);
    assert!(payload(
        "context",
        &options(&["--record", &id, "--file", path, "--revision", &revision])
    )
    .is_err());
    assert!(payload(
        "resource",
        &options(&[
            "--record",
            &id,
            "--file",
            path,
            "--replace",
            "--revision",
            &revision
        ])
    )
    .is_err());
    for revision in [
        Value::Null,
        json!(7),
        json!(""),
        json!("fresh"),
        json!("g".repeat(64)),
    ] {
        fs::write(
            &file,
            serde_json::to_vec(&json!({"revision":revision,"resources":[]})).unwrap(),
        )
        .unwrap();
        for command in ["context", "resource"] {
            let mut args = vec!["--record", &id, "--file", path];
            if command == "resource" {
                args.push("--replace");
            }
            let error = payload(command, &options(&args)).unwrap_err();
            assert!(error.message.contains("captured revision"));
            assert!(error.message.contains("reapply"));
        }
    }
    fs::write(&file, b"{\"resources\":[],\"replace\":true}").unwrap();
    let additive = payload("resource", &options(&["--record", &id, "--file", path])).unwrap();
    assert_eq!(additive["report"]["replace"], false);
    assert!(additive["report"].get("revision").is_none());
    let shown = payload("context", &options(&["--record", &id, "--show"])).unwrap();
    assert_eq!(shown["operation"], "context-read");
    assert!(shown.get("context").is_none());
    let help = command_help("resource");
    assert!(help.contains("report.revision from context --show"));
    assert!(help.contains("re-read context and reapply"));
    assert!(help.contains("[] clears dashboard links only"));
}
#[test]
fn context_show_prints_the_owner_context_for_direct_edit_and_resubmit() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().to_path_buf(), None, true).unwrap();
    let record = repo
        .transaction(|state| {
            super::super::workflows::save_session(
                state,
                &json!({"agent":"codex","title":"Context fixture","cwd":temp.path(),"itemIds":[]}),
            )
        })
        .unwrap();
    let id = text(&record, "id");
    let owner =
        super::super::workflows::register(&repo, &json!({"operation":"context-read","id":id}))
            .unwrap();
    let reply = super::super::workflows::registration_reply(&owner);
    let shown = shown_result("context", &reply);
    assert_eq!(shown, &owner["context"]);
    assert!(shown.get("id").is_none());
    assert!(shown.get("context").is_none());
    assert_eq!(shown["revision"].as_str().unwrap().len(), 64);
    let file = temp.path().join("context.json");
    fs::write(&file, serde_json::to_vec(shown).unwrap()).unwrap();
    let request = payload(
        "context",
        &options(&["--record", id, "--file", file.to_str().unwrap()]),
    )
    .unwrap();
    assert_eq!(request["context"], *shown);
    super::super::workflows::register(&repo, &request).unwrap();
}
#[test]
fn settings_updates_carry_the_explicit_captured_content_hash() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("patch.json");
    std::fs::write(&file, r#"{"theme":"light"}"#).unwrap();
    let file = file.to_str().unwrap();
    let hash = "a".repeat(64);
    let request = payload(
        "settings",
        &options(&["--file", file, "--revision", "7", "--content-hash", &hash]),
    )
    .unwrap();
    assert_eq!(request["revision"], 7);
    assert_eq!(request["contentHash"], hash);
    assert_eq!(request["patch"], json!({"theme":"light"}));
    assert!(
        payload("settings", &options(&["--file", file, "--revision", "7"]))
            .unwrap_err()
            .message
            .contains("--content-hash")
    );
    assert!(payload(
        "settings",
        &options(&[
            "--file",
            file,
            "--revision",
            "7",
            "--content-hash",
            "invalid"
        ])
    )
    .is_err());
}
#[test]
fn native_payloads_preserve_ids_repeated_links_and_validate_input() {
    let id = uuid();
    let session = uuid();
    let o = options(&[
        "--agent",
        "claude",
        "--session",
        &session,
        "--record",
        &id,
        "--url",
        "https://example.com/one",
        "--url",
        "https://example.com/two",
    ]);
    let value = payload("register", &o).unwrap();
    assert_eq!(value["sessionId"], session);
    assert_eq!(arr(&value["urls"]).len(), 2);
    assert!(payload(
        "register",
        &options(&["--agent", "unknown", "--session", &session])
    )
    .is_err());
    assert!(payload(
        "review",
        &options(&["--record", &id, "--state", "complete"])
    )
    .is_err());
    assert!(payload("context", &options(&["--record", &id])).is_err());
    assert!(payload(
        "context",
        &options(&["--record", &id, "--show", "--file", "missing"])
    )
    .is_err());
    assert!(payload(
        "status",
        &options(&["--record", &id, "--state", "working", "--subagents", "-1"])
    )
    .is_err());
    assert!(payload(
        "status",
        &options(&["--record", &id, "--state", "working", "--typo", "ignored"])
    )
    .is_err());
    assert!(payload(
        "artifact",
        &options(&[
            "--record",
            &id,
            "--file",
            "a",
            "--url",
            "https://example.com"
        ])
    )
    .is_err());
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("report.json");
    fs::write(
        &file,
        b"{\"comments\":[],\"overall\":{\"author\":\"Agent\",\"text\":\"No findings\"}}",
    )
    .unwrap();
    let report = payload(
        "review",
        &options(&[
            "--record",
            &id,
            "--request",
            &session,
            "--pass",
            "custom-reviewer",
            "--file",
            file.to_str().unwrap(),
            "--state",
            "complete",
        ]),
    )
    .unwrap();
    assert_eq!(report["report"]["overall"]["text"], "No findings");
    assert_eq!(report["pass"], "custom-reviewer");
    fs::write(&file, vec![b'x'; 96 * 1024 + 1]).unwrap();
    assert!(payload(
        "review",
        &options(&["--record", &id, "--file", file.to_str().unwrap()])
    )
    .is_err());
}
#[test]
fn hooks_keep_counts_only_and_preserve_unknown_and_empty_snapshots() {
    let data = json!({"background_tasks":[{"id":"s","type":"shell","command":"private"},{"id":"s","type":"shell"},{"id":"m","type":"monitor"},{"id":"done","type":"shell","status":"completed"},null,{"type":[]}],"session_crons":[{"prompt":"private"}]});
    assert_eq!(
        activity_snapshot(&data),
        json!({"backgroundCounts":{"shell":1,"monitor":1,"other":1},"scheduledWakeups":1})
    );
    assert_eq!(activity_snapshot(&json!({})), json!({}));
    assert_eq!(
        activity_snapshot(&json!({"background_tasks":[],"session_crons":[]})),
        json!({"backgroundCounts":{},"scheduledWakeups":0})
    );
    assert_eq!(
        activity_snapshot(&json!({"session_crons":vec![json!({});1100]}))["scheduledWakeups"],
        1000
    );
}
#[test]
fn signed_inbox_roundtrip_preserves_unicode_and_nested_replies_and_hooks_need_no_ack() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let record = repo
        .transaction(|s| {
            workflows::save_session(
                s,
                &json!({"agent":"claude","cwd":temp.path(),"title":"日本語 · café"}),
            )
        })
        .unwrap();
    let id = text(&record, "id");
    let inbox = tempfile::Builder::new()
        .prefix("truffle-agent-")
        .tempdir_in("/tmp")
        .unwrap();
    fs::set_permissions(inbox.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let runtime = json!({"token":"private-test-token","inbox":inbox.path()});
    let (root, nonce) = enqueue(&runtime, &json!({"id":id,"operation":"review"}), true).unwrap();
    super::super::http::inbox_once(&repo, &root, "private-test-token");
    let reply = wait_inbox(&root, &nonce).unwrap();
    assert_eq!(reply["id"], id);
    assert!(reply["review"]["comments"].is_array());
    let (root, nonce) = enqueue(
        &runtime,
        &json!({"id":id,"operation":"review","report":{"comments":[],"overall":null}}),
        true,
    )
    .unwrap();
    let request = root.join(format!("{nonce}.request.json"));
    let mut envelope: Value = serde_json::from_slice(&fs::read(&request).unwrap()).unwrap();
    envelope["data"]["payload"]["note"] = json!("changed 日本語");
    fs::write(&request, serde_json::to_vec(&envelope).unwrap()).unwrap();
    super::super::http::inbox_once(&repo, &root, "private-test-token");
    assert!(wait_inbox(&root, &nonce).is_err());
    let(root,nonce)=enqueue(&runtime,&json!({"id":id,"operation":"runtime-event","event":"Stop","backgroundCounts":{"shell":0},"sessionId":""}),false).unwrap();
    super::super::http::inbox_once(&repo, &root, "private-test-token");
    assert!(!root.join(format!("{nonce}.result.json")).exists());
    assert!(!root.join(format!("{nonce}.request.json")).exists());
    fs::set_permissions(inbox.path(), fs::Permissions::from_mode(0o755)).unwrap();
    assert!(enqueue(&runtime, &json!({}), true).is_err());
}
