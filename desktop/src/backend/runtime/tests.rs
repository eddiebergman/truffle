use super::*;
fn hook(old: &Value, hook: Hook, subagent: &str) -> Value {
    let background = Background::default();
    let event = Event::Hook {
        hook,
        subagent,
        background: &background,
    };
    next(old, &event, 1.0).unwrap()
}
fn report(state: &str, reason: &str, subagents: Option<u64>) -> Result<Value> {
    let event = Event::Report {
        state,
        reason,
        subagents,
    };
    next(&json!({}), &event, 1.0)
}
#[test]
fn subagent_hooks_count_each_subagent_once_and_keep_the_state() {
    let working = hook(&json!({}), Hook::UserPromptSubmit, "");
    let started = hook(&working, Hook::SubagentStart, "child");
    let started = hook(&started, Hook::SubagentStart, " child ");
    assert_eq!(started["subagents"], 1);
    assert_eq!(started["state"], "working");
    let waiting = hook(&started, Hook::Stop, "");
    assert_eq!(waiting["state"], "background");
    assert!(!is_idle(&waiting));
    let stopped = hook(&waiting, Hook::SubagentStop, "child");
    assert_eq!(stopped["subagents"], 0);
    assert_eq!(stopped["state"], "waiting");
    assert_eq!(stopped["reason"], "finished");
    assert!(is_idle(&stopped));
    let restarted = hook(&started, Hook::SessionStart, "");
    assert_eq!(restarted["subagentIds"], json!([]));
    assert_eq!(restarted["reason"], "ready");
}
#[test]
fn hooks_reject_oversized_subagent_ids() {
    let background = Background::default();
    let event = Event::Hook {
        hook: Hook::SubagentStart,
        subagent: &"x".repeat(201),
        background: &background,
    };
    assert!(next(&json!({}), &event, 1.0).is_err());
}
#[test]
fn a_report_replaces_hook_state_and_unknown_subagents_stay_unknown() {
    let background = Background {
        background_counts: Some(BTreeMap::from([("shell".to_owned(), 1)])),
        scheduled_wakeups: Some(1),
    };
    let event = Event::Hook {
        hook: Hook::Stop,
        subagent: "",
        background: &background,
    };
    let busy = hook(
        &next(&json!({}), &event, 1.0).unwrap(),
        Hook::SubagentStart,
        "child",
    );
    let event = Event::Report {
        state: "waiting",
        reason: "finished",
        subagents: None,
    };
    let reported = next(&busy, &event, 2.0).unwrap();
    assert_eq!(
        reported,
        json!({"state":"waiting","reason":"finished","subagents":null,"at":2.0})
    );
    assert!(is_idle(&reported));
    assert_eq!(
        report(" working ", "", Some(2)).unwrap()["state"],
        "working"
    );
}
#[test]
fn reports_accept_only_known_states_reasons_and_bounded_counts() {
    assert!(report("background", "", None).is_err());
    assert!(report("waiting", "bored", None).is_err());
    assert!(report("waiting", "", Some(1001)).is_err());
    assert!(report("needs-input", "question", Some(1000)).is_ok());
}
#[test]
fn only_a_waiting_agent_without_subagents_is_idle() {
    for (runtime, idle) in [
        (json!({"state":"waiting","subagents":0}), true),
        (json!({"state":"waiting","subagents":null}), true),
        (json!({"state":"waiting","subagents":2}), false),
        (json!({"state":"background","subagents":0}), false),
        (json!({"state":"working"}), false),
        (json!(null), false),
    ] {
        assert_eq!(is_idle(&runtime), idle, "{runtime}");
    }
}
