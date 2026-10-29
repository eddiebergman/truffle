#!/usr/bin/env python3
"""A loopback-only, dependency-free work dashboard. Python 3.11+."""
from __future__ import annotations

import argparse
import fcntl
from concurrent.futures import ThreadPoolExecutor, wait
import json
import os
from pathlib import Path
import secrets
import shlex
import subprocess
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.error import HTTPError, URLError
from urllib.parse import urlsplit
from urllib.request import Request, urlopen
import uuid
import webbrowser
import sys

import work_links
import work_review
import pr_review
import newsletters
import dashboard_features as features
import sentry_source
import sync_diagnostics

import agents
import agent_inbox
import locations

ROOT = Path(__file__).resolve().parent
STATUSES = ('todo', 'in-progress', 'needs-review', 'ready-to-merge', 'draft', 'blocked', 'done')
INTERVAL = 300
MAX_BODY = 128 * 1024


def load_env(path):
    if path.exists():
        for line in path.read_text().splitlines():
            if not line.strip() or line.lstrip().startswith('#'):
                continue
            key, sep, value = line.partition('=')
            if sep and key.strip() in ('LINEAR_API_KEY', 'GITHUB_TOKEN', 'SENTRY_ORG', 'SENTRY_AUTH_TOKEN', 'SENTRY_URL', 'SENTRY_STATS_PERIOD'):
                parts = shlex.split(value, comments=True)
                if parts:
                    os.environ.setdefault(key.strip(), parts[0])


def safe_url(value):
    if not isinstance(value, str) or len(value) > 4096:
        raise ValueError('Link must be a URL of at most 4096 characters.')
    if not value:
        return ''
    parsed = urlsplit(value)
    if parsed.scheme not in ('https', 'http') or not parsed.hostname or parsed.username or parsed.password:
        raise ValueError('Use an http:// or https:// link without credentials.')
    return value


def status_for(name='', kind='', labels=()):
    if kind in ('completed', 'canceled', 'duplicate'):
        return 'done'
    text = ' '.join([name, *labels]).lower()
    if any(x in text for x in ('blocked', 'on hold', 'on-hold')):
        return 'blocked'
    if 'review' in text:
        return 'needs-review'
    if 'draft' in text:
        return 'draft'
    match kind:
        case 'completed' | 'canceled' | 'duplicate': return 'done'
        case 'started': return 'in-progress'
        case _: return 'todo'


class SyncError(Exception):
    def __init__(self, message, retry_after=0, diagnostic=None):
        super().__init__(message)
        self.retry_after = retry_after
        self.diagnostic = diagnostic or {}


def request_json(url, payload, token):
    req = Request(url, data=json.dumps(payload).encode(), headers={
        'Authorization': token, 'Content-Type': 'application/json', 'User-Agent': 'truffle-local/1'})
    started=time.monotonic()
    diagnostic=dict(transport='http',host=urlsplit(url).hostname,phase='connect',timeoutSeconds=25)
    try:
        with urlopen(req, timeout=25) as response:
            diagnostic.update(phase='read',httpStatus=response.status)
            result = json.load(response)
    except HTTPError as exc:
        retry = exc.headers.get('Retry-After', '0')
        reset = exc.headers.get('X-RateLimit-Reset', '0')
        retry_after = max(int(retry) if retry.isdigit() else 0,
                          int(reset) - int(time.time()) if reset.isdigit() else 0)
        diagnostic.update(kind='http',httpStatus=exc.code,requestMs=round((time.monotonic()-started)*1000))
        raise SyncError(f'API returned HTTP {exc.code}. Check credentials and API limits.', retry_after,diagnostic) from None
    except (URLError, TimeoutError, OSError) as exc:
        diagnostic.update(sync_diagnostics.network_fields(exc),requestMs=round((time.monotonic()-started)*1000))
        raise SyncError('Cannot reach the API. Cached items are still available.',diagnostic=diagnostic) from None
    except (json.JSONDecodeError,UnicodeDecodeError):
        diagnostic.update(kind='invalid-json',requestMs=round((time.monotonic()-started)*1000))
        raise SyncError('API returned an invalid response. Cached items are still available.',diagnostic=diagnostic) from None
    if result.get('errors'):
        diagnostic.update(kind='graphql',requestMs=round((time.monotonic()-started)*1000))
        raise SyncError('API query failed. Check token permissions or API compatibility.',diagnostic=diagnostic)
    return result['data']


def github_graphql(query):
    token = os.environ.get('GITHUB_TOKEN')
    if token:
        return request_json('https://api.github.com/graphql', {'query': query}, f'Bearer {token}')
    started=time.monotonic()
    diagnostic=dict(transport='gh-cli',host='api.github.com',phase='request',timeoutSeconds=35)
    try:
        result = subprocess.run(['gh', 'api', 'graphql', '--input', '-'],
            input=json.dumps({'query': query}), capture_output=True, text=True, timeout=35)
    except FileNotFoundError:
        raise SyncError('Install GitHub CLI and run gh auth login, or set GITHUB_TOKEN in .env.',diagnostic={**diagnostic,'kind':'not-installed'}) from None
    except subprocess.TimeoutExpired:
        diagnostic.update(kind='timeout',reasonType='TimeoutExpired',requestMs=round((time.monotonic()-started)*1000))
        raise SyncError('GitHub timed out. Cached items are still available.',diagnostic=diagnostic) from None
    except OSError as exc:
        diagnostic.update(kind='local-execution',errno=exc.errno)
        raise SyncError('Cannot start GitHub CLI. Cached items are still available.',diagnostic=diagnostic) from None
    diagnostic['requestMs']=round((time.monotonic()-started)*1000)
    if result.returncode:
        diagnostic.update(sync_diagnostics.cli_failure_fields(result.stderr),exitCode=result.returncode)
        raise SyncError('GitHub connection failed. Check network access and account permissions.',300,diagnostic)
    try:data = json.loads(result.stdout)
    except (ValueError,TypeError):
        raise SyncError('GitHub returned an invalid response. Cached items are still available.',diagnostic={**diagnostic,'kind':'invalid-json'}) from None
    if data.get('errors'):
        raise SyncError('GitHub query failed. Check account access and API limits.',300,{**diagnostic,'kind':'graphql'})
    return data['data']


def parse_ci(node):
    commits = (node.get('commits') or {}).get('nodes', [])
    rollup = commits[-1]['commit'].get('statusCheckRollup') if commits else None
    if not rollup:
        return {'state': 'none', 'checks': [], 'total': 0}
    match rollup['state']:
        case 'SUCCESS': state = 'passed'
        case 'FAILURE' | 'ERROR': state = 'failed'
        case 'PENDING' | 'EXPECTED': state = 'pending'
        case _: state = 'unknown'
    checks = []
    for check in rollup['contexts']['nodes']:
        if not check:
            continue
        if 'context' in check:
            name, raw, url = check['context'], check['state'], check.get('targetUrl')
        else:
            name = check['name']
            raw = check.get('conclusion') if check.get('status') == 'COMPLETED' else check.get('status')
            url = check.get('detailsUrl')
        match raw:
            case 'SUCCESS': result = 'passed'
            case 'NEUTRAL' | 'SKIPPED': result = 'skipped'
            case 'FAILURE' | 'ERROR' | 'TIMED_OUT' | 'CANCELLED' | 'ACTION_REQUIRED' | 'STARTUP_FAILURE' | 'STALE': result = 'failed'
            case 'PENDING' | 'EXPECTED' | 'QUEUED' | 'IN_PROGRESS' | 'WAITING' | 'REQUESTED': result = 'pending'
            case _: result = 'unknown'
        checks.append({'name': name, 'state': result, 'result': raw or 'UNKNOWN', 'url': safe_url(url or '')})
    return {'state': state, 'checks': checks, 'total': rollup['contexts']['totalCount']}


