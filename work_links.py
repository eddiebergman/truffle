"""Read source items by validated provider identity; never fetch user-supplied URLs."""
import json
import os
import re
from urllib.parse import urlsplit

import agents
import sentry_source

GH_FIELDS = '''id title body url number updatedAt state repository { nameWithOwner }
  labels(first: 30) { nodes { name } }'''
PR_FIELDS = GH_FIELDS + ''' isDraft reviewDecision mergeable mergeStateStatus
  commits(last: 1) { nodes { commit { statusCheckRollup { state contexts(first: 100) {
    totalCount nodes { ... on CheckRun { name status conclusion detailsUrl }
    ... on StatusContext { context state targetUrl } }
  } } } } }'''
PARENT_FIELDS = 'id identifier title url'
for _ in range(5):
    PARENT_FIELDS = 'id identifier title url parent { ' + PARENT_FIELDS + ' }'
LINEAR_FIELDS = 'id identifier title description url updatedAt archivedAt state { name type } team { id key name } project { id name } labels { nodes { name } } parent { ' + PARENT_FIELDS + ' } children(first: 50) { nodes { id identifier title url archivedAt state { name type } } pageInfo { hasNextPage } }'

LINEAR_FIELDS += ' attachments(first: 50, filter: { sourceType: { eq: "sentry" } }) { nodes { title url sourceType } }'


def linear_parent(node, api, depth=0):
    if not node or depth > 6:
        return None
    return dict(id='linear:' + node['id'], reference=node['identifier'], title=node['title'], url=api.safe_url(node['url']),
                parent=linear_parent(node.get('parent'), api, depth+1))



def github_item(node, api):
    is_pr = 'isDraft' in node
    status = api.status_for(labels=[x['name'] for x in node['labels']['nodes']])
    ci = api.parse_ci(node) if is_pr else None
    if node['state'] in ('CLOSED', 'MERGED'):
        status = 'done'
    elif is_pr and status != 'blocked':
        match (node['isDraft'], node.get('reviewDecision')):
            case (True, _): status = 'draft'
            case (_, 'APPROVED'):
                status = 'ready-to-merge' if ci['state'] in ('passed', 'none') and node['mergeable'] == 'MERGEABLE' and node['mergeStateStatus'] == 'CLEAN' else 'in-progress'
            case (_, 'CHANGES_REQUESTED'): status = 'in-progress'
            case _: status = 'needs-review'
    native = {'APPROVED': 'Approved', 'CHANGES_REQUESTED': 'Changes requested', 'REVIEW_REQUIRED': 'Awaiting review'}.get(node.get('reviewDecision'), 'Open')
    if status == 'done':
        native = node['state'].title()
    elif node.get('isDraft'):
        native = 'Draft'
    return dict(id='github:' + node['id'], source='github', title=node['title'], detail=(node.get('body') or '')[:10000],
        url=api.safe_url(node['url']), reference=f"#{node['number']}", project=node['repository']['nameWithOwner'],
        status=status, sourceStatus=native, kind='Pull request' if is_pr else 'Issue', relations=['linked'],
        updatedAt=node['updatedAt'], ci=ci, mergeState=node.get('mergeStateStatus'), mergeable=node.get('mergeable'))


def linear_item(node, api):
    return dict(id='linear:' + node['id'], source='linear', title=node['title'], detail=(node.get('description') or '')[:10000],
        url=api.safe_url(node['url']), reference=node['identifier'], project=(node.get('project') or {}).get('name') or 'No project',
        status=api.status_for(node['state']['name'], node['state']['type'], [x['name'] for x in node['labels']['nodes']]),
        sourceStatus='Archived' if node.get('archivedAt') else node['state']['name'], archivedAt=node.get('archivedAt'), kind='Issue', relations=['linked'], updatedAt=node['updatedAt'],
        projectId=(node.get('project') or {}).get('id'), projectIsTeam=not bool(node.get('project')),
        teamKey=node['team'].get('key'), teamName=node['team']['name'], parent=linear_parent(node.get('parent'), api),
        sentryLinks=sentry_links(node.get('attachments', {}).get('nodes', []), api),
        children=[dict(id='linear:'+x['id'], reference=x['identifier'], title=x['title'], url=api.safe_url(x['url']),
                       archivedAt=x.get('archivedAt'), status=api.status_for(x['state']['name'], x['state']['type'])) for x in (node.get('children') or {}).get('nodes', [])],
        moreChildren=(node.get('children') or {}).get('pageInfo', {}).get('hasNextPage', False))


def sentry_links(attachments, api):
    links = {}
    for attachment in attachments:
        if (attachment.get('sourceType') or '').lower() != 'sentry':
            continue
        try:
            url = api.safe_url(attachment.get('url', ''))
        except ValueError:
            continue
        parsed = urlsplit(url)
        host = parsed.hostname or ''
        if (host == 'sentry.io' or host.endswith('.sentry.io')) and re.search(r'/issues/\d+(?:/|$)', parsed.path):
            links[url] = dict(url=url, title=attachment.get('title') or 'Sentry issue')
    return list(links.values())


