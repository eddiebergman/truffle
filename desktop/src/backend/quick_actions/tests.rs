use super::*;
fn output_fixture(secret: &str) -> (tempfile::TempDir, Arc<Repository>) {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join(".env"),
        format!("OUTPUT_TEST_SECRET={secret}\n"),
    )
    .unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    (temp, repo)
}
fn output_job(repo: &Repository, id: &str) {
    repo.quick_actions.jobs.lock().unwrap().insert(
        id.into(),
        json!({"id":id,"state":"running","startedAt":0,"output":""}),
    );
}
fn output_text(repo: &Repository, id: &str) -> String {
    text(&repo.quick_actions.jobs.lock().unwrap()[id], "output").to_owned()
}
#[test]
fn stream_output_keeps_unicode_and_redacts_every_secret_byte_boundary() {
    let secret = "benign-stream-é秘密-0123456789";
    let (_temp, repo) = output_fixture(secret);
    for split in 0..=secret.len() {
        let id = format!("split-{split}");
        output_job(&repo, &id);
        let mut decoder = Utf8Output::default();
        let mut redactor = super::super::commands::stream_redactor(&repo);
        repo.quick_actions.output(
            &id,
            &decoder.push_redacted(b"Live safe output: ", &mut redactor),
        );
        assert_eq!(output_text(&repo, &id), "Live safe output: ");
        repo.quick_actions.output(
            &id,
            &decoder.push_redacted(&secret.as_bytes()[..split], &mut redactor),
        );
        assert!(!output_text(&repo, &id).contains("benign-stream"));
        repo.quick_actions.output(
            &id,
            &decoder.push_redacted(&secret.as_bytes()[split..], &mut redactor),
        );
        for byte in " café 🔎 end".as_bytes() {
            repo.quick_actions
                .output(&id, &decoder.push_redacted(&[*byte], &mut redactor));
        }
        repo.quick_actions
            .output(&id, &finish_stream(&mut decoder, &mut redactor));
        assert_eq!(
            output_text(&repo, &id),
            "Live safe output: [redacted] café 🔎 end"
        );
        let public = super::super::agent_access::quick_action_outputs(&repo, &json!({"id":id}));
        assert_eq!(
            public["jobs"][0]["output"],
            "Live safe output: [redacted] café 🔎 end"
        );
    }
}
#[test]
fn stream_output_truncated_unicode_secret_eof_never_releases_withheld_prefix() {
    let secret = "fixture-é-secret";
    let (_temp, repo) = output_fixture(secret);
    for end in 0..=secret.len() {
        let id = format!("eof-{end}");
        output_job(&repo, &id);
        let bytes = &secret.as_bytes()[..end];
        let mut decoder = Utf8Output::default();
        let mut redactor = super::super::commands::stream_redactor(&repo);
        for byte in bytes {
            repo.quick_actions
                .output(&id, &decoder.push_redacted(&[*byte], &mut redactor));
        }
        repo.quick_actions
            .output(&id, &finish_stream(&mut decoder, &mut redactor));
        let expected = if end == 0 {
            ""
        } else if std::str::from_utf8(bytes).is_err() {
            "[redacted]�"
        } else {
            "[redacted]"
        };
        assert_eq!(output_text(&repo, &id), expected, "truncated byte {end}");
        let public = super::super::agent_access::quick_action_outputs(&repo, &json!({"id":id}));
        assert_eq!(public["jobs"][0]["output"], expected);
    }
    for (bytes, expected) in [
        (b"plain \xc3".as_slice(), "plain �"),
        (b"plain \xff end".as_slice(), "plain � end"),
    ] {
        let mut decoder = Utf8Output::default();
        let mut redactor = super::super::commands::stream_redactor(&repo);
        let mut output = decoder.push_redacted(bytes, &mut redactor);
        output.push_str(&finish_stream(&mut decoder, &mut redactor));
        assert_eq!(output, expected);
    }
}
#[test]
fn stream_output_invalid_unicode_boundaries_never_release_withheld_prefixes() {
    let secret = "fixture-é秘密-secret";
    let (_temp, repo) = output_fixture(secret);
    for end in 1..secret.len() {
        let prefix = &secret.as_bytes()[..end];
        if std::str::from_utf8(prefix).is_ok() {
            continue;
        }
        for suffix in [b"X".as_slice(), b"\n".as_slice()] {
            let id = format!("invalid-{end}-{}", suffix[0]);
            output_job(&repo, &id);
            let mut decoder = Utf8Output::default();
            let mut redactor = super::super::commands::stream_redactor(&repo);
            repo.quick_actions
                .output(&id, &decoder.push_redacted(prefix, &mut redactor));
            assert_eq!(output_text(&repo, &id), "");
            repo.quick_actions
                .output(&id, &decoder.push_redacted(suffix, &mut redactor));
            let expected = format!("[redacted]�{}", std::str::from_utf8(suffix).unwrap());
            // Output must remain live before EOF as well as safe afterward.
            assert_eq!(output_text(&repo, &id), expected);
            repo.quick_actions
                .output(&id, &finish_stream(&mut decoder, &mut redactor));
            assert_eq!(output_text(&repo, &id), expected);
            let public = super::super::agent_access::quick_action_outputs(&repo, &json!({"id":id}));
            assert_eq!(public["jobs"][0]["output"], expected);
        }
    }
    let mut decoder = Utf8Output::default();
    let mut redactor = super::super::commands::stream_redactor(&repo);
    assert_eq!(
        decoder.push_redacted(b"plain \xc3", &mut redactor),
        "plain "
    );
    assert_eq!(decoder.push_redacted(b"X\n", &mut redactor), "�X\n");
    assert_eq!(finish_stream(&mut decoder, &mut redactor), "");
}
#[test]
fn stream_output_redacts_long_known_values_before_owner_tail_cut() {
    let secret = format!("benign-long-secret-{}-end", "é".repeat(10000));
    let (_temp, repo) = output_fixture(&secret);
    let id = "tail";
    output_job(&repo, id);
    let raw = format!("safe prefix {secret}{}", "z".repeat(15990));
    let mut decoder = Utf8Output::default();
    let mut redactor = super::super::commands::stream_redactor(&repo);
    for bytes in raw.as_bytes().chunks(2048) {
        repo.quick_actions
            .output(id, &decoder.push_redacted(bytes, &mut redactor));
    }
    repo.quick_actions
        .output(id, &finish_stream(&mut decoder, &mut redactor));
    assert_eq!(
        output_text(&repo, id),
        format!("[redacted]{}", "z".repeat(15990))
    );
    let public = super::super::agent_access::quick_action_outputs(&repo, &json!({"id":id}));
    assert_eq!(public["jobs"][0]["output"], "z".repeat(2000));
}
#[test]
fn stream_output_withholds_ambiguous_prefixes_and_bounds_pending_sensitive_text() {
    let (_temp, repo) = output_fixture("fixture-short");
    let mut redactor = repo
        .providers
        .stream_redactor()
        .with_values(["fixture-short-longer".into()]);
    assert_eq!(redactor.push("visible fixture-short"), "visible ");
    assert_eq!(redactor.push("-long"), "");
    assert_eq!(redactor.push("er done"), "[redacted] done");
    assert_eq!(redactor.finish(), "");
    let mut partial = repo.providers.stream_redactor();
    assert_eq!(partial.push("safe fixture-sh"), "safe ");
    assert_eq!(partial.finish(), "[redacted]");
    let huge = format!("{}end", "q".repeat(65536));
    let mut bounded = repo.providers.stream_redactor().with_values([huge]);
    assert_eq!(bounded.push(&"q".repeat(65535)), "");
    let notice = bounded.push("q");
    assert_eq!(
        notice,
        "[Output omitted: sensitive value exceeds stream safety limit.]"
    );
    assert_eq!(bounded.push("end subsequent text"), "");
    assert_eq!(bounded.finish(), "");
}
#[test]
fn reviewed_command_identity_is_checked_before_lookup_without_execution() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    repo.settings.apply(&json!({"revision":0,"contentHash":repo.settings.view()["contentHash"],"patch":{"quickActions":[{"id":"fixture","title":"Fixture","command":"printf reviewed"}]}}),false).unwrap();
    let baseline = repo.settings.view();
    let request = json!({"revision":baseline["revision"],"contentHash":baseline["contentHash"],"actionId":"fixture"});
    assert_eq!(
        checked_action(&repo, &request).unwrap()["command"],
        "printf reviewed"
    );
    let path = PathBuf::from(text(&baseline["files"], "preferences"));
    let mut external: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    external["values"]["quickActions"][0]["command"] = json!("printf changed");
    std::fs::write(&path, serde_json::to_vec(&external).unwrap()).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(checked_action(&repo, &request).unwrap_err().status, 409);
    let fresh = repo.settings.view();
    assert_eq!(checked_action(&repo,&json!({"revision":fresh["revision"],"contentHash":baseline["contentHash"],"actionId":"fixture"})).unwrap_err().status,409);
    assert_eq!(checked_action(&repo,&json!({"revision":fresh["revision"],"contentHash":fresh["contentHash"],"actionId":"fixture"})).unwrap()["command"],"printf changed");
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert!(arr(&repo.quick_actions.view()["jobs"]).is_empty());
}

