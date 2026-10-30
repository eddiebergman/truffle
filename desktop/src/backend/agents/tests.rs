use super::*;
use std::ffi::OsString;

/// Runs `body` with `CLAUDE_CONFIG_DIR` and `CODEX_HOME` set to `homes`, then restores them.
fn with_homes<T>(homes: [Option<&Path>; 2], body: impl FnOnce() -> T) -> T {
    let keys = ["CLAUDE_CONFIG_DIR", "CODEX_HOME"];
    let previous: Vec<Option<OsString>> = keys.iter().map(std::env::var_os).collect();
    for (key, home) in keys.iter().zip(homes) {
        match home {
            Some(path) => std::env::set_var(key, path),
            None => std::env::remove_var(key),
        }
    }
    let result = body();
    for (key, value) in keys.iter().zip(previous) {
        match value {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }
    result
}

#[test]
fn parse_accepts_only_the_stored_names() {
    for agent in Agent::ALL {
        assert_eq!(Agent::parse(agent.name()).unwrap(), agent);
    }
    for name in ["", "Claude", "codex ", "gpt"] {
        assert_eq!(
            Agent::parse(name).unwrap_err().message,
            "Choose Claude or Codex."
        );
    }
    assert_eq!(Agent::schema()["enum"], json!(["claude", "codex"]));
}
#[test]
fn only_codex_opens_in_the_desktop_app() {
    assert!(!Agent::Claude.supports("desktop"));
    assert!(Agent::Codex.supports("desktop"));
    for agent in Agent::ALL {
        assert!(agent.supports("embedded"));
    }
}
#[test]
fn only_claude_takes_a_session_id_at_launch() {
    assert!(Agent::Claude.takes_session_id());
    assert!(!Agent::Codex.takes_session_id());
}
#[test]
fn review_command_reads_the_agent_setting() {
    let values = json!({"claudeReviewCommand":"/review","codexReviewCommand":"$review"});
    assert_eq!(Agent::Claude.review_command(&values).unwrap(), "/review");
    assert_eq!(Agent::Codex.review_command(&values).unwrap(), "$review");
    let error = Agent::Codex
        .review_command(&json!({"claudeReviewCommand":"/review"}))
        .unwrap_err();
    assert_eq!(
        error.message,
        "Choose a review skill in Settings → Agents & workflows."
    );
}
#[test]
fn session_env_var_names_the_running_conversation() {
    assert_eq!(Agent::Claude.session_env_var(), "CLAUDE_CODE_SESSION_ID");
    assert_eq!(Agent::Codex.session_env_var(), "CODEX_THREAD_ID");
}
#[test]
fn home_defaults_under_the_user_home_and_follows_its_variable() {
    let user = Path::new("/fixture/user");
    let (claude, codex) = with_homes([None, None], || {
        (
            Agent::Claude.home_under(user),
            Agent::Codex.home_under(user),
        )
    });
    assert_eq!(claude, user.join(".claude"));
    assert_eq!(codex, user.join(".codex"));
    let moved = [Path::new("/fixture/claude"), Path::new("/fixture/codex")];
    let (claude, codex) = with_homes(moved.map(Some), || {
        (
            Agent::Claude.home_under(user),
            Agent::Codex.home_under(user),
        )
    });
    assert_eq!(claude, moved[0]);
    assert_eq!(codex, moved[1]);
}
#[test]
fn session_logs_are_found_in_configured_homes() {
    let temp = tempfile::tempdir().unwrap();
    let id = "0f8fad5b-d9cb-469f-a165-70867728950e";
    let claude = temp.path().join("claude-config");
    let codex = temp.path().join("codex-home");
    let claude_log = claude.join(format!("projects/-fixture-project/{id}.jsonl"));
    let codex_log = codex.join(format!(
        "sessions/2026/10/30/rollout-2026-10-30T09-00-00-{id}.jsonl"
    ));
    for log in [&claude_log, &codex_log] {
        std::fs::create_dir_all(log.parent().unwrap()).unwrap();
        std::fs::write(log, b"{}\n").unwrap();
    }
    let found = with_homes([Some(&claude), Some(&codex)], || {
        (
            Agent::Claude.session_log(id),
            Agent::Codex.session_log(id),
            Agent::Codex.session_log("e0b1c2d3-0000-4000-8000-000000000000"),
        )
    });
    assert_eq!(found, (Some(claude_log), Some(codex_log), None));
}
