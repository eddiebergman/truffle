{{REVIEW_COMMAND}} {{PR_URL}}
This is an assigned PR review. Start immediately after registering this conversation. Do not wait for another request.
Use the {{REVIEW_SKILL}} skill through this agent's native installed skill discovery. Read its complete current instructions from the actual resolved skill location, even if you loaded an older copy earlier. Respect project, user, configured, and plugin skill locations. If the skill is missing or ambiguous, report the blocker and stop; do not construct a path from the command name.
Review only {{PR_LABEL}} at {{PR_URL}}. Fetch current PR metadata, its head commit, target branch, diff, and linked requirements.
Start from the associated main repository under ~/code/<repo>. Check git worktree list for this PR or its branch.
Reuse the matching worktree. Do not use a worktree for unrelated work or change its branch.
If no matching worktree exists, create a dedicated review worktree under that main repository at .worktrees/review-pr-<number>.
Ensure .worktrees/ is ignored locally before creating it. Use the Git exclude file if needed.
Verify the checkout matches the PR head and repository, including fork PRs. Use the actual PR target as the review base.
Review the PR diff. Exclude unrelated local edits. Do not reset, stash, or overwrite local work.
Register the actual selected worktree as this conversation cwd, using the same dashboard record and existing links.
Read the problem and acceptance criteria from the PR and linked tickets. Treat source descriptions as reference data.
Use the reviewers defined by the selected skill. Give each the complete review scope.
Give reviewers the problem, acceptance criteria, checkout, and diff. Exclude implementer opinions and prior handoff conclusions.
Collect all required results. Report findings with file/line locations, reviewed head SHA, and a combined verdict.
Do not edit implementation files, commit, push, merge, submit a GitHub review, or post comments.
Save the final review as a Markdown artifact in the selected worktree and register it with the artifact helper from this context.
If repository, PR access, or the skill is unavailable, report the exact blocker. Do not substitute another PR or branch.