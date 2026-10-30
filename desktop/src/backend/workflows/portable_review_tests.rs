use super::*;
use chat_preparation::Kind;
#[test]
fn custom_review_uses_native_discovery_and_preserves_editable_legacy_templates() {
    for (agent, command) in [("claude", "/custom-review"), ("codex", "$custom-review")] {
        let temp = tempfile::tempdir().unwrap();
        let repo = Repository::open(temp.path().into(), None, true).unwrap();
        repo.settings
            .apply(
                &json!({"revision":0,"contentHash":repo.settings.view()["contentHash"],"patch":{
                    "claudeReviewCommand":"/custom-review","codexReviewCommand":"$custom-review"
                }}),
                false,
            )
            .unwrap();
        let path = repo
            .settings
            .config_directory()
            .join(format!("workflows/pr-review-{agent}.md"));
        let template = "{{REVIEW_COMMAND}} {{PR_URL}}\nUse the {{REVIEW_SKILL}} skill. Read its complete instructions at {{HOME}}/.agents/skills{{REVIEW_COMMAND}}/SKILL.md.\nCustom instruction: read /team/reviews/current/SKILL.md; preserve my review scope.\nReview {{PR_LABEL}}.";
        std::fs::write(&path, template).unwrap();
        repo.transaction(|state| {
                state["items"] = json!([{"id":"github:fixture","source":"github","kind":"Pull request","title":"Review fixture","reference":"#1","url":"https://github.com/example/project/pull/1","status":"todo"}]);
                state["itemSettings"]["github:fixture"] = json!({"explicit":true,"cwd":temp.path()});
                Ok(())
            }).unwrap();
        let record = chat_preparation::save(
            &repo,
            Kind::PrReview,
            &json!({"agent":agent,"itemId":"github:fixture"}),
        )
        .unwrap();
        let prompt = text(&record, "instruction");
        assert!(prompt.starts_with(&format!(
            "{command} https://github.com/example/project/pull/1"
        )));
        assert!(prompt.contains("native installed skill discovery"));
        assert!(
            prompt.contains("If the skill is missing or ambiguous, report the blocker and stop")
        );
        assert!(prompt.contains(
            "Custom instruction: read /team/reviews/current/SKILL.md; preserve my review scope."
        ));
        assert!(!prompt.contains("/.agents/skills"));
        assert!(!prompt.contains("{{REVIEW_"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), template);

        let agent_path = repo
            .settings
            .config_directory()
            .join("workflows/agent-review.md");
        let legacy = "{{REVIEW_COMMAND}}\nRead the current {{SKILL_NAME}} SKILL.md from disk at ~/.claude/skills/{{SKILL_NAME}}/SKILL.md (Claude) or ~/.agents/skills/{{SKILL_NAME}}/SKILL.md (Codex), even if you loaded an older copy earlier. Follow its truffle section when present.\nCustom instruction: read /team/reviews/current/SKILL.md.";
        std::fs::write(&agent_path, legacy).unwrap();
        let settings = repo.settings.view();
        let served = text(&settings["workflows"], "agent-review");
        assert!(served.contains(
            "Resolve {{REVIEW_COMMAND}} through this agent's native installed skill discovery"
        ));
        assert!(served.contains("Follow its truffle section when present."));
        assert!(served.contains("Custom instruction: read /team/reviews/current/SKILL.md."));
        assert!(!served.contains("~/.agents"));
        assert_eq!(std::fs::read_to_string(&agent_path).unwrap(), legacy);
    }
    let custom = "Read ~/.agents/skills/local/SKILL.md and /team/SKILL.md exactly as configured.";
    assert_eq!(review_skill_discovery(custom), custom);
}
#[test]
fn builtin_pr_review_reads_profile_skill_for_either_agent() {
    for (agent, command) in [("claude", "/truffle-review"), ("codex", "$truffle-review")] {
        let temp = tempfile::tempdir().unwrap();
        let repo = Repository::open(temp.path().into(), None, true).unwrap();
        repo.settings
            .apply(
                &json!({"revision":0,"contentHash":repo.settings.view()["contentHash"],"patch":{
                    "claudeReviewCommand":"/truffle-review","codexReviewCommand":"$truffle-review"
                }}),
                false,
            )
            .unwrap();
        repo.transaction(|state| {
                state["items"] = json!([{"id":"github:fixture","source":"github","kind":"Pull request","title":"Review fixture","reference":"#1","url":"https://github.com/example/project/pull/1","status":"todo"}]);
                state["itemSettings"]["github:fixture"] = json!({"explicit":true,"cwd":temp.path()});
                Ok(())
            }).unwrap();
        let record = chat_preparation::save(
            &repo,
            Kind::PrReview,
            &json!({"agent":agent,"itemId":"github:fixture"}),
        )
        .unwrap();
        let prompt = text(&record, "instruction");
        assert!(prompt.contains(
            &repo
                .settings
                .config_directory()
                .join("skills/truffle-review/SKILL.md")
                .to_string_lossy()
                .to_string()
        ));
        assert!(prompt.contains("https://github.com/example/project/pull/1"));
        assert!(prompt.contains("No review request ID was supplied"));
        assert!(!prompt.starts_with(command));
        assert!(!prompt.contains("/.agents/skills"));
        assert!(!prompt.contains("/.claude/skills"));
    }
}