def fetch_github():
    login = github_graphql('{ viewer { login } }')['viewer']['login']
    # Each search paginates independently; cap work and report truncation instead of hiding it.
    searches = [('authored', f'is:open is:pr author:{login}'),
                ('assigned', f'is:open is:pr assignee:{login}'),
                ('reviews', f'is:open is:pr review-requested:{login}'),
                ('issues', f'is:open is:issue assignee:{login}')]
    items = {}
    for relation, search in searches:
        cursor = None
        for page in range(10):
            query = '''query { search(type: ISSUE, first: 100, query: SEARCH, after: CURSOR) {
              pageInfo { hasNextPage endCursor }
              nodes {
                ... on PullRequest { id title body url number updatedAt isDraft reviewDecision
                  repository { nameWithOwner } labels(first: 30) { nodes { name } }
                  mergeable mergeStateStatus
                  commits(last: 1) { nodes { commit { statusCheckRollup {
                    state contexts(first: 100) { totalCount nodes {
                      ... on CheckRun { name status conclusion detailsUrl }
                      ... on StatusContext { context state targetUrl }
                    } }
                  } } } } }
                ... on Issue { id title body url number updatedAt
                  repository { nameWithOwner } labels(first: 30) { nodes { name } } }
              } } }'''.replace('SEARCH', json.dumps(search)).replace('CURSOR', json.dumps(cursor))
            try:
                data = github_graphql(query)['search']
            except SyncError as exc:
                exc.diagnostic.update(scope=relation,page=page+1)
                raise
            for node in data['nodes']:
                if not node:
                    continue
                key = 'github:' + node['id']
                labels = [x['name'] for x in node['labels']['nodes']]
                is_pr = 'isDraft' in node
                status = status_for(labels=labels)
                if status != 'blocked' and is_pr:
                    match (node['isDraft'], node.get('reviewDecision')):
                        case (True, _): status = 'draft'
                        case (_, 'APPROVED'): status = 'in-progress'
                        case (_, 'CHANGES_REQUESTED'): status = 'in-progress'
                        case _: status = 'needs-review'
                ci = parse_ci(node) if is_pr else None
                if (is_pr and status != 'blocked' and not node['isDraft']
                        and node.get('reviewDecision') == 'APPROVED'
                        and ci['state'] in ('passed', 'none')
                        and node.get('mergeable') == 'MERGEABLE'
                        and node.get('mergeStateStatus') == 'CLEAN'):
                    status = 'ready-to-merge'
                review = {'APPROVED': 'Approved', 'CHANGES_REQUESTED': 'Changes requested',
                          'REVIEW_REQUIRED': 'Awaiting review'}.get(node.get('reviewDecision'), '')
                if key not in items:
                    items[key] = dict(id=key, source='github', title=node['title'], url=safe_url(node['url']),
                        reference=f"#{node['number']}", project=node['repository']['nameWithOwner'],
                        detail=(node.get('body') or '')[:10000], status=status, sourceStatus='Draft' if node.get('isDraft') else review or 'Open',
                        updatedAt=node['updatedAt'], kind='Pull request' if is_pr else 'Issue', relations=[],
                        ci=ci, mergeState=node.get('mergeStateStatus'), mergeable=node.get('mergeable'))
                items[key]['relations'].append(relation)
            if not data['pageInfo']['hasNextPage']:
                break
            if page == 9:
                raise SyncError('More than 1,000 results in a GitHub search. Narrow account scope before syncing.')
            cursor = data['pageInfo']['endCursor']
    return list(items.values())


def fetch_linear():
    token = os.environ.get('LINEAR_API_KEY')
    if not token:
        raise SyncError('Add LINEAR_API_KEY to .env, then restart the server.')
    viewer_id = request_json('https://api.linear.app/graphql', {'query': 'query { viewer { id } }'}, token)['viewer']['id']
    user = json.dumps(viewer_id)
    related = ('{ assignee: { id: { eq: ' + user + ' } } },'
               '{ creator: { id: { eq: ' + user + ' } } },'
               '{ subscribers: { some: { id: { eq: ' + user + ' } } } },'
               '{ comments: { some: { user: { id: { eq: ' + user + ' } } } } },'
               '{ attachments: { some: { sourceType: { eq: "sentry" } } } }')
    relationship_fields = (' assignee { id } creator { id }'
        ' subscribers(first: 1, filter: { id: { eq: ' + user + ' } }) { nodes { id } }'
        ' comments(first: 1, filter: { user: { id: { eq: ' + user + ' } } }) { nodes { id } }')
    items, cursor = {}, None
    for page in range(20):
        query = ('query { issues(first: 100, after: ' + json.dumps(cursor) +
                 ', filter: { or: [' + related + '], state: { type: { nin: ["completed", "canceled", "duplicate"] } } })'
                 ' { pageInfo { hasNextPage endCursor } nodes { ' + work_links.LINEAR_FIELDS + relationship_fields + ' } } }')
        data = request_json('https://api.linear.app/graphql', {'query': query}, token)['issues']
        for node in data['nodes']:
            item = work_links.linear_item(node, sys.modules[__name__])
            item['relations'] = [name for name, matches in (
                ('assigned', (node.get('assignee') or {}).get('id') == viewer_id),
                ('created', (node.get('creator') or {}).get('id') == viewer_id),
                ('subscribed', bool(node['subscribers']['nodes'])),
                ('commented', bool(node['comments']['nodes'])),
                ('sentry', bool(item.get('sentryLinks')))) if matches]
            items[item['id']] = item
        if not data['pageInfo']['hasNextPage']:
            return list(items.values())
        cursor = data['pageInfo']['endCursor']
    raise SyncError('More than 2,000 related active Linear issues. Narrow the query before syncing.')


def item_ref(item):
    return {key: item.get(key, '') for key in
            ('id', 'source', 'title', 'reference', 'url', 'project', 'status', 'sourceUnavailable', 'projectId', 'parent', 'archivedAt', 'sourceStatus', 'sentryLinks')}