#[test]
fn failed_start_persistence_preserves_jobs_and_allows_retry() {
    let root = tempfile::tempdir().unwrap();
    let repo = Repository::open(root.path().join("profile"), None, false).unwrap();
    repo.settings.apply(&json!({"revision":0,"contentHash":repo.settings.view()["contentHash"],"patch":{"quickActions":[{"id":"fixture","title":"Fixture","command":"printf retry"}]}}), false).unwrap();
    let path = root.path().join("runs.json");
    let host = Arc::new(Host::load(path.clone()).unwrap());
    {
        let mut jobs = host.jobs.lock().unwrap();
        for index in 0..30 {
            jobs.insert(format!("old-{index}"), json!({"id":format!("old-{index}"),"state":"succeeded","startedAt":index,"output":"saved"}));
        }
        host.persist(&jobs).unwrap();
    }
    let baseline = host.view();
    let bytes = std::fs::read(&path).unwrap();
    let blocked_temp = path.with_extension("tmp");
    std::fs::create_dir(&blocked_temp).unwrap();
    let request = json!({"revision":1,"contentHash":repo.settings.view()["contentHash"],"actionId":"fixture"});
    assert!(host.start(&repo, &request).is_err());
    assert_eq!(host.view(), baseline);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    std::fs::remove_dir(blocked_temp).unwrap();
    let run = host.start(&repo, &request).unwrap();
    let start = Instant::now();
    loop {
        let job = arr(&host.view()["jobs"])
            .into_iter()
            .find(|job| job["id"] == run["id"])
            .unwrap();
        if job["state"] != "running" {
            assert_eq!(job["state"], "succeeded");
            assert_eq!(job["output"], "retry");
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(25));
    }
    assert_eq!(arr(&host.view()["jobs"]).len(), 30);
}

