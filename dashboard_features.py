"""Local work triage, relationships, change history, and agent notes. No remote writes."""
import math
import re
import time
import uuid


def initialize(state):
    for key, default in [('snoozes', {}), ('changes', []), ('changeBaselines', {}),
                         ('baselineSources', []), ('workLinks', []), ('briefings', []), ('sentryViews', []),
                         ('sentryEnvironments', {})]:
        state.setdefault(key, default)


def fingerprint(item):
    # Deliberately excludes counters, timestamps, titles, and bot activity.
    return dict(status=item.get('status'), native=item.get('sourceStatus'), ci=(item.get('ci') or {}).get('state'),
                merge=item.get('mergeable'), review='reviews' in item.get('relations', []))


def snoozed(state, item, now=None):
    value = state['snoozes'].get(item['id'])
    if not value:
        return False
    if value.get('until'):
        return value['until'] > (time.time() if now is None else now)
    return value.get('fingerprint') == fingerprint(item)


def your_move(item):
    if item.get('sourceUnavailable') or item.get('status') == 'done' or item.get('archivedAt'):
        return []
    role = set(item.get('relations') or [])
    reasons = []
    if item.get('source') == 'github':
        if 'reviews' in role and item.get('status')!='draft':
            reasons.append('Your review requested')
        if 'authored' in role or 'assigned' in role:
            if (item.get('ci') or {}).get('state') == 'failed':
                reasons.append('Fix failing CI')
            if item.get('mergeable') == 'CONFLICTING':
                reasons.append('Resolve merge conflicts')
            if item.get('sourceStatus') == 'Changes requested':
                reasons.append('Address requested changes')
            if item.get('status') == 'ready-to-merge':
                reasons.append('Ready to merge')
    elif item.get('status') == 'blocked' and (item.get('source') == 'notes' or 'assigned' in role):
        reasons.append('Unblock your work')
    return reasons


def observe(state, source, items, now=None):
    now = time.time() if now is None else now
    seeded = source in state['baselineSources']
    for item in items:
        if item.get('sourceUnavailable'):
            continue
        key = item['id']
        current = fingerprint(item)
        old = state['changeBaselines'].get(key)
        messages = []
        if old and current != old['value']:
            previous = old['value']
            if current['ci'] != previous.get('ci') and current['ci'] in ('failed', 'passed'):
                messages.append('CI failing' if current['ci'] == 'failed' else 'CI recovered' if previous.get('ci') == 'failed' else '')
            if current['native'] != previous.get('native') and current['native']:
                messages.append(f"{previous.get('native') or 'Unknown'} → {current['native']}")
            elif current['status'] != previous.get('status'):
                messages.append(f"{previous.get('status')} → {current['status']}")
            if current['review'] and not previous.get('review'):
                messages.append('Your review requested')
            if current['merge'] == 'CONFLICTING' and previous.get('merge') != 'CONFLICTING':
                messages.append('Merge conflict appeared')
        elif not old and seeded and current['review']:
            messages.append('Your review requested')
        messages = [x for x in messages if x]
        if messages:
            state['changes'].insert(0, dict(id=str(uuid.uuid4()), itemId=key, title=item['title'],
                reference=item.get('reference', ''), source=source, url=item.get('url', ''),
                message=' · '.join(messages), at=now, read=False))
        state['changeBaselines'][key] = dict(value=current, seenAt=now)
    if not seeded:
        state['baselineSources'].append(source)
    state['changes'] = state['changes'][:500]
    if len(state['changeBaselines']) > 10000:
        state['changeBaselines'] = dict(sorted(state['changeBaselines'].items(), key=lambda x: x[1]['seenAt'], reverse=True)[:10000])


def relationships(state):
    catalog = {**{x['id']: x for s in state['sessions'] for x in s['items']}, **state.get('itemArchive', {}),
               **{x['id']: x for x in state['items'] + state['notes']}}
    root = {key: key for key in catalog}
    def find(key):
        while root[key] != key:
            root[key] = root[root[key]]
            key = root[key]
        return key
    def connect(ids):
        ids = [key for key in ids if key in root]
        if len(ids)>1:
            anchor = find(ids[0])
            for key in ids[1:]:
                root[find(key)] = anchor
    refs = {x.get('reference'): key for key, x in catalog.items() if x.get('source') == 'linear'}
    def url_key(url):
        sentry = re.search(r'^https://(?:[^/]+\.)?sentry\.io/(?:organizations/[^/]+/)?issues/(\d+)', url or '')
        return 'sentry-issue:'+sentry[1] if sentry else (url or '').rstrip('/')
    urls = {url_key(x.get('url')): key for key,x in catalog.items() if x.get('url')}
    for key,item in catalog.items():
        if item.get('source') == 'github':
            text = item.get('title','')+'\n'+(item.get('detail') or '')
            connect([key]+[refs[ref] for ref in re.findall(r'\b[A-Z][A-Z0-9]*-\d+\b',text) if ref in refs])
            connect([key]+[urls[url_key(url.rstrip(').,;'))] for url in re.findall(r'https://[^\s<>]+',text) if url_key(url.rstrip(').,;')) in urls])
        for link in item.get('sentryLinks') or []:
            target = urls.get(url_key(link['url']))
            if target:
                connect([key,target])
    for session in state['sessions']:
        if session.get('workflow') != 'work-review':
            connect([x['id'] for x in session['items']])
    for edge in state['workLinks']:
        connect(edge)
    groups = {}
    for key in root:
        groups.setdefault(find(key),[]).append(key)
    return [dict(id=min(ids), itemIds=sorted(ids)) for ids in groups.values() if len(ids)>1]


