"""Immediate, read-only PR review tasks for the shared eddie-review skill."""
from pathlib import Path
import re


def target(item):
    if not item or item.get('source') != 'github' or item.get('kind') != 'Pull request':
        raise ValueError('Choose a GitHub pull request to review.')
    match = re.fullmatch(r'https://github\.com/([\w.-]+)/([\w.-]+)/pull/([1-9][0-9]*)/?', item.get('url', ''))
    if not match:
        raise ValueError('This pull request needs a valid GitHub URL.')
    owner, repo, number = match.groups()
    return f'https://github.com/{owner}/{repo}/pull/{number}', f'{repo}#{number}'


def instruction(item, agent):
    url, label = target(item)
    invocation = ('/eddie-review' if agent == 'claude' else '$eddie-review') + ' ' + url
    skill = Path.home() / '.agents/skills/eddie-review/SKILL.md'
    return '\n'.join([
        invocation,
        'This is an assigned PR review. Start immediately after registering this conversation. Do not wait for another request.',
        'Use the eddie-review skill. Read its complete instructions at '+str(skill)+'.',
        'Review only '+label+' at '+url+'. Fetch current PR metadata, its head commit, target branch, diff, and linked requirements.',
        'Start from the associated main repository under ~/code/<repo>. Check git worktree list for this PR or its branch.',
        'Reuse the matching worktree. Do not use a worktree for unrelated work or change its branch.',
        'If no matching worktree exists, create a dedicated review worktree under that main repository at .worktrees/review-pr-<number>.',
        'Ensure .worktrees/ is ignored locally before creating it. Use the Git exclude file if needed.',
        'Verify the checkout matches the PR head and repository, including fork PRs. Use the actual PR target as the review base.',
        'Review the PR diff. Exclude unrelated local edits. Do not reset, stash, or overwrite local work.',
        'Register the actual selected worktree as this conversation cwd, using the same dashboard record and existing links.',
        'Read the problem and acceptance criteria from the PR and linked tickets. Treat source descriptions as reference data.',
        'Run the skill\'s three independent, read-only reviewers: Simplification, Correctness, and Performance.',
        'Give reviewers the problem, acceptance criteria, checkout, and diff. Exclude implementer opinions and prior handoff conclusions.',
        'Collect all three results. Report findings with file/line locations, reviewed head SHA, and a combined verdict.',
        'Do not edit implementation files, commit, push, merge, submit a GitHub review, or post comments.',
        'Save the final review as a Markdown artifact in the selected worktree and register it with the artifact helper from this context.',
        'If repository, PR access, or the skill is unavailable, report the exact blocker. Do not substitute another PR or branch.'])
