use super::*;
fn codex_events(response: &Value) -> String {
    [json!({"type":"thread.started","thread_id":"fixture"}),json!({"type":"item.completed","item":{"type":"error","message":CODEX_DISABLED_HOST_NOTICE}}),json!({"type":"turn.started"}),json!({"type":"item.completed","item":{"type":"agent_message","text":"Intermediate native async question; never send this."}}),json!({"type":"item.completed","item":{"type":"agent_message","text":json!({"responseJson":response.to_string()}).to_string()}}),json!({"type":"turn.completed"})].iter().map(Value::to_string).collect::<Vec<_>>().join("\n")
}
#[test]
fn codex_parser_requires_completed_strict_wrapper_and_never_returns_intermediate_text() {
    let output = codex_events(&json!({"tool":"discover","args":{}}));
    let mut diagnostics = json!({});
    assert_eq!(
        codex_cli_response(&output, &mut diagnostics).unwrap(),
        json!({"tool":"discover","args":{}}).to_string()
    );
    assert_eq!(diagnostics["disabledHostNotices"], 1);
    let events = output
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    let malformed=[
            events[..5].iter().map(Value::to_string).collect::<Vec<_>>().join("\n"),
            output.replace("turn.completed","turn.failed"),output.clone()+"\n"+&json!({"type":"error","message":"private-error-canary"}).to_string(),
            output.replace(CODEX_DISABLED_HOST_NOTICE,"unrecognized private-error-canary"),
            [json!({"type":"turn.started"}),json!({"type":"item.completed","item":{"type":"error","message":CODEX_DISABLED_HOST_NOTICE}})].iter().map(Value::to_string).collect::<Vec<_>>().join("\n"),
            output.clone()+"\n{truncated",codex_events(&json!({"reply":"Ready","tool":"discover","args":{}})),
        ];
    for invalid in malformed {
        let mut diagnostics = json!({});
        assert!(codex_cli_response(&invalid, &mut diagnostics).is_err());
        assert!(!diagnostics.to_string().contains("private-error-canary"));
    }
    for wrapper in [
        json!({"reply":"wrong-wrapper"}),
        json!({"responseJson":false}),
        json!({"responseJson":"{not-json"}),
        json!({"responseJson":"{\"reply\":\"Ready\"}","extra":true}),
    ] {
        let invalid=[json!({"type":"turn.started"}),json!({"type":"item.completed","item":{"type":"agent_message","text":wrapper.to_string()}}),json!({"type":"turn.completed"})].iter().map(Value::to_string).collect::<Vec<_>>().join("\n");
        assert!(codex_cli_response(&invalid, &mut json!({})).is_err());
    }
    assert_eq!(codex_schema()["required"], json!(["responseJson"]));
    assert_eq!(codex_schema()["additionalProperties"], false);
}
#[test]
fn assistant_selection_prefers_installed_default_and_missing_cli_leaves_no_message_or_job() {
    for (preferred, installed, expected) in [
        ("codex", [true, true], Some(Assistant::Codex)),
        ("claude", [true, true], Some(Assistant::Claude)),
        ("codex", [true, false], Some(Assistant::Claude)),
        ("claude", [false, true], Some(Assistant::Codex)),
        ("codex", [false, false], None),
    ] {
        assert_eq!(
            choose_assistant(preferred, |agent| installed
                [if agent == Assistant::Claude { 0 } else { 1 }]),
            expected
        );
    }
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, false).unwrap();
    let before = repo.state.lock().unwrap().clone();
    let host = Arc::new(Host::default());
    let error=host.start_with(&repo,&json!({"message":"fixture","requestId":uuid(),"context":{"source":"agents","filters":{}}}),|_|false,|_,_,_,_,_,_,_|panic!("No installed assistant must never launch")).unwrap_err();
    assert!(error.message.contains("Claude or Codex"));
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert!(host.job.lock().unwrap().is_none());
}
fn captured_codex_fixture(
    repo: &Repository,
    assistant: Assistant,
    prompt: &str,
    _: &[Value],
    _: &AtomicBool,
    _: Instant,
    _: &mut Value,
) -> Result<String> {
    assert_eq!(assistant, Assistant::Codex);
    let transcript: Value = serde_json::from_str(prompt)?;
    if arr(&transcript["steps"]).is_empty() {
        let baseline = repo.settings.view();
        repo.settings.apply(&json!({"revision":baseline["revision"],"contentHash":baseline["contentHash"],"patch":{"defaultAgent":"claude"}}),false)?;
        Ok(json!({"tool":"discover","args":{}}).to_string())
    } else {
        Ok(json!({"reply":"Fixture completed with captured Codex."}).to_string())
    }
}
#[test]
fn assistant_is_captured_for_entire_reply_even_when_default_changes_between_tool_turns() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, false).unwrap();
    let baseline = repo.settings.view();
    repo.settings.apply(&json!({"revision":baseline["revision"],"contentHash":baseline["contentHash"],"patch":{"defaultAgent":"codex"}}),false).unwrap();
    let host = Arc::new(Host::default());
    let job=host.start_with(&repo,&json!({"message":"fixture","requestId":uuid(),"context":{"source":"agents","filters":{}}}),|_|true,captured_codex_fixture).unwrap();
    assert_eq!(job["assistant"], Assistant::Codex.metadata());
    let until = Instant::now() + Duration::from_secs(3);
    loop {
        let job = host.job.lock().unwrap().clone().unwrap();
        if job["state"] != "running" {
            assert_eq!(job["state"], "succeeded", "{job}");
            assert_eq!(job["assistant"], Assistant::Codex.metadata());
            break;
        }
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(repo.settings.view()["values"]["defaultAgent"], "claude");
    let view = host.view(&repo);
    assert_eq!(view["job"]["assistant"], Assistant::Codex.metadata());
    assert_eq!(arr(&view["assistants"]).len(), 2);
    for choice in arr(&view["assistants"]) {
        assert!(choice["installed"].is_boolean());
        assert_eq!(choice.as_object().unwrap().len(), 3);
    }
}
fn unavailable_codex_fixture(
    repo: &Repository,
    assistant: Assistant,
    _: &str,
    _: &[Value],
    _: &AtomicBool,
    _: Instant,
    _: &mut Value,
) -> Result<String> {
    assert_eq!(assistant, Assistant::Codex);
    repo.transaction(|state| {
        let calls = state["fixtureModelCalls"].as_u64().unwrap_or(0);
        state["fixtureModelCalls"] = json!(calls + 1);
        Ok(())
    })?;
    Err(Error::new("Codex fixture unavailable; no provider retry."))
}
#[test]
fn failed_selected_assistant_does_not_retry_with_another_installed_provider() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, false).unwrap();
    let baseline = repo.settings.view();
    repo.settings.apply(&json!({"revision":baseline["revision"],"contentHash":baseline["contentHash"],"patch":{"defaultAgent":"codex"}}),false).unwrap();
    let host = Arc::new(Host::default());
    host.start_with(
        &repo,
        &json!({"message":"fixture","requestId":uuid(),"context":{"source":"agents","filters":{}}}),
        |_| true,
        unavailable_codex_fixture,
    )
    .unwrap();
    let until = Instant::now() + Duration::from_secs(3);
    loop {
        let job = host.job.lock().unwrap().clone().unwrap();
        if job["state"] != "running" {
            assert_eq!(job["state"], "failed");
            assert_eq!(job["assistant"], Assistant::Codex.metadata());
            assert!(text(&job, "error").contains("Codex fixture unavailable"));
            break;
        }
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(repo.state.lock().unwrap()["fixtureModelCalls"], 1);
}
#[test]
fn codex_command_uses_proven_disable_flags_native_auth_allowlist_and_private_image_lifetime() {
    use std::os::unix::fs::PermissionsExt;
    let values = [
        "HOME",
        "CODEX_HOME",
        "CODEX_API_KEY",
        "OPENAI_API_KEY",
        "ANTHROPIC_API_KEY",
        "LINEAR_API_KEY",
        "SENTRY_AUTH_TOKEN",
        "GITHUB_TOKEN",
        "TRUFFLE_AGENT_TOKEN",
        "CODEX_THREAD_ID",
        "CODEX_MODEL",
        "PATH",
    ]
    .map(|key| (key.to_owned(), "benign-private-env".to_owned()));
    let allowed = assistant_environment(Assistant::Codex, values)
        .into_iter()
        .map(|(key, _)| key)
        .collect::<Vec<_>>();
    assert_eq!(allowed, vec!["HOME", "CODEX_HOME", "CODEX_API_KEY", "PATH"]);
    let mut environment_command = Command::new("/usr/bin/env");
    configure_assistant_environment(
        &mut environment_command,
        Assistant::Codex,
        [
            ("CODEX_API_KEY".into(), "native-fixture".into()),
            ("GITHUB_TOKEN".into(), "integration-fixture".into()),
        ],
    );
    let output = environment_command.output().unwrap();
    let text = String::from_utf8(output.stdout).unwrap();
    assert_eq!(text.lines().count(), 1);
    assert!(text.contains("CODEX_API_KEY=native-fixture"));
    assert!(!text.contains("integration-fixture"));
    let temp = tempfile::tempdir().unwrap();
    let path;
    {
        let files = InvocationFiles::new(&temp.path().join("runtime")).unwrap();
        path = files.root.clone();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let schema = files
            .write(
                "response-schema.json",
                codex_schema().to_string().as_bytes(),
            )
            .unwrap();
        let images=image_blocks(&json!([{"mediaType":"image/png","data":"iVBORw0KGgo="},{"mediaType":"image/jpeg","data":"/9j/"}])).unwrap();
        let paths = files.images(&images).unwrap();
        assert_eq!(paths.len(), 2);
        for file in [&schema, &paths[0], &paths[1]] {
            assert_eq!(
                std::fs::metadata(file).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert!(files
            .write("response-schema.json", b"must-not-overwrite")
            .is_err());
        let command = codex_command("/fixture/codex", &files, &schema, &paths).unwrap();
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(args[0..2], ["--no-daemon", "exec"]);
        for option in [
            "--ignore-user-config",
            "--ignore-rules",
            "--ephemeral",
            "--skip-git-repo-check",
            "--json",
        ] {
            assert!(args.contains(&option.to_owned()));
        }
        for flag in [
            "goals",
            "collaboration_modes",
            "default_mode_request_user_input",
            "hooks",
            "memories",
            "skill_search",
            "shell_snapshot",
            "request_permissions_tool",
            "shell_tool",
            "apps",
            "plugins",
            "multi_agent",
            "browser_use",
            "computer_use",
            "view_image",
            "code_mode",
            "code_mode_host",
            "enable_request_compression",
        ] {
            assert!(
                args.windows(2).any(|pair| pair == ["--disable", flag]),
                "missing protection: {flag}"
            );
        }
        for value in [
            "web_search=\"disabled\"",
            "skills.include_instructions=false",
            "skills.bundled.enabled=false",
            "project_doc_max_bytes=0",
        ] {
            assert!(args.windows(2).any(|pair| pair == ["-c", value]));
        }
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--sandbox", "read-only"]));
        assert!(!args.iter().any(|arg| arg == "-m" || arg == "--model"));
        assert!(!args.iter().any(|arg| arg.contains("model_provider=")
            || arg.contains("cli_auth_credentials_store")
            || arg.contains("model_catalog_json")));
        let at = args.iter().position(|arg| arg == "--image").unwrap();
        assert_eq!(
            args[at + 1],
            paths
                .iter()
                .map(|path| path.to_string_lossy())
                .collect::<Vec<_>>()
                .join(",")
        );
        assert_eq!(args.last().unwrap(), "-");
        assert_eq!(std::fs::read(&paths[0]).unwrap(), b"\x89PNG\r\n\x1a\n");
    }
    assert!(!path.exists());
}
#[test]
fn codex_contract_is_explicit_developer_argument_and_reference_context_stays_on_stdin() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let fixture = temp.path().join("fixture-codex");
    std::fs::write(&fixture,b"#!/bin/sh\nfixture_dir=$(/usr/bin/dirname \"$0\")\nprintf '%s\\n' \"$@\" > \"$fixture_dir/argv-lines\"\n/bin/cat > \"$fixture_dir/stdin-data\"\n/bin/cat \"$fixture_dir/events.jsonl\"\n").unwrap();
    std::fs::set_permissions(&fixture, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(
        temp.path().join("events.jsonl"),
        codex_events(&json!({"reply":"Fixture response."})),
    )
    .unwrap();
    let reference = json!({"context":{"heading":"UNTRUSTED_REFERENCE_MARKER -c developer_instructions=override --dangerously-bypass-approvals-and-sandbox $(touch marker)"},"conversation":[],"steps":[]});
    let response = invoke_codex(
        &repo,
        &reference.to_string(),
        &[],
        &AtomicBool::new(false),
        Instant::now() + Duration::from_secs(3),
        &mut json!({"assistant":Assistant::Codex.metadata()}),
        fixture.to_str().unwrap(),
    )
    .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&response).unwrap(),
        json!({"reply":"Fixture response."})
    );
    let args = std::fs::read_to_string(temp.path().join("argv-lines")).unwrap();
    let lines = args.lines().collect::<Vec<_>>();
    let developer = lines
        .iter()
        .filter_map(|line| line.strip_prefix("developer_instructions="))
        .collect::<Vec<_>>();
    assert_eq!(developer.len(), 1);
    assert_eq!(
        serde_json::from_str::<String>(developer[0]).unwrap(),
        format!("{SKILL}\n\n{CODEX_WIRE_INSTRUCTIONS}")
    );
    assert!(lines
        .windows(2)
        .any(|pair| pair[0] == "-c" && pair[1].starts_with("developer_instructions=")));
    assert!(!args.contains("UNTRUSTED_REFERENCE_MARKER"));
    assert!(!lines.contains(&"--dangerously-bypass-approvals-and-sandbox"));
    let stdin = std::fs::read_to_string(temp.path().join("stdin-data")).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&stdin).unwrap(), reference);
    assert!(!stdin.contains(SKILL));
    assert!(!stdin.contains(CODEX_WIRE_INSTRUCTIONS));
    assert_eq!(
        std::fs::read_dir(repo.data.parent().unwrap().join("app-chat-runtime"))
            .unwrap()
            .count(),
        0
    );
}
#[test]
fn codex_invocation_files_drop_after_process_cancellation_and_errors() {
    let temp = tempfile::tempdir().unwrap();
    let path;
    {
        let files = InvocationFiles::new(&temp.path().join("runtime")).unwrap();
        path = files.root.clone();
        files.write("image-0.png", b"fixture-image").unwrap();
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "sleep 30 & wait"]);
        let mut diagnostics = json!({"assistant":Assistant::Codex.metadata()});
        assert!(execute_diagnosed(
            command,
            "",
            &AtomicBool::new(true),
            Instant::now() + Duration::from_secs(1),
            &mut diagnostics
        )
        .unwrap_err()
        .message
        .contains("stopped"));
        assert!(path.exists());
    }
    assert!(!path.exists());
    let result = (|| -> Result<()> {
        let files = InvocationFiles::new(&temp.path().join("runtime"))?;
        files.write("image-0.png", b"fixture")?;
        let command = Command::new("/fixture/missing-codex");
        execute_diagnosed(
            command,
            "",
            &AtomicBool::new(false),
            Instant::now() + Duration::from_secs(1),
            &mut json!({"assistant":Assistant::Codex.metadata()}),
        )?;
        Ok(())
    })();
    assert!(result.is_err());
    assert_eq!(
        std::fs::read_dir(temp.path().join("runtime"))
            .unwrap()
            .count(),
        0
    );
}
#[test]
fn generic_commands_accept_complete_view_documents_above_the_search_argument_limit() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let before = tool(
        &repo,
        "command",
        &json!({"name":"views.read","args":{}}),
        &mut vec![],
    )
    .unwrap();
    let mut views = arr(&before["views"]);
    for index in 0..20 {
        views.push(json!({"id":format!("view-{index}"),"name":format!("View {index}"),"parent":"all","lifecycle":"any","filter":{"op":"all","conditions":[]},"query":"x".repeat(900),"groupBy":"source"}));
    }
    let args = json!({"name":"views.apply","args":{"revision":before["revision"],"contentHash":before["contentHash"],"views":views}});
    assert!(serde_json::to_vec(&args).unwrap().len() > 16000);
    let result = tool(&repo, "command", &args, &mut vec![]).unwrap();
    assert_eq!(
        arr(&result["views"]).len(),
        arr(&before["views"]).len() + 20
    );
    assert!(tool(
        &repo,
        "query",
        &json!({"query":"x".repeat(16001)}),
        &mut vec![]
    )
    .is_err());
    assert!(tool(
        &repo,
        "command",
        &json!({"name":"views.apply","args":{"unexpected":"x".repeat(65537)}}),
        &mut vec![]
    )
    .is_err());
}
#[test]
fn manual_work_links_tool_loop_discovers_reads_and_changes_only_the_exact_pair() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    repo.transaction(|state|{state["items"]=json!([{"id":"linear:a","source":"linear"},{"id":"github:b","source":"github"},{"id":"sentry:c","source":"sentry"}]);state["workLinks"]=json!([["github:b","sentry:c"]]);Ok(())}).unwrap();
    let mut calls = 0;
    let (reply,queued)=run(&repo,&json!({}),|prompt| {
            let transcript:Value=serde_json::from_str(prompt).unwrap();calls+=1;
            let answer=match calls {
                1=>json!({"tool":"commands","args":{}}),
                2=>{
                    let catalog=&transcript["steps"][0]["result"]["commands"];
                    for name in ["work.links.read","work.links.add","work.links.remove"] {
                        let row=arr(catalog).into_iter().find(|row|row["name"]==name).unwrap();assert_eq!(row["appChat"],true);
                    }
                    json!({"tool":"command","args":{"name":"work.links.read","args":{"itemId":"linear:a"}}})
                },
                3=>{assert_eq!(transcript["steps"][1]["result"]["total"],0);json!({"tool":"command","args":{"name":"work.links.add","args":{"itemIds":["linear:a","github:b"]}}})},
                4=>{
                    let receipt=&transcript["steps"][2]["result"];assert_eq!(receipt["manualOnly"],true);assert_eq!(receipt["linked"],true);assert_eq!(receipt["itemIds"],json!(["github:b","linear:a"]));
                    json!({"tool":"command","args":{"name":"work.links.remove","args":{"itemIds":["github:b","linear:a"]}}})
                },
                5=>{let receipt=&transcript["steps"][3]["result"];assert_eq!(receipt["manualOnly"],true);assert_eq!(receipt["linked"],false);assert_eq!(receipt["changed"],true);json!({"reply":"The manual link was removed. Other relationships may remain."})},
                _=>panic!("Unexpected tool round"),
            };
            Ok(answer.to_string())
        }).unwrap();
    assert_eq!(calls, 5);
    assert!(queued.is_empty());
    assert!(reply.contains("manual link"));
    assert_eq!(
        repo.state.lock().unwrap()["workLinks"],
        json!([["github:b", "sentry:c"]])
    );
    assert!(repo.running.lock().unwrap().is_empty());
    assert!(repo.cooldown.lock().unwrap().is_empty());
}