def text(value, limit):
    if not isinstance(value,str) or len(value)>limit or '\x00' in value:
        raise ValueError('Text is missing or too long.')
    return value.strip()


def edit(state, payload):
    """Validate before mutation; Store supplies rollback and persistence."""
    catalog = {**state.get('itemArchive',{}), **{x['id']:x for x in state['items']+state['notes']}}
    match payload['action']:
        case 'snooze':
            key=payload.get('id')
            if key not in catalog:
                raise ValueError('Work item no longer exists.')
            match payload.get('mode'):
                case 'clear': state['snoozes'].pop(key,None)
                case 'change': state['snoozes'][key]=dict(fingerprint=fingerprint(catalog[key]),createdAt=time.time())
                case 'until':
                    until=payload.get('until')
                    if isinstance(until,bool) or not isinstance(until,(float,int)) or not math.isfinite(until) or not time.time()<until<time.time()+366*86400:
                        raise ValueError('Choose a future time within the next year.')
                    state['snoozes'][key]=dict(until=until,createdAt=time.time())
                case _: raise ValueError('Choose a snooze duration.')
        case 'read-changes':
            for change in state['changes']:
                if payload.get('all') or change['id']==payload.get('id'):
                    change['read']=True
        case 'link-work' | 'unlink-work':
            ids=payload.get('itemIds')
            if not isinstance(ids,list) or len(ids)!=2 or any(not isinstance(key,str) or key not in catalog for key in ids) or ids[0]==ids[1]:
                raise ValueError('Choose two different work items.')
            edge=sorted(ids)
            if payload['action']=='link-work' and edge not in state['workLinks']:
                if len(state['workLinks'])>=2000:
                    raise ValueError('Too many manual work links. Remove unused links first.')
                state['workLinks'].append(edge)
            elif payload['action']=='unlink-work':
                state['workLinks']=[x for x in state['workLinks'] if x!=edge]
        case 'checkpoint':
            session=next((x for x in state['sessions'] if x['id']==payload.get('id')),None)
            if not session:
                raise ValueError('Session no longer exists.')
            session['checkpoint']={k:text(payload.get(k,''),4000) for k in ('done','blocked','next')}
            session['checkpoint']['at']=time.time()
        case 'save-briefing':
            session=next((x for x in state['sessions'] if x['id']==payload.get('id')),None)
            if not session or session.get('workflow')!='work-review':
                raise ValueError('Choose a Work review session to save its briefing.')
            body=text(payload.get('body'),24000)
            if not body:
                raise ValueError('The briefing cannot be empty.')
            entry=dict(id=str(uuid.uuid4()),sessionId=session['id'],body=body,at=time.time(),
                       sourceTimes=session.get('reviewSourceTimes') or {k:v.get('lastSuccess') for k,v in state['sources'].items()})
            state['briefings']=[entry]+[x for x in state['briefings'] if x['sessionId']!=session['id']]
            state['briefings']=state['briefings'][:30]
        case 'save-sentry-view':
            name=text(payload.get('name'),60)
            if not name:
                raise ValueError('Name your filter.')
            raw=payload.get('filters')
            if not isinstance(raw,dict) or set(raw)-{'minUsers','minEvents','days','level','category','environment'}:
                raise ValueError('Invalid Sentry filters.')
            filters={}
            for key in ('minUsers','minEvents','days'):
                value=raw.get(key,0)
                if isinstance(value,bool) or not isinstance(value,int) or not 0<=value<=100000000 or key=='days' and value>90:
                    raise ValueError('Use valid whole-number Sentry limits (up to 90 days).')
                filters[key]=value
            for key,allowed in [('level',('','fatal','error','warning','info','debug')),('category',('','error','performance','replay','metric'))]:
                if raw.get(key,'') not in allowed:
                    raise ValueError('Unsupported Sentry filter.')
                filters[key]=raw.get(key,'')
            environment=text(raw.get('environment',''),100)
            if environment and not re.fullmatch(r'[A-Za-z0-9_.-]+',environment):
                raise ValueError('Use a single Sentry environment name.')
            filters['environment']=environment
            key=str(uuid.UUID(payload['id'])) if payload.get('id') else str(uuid.uuid4())
            existing=next((x for x in state['sentryViews'] if x['id']==key),None)
            if not existing and len(state['sentryViews'])>=20:
                raise ValueError('Keep up to 20 saved Sentry filters.')
            state['sentryViews']=[x for x in state['sentryViews'] if x['id']!=key]+[dict(id=key,name=name,filters=filters)]
        case 'delete-sentry-view':
            state['sentryViews']=[x for x in state['sentryViews'] if x['id']!=payload.get('id')]
        case _:
            raise ValueError('Unknown local feature action.')