def resolve(url, api):
    if not isinstance(url, str) or len(url) > 4096:
        raise ValueError('Enter a GitHub, Linear, or Sentry issue URL.')
    parsed = urlsplit(url)
    if parsed.scheme != 'https' or parsed.username or parsed.password or parsed.port not in (None, 443):
        raise ValueError('Use an HTTPS GitHub, Linear, or Sentry issue link.')
    match parsed.hostname:
        case 'github.com':
            match = re.fullmatch(r'/([A-Za-z0-9_.-]+)/([A-Za-z0-9_.-]+)/(issues|pull)/([0-9]+)/?', parsed.path)
            if not match:
                raise ValueError('Use a GitHub issue or pull request URL.')
            owner, repo, kind, number = match.groups()
            field, fields = ('pullRequest', PR_FIELDS) if kind == 'pull' else ('issue', GH_FIELDS)
            query = 'query { repository(owner: OWNER, name: REPO) { FIELD(number: NUMBER) { FIELDS } } }'
            query = query.replace('OWNER', json.dumps(owner)).replace('REPO', json.dumps(repo)).replace('FIELD(', field + '(').replace('NUMBER', number).replace('FIELDS', fields)
            node = (api.github_graphql(query).get('repository') or {}).get(field)
            if not node:
                raise ValueError('GitHub item is unavailable to the current account.')
            return github_item(node, api)
        case 'linear.app':
            match = re.fullmatch(r'/[^/]+/issue/([A-Za-z][A-Za-z0-9]*-[0-9]+)(?:/[^/]*)?/?', parsed.path)
            if not match:
                raise ValueError('Use a Linear issue URL with an identifier such as ENG-123.')
            token = os.environ.get('LINEAR_API_KEY')
            if not token:
                raise ValueError('Configure LINEAR_API_KEY on the truffle server.')
            data = api.request_json('https://api.linear.app/graphql', {'query': 'query { issue(id: ' + json.dumps(match[1]) + ') { ' + LINEAR_FIELDS + ' } }'}, token)
            if not data.get('issue'):
                raise ValueError('Linear issue is unavailable to the current account.')
            return linear_item(data['issue'], api)
        case host if host and (host == 'sentry.io' or host.endswith('.sentry.io')):
            org, _, _, _ = sentry_source.settings()
            if not sentry_source.configured():
                raise ValueError('Configure Sentry on the truffle server first.')
            match = re.fullmatch(r'/(?:organizations/([^/]+)/)?issues/([0-9]+)/?', parsed.path)
            if not match or match[1] and match[1] != org:
                raise ValueError('Use a Sentry issue link from the configured organization.')
            node, _ = sentry_source.get(f'/api/0/organizations/{org}/issues/{match[2]}/', {}, api)
            if not node:
                raise ValueError('Sentry issue is unavailable to the current account.')
            return sentry_source.item(node, org, api)
        case _:
            raise ValueError('Only GitHub, Linear, and Sentry issue links can be imported.')


def refresh(source, references, api):
    """Fetch only linked items missing from the normal current-user search."""
    if source == 'sentry':
        if len(references)>100:
            raise api.SyncError('More than 100 linked Sentry issues need individual refresh. Archive old sessions or restore hidden work.')
        return sentry_source.refresh(references, api)
    result = []
    for offset in range(0, len(references), 100):
        batch = references[offset:offset + 100]
        match source:
            case 'github':
                ids = [x['id'].removeprefix('github:') for x in batch]
                query = 'query { nodes(ids: ' + json.dumps(ids) + ') { ... on PullRequest { ' + PR_FIELDS + ' } ... on Issue { ' + GH_FIELDS + ' } } }'
                result += [github_item(x, api) for x in api.github_graphql(query)['nodes'] if x]
            case 'linear':
                token = os.environ.get('LINEAR_API_KEY')
                if not token:
                    raise api.SyncError('Configure LINEAR_API_KEY to refresh linked issues.')
                ids = [x['id'].removeprefix('linear:') for x in batch]
                query = 'query { issues(first: 100, includeArchived: true, filter: { id: { in: ' + json.dumps(ids) + ' } }) { nodes { ' + LINEAR_FIELDS + ' } } }'
                data = api.request_json('https://api.linear.app/graphql', {'query': query}, token)
                result += [linear_item(x, api) for x in data['issues']['nodes']]
            case _:
                raise ValueError('Unsupported source.')
    found = {x['id'] for x in result}
    result += [{**x, 'sourceUnavailable': True} for x in references if x['id'] not in found]
    return result