#[test]
fn page_visibility_choice_workflow_preserves_unmatched_saved_scopes() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    repo.transaction(|state|{state["items"]=json!([{"id":"linear:choice","source":"linear","project":"Target project","title":"Cached work"}]);Ok(())}).unwrap();
    let initial = super::super::page_settings::view(&repo).unwrap();
    let before=super::super::page_settings::update(&repo,&json!({"revision":initial["revision"],"contentHash":initial["contentHash"],"page":"linear","scopes":["Retained project"]})).unwrap();
    let mut calls = 0;
    let (reply,queued)=run(&repo,&json!({}),|prompt|{
            let transcript:Value=serde_json::from_str(prompt).unwrap();calls+=1;
            Ok(match calls {
                1=>json!({"tool":"command","args":{"name":"pages.scopes.read","args":{"page":"linear","query":"target"}}}),
                2=>{
                    let choices=&transcript["steps"][0]["result"];
                    assert_eq!(choices["choices"],json!([{"key":"Target project","cached":true,"visible":false}]));
                    json!({"tool":"command","args":{"name":"pages.settings.read","args":{}}})
                },
                3=>{
                    let current=&transcript["steps"][1]["result"];let mut selected=arr(&current["pages"]["linear"]["scopes"]);
                    selected.push(transcript["steps"][0]["result"]["choices"][0]["key"].clone());
                    json!({"tool":"command","args":{"name":"pages.settings.apply","args":{"revision":current["revision"],"contentHash":current["contentHash"],"page":"linear","scopes":selected}}})
                },
                4=>{
                    assert!(transcript["steps"][2]["result"].get("error").is_none());
                    json!({"reply":"Added Target project to the Linear page selection."})
                },
                _=>panic!("Unexpected tool round"),
            }.to_string())
        }).unwrap();
    assert_eq!(calls, 4);
    assert!(queued.is_empty());
    assert!(reply.contains("Target project"));
    let saved = super::super::page_settings::view(&repo).unwrap();
    assert_eq!(
        saved["pages"]["linear"]["scopes"],
        json!(["Retained project", "Target project"])
    );
    for page in ["github", "sentry"] {
        assert_eq!(saved["pages"][page], before["pages"][page]);
    }
    assert!(repo.running.lock().unwrap().is_empty());
    assert!(repo.cooldown.lock().unwrap().is_empty());
}
#[test]
fn standup_view_creation_preserves_views_and_queues_the_saved_identity() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let before = super::super::views::load(&repo).unwrap();
    let mut calls = 0;
    let (reply,commands)=run(&repo,&json!({"ui":{"timezone":"Europe/Berlin"}}),|prompt| {
            let transcript:Value=serde_json::from_str(prompt).unwrap();calls+=1;
            let answer=match calls {
                1=>json!({"tool":"command","args":{"name":"views.read","args":{}}}),
                2=>{
                    let doc=&transcript["steps"][0]["result"];let mut views=arr(&doc["views"]);
                    views.push(json!({"id":"standup","name":"Standup","parent":"all","lifecycle":"any","filter":{"op":"calendarPeriod","field":"updatedAt","period":"sinceYesterday","timezone":transcript["context"]["ui"]["timezone"]},"query":"","groupBy":"source"}));
                    json!({"tool":"command","args":{"name":"views.apply","args":{"revision":doc["revision"],"contentHash":doc["contentHash"],"views":views}}})
                },
                3=>{assert!(transcript["steps"][1]["result"].get("error").is_none());json!({"tool":"show-view","args":{"id":"standup"}})},
                4=>{assert_eq!(transcript["steps"][2]["result"]["queued"]["id"],"standup");json!({"reply":"Created Standup. The saved view is queued to open."})},
                _=>panic!("Unexpected model call"),
            };Ok(answer.to_string())
        }).unwrap();
    assert_eq!(calls, 4);
    assert!(reply.contains("Standup"));
    assert_eq!(
        commands,
        json!([{"type":"saved-view","id":"standup","name":"Standup"}])
            .as_array()
            .unwrap()
            .clone()
    );
    let after = super::super::views::load(&repo).unwrap();
    assert_eq!(after.views.len(), before.views.len() + 1);
    assert_eq!(
        serde_json::to_value(&after.views[..before.views.len()]).unwrap(),
        serde_json::to_value(&before.views).unwrap()
    );
    let saved = serde_json::to_value(after.views.last().unwrap()).unwrap();
    assert_eq!(saved["lifecycle"], "any");
    assert_eq!(
        saved["filter"],
        json!({"op":"calendarPeriod","field":"updatedAt","period":"sinceYesterday","timezone":"Europe/Berlin"})
    );
}
#[test]
fn actual_context_and_model_prompt_use_the_sanitized_setup_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join(".env"),
        "SETUP_TEST_SECRET=benign-private-env-marker",
    )
    .unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    repo.transaction(|state| {
            state["onboarding"]=json!({"version":1,"revision":2,"step":"integrations","completed":false,"steps":{"integrations":"pending"},"privateExtension":"benign-private-progress-marker"});
            state["sources"]["linear"]=json!({"disabled":false,"lastSuccess":200,"error":"benign-private-error-marker"});
            state["sources"]["github"]=json!({"disabled":false,"lastSuccess":"benign-private-timestamp-marker"});
            Ok(())
        }).unwrap();
    let path = repo.data.parent().unwrap().join("config/connections.json");
    std::fs::write(&path,serde_json::to_vec(&json!({"version":1,"storage":"macos-login-keychain","revision":1,"providers":{"github":{"credential_ref":uuid(),"identity":"benign-private-identity-marker","org":"","base":""}}})).unwrap()).unwrap();
    let before = repo.state.lock().unwrap().clone();
    let bytes = std::fs::read(&repo.data).unwrap();
    let snapshot = context(&repo, &json!({"source":"settings","filters":{}})).unwrap();
    assert_eq!(
        snapshot["setup"],
        super::super::onboarding::assistant_context(&repo)
    );
    assert!(!snapshot.to_string().contains("benign-private"));
    let mut calls = 0;
    run(&repo,&snapshot,|prompt| {calls+=1;assert!(!prompt.contains("benign-private"));let transcript:Value=serde_json::from_str(prompt).unwrap();assert_eq!(transcript["context"]["setup"]["credentialMetadata"],"available");Ok(if calls==1 {json!({"tool":"command","args":{"name":"onboarding.read","args":{}}})}else{json!({"reply":"Setup metadata is a snapshot. I can verify current setup before proceeding."})}.to_string())}).unwrap();
    assert_eq!(calls, 2);
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    std::fs::write(&path, "benign-private-broken-metadata").unwrap();
    let unavailable = context(&repo, &json!({"source":"settings","filters":{}})).unwrap();
    assert_eq!(unavailable["setup"]["credentialMetadata"], "unavailable");
    assert!(!unavailable.to_string().contains("benign-private"));
}