#[test]
fn persisted_output_survives_service_restart_without_replaying_commands() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("runs.json");
    let host = Host::load(path.clone()).unwrap();
    {
        let mut jobs = host.jobs.lock().unwrap();
        jobs.insert(
            "run".into(),
            json!({"state":"running","output":"diagnostic output"}),
        );
        host.persist(&jobs).unwrap();
    }
    drop(host);
    let restored = Host::load(path).unwrap();
    let value = restored.view();
    assert_eq!(value["jobs"][0]["output"], "diagnostic output");
    assert_eq!(value["jobs"][0]["state"], "interrupted");
}
#[test]
fn quick_action_context_is_literal_and_rejects_unlinked_items() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().to_path_buf(), None, false).unwrap();
    let id = uuid();
    let title = "$(printf injected) ; quoted ' \" title";
    repo.transaction(|s| {
        s["sessions"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":id,"agent":"claude","title":title,"cwd":temp.path(),"items":[]}));
        Ok(())
    })
    .unwrap();
    let context = context(&repo, &json!({"recordId":id})).unwrap();
    assert_eq!(context["CHAT_TITLE"], title);
    assert!(super::context(&repo, &json!({"recordId":id,"prId":"unlinked"})).is_err());
    repo.settings.apply(&json!({"revision":0,"contentHash":repo.settings.view()["contentHash"],"patch":{"quickActions":[{"id":"literal","title":"Literal","command":"printf '%s' \"$CHAT_TITLE\""}]}}),false).unwrap();
    let run = repo
            .quick_actions
            .start(
                &repo,
                &json!({"revision":1,"contentHash":repo.settings.view()["contentHash"],"actionId":"literal","recordId":id}),
            )
            .unwrap();
    let start = Instant::now();
    loop {
        let view = repo.quick_actions.view();
        let job = arr(&view["jobs"])
            .into_iter()
            .find(|j| j["id"] == run["id"])
            .unwrap();
        if job["state"] != "running" {
            assert_eq!(job["state"], "succeeded");
            assert_eq!(job["output"], title);
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(25));
    }
}
