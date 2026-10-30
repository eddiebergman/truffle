use super::*;
use std::os::unix::fs::PermissionsExt;

static NOT_CANCELLED: AtomicBool = AtomicBool::new(false);

fn request<'a>(mode: Mode<'a>, scratch: &'a Path, cancel: &'a AtomicBool) -> Request<'a> {
    Request {
        prompt: "fixture prompt",
        instructions: "fixture instructions",
        mode,
        schema: None,
        images: &[],
        scratch,
        limit: 128 * 1024,
        timeout: Duration::from_secs(5),
        cancel,
    }
}
fn arguments(command: &Command) -> Vec<String> {
    command
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}
fn has(args: &[String], pair: [&str; 2]) -> bool {
    args.windows(2).any(|window| window == pair)
}
fn images() -> Vec<Value> {
    vec![
        json!({"type":"image","source":{"type":"base64","media_type":"image/png","data":"iVBORw0KGgo="}}),
        json!({"type":"image","source":{"type":"base64","media_type":"image/jpeg","data":"/9j/"}}),
    ]
}
/// A fake agent that records its arguments, environment and stdin next to itself.
fn fake_agent(dir: &Path, script: &str) -> String {
    let path = dir.join("fake-agent");
    std::fs::write(
        &path,
        format!("#!/bin/sh\nhere=$(/usr/bin/dirname \"$0\")\nprintf '%s\\n' \"$@\" > \"$here/argv\"\n/usr/bin/env > \"$here/env\"\n/bin/cat > \"$here/stdin\"\n{script}\n"),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    path.to_string_lossy().into_owned()
}

#[test]
fn claude_ask_is_hardened_and_runs_in_its_private_folder() {
    let temp = tempfile::tempdir().unwrap();
    let files = Files::new(temp.path()).unwrap();
    let schema = json!({"type":"object"});
    let mut ask = request(Mode::Ask, temp.path(), &NOT_CANCELLED);
    ask.schema = Some(&schema);
    let command = build("claude", Agent::Claude, &ask, &files, []).unwrap();
    let args = arguments(&command);
    assert_claude_protections(&args);
    assert!(has(&args, ["--output-format", "json"]));
    assert!(has(&args, ["--json-schema", "{\"type\":\"object\"}"]));
    assert!(!args
        .iter()
        .any(|arg| arg == "--resume" || arg == "--verbose"));
    assert_eq!(command.get_current_dir(), Some(files.root.as_path()));
    let images = images();
    ask.images = &images;
    let args = arguments(&build("claude", Agent::Claude, &ask, &files, []).unwrap());
    assert!(has(&args, ["--output-format", "stream-json"]));
    assert!(has(&args, ["--input-format", "stream-json"]));
    let line: Value = serde_json::from_str(&input(Agent::Claude, &ask)).unwrap();
    assert_eq!(line["message"]["content"][0]["text"], "fixture prompt");
    assert_eq!(line["message"]["content"][1], images[0]);
}
#[test]
fn claude_fork_resumes_a_copy_with_the_same_hardening() {
    let temp = tempfile::tempdir().unwrap();
    let files = Files::new(temp.path()).unwrap();
    let fork = request(
        Mode::ForkSession {
            session: "0f8fad5b-d9cb-469f-a165-70867728950e",
            cwd: Path::new("/fixture/project"),
        },
        temp.path(),
        &NOT_CANCELLED,
    );
    let command = build("claude", Agent::Claude, &fork, &files, []).unwrap();
    let args = arguments(&command);
    assert_claude_protections(&args);
    assert!(has(
        &args,
        ["--resume", "0f8fad5b-d9cb-469f-a165-70867728950e"]
    ));
    assert!(args.contains(&"--fork-session".to_owned()));
    assert!(has(&args, ["--system-prompt-snapshot", "off"]));
    assert_eq!(
        command.get_current_dir(),
        Some(Path::new("/fixture/project"))
    );
    assert_eq!(input(Agent::Claude, &fork), "fixture prompt");
}
fn assert_claude_protections(args: &[String]) {
    for flag in [
        "--safe-mode",
        "--print",
        "--no-session-persistence",
        "--strict-mcp-config",
        "--disable-slash-commands",
    ] {
        assert!(
            args.contains(&flag.to_owned()),
            "missing protection: {flag}"
        );
    }
    for pair in [
        ["--setting-sources", ""],
        ["--tools", ""],
        ["--mcp-config", "{\"mcpServers\":{}}"],
        ["--settings", "{\"disableAllHooks\":true}"],
        ["--max-budget-usd", "3"],
        ["--system-prompt", "fixture instructions"],
    ] {
        assert!(has(args, pair), "missing protection: {pair:?}");
    }
}
#[test]
fn codex_ask_uses_proven_disable_flags_and_private_files() {
    let temp = tempfile::tempdir().unwrap();
    let path;
    {
        let files = Files::new(&temp.path().join("runtime")).unwrap();
        path = files.root.clone();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let schema = json!({"type":"object"});
        let images = images();
        let mut ask = request(Mode::Ask, temp.path(), &NOT_CANCELLED);
        ask.schema = Some(&schema);
        ask.images = &images;
        let command = build("/fixture/codex", Agent::Codex, &ask, &files, []).unwrap();
        let args = arguments(&command);
        assert_codex_protections(&args);
        let paths = [path.join("image-0.png"), path.join("image-1.jpg")];
        let schema_path = path.join("response-schema.json");
        for file in [&schema_path, &paths[0], &paths[1]] {
            assert_eq!(
                std::fs::metadata(file).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert!(has(
            &args,
            ["--output-schema", schema_path.to_str().unwrap()]
        ));
        assert!(has(
            &args,
            [
                "--image",
                &format!("{},{}", paths[0].display(), paths[1].display())
            ]
        ));
        assert!(has(&args, ["-C", path.to_str().unwrap()]));
        assert!(!args.contains(&"fork".to_owned()));
        assert_eq!(args.last().unwrap(), "-");
        assert_eq!(command.get_current_dir(), Some(path.as_path()));
        assert_eq!(std::fs::read(&paths[0]).unwrap(), b"\x89PNG\r\n\x1a\n");
        assert!(files
            .write("response-schema.json", b"must-not-overwrite")
            .is_err());
        assert_eq!(input(Agent::Codex, &ask), "fixture prompt");
    }
    assert!(!path.exists());
}
#[test]
fn codex_fork_keeps_the_same_hardening_and_names_the_session_last() {
    let temp = tempfile::tempdir().unwrap();
    let files = Files::new(temp.path()).unwrap();
    let fork = request(
        Mode::ForkSession {
            session: "0f8fad5b-d9cb-469f-a165-70867728950e",
            cwd: Path::new("/fixture/project"),
        },
        temp.path(),
        &NOT_CANCELLED,
    );
    let command = build("/fixture/codex", Agent::Codex, &fork, &files, []).unwrap();
    let args = arguments(&command);
    assert_codex_protections(&args);
    assert_eq!(
        args[args.len() - 3..],
        ["fork", "0f8fad5b-d9cb-469f-a165-70867728950e", "-"]
    );
    assert!(has(&args, ["-C", "/fixture/project"]));
    assert_eq!(
        command.get_current_dir(),
        Some(Path::new("/fixture/project"))
    );
}
fn assert_codex_protections(args: &[String]) {
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
    for flag in CODEX_DISABLED {
        assert!(has(args, ["--disable", flag]), "missing protection: {flag}");
    }
    assert_eq!(CODEX_DISABLED.len(), 18);
    for value in [
        "web_search=\"disabled\"",
        "skills.include_instructions=false",
        "skills.bundled.enabled=false",
        "project_doc_max_bytes=0",
        "developer_instructions=\"fixture instructions\"",
    ] {
        assert!(has(args, ["-c", value]), "missing protection: {value}");
    }
    assert!(has(args, ["--sandbox", "read-only"]));
    assert!(!args.iter().any(|arg| arg == "-m" || arg == "--model"));
    assert!(!args.iter().any(|arg| arg.contains("model_provider=")
        || arg.contains("cli_auth_credentials_store")
        || arg.contains("model_catalog_json")));
}
#[test]
fn environment_keeps_only_what_each_cli_needs() {
    let values = [
        "HOME",
        "CODEX_HOME",
        "CODEX_API_KEY",
        "CLAUDE_CONFIG_DIR",
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "LINEAR_API_KEY",
        "GITHUB_TOKEN",
        "TRUFFLE_AGENT_TOKEN",
        "TRUFFLE_RECORD_ID",
        "CLAUDECODE",
        "CODEX_THREAD_ID",
        "PATH",
    ]
    .map(|key| (key.to_owned(), "benign-private-env".to_owned()));
    let kept = |agent| {
        allowed_environment(agent, values.clone())
            .into_iter()
            .map(|(key, _)| key)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        kept(Agent::Codex),
        ["HOME", "CODEX_HOME", "CODEX_API_KEY", "PATH"]
    );
    assert_eq!(
        kept(Agent::Claude),
        ["HOME", "CLAUDE_CONFIG_DIR", "ANTHROPIC_API_KEY", "PATH"]
    );
    let temp = tempfile::tempdir().unwrap();
    let files = Files::new(temp.path()).unwrap();
    let ask = request(Mode::Ask, temp.path(), &NOT_CANCELLED);
    let command = build("claude", Agent::Claude, &ask, &files, values).unwrap();
    let mut set = command
        .get_envs()
        .map(|(key, _)| key.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    set.sort();
    assert_eq!(
        set,
        ["ANTHROPIC_API_KEY", "CLAUDE_CONFIG_DIR", "HOME", "PATH"]
    );
}
#[test]
fn a_run_clears_the_service_environment_and_keeps_the_prompt_off_the_command_line() {
    let temp = tempfile::tempdir().unwrap();
    let program = fake_agent(temp.path(), "printf '{\"type\":\"turn.started\"}'");
    let scratch = temp.path().join("runtime");
    let previous = std::env::var_os("TRUFFLE_AGENT_TOKEN");
    std::env::set_var("TRUFFLE_AGENT_TOKEN", "service-token-canary");
    let mut run = request(Mode::Ask, &scratch, &NOT_CANCELLED);
    run.prompt = "UNTRUSTED_REFERENCE_MARKER -c developer_instructions=override $(touch marker)";
    let mut diagnostics = json!({});
    let output = run_program(&program, Agent::Codex, &run, &mut diagnostics);
    match previous {
        Some(value) => std::env::set_var("TRUFFLE_AGENT_TOKEN", value),
        None => std::env::remove_var("TRUFFLE_AGENT_TOKEN"),
    }
    assert_eq!(output.unwrap(), "{\"type\":\"turn.started\"}");
    let environment = std::fs::read_to_string(temp.path().join("env")).unwrap();
    assert!(!environment.contains("service-token-canary"));
    assert!(environment.contains("PATH="));
    let argv = std::fs::read_to_string(temp.path().join("argv")).unwrap();
    let developer = argv
        .lines()
        .filter(|line| line.starts_with("developer_instructions="))
        .collect::<Vec<_>>();
    assert_eq!(
        developer,
        ["developer_instructions=\"fixture instructions\""]
    );
    assert!(!argv.contains("UNTRUSTED_REFERENCE_MARKER"));
    assert_eq!(
        std::fs::read_to_string(temp.path().join("stdin")).unwrap(),
        run.prompt
    );
    assert!(!temp.path().join("marker").exists());
    assert_eq!(diagnostics["stage"], "process");
    assert_eq!(std::fs::read_dir(&scratch).unwrap().count(), 0);
}
#[test]
fn the_agent_runs_as_the_leader_of_its_own_process_group() {
    let temp = tempfile::tempdir().unwrap();
    let mut command = Command::new("/bin/sh");
    command.args(["-c", "echo $$ $(/bin/ps -o pgid= -p $$)"]);
    let output = execute(
        command,
        "",
        &request(Mode::Ask, temp.path(), &NOT_CANCELLED),
        &mut json!({}),
    )
    .unwrap();
    let ids = output.split_whitespace().collect::<Vec<_>>();
    assert_eq!(ids.len(), 2, "{output}");
    assert_eq!(ids[0], ids[1]);
}
#[test]
fn cancellation_and_timeout_reap_background_processes() {
    let temp = tempfile::tempdir().unwrap();
    for cancelled in [false, true] {
        let flag = AtomicBool::new(cancelled);
        let mut run = request(Mode::Ask, temp.path(), &flag);
        run.timeout = Duration::from_millis(30);
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "sleep 30 & wait"]);
        let start = Instant::now();
        let mut diagnostics = json!({});
        let failure = execute(command, "", &run, &mut diagnostics).unwrap_err();
        assert!(start.elapsed() < Duration::from_secs(2));
        if cancelled {
            assert!(matches!(failure, Failure::Cancelled));
            assert_eq!(diagnostics["code"], "cancelled");
        } else {
            assert!(matches!(failure, Failure::TimedOut));
            assert_eq!(diagnostics["code"], "timed_out");
        }
    }
}
#[test]
fn process_failure_diagnostics_exclude_stdout_and_stderr_contents() {
    let temp = tempfile::tempdir().unwrap();
    let mut command = Command::new("/bin/sh");
    command.args([
        "-c",
        "printf benign-private-stdout; printf benign-private-stderr >&2; exit 7",
    ]);
    let mut diagnostics = json!({});
    let failure = execute(
        command,
        "",
        &request(Mode::Ask, temp.path(), &NOT_CANCELLED),
        &mut diagnostics,
    )
    .unwrap_err();
    assert!(matches!(failure, Failure::Exited));
    assert_eq!(diagnostics["code"], "process_failed");
    assert_eq!(diagnostics["exitCode"], 7);
    assert_eq!(diagnostics["stdoutBytes"], 21);
    assert_eq!(diagnostics["stderrBytes"], 21);
    assert_eq!(diagnostics["stdoutShape"], "non-json");
    assert!(!diagnostics.to_string().contains("benign-private"));
}
#[test]
fn output_at_the_limit_fails_the_run() {
    let temp = tempfile::tempdir().unwrap();
    let mut run = request(Mode::Ask, temp.path(), &NOT_CANCELLED);
    run.limit = 4;
    let mut command = Command::new("/bin/sh");
    command.args(["-c", "printf 1234"]);
    let mut diagnostics = json!({});
    let failure = execute(command, "", &run, &mut diagnostics).unwrap_err();
    assert!(matches!(failure, Failure::Oversized));
    assert_eq!(diagnostics["code"], "oversized_output");
}
#[test]
fn private_files_are_removed_after_cancellation_and_spawn_failure() {
    let temp = tempfile::tempdir().unwrap();
    let scratch = temp.path().join("runtime");
    let images = images();
    let cancelled = AtomicBool::new(true);
    let program = fake_agent(temp.path(), "sleep 30 & wait");
    for (program, cancel) in [
        (program.as_str(), &cancelled),
        ("/fixture/missing-codex", &NOT_CANCELLED),
    ] {
        let mut run = request(Mode::Ask, &scratch, cancel);
        run.images = &images;
        let failure = run_program(program, Agent::Codex, &run, &mut json!({})).unwrap_err();
        assert!(matches!(failure, Failure::Cancelled | Failure::Spawn));
        assert_eq!(std::fs::read_dir(&scratch).unwrap().count(), 0);
    }
}