#[test]
fn saved_view_navigation_uses_verified_registry_identity() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let doc = super::super::views::load(&repo).unwrap();
    let view = &doc.views[0];
    let mut commands = vec![];
    let result = tool(&repo, "show-view", &json!({"id":view.id}), &mut commands).unwrap();
    assert_eq!(result["queued"]["id"], view.id);
    assert_eq!(result["queued"]["name"], view.name);
    assert_eq!(commands.len(), 1);
    assert!(tool(
        &repo,
        "show-view",
        &json!({"id":"missing-view"}),
        &mut commands
    )
    .is_err());
    assert_eq!(commands.len(), 1);
}
#[test]
fn settings_proposal_is_validated_without_mutation_or_private_setting_echo() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let current = repo.settings.view();
    repo.settings.apply(&json!({"revision":current["revision"],"contentHash":repo.settings.view()["contentHash"],"patch":{"quickActions":[{"id":"private-current","title":"Private current","command":"printf benign-private-command-sentinel"}]}}),false).unwrap();
    let before = repo.settings.view();
    let state = repo.state.lock().unwrap().clone();
    let preferences = PathBuf::from(text(&before["files"], "preferences"));
    let saved = std::fs::read(&preferences).unwrap();
    let request = json!({"revision":before["revision"],"contentHash":repo.settings.view()["contentHash"],"patch":{"editor":"code"}});
    let mut queued = vec![];
    let result = tool(&repo, "settings-propose", &request, &mut queued).unwrap();
    assert_eq!(result["applied"], false);
    assert_eq!(result["userApprovalRequired"], true);
    assert_eq!(result["proposedFields"], json!(["editor"]));
    assert_eq!(queued,json!([{"type":"settings-proposal","revision":before["revision"],"contentHash":repo.settings.view()["contentHash"],"patch":{"editor":"code"}}]).as_array().unwrap().clone());
    for forbidden in [
        "benign-private-command-sentinel",
        "private-current",
        "quickActions",
        "queued",
        "values",
    ] {
        assert!(!result.to_string().contains(forbidden));
    }
    assert_eq!(repo.settings.view()["values"], before["values"]);
    assert_eq!(repo.settings.view()["revision"], before["revision"]);
    assert_eq!(*repo.state.lock().unwrap(), state);
    assert_eq!(std::fs::read(&preferences).unwrap(), saved);
    let mut calls = 0;
    let (_,commands)=run(&repo,&json!({}),|prompt| {calls+=1;if calls==1 {Ok(json!({"tool":"settings-propose","args":request}).to_string())}else {assert!(!prompt.contains("benign-private-command-sentinel"));assert!(!prompt.contains("private-current"));Ok(json!({"reply":"Review the proposed editor change in Settings. Nothing was applied."}).to_string())}}).unwrap();
    assert_eq!(commands.len(), 1);
    assert_eq!(repo.settings.view()["revision"], before["revision"]);
}
#[test]
fn settings_proposal_rejects_bad_schema_stale_requests_and_queue_overflow() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let before = repo.settings.view();
    let revision = before["revision"].as_u64().unwrap();
    for args in [
        json!({"revision":revision,"patch":{}}),
        json!({"revision":revision,"contentHash":"0".repeat(64),"patch":{"editor":"code"}}),
        json!({"revision":revision+1,"contentHash":repo.settings.view()["contentHash"],"patch":{"theme":"light"}}),
        json!({"revision":-1,"contentHash":repo.settings.view()["contentHash"],"patch":{}}),
        json!({"revision":1.5,"contentHash":repo.settings.view()["contentHash"],"patch":{}}),
        json!({"revision":revision,"contentHash":repo.settings.view()["contentHash"],"patch":[]}),
        json!({"revision":revision,"contentHash":repo.settings.view()["contentHash"],"patch":{},"extra":true}),
        json!({"revision":revision,"contentHash":repo.settings.view()["contentHash"],"patch":{"token":"benign-secret-fixture"}}),
        json!({"revision":revision,"contentHash":repo.settings.view()["contentHash"],"patch":{"sharing":{"root":"/not-used"}}}),
        json!({"revision":revision,"contentHash":repo.settings.view()["contentHash"],"patch":{"uiScale":-1}}),
        json!({"revision":revision,"contentHash":repo.settings.view()["contentHash"],"patch":{"theme":"x".repeat(65536)}}),
    ] {
        let mut commands = vec![];
        assert!(tool(&repo, "settings-propose", &args, &mut commands).is_err());
        assert!(commands.is_empty());
    }
    let mut commands = vec![json!({}); 4];
    assert!(tool(&repo,"settings-propose",&json!({"revision":revision,"contentHash":repo.settings.view()["contentHash"],"patch":{"editor":"code"}}),&mut commands).is_err());
    assert_eq!(commands.len(), 4);
    assert_eq!(repo.settings.view()["values"], before["values"]);
    assert_eq!(repo.settings.view()["revision"], before["revision"]);
}
#[test]
fn both_settings_read_tools_omit_custom_workflow_content() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let marker = "benign-private-workflow-marker";
    std::fs::write(
        repo.settings
            .config_directory()
            .join("workflows/agent-review.md"),
        format!("Custom review instructions {marker}"),
    )
    .unwrap();
    assert!(repo.settings.view().to_string().contains(marker));
    let mut commands = vec![];
    let alias = tool(&repo, "settings-read", &json!({}), &mut commands).unwrap();
    let generic = tool(
        &repo,
        "command",
        &json!({"name":"settings.read","args":{}}),
        &mut commands,
    )
    .unwrap();
    assert!(!alias.to_string().contains(marker));
    assert!(!generic.to_string().contains(marker));
    assert!(commands.is_empty());
}

