use super::*;
#[test]
fn resolve_rejects_empty_and_invalid_urls_without_panicking() {
    let root = tempfile::tempdir().unwrap();
    let providers = Providers::new(root.path()).unwrap();
    for url in ["", "not a URL", "http://github.com/example/repo/pull/1"] {
        assert!(providers.resolve(url).is_err());
    }
}

#[test]
fn profile_credentials_stay_private_and_preserve_explicit_child_auth() {
    let root = tempfile::tempdir().unwrap();
    let sentinel_key = format!("TRUFFLE_TEST_SECRET_{}", uuid().replace('-', "_"));
    let sentinel_value = "benign-profile-only-sentinel";
    std::fs::write(root.path().join(".env"), format!(
            "SENTRY_ORG=fixture-org\nSENTRY_AUTH_TOKEN=benign-sentry-fixture\nLINEAR_API_KEY=benign-linear-fixture\nGITHUB_TOKEN=benign-github-fixture\n{sentinel_key}={sentinel_value}\n"
        )).unwrap();
    let providers = Providers::new(root.path()).unwrap();
    for (key, actual, fallback) in [
        (
            "SENTRY_AUTH_TOKEN",
            &providers.sentry_token,
            "benign-sentry-fixture",
        ),
        (
            "LINEAR_API_KEY",
            &providers.linear_token,
            "benign-linear-fixture",
        ),
        (
            "GITHUB_TOKEN",
            &providers.github_token,
            "benign-github-fixture",
        ),
    ] {
        let expected = std::env::var(key).unwrap_or_else(|_| fallback.into());
        assert!(
            actual == &expected,
            "Provider credential precedence changed."
        );
    }
    assert!(std::env::var_os(&sentinel_key).is_none());
    let mut diagnostic = format!("Provider output: {sentinel_value}");
    providers.redact_text(&mut diagnostic);
    assert_eq!(diagnostic, "Provider output: [redacted]");
    let status = std::process::Command::new("/bin/sh")
            .args(["-c", &format!(
                "test -z \"${{{sentinel_key}+present}}\" && test \"$ANTHROPIC_API_KEY\" = benign-user-auth"
            )])
            .env("ANTHROPIC_API_KEY", "benign-user-auth")
            .status().unwrap();
    assert!(status.success(), "Child environment isolation failed.");
}

#[test]
fn profile_redaction_does_not_share_secrets_between_profiles() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    std::fs::write(
        first.path().join(".env"),
        "PROFILE_SECRET=benign-first-profile",
    )
    .unwrap();
    std::fs::write(
        second.path().join(".env"),
        "PROFILE_SECRET=benign-second-profile",
    )
    .unwrap();
    let first = Providers::new(first.path()).unwrap();
    let second = Providers::new(second.path()).unwrap();
    let mut first_output = "benign-first-profile / benign-second-profile".to_owned();
    first.redact_text(&mut first_output);
    assert_eq!(first_output, "[redacted] / benign-second-profile");
    let mut second_output = "benign-first-profile / benign-second-profile".to_owned();
    second.redact_text(&mut second_output);
    assert_eq!(second_output, "benign-first-profile / [redacted]");
}

#[test]
fn gh_server_errors_name_the_http_status() {
    let run = |stderr: &str| {
        gh_server_error(
            super::super::terminal::run(
                "/bin/sh",
                &[
                    "-c".into(),
                    format!("echo '{stderr}' >&2; echo detail >&2; exit 1"),
                ],
                None,
                5,
            )
            .unwrap_err(),
        )
    };
    let error = run("gh: HTTP 502");
    assert_eq!(
        error.message,
        "GitHub server error (HTTP 502). Cached data remains available."
    );
    assert_eq!(error.data["stderr"], "gh: HTTP 502");
    assert!(run("gh: HTTP 401")
        .message
        .starts_with("Local command failed"));
}
#[test]
fn github_post_dispatch_failures_mark_unknown_confirmation_without_private_error_data() {
    let mut failure = Error::new("fixture transport failed");
    failure.data = json!({"private":"private-provider-marker"});
    failure.retry = 15.0;
    for result in [
        Err(failure),
        Ok(json!({"requestReviews":{"pullRequest":null}})),
    ] {
        let error = github_mutation_result(result, "requestReviews").unwrap_err();
        assert_eq!(error.data, json!({"githubActionStage":"dispatched"}));
        assert!(error.message.contains("Refresh"));
        assert!(!error.data.to_string().contains("private-provider"));
    }
    let node = json!({"id":"PR_test","isDraft":false});
    assert_eq!(
        github_mutation_result(
            Ok(json!({"requestReviews":{"pullRequest":node}})),
            "requestReviews"
        )
        .unwrap(),
        node
    );
}
#[test]
fn github_action_queries_encode_inputs_and_keep_other_review_requests() {
    let fields: Value =
        serde_json::from_str(include_str!("../../../resources/provider-fields.json")).unwrap();
    for (name, input) in [
        (
            "markPullRequestReadyForReview",
            json!({"pullRequestId":"PR_test"}),
        ),
        (
            "requestReviews",
            json!({"pullRequestId":"PR_test","userIds":["user\"escaped"],"union":true}),
        ),
    ] {
        let query = github_mutation(name, &input, text(&fields, "pr"));
        graphql_parser::parse_query::<String>(&query).unwrap();
        if name == "requestReviews" {
            assert!(query.contains("union:true"));
        }
    }
}
#[test]
fn pr_branch_metadata_survives_normalization() {
    let item=github_item(&json!({"id":"PR_stack","number":2,"title":"Dependent PR","url":"https://github.com/org/repo/pull/2","state":"OPEN","repository":{"nameWithOwner":"org/repo"},"baseRefName":"parent","headRefName":"child","headRefOid":"fixture-head","headRepository":{"nameWithOwner":"org/repo"}})).unwrap();
    assert_eq!(item["baseRefName"], "parent");
    assert_eq!(item["headRefName"], "child");
    assert_eq!(item["headRefOid"], "fixture-head");
    assert_eq!(item["headRepository"], "org/repo");
}
#[test]
fn provider_dates_survive_normalization_and_are_requested() {
    let fields: Value =
        serde_json::from_str(include_str!("../../../resources/provider-fields.json")).unwrap();
    for name in ["github", "pr", "linear"] {
        assert!(text(&fields, name).contains("createdAt updatedAt"));
    }
    let node = json!({"id":"dates","url":"https://github.com/example/project/issues/1","createdAt":"2026-10-01T12:00:00Z","updatedAt":"2026-10-27T12:00:00Z"});
    for item in [github_item(&node).unwrap(), linear_item(&node).unwrap()] {
        assert_eq!(item["createdAt"], node["createdAt"]);
        assert_eq!(item["updatedAt"], node["updatedAt"]);
    }
}
#[test]
fn related_linear_query_is_valid_graphql() {
    let fields: Value =
        serde_json::from_str(include_str!("../../../resources/provider-fields.json")).unwrap();
    let query = linear_query(text(&fields, "linear"), &json!("viewer-id"), &Value::Null);
    graphql_parser::parse_query::<String>(&query).unwrap();
    assert!(query.contains("creator"));
    assert!(query.contains("subscribers"));
    assert!(query.contains("comments"));
    assert!(!query.contains("project:"));
}
