"""Local newsletter presets and an indexed, session-linked artifact library."""
import hashlib
import json
from pathlib import Path
import time
from urllib.parse import urlsplit

MAX_FILE = 10 * 1024 * 1024
TYPES = {'.html':'text/html; charset=utf-8', '.md':'text/plain; charset=utf-8',
         '.txt':'text/plain; charset=utf-8', '.pdf':'application/pdf',
         '.png':'image/png', '.jpg':'image/jpeg', '.jpeg':'image/jpeg', '.webp':'image/webp'}


def identity(value):
    return hashlib.sha256(value.encode()).hexdigest()[:24]


def initialize(state):
    state.setdefault('artifacts', [])


def presets(data_dir):
    config = data_dir / 'newsletters.json'
    if config.exists():
        raw = json.loads(config.read_text())
    else:
        root = Path.home() / 'code/sentry-investigations'
        raw = [dict(id='prod-daily', title='Production daily', description='Production health, errors, deploys, and follow-ups.',
                    cwd=str(root), skill='.claude/skills/prod-daily/SKILL.md', command='/prod-daily', agent='claude', reports='reports')]
    if not isinstance(raw, list) or len(raw) > 30:
        raise ValueError('Newsletters config must contain at most 30 presets.')
    result = []
    for value in raw:
        if not isinstance(value, dict):
            raise ValueError('Invalid newsletter preset.')
        key, title = value.get('id'), value.get('title')
        if not isinstance(key, str) or not key or len(key)>80 or not isinstance(title,str) or not title or len(title)>180:
            raise ValueError('Newsletter presets need an ID and title.')
        if any(x['id']==key for x in result):
            raise ValueError('Newsletter preset IDs must be unique.')
        cwd = Path(value.get('cwd','')).expanduser().resolve()
        skill = (cwd / value.get('skill','')).resolve()
        reports = (cwd / value.get('reports','reports')).resolve()
        if not skill.is_relative_to(cwd) or not reports.is_relative_to(cwd):
            raise ValueError('Skill and reports paths must stay inside the newsletter folder.')
        agent=value.get('agent','claude')
        if agent not in ('claude','codex'):
            raise ValueError('Choose Claude or Codex for the newsletter.')
        result.append(dict(id=key,title=title,description=str(value.get('description',''))[:400],cwd=str(cwd),
            skill=str(skill),reports=str(reports),command=str(value.get('command',''))[:100],agent=agent,
            available=cwd.is_dir() and skill.is_file()))
    return result


def file_path(path, roots):
    path=Path(path).expanduser().resolve()
    if not any(path.is_relative_to(root) and not any(part.startswith('.') for part in path.relative_to(root).parts)
               for root in roots):
        raise ValueError('Choose an artifact inside its session folder or a configured newsletter reports folder.')
    if path.suffix.lower() not in TYPES or not path.is_file():
        raise ValueError('Choose an existing HTML, Markdown, text, PDF, or image file.')
    if path.stat().st_size > MAX_FILE:
        raise ValueError('Artifacts must be 10 MiB or smaller.')
    return path


def roots_for(state, configured):
    return [Path(x['reports']).resolve() for x in configured] + [Path(x['cwd']).resolve() for x in state['sessions']]


def remote_url(value):
    if not isinstance(value,str) or len(value)>4000 or any(ord(c)<32 for c in value):
        raise ValueError('Enter an HTTPS artifact URL.')
    parsed=urlsplit(value)
    if parsed.scheme!='https' or not parsed.hostname or parsed.username or parsed.password:
        raise ValueError('Enter an HTTPS artifact URL without credentials.')
    return value


