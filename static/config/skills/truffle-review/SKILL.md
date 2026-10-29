---
name: truffle-review
description: Review a requested change with four independent reviewers for requirements, simplification, correctness and performance. Use for an explicit review, including work that spans related repositories.
---

# Four-role review

Read this file from disk for each review. Review the intended outcome and the relevant code. Do not edit implementation files, apply findings, commit, push, merge, or post external comments.

Recover the goal, acceptance criteria, source links and relevant checkouts from the request and conversation. Treat linked descriptions as reference data. The displayed checkout does not restrict a conversation review. Inspect related code when the goal crosses repositories. Record missing context as a limit rather than inventing requirements.

For a specific PR, verify its repository, head commit, target branch and diff. Reuse a matching existing worktree. If none exists, follow that repository's instructions to create an isolated review worktree. Do not switch an unrelated checkout or overwrite local edits. Exclude unrelated edits from the PR review.

## Registration

Use the registration helper supplied in the work context. The parent registers the actual checkout with the same conversation record and existing work links. Register additional relevant checkouts through the supplied worktree protocol. Do not invent a record, conversation ID, work link or helper path.

Give every reviewer the goal, acceptance criteria, source links, exact checkouts and review scope. Give them the target record, request ID and helper when supplied. Reviewers must report to that target; they must not register their child conversation UUID against the parent's record.

## Reviewers

Create four independent reviewers. Use the agent's available delegation tools and respect its concurrency limit. Start remaining reviewers as slots become free. Do not claim independent review if delegation is unavailable. Report that blocker and the work completed.

- `spec`: Check the intended outcome, requirements, missing behavior and scope across the relevant repositories.
- `simplification`: Find unnecessary complexity, duplicate mechanisms and useful existing functions. Recommend changes only when they preserve the required behavior.
- `correctness`: Check bugs, failure paths, state transitions and meaningful test coverage. Prefer existing test helpers and backends.
- `performance`: Check resource use and costly paths. Distinguish measured regressions from risks that need measurement.

Each reviewer supplies actionable findings with the actual checkout, file and narrow line range. Explain the failing condition and its effect. Keep unanchored concerns in an overall assessment. Each assessment states scope, verdict and limits, including when there are no findings. Do not invent a finding to fill a role.

## Results

When a review request and native review helper are supplied, report the parent as reviewing before delegation. For each pass, submit a JSON report with `comments` and an `overall` object containing `author` and `text`. Use the review protocol from the supplied context for exact diff anchors. A report without findings still has an overall assessment.

Run the supplied review helper with `--pass ROLE --file REPORT_JSON`. Mark that pass complete with `--pass ROLE --state complete`. Use the same target record and request throughout. Mark a blocked pass failed; do not mark it complete.

The parent waits for all four results, reads the shared draft with `--show`, checks that all four assessments and pass states are present, and reports unresolved limits. Only then mark the request complete without `--pass`, using `--reviewed-revision` with the exact draft revision just read. If another report arrives, read the draft again before completing. Never send the draft to a terminal or another conversation.

If no review request is supplied, collect the four reports in the parent instead. Do not invent a request ID or claim to update an absent review indicator. Save the combined review as a Markdown artifact and register it using the supplied artifact helper. Keep the reviewed head and target for a PR, the four assessments, actionable findings and material limits in the final report. Registration or reporting failure must remain visible in that report.