#[test]
fn settings_proposal_excludes_other_ui_changes_in_both_orders() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let before = repo.settings.view();
    let state = repo.state.lock().unwrap().clone();
    let proposal = json!({"revision":before["revision"],"contentHash":repo.settings.view()["contentHash"],"patch":{"editor":"code"}});
    let mut commands = vec![];
    tool(
        &repo,
        "navigate",
        &json!({"destination":"settings","category":"agents"}),
        &mut commands,
    )
    .unwrap();
    let navigation = commands.clone();
    assert!(tool(&repo, "settings-propose", &proposal, &mut commands)
        .unwrap_err()
        .message
        .contains("separate reply"));
    assert_eq!(commands, navigation);
    commands.clear();
    tool(&repo, "settings-propose", &proposal, &mut commands).unwrap();
    let pending = commands.clone();
    for name in [
        "settings-propose",
        "navigate",
        "show-view",
        "show-items",
        "show-chats",
        "open-chat",
        "show",
        "issue-draft",
    ] {
        assert!(tool(&repo, name, &proposal, &mut commands)
            .unwrap_err()
            .message
            .contains("Finish your reply"));
        assert_eq!(commands, pending);
    }
    for name in ["discover", "commands", "settings-read", "query"] {
        assert!(tool(&repo, name, &json!({}), &mut commands).is_ok());
        assert_eq!(commands, pending);
    }
    assert!(tool(
        &repo,
        "command",
        &json!({"name":"settings.read","args":{}}),
        &mut commands
    )
    .is_ok());
    assert_eq!(commands, pending);
    assert_eq!(repo.settings.view()["revision"], before["revision"]);
    assert_eq!(repo.settings.view()["values"], before["values"]);
    assert_eq!(*repo.state.lock().unwrap(), state);
}
#[test]
fn model_finishes_with_the_pending_proposal_after_rejected_navigation() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let revision = repo.settings.view()["revision"].clone();
    let mut calls = 0;
    let (answer,commands)=run(&repo,&json!({}),|prompt| {calls+=1;Ok(match calls {
            1=>json!({"tool":"settings-propose","args":{"revision":revision,"contentHash":repo.settings.view()["contentHash"],"patch":{"editor":"code"}}}),
            2=>json!({"tool":"navigate","args":{"destination":"settings"}}),
            _=>{assert!(prompt.contains("Finish your reply"));json!({"reply":"Please review the pending settings proposal."})}
        }.to_string())}).unwrap();
    assert_eq!(answer, "Please review the pending settings proposal.");
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0]["type"], "settings-proposal");
    assert_eq!(repo.settings.view()["revision"], revision);
}

