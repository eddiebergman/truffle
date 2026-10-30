//! Agent CLIs already installed on disk keep posting these payloads, so they must not change.
use super::*;
const RECORD: &str = "11111111-1111-4111-8111-111111111111";
const SESSION: &str = "22222222-2222-4222-8222-222222222222";
const REQUEST: &str = "33333333-3333-4333-8333-333333333333";
struct Files {
    _temp: tempfile::TempDir,
    dir: String,
}
impl Files {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let dir = fs::canonicalize(temp.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        for (name, body) in [
            ("args.json", r#"{"title":"From agent"}"#),
            ("patch.json", r#"{"theme":"light"}"#),
            (
                "review.json",
                r#"{"comments":[{"path":"a.rs","line":1}],"overall":{"text":"ok"}}"#,
            ),
            (
                "context.json",
                &format!(r#"{{"revision":"{}","resources":[]}}"#, "a".repeat(64)),
            ),
            (
                "resources.json",
                r#"{"resources":[{"id":"vm-1","state":"running"}]}"#,
            ),
            (
                "replace.json",
                &format!(r#"{{"revision":"{}","resources":[]}}"#, "b".repeat(64)),
            ),
            ("checkpoint.json", r#"{"done":"Tests","next":"Docs"}"#),
            ("briefing.md", "# Brief\n"),
            ("report.md", "# Report\n"),
        ] {
            fs::write(temp.path().join(name), body).unwrap();
        }
        Self { _temp: temp, dir }
    }
    fn path(&self, name: &str) -> String {
        format!("{}/{name}", self.dir)
    }
}
fn cli(command: &str, args: &[&str]) -> Value {
    payload(command, &options_of(args)).unwrap()
}
fn options_of(args: &[&str]) -> Options {
    Options::parse(&args.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>()).unwrap()
}
fn hook(event: &str, data: &Value) -> Value {
    hook_payload(&options_of(&["--record", RECORD, "--event", event]), data)
        .unwrap()
        .unwrap()
}
fn chat_cases(f: &Files) -> Vec<(Value, Value)> {
    let hash = "c".repeat(64);
    let dir = f.dir.as_str();
    vec![
        (
            cli(
                "command",
                &[
                    "--record",
                    RECORD,
                    "--name",
                    "projects.create",
                    "--file",
                    &f.path("args.json"),
                ],
            ),
            json!({"id":RECORD,"operation":"command","name":"projects.create","args":{"title":"From agent"}}),
        ),
        (
            cli("command", &["--record", RECORD, "--name", "commands.list"]),
            json!({"id":RECORD,"operation":"command","name":"commands.list","args":{}}),
        ),
        (
            cli("settings", &["--show"]),
            json!({"operation":"settings-read"}),
        ),
        (
            cli("settings", &["--record", RECORD, "--show"]),
            json!({"id":RECORD,"operation":"settings-read"}),
        ),
        (
            cli(
                "settings",
                &[
                    "--file",
                    &f.path("patch.json"),
                    "--revision",
                    "7",
                    "--content-hash",
                    &hash,
                ],
            ),
            json!({"operation":"settings-apply","patch":{"theme":"light"},"revision":7,"contentHash":hash}),
        ),
        (
            cli(
                "register",
                &[
                    "--agent",
                    "codex",
                    "--session",
                    SESSION,
                    "--record",
                    RECORD,
                    "--title",
                    "Fix it",
                    "--cwd",
                    dir,
                    "--url",
                    "https://example.com/1",
                    "--url",
                    "https://example.com/2",
                    "--item",
                    "note:1",
                    "--project",
                    "Truffle",
                    "--destination",
                    "desktop",
                ],
            ),
            json!({"agent":"codex","sessionId":SESSION,"cwd":dir,"urls":["https://example.com/1","https://example.com/2"],"itemIds":["note:1"],"id":RECORD,"title":"Fix it","project":"Truffle","destination":"desktop"}),
        ),
        (
            cli(
                "register",
                &["--agent", "claude", "--session", SESSION, "--cwd", dir],
            ),
            json!({"agent":"claude","sessionId":SESSION,"cwd":dir,"urls":[],"itemIds":[]}),
        ),
    ]
}
fn report_cases(f: &Files) -> Vec<(Value, Value)> {
    let dir = f.dir.as_str();
    vec![
        (
            cli("review", &["--record", RECORD, "--show"]),
            json!({"id":RECORD,"operation":"review"}),
        ),
        (
            cli(
                "review",
                &[
                    "--record",
                    RECORD,
                    "--request",
                    REQUEST,
                    "--pass",
                    "simplification",
                    "--state",
                    "complete",
                    "--file",
                    &f.path("review.json"),
                    "--reviewed-revision",
                    "3",
                ],
            ),
            json!({"id":RECORD,"operation":"review","requestId":REQUEST,"pass":"simplification","state":"complete","report":{"comments":[{"path":"a.rs","line":1}],"overall":{"text":"ok"}},"reviewedRevision":3}),
        ),
        (
            cli("context", &["--record", RECORD, "--show"]),
            json!({"id":RECORD,"operation":"context-read"}),
        ),
        (
            cli(
                "context",
                &["--record", RECORD, "--file", &f.path("context.json")],
            ),
            json!({"id":RECORD,"operation":"context-replace","context":{"revision":"a".repeat(64),"resources":[]}}),
        ),
        (
            cli(
                "resource",
                &["--record", RECORD, "--file", &f.path("resources.json")],
            ),
            json!({"id":RECORD,"operation":"resources","report":{"resources":[{"id":"vm-1","state":"running"}],"replace":false}}),
        ),
        (
            cli(
                "resource",
                &[
                    "--record",
                    RECORD,
                    "--file",
                    &f.path("replace.json"),
                    "--replace",
                ],
            ),
            json!({"id":RECORD,"operation":"resources","report":{"revision":"b".repeat(64),"resources":[],"replace":true}}),
        ),
        (
            cli(
                "worktree",
                &[
                    "--record",
                    RECORD,
                    "--path",
                    dir,
                    "--url",
                    "https://example.com/1",
                    "--item",
                    "note:1",
                    "--primary",
                ],
            ),
            json!({"id":RECORD,"operation":"worktree","path":dir,"urls":["https://example.com/1"],"itemIds":["note:1"],"primary":true}),
        ),
        (
            cli("worktree", &["--record", RECORD, "--path", dir]),
            json!({"id":RECORD,"operation":"worktree","path":dir,"urls":[],"itemIds":[],"primary":false}),
        ),
    ]
}
fn output_cases(f: &Files) -> Vec<(Value, Value)> {
    vec![
        (
            cli(
                "status",
                &[
                    "--record",
                    RECORD,
                    "--state",
                    "waiting",
                    "--subagents",
                    "2",
                    "--reason",
                    "finished",
                ],
            ),
            json!({"id":RECORD,"operation":"runtime","state":"waiting","subagents":2,"reason":"finished"}),
        ),
        (
            cli("status", &["--record", RECORD, "--state", "working"]),
            json!({"id":RECORD,"operation":"runtime","state":"working"}),
        ),
        (
            cli(
                "ci",
                &[
                    "--record",
                    RECORD,
                    "--item",
                    "github:1",
                    "--request",
                    REQUEST,
                    "--state",
                    "investigating",
                ],
            ),
            json!({"id":RECORD,"operation":"ci-investigation","itemId":"github:1","requestId":REQUEST,"state":"investigating"}),
        ),
        (
            cli(
                "artifact",
                &[
                    "--record",
                    RECORD,
                    "--file",
                    &f.path("report.md"),
                    "--title",
                    "Report",
                    "--skill-output",
                ],
            ),
            json!({"id":RECORD,"operation":"save-artifact","path":f.path("report.md"),"title":"Report","skillOutput":true}),
        ),
        (
            cli(
                "artifact",
                &["--record", RECORD, "--url", "https://example.com/a"],
            ),
            json!({"id":RECORD,"operation":"save-artifact","url":"https://example.com/a"}),
        ),
        (
            cli(
                "checkpoint",
                &["--record", RECORD, "--file", &f.path("checkpoint.json")],
            ),
            json!({"id":RECORD,"operation":"checkpoint","done":"Tests","next":"Docs"}),
        ),
        (
            cli(
                "briefing",
                &["--record", RECORD, "--file", &f.path("briefing.md")],
            ),
            json!({"id":RECORD,"operation":"save-briefing","body":"# Brief\n"}),
        ),
    ]
}
fn hook_cases() -> Vec<(Value, Value)> {
    vec![
        (
            hook(
                "Stop",
                &json!({"session_id":SESSION,"background_tasks":[{"id":"a","type":"shell"}],"session_crons":[]}),
            ),
            json!({"id":RECORD,"operation":"runtime-event","event":"Stop","sessionId":SESSION,"subagentId":"","backgroundCounts":{"shell":1},"scheduledWakeups":0}),
        ),
        (
            hook(
                "SubagentStart",
                &json!({"session_id":SESSION,"agent_id":"child"}),
            ),
            json!({"id":RECORD,"operation":"runtime-event","event":"SubagentStart","sessionId":SESSION,"subagentId":"child"}),
        ),
        (
            hook("PermissionRequest", &json!({})),
            json!({"id":RECORD,"operation":"runtime-event","event":"PermissionRequest","sessionId":"","subagentId":""}),
        ),
    ]
}
#[test]
fn every_cli_payload_keeps_its_wire_format() {
    let files = Files::new();
    let cases = [
        chat_cases(&files),
        report_cases(&files),
        output_cases(&files),
        hook_cases(),
    ];
    for (sent, pinned) in cases.into_iter().flatten() {
        assert_eq!(sent, pinned);
    }
}
#[test]
fn subagent_turn_events_are_not_sent() {
    let data = json!({"agent_id":"child"});
    for event in ["SessionStart", "Stop", "UserPromptSubmit"] {
        assert!(
            hook_payload(&options_of(&["--record", RECORD, "--event", event]), &data)
                .unwrap()
                .is_none()
        );
    }
}
