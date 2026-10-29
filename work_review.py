"""Private cached evidence and a read-only brief for an agent work review."""
import json
import time
import sentry_source
import dashboard_features as features

LINK_INSTRUCTION = """Make every Linear ticket ID and GitHub PR/issue reference a Markdown link to its verified source URL.
This applies to every mention, including bold text, the top three actions, repeated mentions, and table cells.
Use repository-qualified GitHub labels such as [api#1419](https://github.com/acme/api/pull/1419).
Use [ENG-1211](https://linear.app/acme/issue/ENG-1211) for Linear tickets.
Keep each action self-contained. A link elsewhere in the report does not replace a link in the action.
Resolve URLs from the snapshot or live source. Never invent a URL or guess which repository owns a bare PR number.
Before saving the report, check prose and lists for unlinked work references. Code examples can remain literal.
If a reference cannot be verified, state that explicitly instead of presenting it as a verified action."""

INSTRUCTION = '''Investigate my work across GitHub, Linear, Sentry, and local projects. Produce a concise action report.
Start the investigation now. Do not wait for another task description after registration.
The snapshot can be large. Read it with short scripts to count, group, and filter. Do not print the entire JSON into your context.
Select candidate records, then check their source evidence.
Use the attached cache as a starting point. Check live source records before you make recommendations.
Use available read-only provider tools or the authenticated GitHub CLI. Report any missing access.
Do not change tickets, PRs, comments, branches, dashboard statuses, or project files.
Register only this review conversation with truffle. Do not attach every reviewed item to this conversation.
Do not create a local project for this review. Existing work sessions must keep their associations.

Check these categories:
1. Likely finished: open tickets with merged implementation PRs or completed sub-issues.
Check acceptance criteria, other required PRs, rollout, and verification before recommending closure.
A merged PR alone does not prove the whole ticket is complete.
2. Waiting on others: review requests, unanswered questions, and unresolved dependencies.
Measure time since the relevant human action. Do not treat bot updates as a fresh review response.
Distinguish my authored PRs from PRs that request my review.
3. Needs my action: requested reviews, failed CI, merge conflicts, requested changes, and ready-to-merge PRs.
Check the current head commit and required checks. Do not infer merge readiness from one successful check.
4. Sentry follow-up: prioritize recurring or high-impact errors, regressions, and gaps in Linear tracking.
Compare linked errors and tickets that may now be obsolete. Group related errors instead of listing every issue.
Read-only Sentry credentials may be available in ~/.sentryclirc or the server configuration. Never print credentials.
Check live Sentry evidence when access exists. Otherwise state that only Linear tracking status is available.
Do not assume that every Sentry error creates a Linear ticket automatically.
5. Tracking gaps: source statuses that disagree, resolved blockers, duplicate work, and inaccessible source links.
Check parent/sub-issue relationships. Distinguish source statuses from dashboard overrides.
Do not recommend work solely because a ticket is old, in Triage, or outside a project.

Group related tickets and PRs into one finding. Start with the three most useful next actions.
For each finding, give source links, current evidence, elapsed time when relevant, and a concrete suggested action.
Separate confirmed facts from uncertain closure candidates. Say what verification remains.
Keep low-priority maintenance updates brief. Omit empty categories.
Hidden, snoozed, and archived items are background context. Do not promote them unless they affect active work.
State the review time and any access or coverage limits. Do not invent evidence when a source is unavailable.
The snapshot does not contain chat histories. Do not claim to know an agent's unrecorded progress.
''' + '\n' + LINK_INSTRUCTION


def snapshot(state):
    catalog = {**{x['id']: x for s in state['sessions'] for x in s['items']},
               **state['itemArchive'], **{x['id']: x for x in state['items'] + state['notes']}}
    ids = list(dict.fromkeys([x['id'] for x in state['items'] + state['notes']] +
               [x['id'] for s in state['sessions'] if not s.get('archivedAt')
                for x in s['items']]))
    fields = ('id', 'source', 'title', 'reference', 'url', 'project', 'projectId', 'teamName',
              'status', 'sourceStatus', 'firstSeen', 'lastSeen', 'eventCount', 'userCount', 'level', 'priority', 'category', 'archivedAt', 'sourceUnavailable', 'updatedAt',
              'relations', 'sentryLinks', 'parent', 'children', 'moreChildren', 'mergeState', 'mergeable')
    items, budget = [], 30000
    for key in ids:
        item = catalog[key]
        row = {k: item[k] for k in fields if k in item}
        override = state['overrides'].get(key, {})
        row.update(dashboardStatus=override.get('status') or item.get('status'),
                   hidden=bool(override.get('hidden') or item.get('hidden')), snoozed=features.snoozed(state,item))
        detail = (item.get('detail') or '')[:min(1500, budget)]
        budget -= len(detail)
        if detail:
            row['detail'] = detail
        if item.get('ci'):
            row['ci'] = {k: item['ci'][k] for k in ('state', 'total', 'summary', 'headSha') if k in item['ci']}
        items.append(row)
    sessions = [{k: s.get(k) for k in ('id', 'title', 'agent', 'cwd', 'archivedAt', 'hidden', 'updatedAt', 'checkpoint')} |
                {'itemIds': [x['id'] for x in s['items']]}
                for s in state['sessions'] if s.get('workflow') != 'work-review']
    coverage = None
    if sentry_source.configured():
        org, _, _, period = sentry_source.settings()
        coverage = dict(org=org, period=period)
    return json.loads(json.dumps(dict(bundles=features.relationships(state), recentChanges=state.get('changes',[])[:50], sentryScope=coverage, capturedAt=time.time(), sources=state['sources'], items=items, sessions=sessions,
                scope='All cached work, independent of browser filters. Fetch current evidence from the sources. Sentry contains unresolved errors seen within the configured rolling window, not all historical errors.')))