#[test]
fn settings_proposal_queues_requested_array_replacement_only() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let revision = repo.settings.view()["revision"].clone();
    let patch = json!({"quickActions":[{"id":"new-action","title":"New action","command":"printf fixture"}]});
    let mut commands = vec![];
    let result=tool(&repo,"settings-propose",&json!({"revision":revision,"contentHash":repo.settings.view()["contentHash"],"patch":patch}),&mut commands).unwrap();
    assert_eq!(commands[0]["patch"], patch);
    assert!(result["note"]
        .as_str()
        .unwrap()
        .contains("replace complete arrays"));
    assert_eq!(repo.settings.view()["values"]["quickActions"], json!([]));
}

#[test]
fn structured_cli_results_decode_tools_and_fenced_legacy_replies() {
    let mut diagnostics = json!({});
    let structured =
        json!({"type":"result","is_error":false,"structured_output":{"tool":"discover","args":{}}});
    let decoded = cli_response(&structured.to_string(), false, &mut diagnostics).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&decoded).unwrap(),
        json!({"tool":"discover","args":{}})
    );
    assert_eq!(diagnostics["stage"], "complete");
    let stream = format!(
        "{{\"type\":\"system\"}}\n{}\n",
        json!({"type":"result","result":"```json\n{\"reply\":\"Ready\"}\n```"})
    );
    assert_eq!(
        serde_json::from_str::<Value>(&cli_response(&stream, true, &mut diagnostics).unwrap())
            .unwrap()["reply"],
        "Ready"
    );
    let schema = response_schema();
    for unsupported in ["anyOf", "oneOf", "allOf"] {
        assert!(schema.get(unsupported).is_none());
    }
    assert_eq!(schema["type"], "object");
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["properties"]["reply"]["type"], "string");
    assert_eq!(schema["properties"]["tool"]["type"], "string");
    assert_eq!(schema["properties"]["args"]["type"], "object");
}
#[test]
fn response_failures_record_only_shape_and_position_metadata() {
    for (output,code) in [
            (json!({"type":"result","result":"benign-private-output-not-json"}).to_string(),"invalid_assistant_json"),
            (json!({"type":"result","structured_output":{"reply":"benign-private-answer","tool":"discover","args":{}}}).to_string(),"invalid_assistant_shape"),
            (json!({"type":"result","is_error":true,"result":"benign-private-error"}).to_string(),"cli_reported_error"),
            (json!({"type":"result"}).to_string(),"missing_result"),
            ("benign-private-non-json".to_owned(),"invalid_cli_json"),
        ] {
            let mut diagnostics=json!({"requestId":"safe-request"});
            assert!(cli_response(&output,false,&mut diagnostics).is_err());
            assert_eq!(diagnostics["code"],code);
            assert!(!diagnostics.to_string().contains("benign-private"));
            assert_eq!(diagnostics["requestId"],"safe-request");
        }
    for value in [
        json!(null),
        json!([]),
        json!({"reply":""}),
        json!({"tool":"discover","args":null}),
        json!({"reply":"Ready","extra":true}),
    ] {
        let mut diagnostics = json!({});
        assert!(assistant_response(&value.to_string(), &mut diagnostics).is_err());
        assert_eq!(diagnostics["code"], "invalid_assistant_shape");
    }
}
#[test]
fn process_failure_diagnostics_exclude_stdout_and_stderr_contents() {
    let mut command = Command::new("/bin/sh");
    command.args([
        "-c",
        "printf benign-private-stdout; printf benign-private-stderr >&2; exit 7",
    ]);
    let mut diagnostics = json!({});
    assert!(execute_diagnosed(
        command,
        "",
        &AtomicBool::new(false),
        Instant::now() + Duration::from_secs(5),
        &mut diagnostics
    )
    .is_err());
    assert_eq!(diagnostics["code"], "process_failed");
    assert_eq!(diagnostics["exitCode"], 7);
    assert_eq!(diagnostics["stdoutBytes"], 21);
    assert_eq!(diagnostics["stderrBytes"], 21);
    assert!(!diagnostics.to_string().contains("benign-private"));
}