class Store:
    def __init__(self, path, demo=False):
        self.path = path
        path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        self.file_lock = open(path.with_suffix('.lock'), 'a')
        try:
            fcntl.flock(self.file_lock.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            self.file_lock.close()
            raise ValueError('Another truffle process is using this data file.') from None
        self.lock = threading.RLock()
        self.force_refresh_after = 0
        self.review_started_at = -100
        self.diagnostics = sync_diagnostics.DiagnosticLog(path.parent)
        self.running = set()
        self.quick_lock = threading.Lock()
        self.quick_started_at = -100
        self.ticket_lock = threading.Lock()
        self.ticket_requests = []
        self.demo = demo
        self.state = {'version': 1, 'items': [], 'notes': [], 'overrides': {}, 'sources': {}}
        if path.exists():
            self.state = json.loads(path.read_text())
            if self.state.get('version') != 1:
                raise ValueError('Unsupported data version. Restore a version 1 backup.')
        elif demo:
            self.seed_demo()
        self.state.setdefault('sessions', [])
        features.initialize(self.state)
        newsletters.initialize(self.state)
        self.newsletter_started = {}
        self.pr_review_started = {}
        self.state.setdefault('itemSettings', {})
        self.state.setdefault('itemArchive', {})
        self.state.setdefault('folderMappings', {})
        self.state.setdefault('linearLinks', {})
        for session in self.state['sessions']:
            for item in session['items']:
                if item.get('detail'):
                    self.state['itemArchive'].setdefault(item['id'], item)
            session['items'] = [item_ref(x) for x in session['items']]
        # Migrate old local overrides into canonical project statuses once.
        for project in self.state['notes']:
            override = self.state['overrides'].get(project['id'], {})
            if override.get('status'):
                project['status'] = override.pop('status')
        for value in self.state['sentryEnvironments'].values():
            value['running'] = False
        self.gateway = agents.Gateway(path.parent, demo)
        for source in ('github', 'linear', 'sentry'):
            self.state['sources'].setdefault(source, {'lastSuccess': None, 'nextAttempt': 0,
                'error': None, 'failures': 0})
        self.state['sources']['sentry']['disabled'] = not sentry_source.configured()
        linear = self.state['sources']['linear']
        if not linear.get('hierarchyVersion'):
            linear['hierarchyVersion'] = 1
            if not linear['failures']:
                linear['nextAttempt'] = 0
        if linear.get('relationshipScopeVersion') != 2:
            linear['relationshipScopeVersion'] = 2
            if not linear['failures']:
                linear['nextAttempt'] = 0

    def save(self):
        self.path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        temp = self.path.with_suffix('.tmp')
        fd = os.open(temp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
        with os.fdopen(fd, 'w') as output:
            json.dump(self.state, output, indent=2)
            output.flush()
            os.fsync(output.fileno())
        os.replace(temp, self.path)

    def snapshot(self, export=False):
        with self.lock:
            data = json.loads(json.dumps({k: v for k, v in self.state.items() if export or k not in ('itemArchive','changeBaselines')}))
            current = {x['id']: x for x in data['items'] + data['notes']}
            for session in data['sessions']:
                session['items'] = [item_ref(current.get(x['id'], x)) for x in session['items']]
            data['bundles'] = features.relationships(self.state)
            related_ids={key for bundle in data['bundles'] for key in bundle['itemIds']}
            current_ids={x['id'] for x in data['items']+data['notes']}
            data['relatedItems']=[item_ref(item) for key,item in self.state['itemArchive'].items() if key in related_ids and key not in current_ids]
            for item in data['items'] + data['notes']:
                item['snoozed'] = features.snoozed(self.state, item)
                item['actionReasons'] = features.your_move(item)
            data['syncing'] = sorted(self.running)
            data['forceRefreshAfter'] = self.force_refresh_after
            if sentry_source.configured():
                org, _, _, period = sentry_source.settings()
                data['sentryScope'] = dict(org=org, period=period)
            data['demo'] = self.demo
            data['interval'] = INTERVAL
            return data

    def schedule(self, manual=False, force=False):
        if self.demo:
            return []
        with self.lock:
            now = time.time()
            if force and now < self.force_refresh_after:
                raise ValueError('Give the last refresh a moment. Try again in a few seconds.')
            started = []
            for source, fetch in (('github', fetch_github), ('linear', fetch_linear), ('sentry', lambda: sentry_source.fetch(sys.modules[__name__]))):
                if self.state['sources'][source].get('disabled'):
                    continue
                info = self.state['sources'][source]
                # Force skips the routine interval, but preserves failure/rate-limit backoff.
                if source not in self.running and (now >= info['nextAttempt'] or force and not info.get('failures')):
                    self.running.add(source)
                    started.append(source)
                    info['nextAttempt'] = now + INTERVAL
                    threading.Thread(target=self.sync, args=(source, fetch), daemon=True).start()
            if force and started:
                self.force_refresh_after = now + 15
            return started

    def sync(self, source, fetch):
        started=time.monotonic()
        try:
            items = fetch()
            with self.lock:
                catalog = {**self.state['itemArchive'], **{x['id']: x for x in self.state['items']}}
                references = {item['id']: json.loads(json.dumps(catalog.get(item['id'], item))) for session in self.state['sessions'] if not session.get('archivedAt')
                              for item in session['items'] if item['source'] == source}
                references.update({key: json.loads(json.dumps(item)) for key, item in catalog.items()
                                   if item['source'] == source and self.state['overrides'].get(key, {}).get('hidden')})
                if source in ('github','linear'):
                    current_ids = {x['id'] for x in self.state['items']}
                    references.update({x['id']:json.loads(json.dumps(x)) for x in catalog.values() if x['source']==source and
                        x['id'] in current_ids and x.get('status')!='done' and not x.get('archivedAt')})
            found = {x['id'] for x in items}
            missing = [x for key, x in references.items() if key not in found]
            if len(missing) > 1000:
                raise SyncError('More than 1,000 linked or hidden items need a refresh. Archive old sessions or restore hidden work.')
            if missing:
                items += work_links.refresh(source, missing, sys.modules[__name__])
            with self.lock:
                items = [{**catalog.get(x['id'], {}), **x} if x.get('sourceUnavailable') else x for x in items]
                by_id = {x['id']: x for x in items}
                # A hide can arrive while provider requests are in flight. Keep its
                # cached row available for restoration until the next targeted refresh.
                for item in self.state['items']:
                    if item['source'] == source and item['id'] not in by_id and self.state['overrides'].get(item['id'], {}).get('hidden'):
                        items.append(item)
                        by_id[item['id']] = item
                for session in self.state['sessions']:
                    session['items'] = [item_ref(by_id.get(x['id'], x)) for x in session['items']]
                for item_id in references:
                    if item_id in by_id:
                        self.state['itemArchive'][item_id] = by_id[item_id]
                self.state['items'] = [x for x in self.state['items'] if x['source'] != source] + items
                self.diagnostics.write(source=source,operation='sync',outcome='success',itemCount=len(items),elapsedMs=round((time.monotonic()-started)*1000))
                features.observe(self.state, source, items)
                self.state['sources'][source].update(lastSuccess=time.time(), error=None,
                    failures=0, nextAttempt=time.time() + INTERVAL)
        except Exception as exc:
            with self.lock:
                info = self.state['sources'][source]
                info['failures'] += 1
                delay = max(min(3600, INTERVAL * 2 ** min(info['failures'] - 1, 4)),
                            min(86400, getattr(exc, 'retry_after', 0)))
                self.diagnostics.write(source=source,operation='sync',outcome='failure',elapsedMs=round((time.monotonic()-started)*1000),retrySeconds=delay,**sync_diagnostics.failure_fields(exc))
                info.update(error=str(exc) if isinstance(exc, SyncError) else 'Sync failed. Cached items are still available.',
                            nextAttempt=time.time() + delay)
        finally:
            with self.lock:
                self.running.discard(source)
                try:
                    self.save()
                except OSError:
                    self.state['sources'][source]['error'] = 'Cannot save cache. Check disk permissions and free space.'

    def sentry_environment(self, payload):
        import re
        name = payload.get('environment')
        if not isinstance(name, str) or not re.fullmatch(r'[A-Za-z0-9_.-]{1,100}', name):
            raise ValueError('Choose a valid Sentry environment name.')
        if not self.demo and not sentry_source.configured():
            raise ValueError('Configure Sentry first.')
        with self.lock:
            if name not in self.state['sentryEnvironments'] and len(self.state['sentryEnvironments'])>=20:
                raise ValueError('Up to 20 environment caches are supported.')
            cache = self.state['sentryEnvironments'].setdefault(name, {})
            now=time.time()
            source_info=self.state['sources']['sentry']
            if source_info.get('failures') and now<source_info.get('nextAttempt',0):
                raise ValueError('Sentry is waiting for its connection retry window. Cached results remain available.')
            if any(value.get('running') for value in self.state['sentryEnvironments'].values()) or now<self.state.get('sentryEnvNextAttempt',0):
                return
            if cache.get('running') or time.time() < cache.get('nextAttempt', 0):
                return
            cache.update(running=True, nextAttempt=time.time()+INTERVAL)
            self.state['sentryEnvNextAttempt']=time.time()+15
        def fetch():
            started=time.monotonic()
            try:
                items = ([x for x in self.state['items'] if x.get('source')=='sentry' and name in x.get('environments',[])]
                         if self.demo else sentry_source.fetch(sys.modules[__name__], environment=name))
                with self.lock:
                    self.diagnostics.write(source='sentry',operation='environment-sync',outcome='success',itemCount=len(items),elapsedMs=round((time.monotonic()-started)*1000))
                    self.state['sentryEnvironments'][name] = dict(itemIds=[x['id'] for x in items],
                        lastSuccess=time.time(), nextAttempt=time.time()+INTERVAL, running=False, error=None, failures=0)
                    self.save()
            except Exception as exc:
                with self.lock:
                    cache=self.state['sentryEnvironments'][name]
                    self.diagnostics.write(source='sentry',operation='environment-sync',outcome='failure',elapsedMs=round((time.monotonic()-started)*1000),**sync_diagnostics.failure_fields(exc))
                    cache['failures']=cache.get('failures',0)+1
                    cache.update(running=False, error=str(exc) if isinstance(exc,SyncError) else 'Environment lookup failed. Cached results remain available.',
                                 nextAttempt=time.time()+max(min(3600,INTERVAL*2**min(cache['failures']-1,4)), min(86400,getattr(exc,'retry_after',0))))
                    self.save()
        threading.Thread(target=fetch,daemon=True).start()

    def edit(self, payload):
        with self.lock:
            before = json.loads(json.dumps(self.state))
            try:
                match payload.get('action'):
                    case 'save-artifact':
                        newsletters.save(self.state, payload, newsletters.presets(self.path.parent))
                    case 'save-note':
                        title = payload.get('title', '')
                        detail = payload.get('detail', '')
                        status = payload.get('status', 'todo')
                        if not isinstance(title, str) or not title.strip() or len(title) > 300:
                            raise ValueError('Enter a title of 1–300 characters.')
                        if not isinstance(detail, str) or len(detail) > 10000 or status not in STATUSES:
                            raise ValueError('Invalid note or status.')
                        note_id = payload.get('id') or 'note:' + str(uuid.uuid4())
                        existing = next((x for x in self.state['notes'] if x['id'] == note_id), None)
                        if payload.get('id') and not existing:
                            raise ValueError('This note no longer exists. Refresh and try again.')
                        note = dict(id=note_id, title=title.strip(), detail=detail, status=status,
                            url=safe_url(payload.get('url', '')), source='notes', kind='Project', cwd=agents.directory(payload['cwd']) if payload.get('cwd') else '',
                            project='Personal', reference='', sourceStatus='', archivedAt=(existing or {}).get('archivedAt'), updatedAt=time.time())
                        self.state['notes'] = [x for x in self.state['notes'] if x['id'] != note_id] + [note]
                        self.state['overrides'].get(note_id, {}).pop('status', None)
                    case 'delete-note':
                        self.state['notes'] = [x for x in self.state['notes'] if x['id'] != payload.get('id')]
                        self.state['overrides'].pop(payload.get('id'), None)
                    case 'save-session':
                        self.save_session(payload)
                    case 'archive-project':
                        project = next((x for x in self.state['notes'] if x['id'] == payload.get('id')), None)
                        if not project:
                            raise ValueError('Project no longer exists.')
                        project['archivedAt'] = None if payload.get('restore') else time.time()
                    case 'archive-session':
                        session = next((x for x in self.state['sessions'] if x['id'] == payload.get('id')), None)
                        if not session:
                            raise ValueError('Session no longer exists.')
                        session['archivedAt'] = None if payload.get('restore') else time.time()
                    case 'unlink-session':
                        self.state['sessions'] = [x for x in self.state['sessions'] if x['id'] != payload.get('id')]
                    case 'hide-session':
                        session = next((x for x in self.state['sessions'] if x['id'] == payload.get('id')), None)
                        if not session or not isinstance(payload.get('hidden'), bool):
                            raise ValueError('Choose an existing session and a valid visibility setting.')
                        session['hidden'] = payload['hidden']
                    case 'override':
                        item_id = payload.get('id')
                        if not any(x['id'] == item_id for x in self.state['items'] + self.state['notes']):
                            raise ValueError('Item no longer exists. Refresh and try again.')
                        value = payload.get('value', {})
                        if not isinstance(value, dict) or set(value) - {'status', 'pinned', 'hidden'}:
                            raise ValueError('Invalid local override.')
                        if 'status' in value and value['status'] is not None and value['status'] not in STATUSES:
                            raise ValueError('Invalid status.')
                        if 'pinned' in value and not isinstance(value['pinned'], bool):
                            raise ValueError('Invalid pin.')
                        if 'hidden' in value and not isinstance(value['hidden'], bool):
                            raise ValueError('Invalid visibility setting.')
                        project = next((x for x in self.state['notes'] if x['id'] == item_id), None)
                        if project and value.get('status'):
                            project['status'] = value.pop('status')
                        self.state['overrides'].setdefault(item_id, {}).update(value)
                    case _:
                        features.edit(self.state, payload)
                self.save()
            except Exception:
                self.state = before
                raise

    def save_session(self, payload):
        record_id = payload.get('id')
        existing = next((x for x in self.state['sessions'] if x['id'] == record_id), None)
        if record_id and not existing:
            raise ValueError('Session association no longer exists. Refresh and try again.')
        if not record_id and payload.get('sessionId'):
            sid = agents.session_id(payload['sessionId'])
            existing = next((x for x in self.state['sessions'] if x.get('sessionId') == sid
                             and x['agent'] == payload.get('agent')), None)
            if existing:
                record_id = existing['id']
                payload = {**payload, 'id': record_id, 'title': existing['title'],
                           'cwd': existing['cwd'], 'instruction': existing.get('instruction', ''),
                           'destination': existing.get('destination', 'cmux'), '_reattach': True,
                           'itemIds': list(dict.fromkeys([x['id'] for x in existing['items']] + (payload.get('itemIds') or [])))}
        agent = payload.get('agent', (existing or {}).get('agent'))
        if agent not in ('claude', 'codex'):
            raise ValueError('Choose Claude or Codex.')
        title = agents.text(payload.get('title', (existing or {}).get('title', '')), 180, 'session name', required=True)
        if any(ord(c) < 32 for c in title):
            raise ValueError('Session names cannot contain control characters.')
        cwd = agents.directory(payload.get('cwd', (existing or {}).get('cwd', '')))
        instruction = agents.text(payload.get('instruction', (existing or {}).get('instruction', '')), 8000, 'instruction')
        ids = payload.get('itemIds', [x['id'] for x in (existing or {}).get('items', [])])
        if not isinstance(ids, list) or len(ids) > 100 or not all(isinstance(x, str) for x in ids):
            raise ValueError('Choose up to 100 linked items.')
        ids = list(dict.fromkeys(ids))
        available = {x['id']: x for x in (existing or {}).get('items', [])}
        available.update({x['id']: x for x in self.state['items'] + self.state['notes']})
        if any(x not in available for x in ids):
            raise ValueError('One of the selected items no longer exists. Refresh and try again.')
        items = [item_ref(available[item_id]) for item_id in ids]
        for item_id in ids:
            if available[item_id].get('detail'):
                self.state['itemArchive'][item_id] = available[item_id]
        sid = payload.get('sessionId', (existing or {}).get('sessionId', ''))
        sid = agents.session_id(sid) if sid else None
        # A saved identity cannot silently become a different agent conversation.
        if existing and (agent != existing['agent'] or (existing.get('sessionId') and sid != existing['sessionId'])):
            raise ValueError('Remove this association to attach a different conversation.')
        launch_new = bool(existing.get('launchNew')) if existing else not bool(sid)
        if existing and not existing.get('sessionId') and sid:
            launch_new = False
        if not existing and agent == 'claude' and not sid:
            sid, launch_new = str(uuid.uuid4()), True
        duplicate = next((x for x in self.state['sessions'] if sid and x.get('sessionId') == sid
                          and x['agent'] == agent and x['id'] != record_id), None)
        if duplicate:
            raise ValueError('This conversation is already attached. Edit its linked items in Agent sessions.')
        destination = payload.get('destination', (existing or {}).get('destination', 'cmux'))
        if destination not in ('cmux', 'desktop') or (agent == 'claude' and destination != 'cmux'):
            raise ValueError('Choose cmux, or the Codex desktop app.')
        session = dict(id=record_id or str(uuid.uuid4()), agent=agent, title=title, cwd=cwd,
                       sessionId=sid, launchNew=launch_new, destination=destination, instruction=instruction, items=items,
                       archivedAt=None if payload.get('_reattach') else (existing or {}).get('archivedAt'),
                       hidden=bool((existing or {}).get('hidden')),
                       reviewItemId=(existing or {}).get('reviewItemId'), newsletterId=(existing or {}).get('newsletterId'), reviewSourceTimes=(existing or {}).get('reviewSourceTimes'), workflow=(existing or {}).get('workflow'), checkpoint=(existing or {}).get('checkpoint'),
                       workspaceId=(existing or {}).get('workspaceId'), surfaceId=(existing or {}).get('surfaceId'),
                       lastOpenedAt=(existing or {}).get('lastOpenedAt'),
                       createdAt=(existing or {}).get('createdAt', time.time()), updatedAt=time.time())
        self.state['sessions'] = [x for x in self.state['sessions'] if x['id'] != session['id']] + [session]
        for item_id in ids:
            self.state['itemSettings'].setdefault(item_id, {}).update(cwd=cwd)
        return session

    def import_sources(self, urls):
        if not isinstance(urls, list) or len(urls) > 10 or not all(isinstance(x, str) for x in urls):
            raise ValueError('Add at most 10 source links.')
        with self.lock:
            cached = {x.get('url'): json.loads(json.dumps(x)) for x in self.state['items']}
        urls = list(dict.fromkeys(urls))
        pool = ThreadPoolExecutor(max_workers=3)
        futures = [pool.submit(lambda url: cached[url] if url in cached else
                               work_links.resolve(url, sys.modules[__name__]), url) for url in urls]
        try:
            _, pending = wait(futures, timeout=75)
            if pending:
                raise ValueError('Import timed out. No dashboard changes were saved. Retry with fewer links.')
            return [future.result() for future in futures]
        finally:
            pool.shutdown(wait=False, cancel_futures=True)

    def validate_launch_fields(self, payload):
        with self.lock:
            existing = next((x for x in self.state['sessions'] if x['id'] == payload.get('id')), {})
        agents.directory(payload.get('cwd', existing.get('cwd', '')))
        agents.text(payload.get('title') or existing.get('title') or payload.get('project') or Path(payload.get('cwd', '')).name, 180, 'session name', True)
        if payload.get('agent', existing.get('agent')) not in ('claude', 'codex'):
            raise ValueError('Choose Claude or Codex.')
        if payload.get('sessionId'):
            agents.session_id(payload['sessionId'])
        ids = payload.get('itemIds', [])
        if not isinstance(ids, list) or len(ids) > 100 or not all(isinstance(x, str) for x in ids):
            raise ValueError('Choose up to 100 linked items.')

    def prepare_session(self, payload):
        self.validate_launch_fields(payload)
        urls = payload.get('urls', [])
        if not isinstance(urls, list) or len(urls) > 10 or not all(isinstance(x, str) for x in urls):
            raise ValueError('Add at most 10 source links.')
        # Provider requests happen before the write lock, so notes remain responsive.
        imported = self.import_sources(urls)
        with self.lock:
            before = json.loads(json.dumps(self.state))
            try:
                for item in imported:
                    self.state['items'] = [x for x in self.state['items'] if x['id'] != item['id']] + [item]
                ids = list(payload.get('itemIds') or []) + [x['id'] for x in imported]
                project_name = payload.get('projectName')
                if project_name:
                    name = agents.text(project_name, 300, 'project name', True)
                    project = dict(id='note:' + str(uuid.uuid4()), title=name, detail=payload.get('instruction', ''),
                        status='todo', url='', source='notes', kind='Project', project='Personal', reference='',
                        sourceStatus='', cwd=agents.directory(payload.get('cwd', '')), updatedAt=time.time())
                    self.state['notes'].append(project)
                    ids.append(project['id'])
                record = self.save_session({**payload, 'itemIds': ids})
                self.save()
                return record
            except Exception:
                self.state = before
                raise

    def quick_session(self, payload, review=False):
        agent = payload.get('agent')
        if agent not in ('claude', 'codex'):
            raise ValueError('Choose Claude or Codex.')
        item_id = payload.get('itemId')
        if item_id is not None and not isinstance(item_id, str):
            raise ValueError('Choose a valid work item.')
        with self.quick_lock:
            with self.lock:
                item = next((x for x in self.state['items'] + self.state['notes'] if x['id'] == item_id), None)
                if item_id is not None and (not item or item.get('archivedAt')):
                    raise ValueError('This item is unavailable. Refresh and try again.')
                if review:
                    pr_review.target(item)
                    existing = max((s for s in self.state['sessions'] if s.get('workflow')=='pr-review'
                        and s['agent']==agent and s.get('reviewItemId')==item_id and not s.get('archivedAt')),
                        key=lambda s:(s.get('lastOpenedAt') or s.get('createdAt',0),s.get('createdAt',0)), default=None)
                    if existing:return existing
                    if time.monotonic()-self.pr_review_started.get((item_id,agent),float('-inf'))<30:
                        raise agents.AgentError('A review for this PR just started. Open it from Agent sessions, or wait 30 seconds.')
                if time.monotonic() - self.quick_started_at < 3:
                    raise agents.AgentError('A session is already starting. Give it a moment.')
                choices = locations.contexts(item) if item else []
                if not item and payload.get('scopeSource') in ('github','linear','sentry') and payload.get('scope'):
                    candidates = [x for x in self.state['items'] if x['source'] == payload['scopeSource'] and
                        (locations.repository(x) if x['source'] == 'github' else x['project']) == payload['scope']]
                    context_keys = {c['key']: c for x in candidates for c in locations.contexts(x)
                                    if c['key'].startswith('github:' if payload['scopeSource']=='github' else 'sentry-project:' if payload['scopeSource']=='sentry' else 'linear-project:')}
                    if len(context_keys) == 1:
                        choices = list(context_keys.values())
                        if payload['scopeSource']=='github':
                            item = candidates[0]
                settings = self.state['itemSettings'].get(item_id, {})
                cwd, new_name = locations.choose(item, choices, settings, self.state['folderMappings'], payload, locations.folders())
                remember_key = payload.get('rememberKey', choices[0]['key'] if choices else '')
                if not isinstance(remember_key, str) or remember_key and remember_key not in {x['key'] for x in choices}:
                    raise ValueError('Choose a valid scope for remembering this folder.')
                if new_name:
                    locations.root().mkdir(parents=True, exist_ok=True, mode=0o700)
                    Path(cwd).mkdir(mode=0o700)
                title = ('Review '+pr_review.target(item)[1]) if review else (item['title'][:180] if item_id else f'New {agent.title()} session')
                record = self.prepare_session(dict(agent=agent, destination='cmux', title=title, cwd=cwd,
                    instruction=pr_review.instruction(item,agent) if review else '',
                    itemIds=[item_id] if review else next((b['itemIds'] for b in features.relationships(self.state) if item_id in b['itemIds']), [item_id]) if item_id else []))
                if review:
                    record.update(workflow='pr-review',reviewItemId=item_id)
                    self.save()
                    self.pr_review_started[(item_id,agent)]=time.monotonic()
                if payload.get('folderPath') or new_name:
                    if remember_key:
                        label = next(x['label'] for x in choices if x['key']==remember_key)
                        self.state['folderMappings'][remember_key] = dict(path=cwd, label=label)
                    if item_id and remember_key == 'item:' + item_id:
                        self.state['itemSettings'][item_id]['explicit'] = True
                    self.save()
            self.quick_started_at = time.monotonic()
            return record

    def linear_ticket_url(self, identifier):
        import re
        if not re.fullmatch(r'[A-Z][A-Z0-9]*-[0-9]+', identifier):
            raise ValueError('Invalid Linear ticket identifier.')
        with self.lock:
            cached = next((x['url'] for x in self.state['items'] if x['source']=='linear' and x['reference']==identifier), None)
            cached = cached or self.state['linearLinks'].get(identifier)
        if cached and urlsplit(cached).hostname == 'linear.app':
            return cached
        with self.ticket_lock:
            with self.lock:
                cached = self.state['linearLinks'].get(identifier)
                if cached and urlsplit(cached).hostname == 'linear.app':
                    return cached
            now = time.monotonic()
            self.ticket_requests = [x for x in self.ticket_requests if now-x<60]
            if len(self.ticket_requests) >= 20:
                raise ValueError('Ticket link lookup is cooling down. Try again in a minute.')
            token = os.environ.get('LINEAR_API_KEY')
            if not token:
                raise ValueError('Configure the Linear connection to open this ticket.')
            self.ticket_requests.append(now)
            data = request_json('https://api.linear.app/graphql', {'query':'query { issue(id: '+json.dumps(identifier)+') { url } }'}, token)
            url = safe_url((data.get('issue') or {}).get('url', ''))
            if urlsplit(url).hostname != 'linear.app':
                raise ValueError('This Linear ticket is unavailable.')
            with self.lock:
                self.state['linearLinks'][identifier] = url
                self.save()
            return url

    def register_agent(self, payload):
        # Local associations and notes only. This endpoint cannot launch commands.
        if payload.get('operation') == 'save-artifact':
            with self.lock:
                record = next((x for x in self.state['sessions'] if x['id']==payload.get('id')), None)
                if not record: raise ValueError('Register this conversation before attaching artifacts.')
                self.edit({**payload, 'id':None, 'sessionIds':[record['id']], 'action':'save-artifact'})
                return record
        if payload.get('operation') in ('checkpoint','save-briefing'):
            self.edit({**payload, 'action':payload['operation']})
            return next(x for x in self.state['sessions'] if x['id']==payload['id'])
        self.validate_launch_fields(payload)
        imported = self.import_sources(payload.get('urls', []))
        with self.lock:
            before = json.loads(json.dumps(self.state))
            try:
                sid = agents.session_id(payload.get('sessionId'))
                record_id = payload.get('id')
                if record_id:
                    existing = next((x for x in self.state['sessions'] if x['id'] == record_id), None)
                else:
                    existing = next((x for x in self.state['sessions'] if
                                     x.get('sessionId') == sid and x['agent'] == payload.get('agent')), None)
                if record_id and not existing:
                    raise ValueError('The dashboard record no longer exists.')
                ids = [x['id'] for x in (existing or {}).get('items', [])]
                ids += payload.get('itemIds') or []
                for item in imported:
                    self.state['items'] = [x for x in self.state['items'] if x['id'] != item['id']] + [item]
                    ids.append(item['id'])
                project_title = payload.get('project')
                if project_title:
                    project_title = agents.text(project_title, 300, 'project name', True)
                    project = next((x for x in self.state['notes'] if x['title'] == project_title and not x.get('archivedAt')), None)
                    if not project:
                        project = dict(id='note:' + str(uuid.uuid4()), title=project_title, detail='', source='notes',
                                       status='todo', kind='Project', url='', project='Personal', reference='',
                                       sourceStatus='', cwd=agents.directory(payload.get('cwd', '')), updatedAt=time.time())
                        self.state['notes'].append(project)
                    ids.append(project['id'])
                session = self.save_session({**payload, 'title': payload.get('title') or (existing or {}).get('title') or project_title or Path(payload.get('cwd', '')).name, 'id': (existing or {}).get('id'), 'sessionId': sid,
                                             'itemIds': list(dict.fromkeys(ids))})
                session['launchNew'] = False
                self.save()
                return session
            except Exception:
                self.state = before
                raise

    def session_context(self, record_id):
        with self.lock:
            record = next((x for x in self.state['sessions'] if x['id'] == record_id), None)
            if not record:
                raise ValueError('Session association no longer exists.')
            current = {**self.state['itemArchive'], **{x['id']: x for x in self.state['items'] + self.state['notes']}}
            context = {**record, 'items': [current.get(x['id'], x) for x in record['items']]}
            bundles=features.relationships(self.state)
            related={key for item in record['items'] for bundle in bundles if item['id'] in bundle['itemIds'] for key in bundle['itemIds']}
            related.update(x['id'] for x in record['items'])
            handoffs=[x for x in self.state['sessions'] if x['id']!=record_id and x.get('checkpoint') and not x.get('archivedAt') and any(item['id'] in related for item in x['items'])]
            context['handoffs']=[{key:entry[key] for key in ('title','agent','checkpoint')} for entry in sorted(handoffs,key=lambda x:x['checkpoint']['at'],reverse=True)[:5]]
            context['artifacts'] = [x for x in newsletters.catalog(self.state, newsletters.presets(self.path.parent))['artifacts'] if record_id in x.get('sessionIds', [])]
            if record.get('workflow') == 'work-review':
                context['reviewData'] = work_review.snapshot(self.state)
            return context

    def newsletter_library(self):
        with self.lock:
            configured = newsletters.presets(self.path.parent)
            return {**newsletters.catalog(self.state, configured), 'presets': configured}

    def prepare_newsletter(self, payload):
        with self.lock:
            preset = next((x for x in newsletters.presets(self.path.parent) if x['id']==payload.get('presetId')), None)
            if not preset or not preset['available']:
                raise ValueError('This newsletter skill or working folder is unavailable.')
            pending = next((x for x in self.state['sessions'] if x.get('newsletterId')==preset['id'] and x.get('launchNew') and not x.get('archivedAt')), None)
            if pending:return pending
            if time.monotonic()-self.newsletter_started.get(preset['id'],float('-inf'))<30:
                raise ValueError('This newsletter just started. Open its session, or wait 30 seconds.')
            before=json.loads(json.dumps(self.state))
            try:
                record=self.save_session(dict(agent=preset['agent'],destination='cmux',title=preset['title']+' · '+time.strftime('%Y-%m-%d'),
                    cwd=preset['cwd'],instruction=newsletters.instruction(preset),itemIds=[]))
                record.update(workflow='newsletter',newsletterId=preset['id'])
                self.save()
                self.newsletter_started[preset['id']]=time.monotonic()
                return record
            except Exception:
                self.state=before
                raise

    def artifact_file(self, artifact_id):
        with self.lock:
            configured=newsletters.presets(self.path.parent)
            entry=next((x for x in newsletters.catalog(self.state,configured)['artifacts'] if x['id']==artifact_id),None)
            if not entry or not entry.get('path'):
                raise ValueError('Local artifact unavailable.')
            path=newsletters.file_path(entry['path'],newsletters.roots_for(self.state,configured))
            with path.open('rb') as source:
                body=source.read(newsletters.MAX_FILE+1)
            if len(body)>newsletters.MAX_FILE:raise ValueError('Artifact exceeds the size limit.')
            return body,newsletters.TYPES[path.suffix.lower()],path.name

    def prepare_review(self, payload):
        agent = payload.get('agent')
        if agent not in ('claude', 'codex'):
            raise ValueError('Choose Claude or Codex.')
        with self.lock:
            pending = next((s for s in self.state['sessions'] if s.get('workflow') == 'work-review'
                            and s['agent'] == agent and s.get('launchNew') and not s.get('archivedAt')), None)
            if pending:
                return pending
            if time.monotonic() - self.review_started_at < 30:
                raise ValueError('A work review just started. Open it from Agent sessions, or wait 30 seconds.')
            before = json.loads(json.dumps(self.state))
            try:
                record = self.save_session(dict(agent=agent, destination='cmux', title='Work review',
                    cwd=str(ROOT), instruction=work_review.INSTRUCTION, itemIds=[]))
                record['workflow'] = 'work-review'
                record['reviewSourceTimes'] = {key:value.get('lastSuccess') for key,value in self.state['sources'].items()}
                self.save()
                self.review_started_at = time.monotonic()
                return record
            except Exception:
                self.state = before
                raise

    def recover_session(self, payload):
        with self.lock:
            original = self.session_context(payload.get('id'))
            cwd = agents.directory(payload.get('cwd') or original['cwd'])
            # A recovery is a separate conversation. Keep the old association and links.
            pending = next((x for x in self.state['sessions']
                            if x.get('recoveryOf') == original['id'] and x.get('launchNew')
                            and x['cwd'] == cwd), None)
            if pending:
                return pending
            before = json.loads(json.dumps(self.state))
            try:
                # Retain reference material even if linked work has left the active cache.
                for item in original['items']:
                    self.state['itemArchive'][item['id']] = item
                record = self.save_session(dict(agent=original['agent'], title=original['title'],
                    cwd=cwd, destination=original['destination'], instruction=original.get('instruction', ''),
                    itemIds=[]))
                record['items'] = [item_ref(x) for x in original['items']]
                record['recoveryOf'] = original['id']
                record['workflow'] = original.get('workflow')
                record['newsletterId'] = original.get('newsletterId')
                record['reviewItemId'] = original.get('reviewItemId')
                record['reviewSourceTimes'] = original.get('reviewSourceTimes')
                self.save()
                return record
            except Exception:
                self.state = before
                raise

    def open_session(self, record_id):
        with self.lock:
            session = next((json.loads(json.dumps(x)) for x in self.state['sessions'] if x['id'] == record_id), None)
        if not session:
            raise ValueError('Session association no longer exists.')
        result = self.gateway.open(self.session_context(record_id), new=session.get('launchNew', False))
        with self.lock:
            for current in self.state['sessions']:
                if current['id'] == record_id:
                    if not self.demo or result.get('preview'):
                        current['launchNew'] = False
                        current['lastOpenedAt'] = time.time()
                        for key in ('workspaceId', 'surfaceId'):
                            if result.get(key):
                                current[key] = result[key]
                    self.save()
                    break
        return result

    def seed_demo(self):
        examples = [
            ('github', 'Tighten validation for dataset uploads', 'needs-review', '#142', 'acme/api', 'Awaiting review', ['authored']),
            ('linear', 'Investigate slow cold starts', 'in-progress', 'ENG-238', 'Platform', 'In Progress', ['assigned']),
            ('github', 'Add pagination to the activity feed', 'draft', '#138', 'acme/dashboard', 'Draft', ['authored']),
            ('linear', 'Confirm access to the staging environment', 'blocked', 'ENG-241', 'Platform', 'Blocked', ['assigned']),
            ('github', 'Simplify the cache invalidation path', 'needs-review', '#129', 'acme/api', 'Awaiting review', ['reviews']),
            ('linear', 'Write a short rollout checklist', 'todo', 'ENG-245', 'Launch', 'Todo', ['assigned']),
        ]
        for i, (source, title, status, reference, project, raw, relations) in enumerate(examples):
            self.state['items'].append(dict(id=f'{source}:demo-{i}', source=source, title=title,
                status=status, reference=reference, project=project, sourceStatus=raw, relations=relations,
                kind='Pull request' if source == 'github' else 'Issue', url='', updatedAt=time.time()))
        self.state['notes'] = [dict(id='note:demo', source='notes', title='Sketch the next small improvement',
            detail='Keep it focused. One useful thing at a time.', status='todo', url='', project='Personal',
            reference='', sourceStatus='', kind='Note', updatedAt=time.time())]
        self.state['overrides'] = {'linear:demo-1': {'pinned': True}}


class Handler(BaseHTTPRequestHandler):
    server_version = 'truffle'

    def log_message(self, *_):
        pass

    def valid_host(self):
        return self.headers.get('Host') == self.server.authority

    def respond(self, status, body, content_type='application/json', artifact=False):
        if not isinstance(body, bytes):
            body = json.dumps(body).encode()
        self.send_response(status)
        self.send_header('Content-Type', content_type)
        self.send_header('Content-Length', str(len(body)))
        self.send_header('Cache-Control', 'no-store')
        self.send_header('X-Content-Type-Options', 'nosniff')
        self.send_header('Referrer-Policy', 'no-referrer')
        if not artifact:self.send_header('X-Frame-Options', 'DENY')
        if artifact:
            self.send_header('Content-Security-Policy', "sandbox allow-scripts allow-popups allow-popups-to-escape-sandbox; default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data:; connect-src 'none'; form-action 'none'; base-uri 'none'; frame-ancestors 'self'")
        else:self.send_header('Content-Security-Policy', "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'")
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if not self.valid_host():
            return self.respond(403, {'error': 'Invalid host.'})
        match urlsplit(self.path).path:
            case path if path.startswith('/linear/'):
                try:
                    url = self.server.store.linear_ticket_url(path.removeprefix('/linear/'))
                except (ValueError, SyncError) as exc:
                    return self.respond(404, {'error':str(exc)})
                self.send_response(302)
                self.send_header('Location', url)
                self.send_header('Cache-Control', 'no-store')
                self.send_header('Content-Length', '0')
                self.end_headers()
            case '/api/health':
                self.respond(200, {'service':'truffle','apiVersion':1,'storageVersion':1})
            case '/api/state':
                self.respond(200, {**self.server.store.snapshot(), 'csrf': self.server.csrf})
            case '/api/library':
                try:self.respond(200,self.server.store.newsletter_library())
                except (ValueError,OSError):self.respond(400,{'error':'Cannot load newsletter configuration or artifacts. Check data/newsletters.json and file permissions.'})
            case path if path.startswith('/artifacts/'):
                try:
                    body,kind,_ = self.server.store.artifact_file(path.removeprefix('/artifacts/'))
                    self.respond(200,body,kind,artifact=True)
                except (ValueError,OSError):self.respond(404,{'error':'Artifact unavailable. The original file may have moved.'})
            case '/api/logs':
                try:
                    self.respond(200, {'entries':self.server.store.diagnostics.read(), 'limit':500, 'path':'data/logs/sync.jsonl'})
                except OSError:
                    self.respond(500, {'error':'Cannot read local sync logs. Check file permissions.'})
            case '/api/agents':
                self.respond(200, self.server.store.gateway.inventory())
            case '/api/export':
                self.respond(200, self.server.store.snapshot(export=True))
            case '/' | '/index.html':
                self.respond(200, (ROOT / 'static/index.html').read_bytes(), 'text/html; charset=utf-8')
            case '/client.js' | '/desktop.js' | '/performance.js' | '/workspace.js':
                self.respond(200, (ROOT / 'static' / urlsplit(self.path).path.removeprefix('/')).read_bytes(), 'text/javascript; charset=utf-8')
            case '/app.js':
                self.respond(200, (ROOT / 'static/app.js').read_bytes(), 'text/javascript; charset=utf-8')
            case '/vendor/markdown-it.min.js':
                self.respond(200, (ROOT / 'static/vendor/markdown-it.min.js').read_bytes(), 'text/javascript; charset=utf-8')
            case '/markdown.js':
                self.respond(200, (ROOT / 'static/markdown.js').read_bytes(), 'text/javascript; charset=utf-8')
            case '/newsletters.js':
                self.respond(200, (ROOT / 'static/newsletters.js').read_bytes(), 'text/javascript; charset=utf-8')
            case '/features.js':
                self.respond(200, (ROOT / 'static/features.js').read_bytes(), 'text/javascript; charset=utf-8')
            case '/sessions.js':
                self.respond(200, (ROOT / 'static/sessions.js').read_bytes(), 'text/javascript; charset=utf-8')
            case '/style.css':
                self.respond(200, (ROOT / 'static/style.css').read_bytes(), 'text/css; charset=utf-8')
            case '/favicon.ico':
                self.respond(204, b'', 'image/x-icon')
            case _:
                self.respond(404, {'error': 'Not found.'})

    def do_POST(self):
        origin = self.headers.get('Origin')
        bridge = self.path == '/api/agent/register'
        bridge_allowed = (bridge and not origin and bool(getattr(self.server, 'agent_token', None)) and secrets.compare_digest(
            self.headers.get('Authorization', ''), 'Bearer ' + getattr(self.server, 'agent_token', 'unavailable')))
        if (not self.valid_host() or (not bridge_allowed and (origin != 'http://' + self.server.authority
                or not secrets.compare_digest(self.headers.get('X-CSRF-Token', ''), self.server.csrf)))
                or (bridge and not bridge_allowed)
                or self.headers.get('Content-Type') != 'application/json'):
            return self.respond(403, {'error': 'Request rejected. Reload this dashboard and try again.'})
        try:
            length = int(self.headers.get('Content-Length', '0'))
            if not 0 < length <= MAX_BODY:
                return self.respond(413, {'error': 'Request too large or empty.'})
            payload = json.loads(self.rfile.read(length))
            if not isinstance(payload, dict):
                raise ValueError('Expected a JSON object.')
            result = {}
            match self.path:
                case '/api/agent/register':
                    record = self.server.store.register_agent(payload)
                    return self.respond(200, {'id': record['id'], 'linkedItems': len(record['items'])})
                case '/api/edit': self.server.store.edit(payload)
                case '/api/session/prepare':
                    record = self.server.store.prepare_session(payload)
                    result = {'savedSessionId': record['id']}
                case '/api/session/pr-review':
                    record = self.server.store.quick_session(payload, review=True)
                    result = {'savedSessionId':record['id']}
                case '/api/session/quick':
                    record = self.server.store.quick_session(payload)
                    result = {'savedSessionId': record['id']}
                case '/api/session/newsletter':
                    record = self.server.store.prepare_newsletter(payload)
                    result = {'savedSessionId':record['id']}
                case '/api/session/review':
                    record = self.server.store.prepare_review(payload)
                    result = {'savedSessionId': record['id']}
                case '/api/session/open':
                    result = self.server.store.open_session(payload.get('id'))
                case '/api/session/recover':
                    record = self.server.store.recover_session(payload)
                    result = {'savedSessionId': record['id']}
                case '/api/session/context':
                    record = next((x for x in self.server.store.snapshot()['sessions'] if x['id'] == payload.get('id')), None)
                    if not record:
                        raise ValueError('Session no longer exists.')
                    return self.respond(200, {'context': agents.context_packet(self.server.store.session_context(record['id']))})
                case '/api/sentry/environment':
                    self.server.store.sentry_environment(payload)
                case '/api/sync':
                    started = self.server.store.schedule(manual=True, force=payload.get('force') is True)
                    result = {'message': 'Refreshing ' + ' & '.join(started) + '…' if started else 'Nothing started: connections are already refreshing or waiting for their retry window.'}
                case _: return self.respond(404, {'error': 'Not found.'})
            self.respond(200, {**self.server.store.snapshot(), **result})
        except locations.FolderRequired as exc:
            self.respond(409, {'error':str(exc),'location':exc.location})
        except agents.AgentError as exc:
            self.respond(409, {'error': str(exc)})
        except ValueError as exc:
            self.respond(400, {'error': str(exc) or 'Invalid input.'})
        except (TypeError, KeyError):
            self.respond(400, {'error': 'Invalid input. Check the title, link, and status.'})
        except OSError:
            self.respond(500, {'error': 'Could not save. Check disk permissions and free space.'})

    def setup(self):
        super().setup()
        self.connection.settimeout(10)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--port', type=int, default=8765)
    parser.add_argument('--open', action='store_true', help='Open the dashboard in your browser')
    parser.add_argument('--demo', action='store_true', help='Use separate sample data, with no external requests')
    parser.add_argument('--data', type=Path, help='Custom JSON state file')
    args = parser.parse_args()
    os.umask(0o077)
    load_env(ROOT / '.env')
    store = Store(args.data or ROOT / 'data' / ('demo.json' if args.demo else 'state.json'), args.demo)
    server = ThreadingHTTPServer(('127.0.0.1', args.port), Handler)
    server.daemon_threads = True
    server.authority = f'127.0.0.1:{server.server_port}'
    server.csrf = secrets.token_urlsafe(32)
    server.store = store
    server.agent_token = secrets.token_urlsafe(32)
    runtime = store.path.parent / ('demo-runtime.json' if args.demo else 'runtime.json')
    inbox = agent_inbox.Inbox(server.agent_token)
    runtime.write_text(json.dumps({'url': 'http://' + server.authority, 'token': server.agent_token, 'inbox': str(inbox.path)}))
    runtime.chmod(0o600)
    stopped = threading.Event()
    def registrations():
        while not stopped.is_set():
            try:
                inbox.process(store.register_agent)
            except (OSError, ValueError):
                pass  # A transient inbox error must not stop future registrations.
            stopped.wait(0.5)
    threading.Thread(target=registrations, daemon=True).start()
    def background():
        while not stopped.is_set():
            store.schedule()
            stopped.wait(10)
    threading.Thread(target=background, daemon=True).start()
    url = 'http://' + server.authority
    print(f'truffle → {url}' + (' (sample data)' if args.demo else ''), flush=True)
    if args.open:
        webbrowser.open(url)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        stopped.set()
        server.server_close()
        runtime.unlink(missing_ok=True)


if __name__ == '__main__':
    main()