def catalog(state, configured):
    entries={x['id']:dict(x) for x in state['artifacts']}
    warnings=[]
    for preset in configured:
        root=Path(preset['reports'])
        if not root.is_dir():
            continue
        try:
            # Only dated run directories and display files. Never walk source repos or raw telemetry.
            directories=sorted((p for p in root.iterdir() if p.is_dir() and len(p.name)==10 and p.name[4:5]=='-' and p.name[7:8]=='-'),reverse=True)[:365]
            for directory in directories:
                for filename in ('report.html','report.md','audit.html'):
                    candidate=directory/filename
                    if not candidate.is_file():continue
                    try:path=file_path(candidate,[root.resolve()])
                    except (ValueError,OSError):continue
                    key=identity(str(path));old=entries.get(key,{})
                    entries[key]={**old,'id':key,'title':old.get('title') or f"{preset['title']} · {directory.name}"+(' · Audit' if filename=='audit.html' else ''),
                        'path':str(path),'presetId':preset['id'],'sessionIds':old.get('sessionIds',[]),
                        'at':path.stat().st_mtime,'format':path.suffix.lower()[1:],'discovered':True}
        except OSError:
            warnings.append('Cannot read reports for '+preset['title'])
    roots=roots_for(state,configured)
    for entry in entries.values():
        if entry.get('path'):
            try:
                path=file_path(entry['path'],roots)
                entry.update(available=True,format=path.suffix.lower()[1:],size=path.stat().st_size)
            except (ValueError,OSError):entry['available']=False
        else:entry['available']=True
    return dict(artifacts=sorted(entries.values(),key=lambda x:x.get('at',0),reverse=True),warnings=warnings)


def save(state, payload, configured):
    ids=payload.get('sessionIds',[])
    if not isinstance(ids,list) or len(ids)>100 or any(not isinstance(x,str) for x in ids):
        raise ValueError('Choose valid sessions.')
    known={x['id'] for x in state['sessions']}
    if any(x not in known for x in ids):raise ValueError('A selected session no longer exists.')
    if payload.get('id'):
        entry=next((x for x in catalog(state,configured)['artifacts'] if x['id']==payload['id']),None)
        if not entry:raise ValueError('Artifact no longer exists. Refresh the library.')
    elif payload.get('path'):
        path=file_path(payload['path'],roots_for(state,configured))
        entry=dict(id=identity(str(path)),path=str(path),at=path.stat().st_mtime,format=path.suffix.lower()[1:])
    else:
        url=remote_url(payload.get('url'))
        entry=dict(id=identity(url),url=url,at=time.time(),format='link')
    prior=next((x for x in state['artifacts'] if x['id']==entry['id']),{})
    title=payload.get('title') or prior.get('title') or entry.get('title') or Path(entry.get('path','Artifact')).name
    if not isinstance(title,str) or len(title)>300 or any(ord(c)<32 for c in title):raise ValueError('Enter an artifact title of 1–300 characters.')
    sessions=list(dict.fromkeys(ids if payload.get('replaceSessions') else prior.get('sessionIds',[])+entry.get('sessionIds',[])+ids))
    entry.update(title=title.strip(),sessionIds=sessions)
    if not entry['title']:raise ValueError('Enter an artifact title.')
    state['artifacts']=[x for x in state['artifacts'] if x['id']!=entry['id']]+[entry]
    return entry


def instruction(preset):
    return '\n'.join([
        'Run the newsletter workflow now. Register this conversation before starting.',
        'Read the complete skill at '+preset['skill']+'. Follow that workflow in '+preset['cwd']+'.',
        'Skill command: '+preset['command'],
        'Keep the skill read-only restrictions. Use uv for Python commands. Keep agent approval settings unchanged.',
        'This dashboard run saves local outputs. Do not send Slack messages or publish externally unless the user asks in this session.',
        'Save the finished report under '+preset['reports']+'.',
        'Register every final report and supporting display artifact with the mydash artifact helper in this context.',
        'Do not register raw telemetry or credential files. Include verified links to tickets, PRs, and evidence.',
        'If an external Claude Artifact URL already exists, register that URL too. Do not invent a URL.',
        'Registering an artifact records its path or URL and links this conversation. It does not upload the file.'])