#[test]
fn new_chat_keeps_bounded_history_when_new_messages_arrive() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    for i in 0..12 {
        append(&repo, "user", &format!("Question {i}")).unwrap();
        repo.app_chat.new_chat(&repo).unwrap();
    }
    append(&repo, "user", "Current question").unwrap();
    let state = repo.state.lock().unwrap();
    assert_eq!(arr(&state["appChat"]["history"]).len(), 10);
    assert_eq!(
        state["appChat"]["history"][0]["messages"][0]["text"],
        "Question 2"
    );
    assert_eq!(state["appChat"]["messages"][0]["text"], "Current question");
    drop(state);
    repo.transaction(|state| {
        state["appChat"]["version"] = json!(2);
        Ok(())
    })
    .unwrap();
    assert!(append(&repo, "user", "No overwrite").is_err());
    assert!(repo.app_chat.new_chat(&repo).is_err());
}
#[test]
fn agent_query_item_and_display_share_effective_work_projection() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    repo.transaction(|state| {
        state["items"] =
            json!([{"id":"linear:one","source":"linear","title":"Thinking-v2","status":"todo"}]);
        state["overrides"] = json!({"linear:one":{"status":"done","pinned":true}});
        Ok(())
    })
    .unwrap();
    let mut commands = vec![];
    let query = tool(
        &repo,
        "query",
        &json!({"query":"thinkingv2"}),
        &mut commands,
    )
    .unwrap();
    let generic = super::super::commands::invoke(
        &repo,
        &json!({"name":"work.query","args":{"query":"thinkingv2"}}),
        true,
    )
    .unwrap();
    assert_eq!(query, generic);
    let item = tool(&repo, "item", &json!({"id":"linear:one"}), &mut commands).unwrap();
    let shown = tool(
        &repo,
        "show-items",
        &json!({"items":[{"type":"work","id":"linear:one"}]}),
        &mut commands,
    )
    .unwrap();
    for row in [
        &query["items"][0],
        &item,
        &shown["queued"]["items"][0]["item"],
    ] {
        assert_eq!(row["status"], "done");
        assert_eq!(row["pinned"], true);
    }
}
#[test]
fn image_inputs_are_bounded_and_stream_results_are_parsed() {
    use base64::Engine;
    let valid = base64::engine::general_purpose::STANDARD.encode(b"\x89PNG\r\n\x1a\nfixture");
    let blocks = image_blocks(&json!([{"mediaType":"image/png","data":valid}])).unwrap();
    assert_eq!(blocks[0]["source"]["media_type"], "image/png");
    assert!(image_blocks(&json!([{"mediaType":"image/svg+xml","data":"anything"}])).is_err());
    assert!(image_blocks(&json!([{"mediaType":"image/jpeg","data":"not base64"}])).is_err());
    assert!(image_blocks(&json!([{}, {}, {}])).is_err());
    assert_eq!(
        image_reply("{\"type\":\"system\"}\n{\"type\":\"result\",\"result\":\"image reply\"}\n")
            .unwrap(),
        "image reply"
    );
    assert!(image_reply("{\"type\":\"result\",\"is_error\":true,\"result\":\"failed\"}").is_err());
}
#[test]
fn chat_search_opens_verified_match_instead_of_filtering_history() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let id = uuid();
    repo.transaction(|s|{s["sessions"]=json!([{"id":id,"title":"benchmark","agent":"claude","cwd":"/projects/.worktrees/thinking-v2-repro","items":[]}]);Ok(())}).unwrap();
    let mut commands = vec![];
    let found = tool(
        &repo,
        "chats-query",
        &json!({"query":"benchmarking thinkingv2"}),
        &mut commands,
    )
    .unwrap();
    assert_eq!(found["chats"][0]["id"], id);
    tool(&repo, "open-chat", &json!({"id":id}), &mut commands).unwrap();
    assert_eq!(
        commands,
        json!([{"type":"open-chat","id":id,"results":null}])
            .as_array()
            .unwrap()
            .clone()
    );
    assert!(tool(&repo, "open-chat", &json!({"id":"missing"}), &mut commands).is_err());
    assert_eq!(commands.len(), 1);
    tool(
        &repo,
        "show-chats",
        &json!({"query":"thinkingv2","recommendedId":id,"reason":"The folder matches."}),
        &mut commands,
    )
    .unwrap();
    assert_eq!(commands[1]["recommendedId"], id);
    assert!(tool(
        &repo,
        "show-chats",
        &json!({"query":"thinkingv2","recommendedId":"invented"}),
        &mut commands
    )
    .is_err());
    assert_eq!(commands.len(), 2);
}
#[test]
fn navigation_intents_allow_only_known_ui_destinations_and_settings_categories() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    for destination in [
        "setup",
        "skills",
        "artifacts",
        "logs",
        "settings",
        "sharing-inbox",
        "chat-recovery",
    ] {
        let mut commands = vec![];
        tool(
            &repo,
            "navigate",
            &json!({"destination":destination}),
            &mut commands,
        )
        .unwrap();
        assert_eq!(
            commands,
            vec![json!({"type":"navigate","destination":destination})]
        );
    }
    let mut commands = vec![];
    tool(
        &repo,
        "navigate",
        &json!({"destination":"settings","category":"connections"}),
        &mut commands,
    )
    .unwrap();
    assert_eq!(commands[0]["category"], "connections");
    for args in [
        json!({"destination":"setup","category":"appearance"}),
        json!({"destination":"setup","step":"complete"}),
        json!({"destination":"setup","reset":true}),
        json!({"destination":null}),
        json!({"destination":"run-skill"}),
        json!({"destination":"https://example.com"}),
        json!({"destination":"logs","category":"data"}),
        json!({"destination":"settings","category":"invented"}),
        json!({"destination":"settings","category":null}),
        json!({"destination":"settings","token":"benign-canary"}),
    ] {
        assert!(tool(&repo, "navigate", &args, &mut commands).is_err());
    }
    assert_eq!(commands.len(), 1);
    for _ in 0..3 {
        tool(
            &repo,
            "navigate",
            &json!({"destination":"skills"}),
            &mut commands,
        )
        .unwrap();
    }
    assert!(tool(
        &repo,
        "navigate",
        &json!({"destination":"skills"}),
        &mut commands
    )
    .is_err());
}
#[test]
fn chat_recovery_navigation_queues_only_a_local_view_without_replaying_requests() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    repo.transaction(|_| Ok(())).unwrap();
    let before = repo.state.lock().unwrap().clone();
    let bytes = std::fs::read(&repo.data).unwrap();
    let settings = repo.settings.view();
    let navigation = arr(&guide()["tools"])
        .into_iter()
        .find(|entry| entry["name"] == "navigate")
        .unwrap();
    assert!(navigation["args"]["destination"]
        .as_str()
        .unwrap()
        .split('|')
        .any(|destination| destination == "chat-recovery"));
    assert!(SKILL.contains(
        "You may select a verified saved chat with open-chat; this changes only the view."
    ));
    let mut calls = 0;
    let (_,commands)=run(&repo,&json!({}),|_|{
            calls+=1;
            Ok(if calls==1 {json!({"tool":"navigate","args":{"destination":"chat-recovery"}})}else{json!({"reply":"The pending requests list is queued to open. No request was repeated or cleared."})}.to_string())
        }).unwrap();
    assert_eq!(calls, 2);
    assert_eq!(
        commands,
        vec![json!({"type":"navigate","destination":"chat-recovery"})]
    );
    let mut queued = vec![];
    for args in [
        json!({"destination":"chat-recovery","requestId":uuid()}),
        json!({"destination":"chat-recovery","clear":true}),
        json!({"destination":"chat-recovery","category":"agents"}),
        json!({"destination":"chat-recovery","open":true}),
    ] {
        assert!(tool(&repo, "navigate", &args, &mut queued).is_err());
    }
    assert!(queued.is_empty());
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    assert_eq!(repo.settings.view(), settings);
}

#[test]
fn setup_navigation_reads_progress_and_queues_without_mutating_setup() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    repo.transaction(|state| {state["onboarding"]=json!({"version":1,"revision":3,"step":"company","completed":false,"steps":{"theme":"complete","company":"pending"}});Ok(())}).unwrap();
    let before = repo.state.lock().unwrap().clone();
    let settings = repo.settings.view();
    let bytes = std::fs::read(&repo.data).unwrap();
    let navigation = arr(&guide()["tools"])
        .into_iter()
        .find(|entry| entry["name"] == "navigate")
        .unwrap();
    assert!(navigation["args"]["destination"]
        .as_str()
        .unwrap()
        .split('|')
        .any(|destination| destination == "setup"));
    assert!(navigation["description"]
        .as_str()
        .unwrap()
        .contains("onboarding.read"));
    assert!(SKILL.contains("Before guiding setup"));
    let mut calls = 0;
    let (_,commands)=run(&repo,&json!({}),|prompt| {
            calls+=1;let transcript:Value=serde_json::from_str(prompt).unwrap();
            Ok(match calls {
                1=>json!({"tool":"command","args":{"name":"onboarding.read","args":{}}}),
                2=>{assert_eq!(transcript["steps"][0]["result"]["progress"]["step"],"company");json!({"tool":"navigate","args":{"destination":"setup"}})},
                _=>{assert_eq!(transcript["steps"][1]["result"]["queued"],json!({"type":"navigate","destination":"setup"}));json!({"reply":"Setup is queued to open at your saved progress. No setup choices were changed."})}
            }.to_string())
        }).unwrap();
    assert_eq!(calls, 3);
    assert_eq!(
        commands,
        vec![json!({"type":"navigate","destination":"setup"})]
    );
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(repo.settings.view(), settings);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
}
#[test]
fn settings_filter_context_reaches_the_model_without_drafts_or_writes() {
    let temp = tempfile::tempdir().unwrap();
    let long_secret = format!("private-cutoff-{}", "z".repeat(220));
    std::fs::write(temp.path().join(".env"), format!("SETTINGS_CONTEXT_SECRET=private-filter-secret\nSETTINGS_CONTEXT_LONG_SECRET={long_secret}\n")).unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    repo.transaction(|_| Ok(())).unwrap();
    let before = repo.state.lock().unwrap().clone();
    let bytes = std::fs::read(&repo.data).unwrap();
    let preferences = repo.settings.view();
    let filter = json!({"query":"  private-filter-secret scale  ","queryTruncated":true,"modifiedOnly":true,"preferencesDirty":true});
    let snapshot = context(&repo, &json!({"source":"settings","filters":{},"ui":{"settingsFilter":filter,"draft":{"uiScale":130,"token":"private-draft-marker"}}})).unwrap();
    let projected = &snapshot["ui"]["settingsFilter"];
    assert_eq!(projected.as_object().unwrap().len(), 5);
    assert_eq!(projected["scope"], "individual-preferences-and-shortcuts");
    assert_eq!(projected["query"], "  [redacted] scale  ");
    for key in ["queryTruncated", "modifiedOnly", "preferencesDirty"] {
        assert_eq!(projected[key], true);
    }
    let mut calls = 0;
    let (_, commands) = run(&repo, &snapshot, |prompt| {
            calls += 1;
            assert!(!prompt.contains("private-filter-secret"));
            assert!(!prompt.contains("private-draft-marker"));
            let transcript: Value = serde_json::from_str(prompt).unwrap();
            assert_eq!(transcript["context"]["ui"]["settingsFilter"], *projected);
            Ok(if calls == 1 { json!({"tool":"settings-read","args":{}}) } else {
                assert_eq!(transcript["steps"][0]["result"]["values"]["uiScale"], preferences["values"]["uiScale"]);
                assert!(prompt.contains("settings-read returns saved preferences"));
                json!({"reply":"The visible Settings filter is modified-only and has unsaved preferences. The saved settings remain separate."})
            }.to_string())
        }).unwrap();
    assert_eq!(calls, 2);
    assert!(commands.is_empty());
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
    assert_eq!(repo.settings.view(), preferences);
    assert!(
        context(&repo, &json!({"source":"settings","filters":{}})).unwrap()["ui"]
            .get("settingsFilter")
            .is_none()
    );
    let unicode = json!({"query":"🙂".repeat(200),"queryTruncated":false,"modifiedOnly":false,"preferencesDirty":false});
    assert_eq!(
        context(
            &repo,
            &json!({"source":"settings","filters":{},"ui":{"settingsFilter":unicode}})
        )
        .unwrap()["ui"]["settingsFilter"]["query"],
        "🙂".repeat(200)
    );
    let prefix = format!("scale {}", long_secret)
        .chars()
        .take(200)
        .collect::<String>();
    let truncated = context(&repo, &json!({"source":"settings","filters":{},"ui":{"settingsFilter":{"query":prefix,"queryTruncated":true,"modifiedOnly":false,"preferencesDirty":false}}})).unwrap();
    assert_eq!(
        truncated["ui"]["settingsFilter"]["query"],
        "scale [redacted]"
    );
    run(&repo, &truncated, |prompt| {
        assert!(!prompt.contains("private-cutoff-"));
        Ok(json!({"reply":"The visible search is incomplete."}).to_string())
    })
    .unwrap();
}
#[test]
fn settings_filter_context_rejects_private_fields_and_malformed_snapshots() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    repo.transaction(|_| Ok(())).unwrap();
    let before = repo.state.lock().unwrap().clone();
    let bytes = std::fs::read(&repo.data).unwrap();
    let valid = json!({"query":"scale","queryTruncated":false,"modifiedOnly":true,"preferencesDirty":false});
    let mut invalid = vec![Value::Null, json!([]), json!({})];
    for (key, value) in [
        ("query", json!("🙂".repeat(201))),
        ("query", json!("bad\0query")),
        ("query", json!(false)),
        ("queryTruncated", json!("true")),
        ("modifiedOnly", Value::Null),
        ("preferencesDirty", json!(1)),
        ("draft", json!({"token":"private-marker"})),
    ] {
        let mut filter = valid.clone();
        filter[key] = value;
        invalid.push(filter);
    }
    for key in [
        "query",
        "queryTruncated",
        "modifiedOnly",
        "preferencesDirty",
    ] {
        let mut filter = valid.clone();
        filter.as_object_mut().unwrap().remove(key);
        invalid.push(filter);
    }
    for filter in invalid {
        assert!(context(
            &repo,
            &json!({"source":"settings","filters":{},"ui":{"settingsFilter":filter}})
        )
        .is_err());
    }
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
}
#[test]
fn sharing_context_records_navigation_without_private_inputs() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let value = context(&repo, &json!({"source":"github","filters":{},"ui":{"sharingOpen":true,"sharingInbox":true,"token":"benign-private-input"}})).unwrap();
    assert_eq!(value["ui"]["sharingOpen"], true);
    assert_eq!(value["ui"]["sharingInbox"], true);
    assert_eq!(value["ui"]["timezone"], "UTC");
    let local = context(
        &repo,
        &json!({"source":"all","filters":{},"ui":{"timezone":"Europe/Berlin"}}),
    )
    .unwrap();
    assert_eq!(local["ui"]["timezone"], "Europe/Berlin");
    assert!(context(
        &repo,
        &json!({"source":"all","filters":{},"ui":{"timezone":"not/a-zone"}})
    )
    .is_err());
    assert!(!value.to_string().contains("benign-private-input"));
    assert!(context(
        &repo,
        &json!({"source":"github","filters":{},"ui":{"sharingInbox":"invalid"}})
    )
    .is_err());
}
#[test]
fn query_guide_exposes_view_id_and_preserves_saved_query_defaults() {
    let guide = guide();
    let query = guide["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "query")
        .unwrap();
    assert!(query["args"]["viewId"].is_string());
    assert!(text(query, "description").contains("omit query"));
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let mut document = super::super::views::view(&repo).unwrap();
    document["views"][0]["query"] = json!("latency");
    let id = document["views"][0]["id"].clone();
    super::super::views::update(&repo, &json!({"revision":document["revision"],"contentHash":document["contentHash"],"views":document["views"]})).unwrap();
    repo.transaction(|state| {
            state["items"] = json!([
                {"id":"github:latency","source":"github","kind":"Pull request","relations":["authored"],"title":"Reduce latency","status":"todo"},
                {"id":"github:cache","source":"github","kind":"Pull request","relations":["authored"],"title":"Fix cache","status":"todo"},
                {"id":"github:done","source":"github","kind":"Pull request","relations":["authored"],"title":"Previous latency fix","status":"done"}
            ]);
            Ok(())
        }).unwrap();
    let mut commands = vec![];
    let result = tool(&repo, "query", &json!({"viewId":id}), &mut commands).unwrap();
    assert_eq!(result["total"], 1);
    assert_eq!(result["items"][0]["id"], "github:latency");
    assert_eq!(
        tool(
            &repo,
            "query",
            &json!({"viewId":id,"query":""}),
            &mut commands
        )
        .unwrap()["total"],
        2
    );
    assert!(commands.is_empty());
}
#[test]
fn typed_tools_find_work_navigate_and_gate_settings() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    repo.transaction(|s|{s["items"]=json!([{"id":"linear:a","source":"linear","title":"Reduce latency","detail":"request path"},{"id":"github:b","source":"github","title":"Fix cache"}]);Ok(())}).unwrap();
    let mut commands = vec![];
    assert_eq!(
        tool(
            &repo,
            "query",
            &json!({"query":"latency","source":"all"}),
            &mut commands
        )
        .unwrap()["total"],
        1
    );
    assert!(tool(
        &repo,
        "show",
        &json!({"source":"javascript:evil"}),
        &mut commands
    )
    .is_err());
    assert!(tool(
            &repo,
            "settings-apply",
            &json!({"revision":0,"contentHash":repo.settings.view()["contentHash"],"patch":{"editor":"code"}}),
            &mut commands
        )
        .is_err());
    assert!(routine_settings(&repo)["values"]
        .get("quickActions")
        .is_none());
    let responses = [
        json!({"tool":"query","args":{"query":"latency"}}),
        json!({"tool":"show","args":{"source":"all","query":"latency","status":"all"}}),
        json!({"tool":"settings-apply","args":{"revision":0,"contentHash":repo.settings.view()["contentHash"],"patch":{"theme":"github-light"}}}),
        json!({"reply":"Found one cached ticket. The theme is now light."}),
    ];
    let mut i = 0;
    let result = run(&repo, &json!({}), |prompt| {
        if i == 1 {
            assert!(prompt.contains("Reduce latency"));
        }
        let value = responses[i].to_string();
        i += 1;
        Ok(value)
    })
    .unwrap();
    assert_eq!(result.1[0]["query"], "latency");
    assert_eq!(repo.settings.values()["theme"], "github-light");
}
#[test]
fn recovery_context_verifies_receipts_and_reaches_the_model_without_replay() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let request = uuid();
    let missing = uuid();
    let receipt = crate::backend::chat_preparation::prepare(&repo,"new",&json!({"requestId":request,"agent":"codex","title":"Recovery fixture","cwd":repo.root,"destination":"embedded","instruction":"private-recovery-instruction-canary","itemIds":[]})).unwrap();
    let before = repo.state.lock().unwrap().clone();
    let bytes = std::fs::read(&repo.data).unwrap();
    let context = context(&repo,&json!({"source":"all","filters":{},"chatRecovery":{"availability":"available","total":3,"entries":[
            {"requestId":request,"kind":"quick","recordId":uuid(),"pending":true,"openAttempted":true},
            {"requestId":missing,"kind":"new","pending":false,"openAttempted":false}
        ]}})).unwrap();
    let recovery = &context["chatRecovery"];
    assert_eq!(recovery["more"], true);
    assert_eq!(recovery["entries"][0]["client"]["kind"], "quick");
    assert_eq!(recovery["entries"][0]["preparation"], receipt);
    assert_eq!(
        recovery["entries"][1]["preparation"],
        json!({"requestId":missing,"status":"not-found"})
    );
    let mut calls = 0;
    let result = run(&repo,&context,|prompt| {
            calls += 1;
            assert!(!prompt.contains("private-recovery-instruction-canary"));
            let transcript:Value = serde_json::from_str(prompt).unwrap();
            assert_eq!(transcript["context"]["chatRecovery"],*recovery);
            Ok(if calls == 1 {
                json!({"tool":"command","args":{"name":"chats.preparation.read","args":{"requestId":request}}})
            } else {
                assert!(prompt.contains("not-found can race an in-flight preparation"));
                json!({"reply":"The original request saved a chat. This does not confirm an open request. Inspect the existing chat before another action."})
            }.to_string())
        }).unwrap();
    assert_eq!(calls, 2);
    assert!(result.1.is_empty());
    assert_eq!(*repo.state.lock().unwrap(), before);
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
}

#[test]
fn recovery_context_rejects_extra_data_bad_identities_and_unbounded_input() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    repo.transaction(|_| Ok(())).unwrap();
    let entry = json!({"requestId":uuid(),"kind":"new","pending":false,"openAttempted":false});
    let valid = json!({"availability":"available","total":1,"entries":[entry]});
    let mut bad = vec![
        Value::Null,
        json!({"availability":"unavailable","entries":[]}),
    ];
    let excessive = (0..9)
        .map(|_| json!({"requestId":uuid(),"kind":"new","pending":false,"openAttempted":false}))
        .collect::<Vec<_>>();
    for (field, value) in [
        ("total", json!(65)),
        ("total", json!(0)),
        ("total", json!("1")),
        ("entries", json!(excessive)),
        ("private", json!("private-recovery-canary")),
    ] {
        let mut value_context = valid.clone();
        value_context[field] = value;
        bad.push(value_context);
    }
    for (field, value) in [
        ("requestId", json!("not-a-uuid")),
        ("requestId", json!(format!("urn:uuid:{}", uuid()))),
        ("kind", json!("shell")),
        ("recordId", Value::Null),
        ("pending", json!("true")),
        ("openAttempted", json!(true)),
        ("instruction", json!("private-recovery-canary")),
    ] {
        let mut value_context = valid.clone();
        value_context["entries"][0][field] = value;
        bad.push(value_context);
    }
    bad.push(json!({"availability":"available","total":2,"entries":[entry,entry]}));
    let excessive = json!({"availability":"available","total":9,"entries":(0..9).map(|_|json!({"requestId":uuid(),"kind":"new","pending":false,"openAttempted":false})).collect::<Vec<_>>()});
    bad.push(excessive);
    let bytes = std::fs::read(&repo.data).unwrap();
    for value in bad {
        assert!(chat_recovery_context(&repo, &json!({"chatRecovery":value})).is_err());
    }
    assert_eq!(
        chat_recovery_context(&repo, &json!({})).unwrap(),
        json!({"availability":"not-provided"})
    );
    assert_eq!(
        chat_recovery_context(
            &repo,
            &json!({"chatRecovery":{"availability":"unavailable"}})
        )
        .unwrap(),
        json!({"availability":"unavailable"})
    );
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
}

#[test]
fn recovery_lookup_failure_stays_bounded_and_does_not_block_help() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let request = uuid();
    let receipt=crate::backend::chat_preparation::prepare(&repo,"new",&json!({"requestId":request,"agent":"codex","title":"Recovery fixture","cwd":repo.root,"destination":"embedded","instruction":"private-recovery-canary","itemIds":[]})).unwrap();
    repo.transaction(|state| {
        let record = state["sessions"][0].clone();
        state["sessions"].as_array_mut().unwrap().push(record);
        Ok(())
    })
    .unwrap();
    let bytes = std::fs::read(&repo.data).unwrap();
    let c=context(&repo,&json!({"source":"all","filters":{},"chatRecovery":{"availability":"available","total":1,"entries":[{"requestId":request,"recordId":receipt["recordId"],"kind":"new","pending":false,"openAttempted":true}]}})).unwrap();
    assert_eq!(
        c["chatRecovery"]["entries"][0]["preparation"],
        json!({"requestId":request,"status":"unavailable"})
    );
    assert!(!c.to_string().contains("private-recovery-canary"));
    assert_eq!(std::fs::read(&repo.data).unwrap(), bytes);
}

#[test]
fn history_is_bounded_and_context_omits_terminal_content() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    for _ in 0..40 {
        append(&repo, "user", "hello").unwrap();
    }
    assert_eq!(
        Host::default().view(&repo)["messages"]
            .as_array()
            .unwrap()
            .len(),
        MAX_MESSAGES
    );
    let c = context(
        &repo,
        &json!({"source":"all","query":"latency","filters":{},"terminal":"private input"}),
    )
    .unwrap();
    assert!(!c.to_string().contains("private input"));
    {
        let mut state = repo.state.lock().unwrap();
        state["sessions"] = json!([{ "id":"remembered-chat","title":"Hidden chat" }]);
    }
    let dashboard = context(
        &repo,
        &json!({"source":"linear","recordId":"remembered-chat","worktree":"/hidden","filters":{}}),
    )
    .unwrap();
    assert!(dashboard["selectedChat"].is_null());
    assert!(dashboard["selectedWorktree"].is_null());
    let chat = context(
        &repo,
        &json!({"source":"agents","recordId":"remembered-chat","filters":{}}),
    )
    .unwrap();
    assert_eq!(chat["selectedChat"]["title"], "Hidden chat");
    assert!(run(&repo, &c, |_| Ok("not json".into())).is_err());
}
#[test]
fn new_chat_preserves_previous_messages_and_rejects_running_reply() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let host = Host::default();
    append(&repo, "user", "Previous conversation").unwrap();
    *host.job.lock().unwrap() = Some(json!({"state":"running"}));
    assert!(host.new_chat(&repo).is_err());
    *host.job.lock().unwrap() = None;
    host.new_chat(&repo).unwrap();
    assert!(arr(&host.view(&repo)["messages"]).is_empty());
    let state = repo.state.lock().unwrap();
    assert_eq!(
        state["appChat"]["history"][0]["messages"][0]["text"],
        "Previous conversation"
    );
}
#[test]
fn hotkey_changes_use_discovery_and_current_revision() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let mut bindings = repo.settings.values()["keybindings"].clone();
    bindings["view-all"] = json!("Primary+Alt+I");
    let responses = [
        json!({"tool":"settings-read","args":{}}),
        json!({"tool":"settings-apply","args":{"revision":0,"contentHash":repo.settings.view()["contentHash"],"patch":{"keybindings":bindings}}}),
        json!({"reply":"All Items now uses Primary+Alt+I."}),
    ];
    let mut index = 0;
    run(&repo, &json!({}), |prompt| {
        if index == 1 {
            assert!(prompt.contains("Go to all items"));
            assert!(prompt.contains("view-all"));
        }
        let response = responses[index].to_string();
        index += 1;
        Ok(response)
    })
    .unwrap();
    assert_eq!(
        repo.settings.values()["keybindings"]["view-all"],
        "Primary+Alt+I"
    );
    assert!(tool(
            &repo,
            "settings-apply",
            &json!({"revision":0,"contentHash":repo.settings.view()["contentHash"],"patch":{"theme":"light"}}),
            &mut vec![]
        )
        .is_err());
}
#[test]
fn cancellation_and_deadline_reap_background_processes() {
    for cancelled in [false, true] {
        let flag = AtomicBool::new(cancelled);
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "sleep 30 & wait"]);
        let start = Instant::now();
        let error = execute(command, "", &flag, start + Duration::from_millis(30)).unwrap_err();
        assert!(start.elapsed() < Duration::from_secs(2));
        assert!(error
            .message
            .contains(if cancelled { "stopped" } else { "timed out" }));
    }
}
#[test]
fn tool_errors_are_returned_to_the_model_without_mutating_settings() {
    let temp = tempfile::tempdir().unwrap();
    let repo = Repository::open(temp.path().into(), None, true).unwrap();
    let mut call = 0;
    let result=run(&repo,&json!({}),|prompt|{call+=1;if call==1{Ok(json!({"tool":"settings-apply","args":{"revision":0,"contentHash":repo.settings.view()["contentHash"],"patch":{"quickActions":[{"id":"no","title":"No","command":"echo no"}]}}}).to_string())}else{assert!(prompt.contains("Ask the user to approve it in Settings"));Ok(json!({"reply":"Approve executable actions in Settings."}).to_string())}}).unwrap();
    assert!(result.1.is_empty());
    assert_eq!(repo.settings.view()["revision"], 0);
}
