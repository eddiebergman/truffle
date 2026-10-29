"""Local launch folders and reusable work-to-folder contexts. No remote requests."""
from pathlib import Path
import os
import re
from urllib.parse import urlsplit


class FolderRequired(ValueError):
    def __init__(self, location):
        super().__init__('Choose a folder for this session.')
        self.location = location


def root():
    return Path(os.environ.get('MYDASH_WORKDIR', str(Path.home() / 'code'))).expanduser().resolve()


def folders():
    base = root()
    if not base.is_dir():
        return []
    return [dict(name=p.name, path=str(p.resolve())) for p in sorted(base.iterdir(), key=lambda p:p.name.casefold())
            if not p.name.startswith('.') and p.is_dir()]


def repository(item):
    path = urlsplit(item.get('url', '')).path.strip('/').split('/')
    if urlsplit(item.get('url', '')).hostname == 'github.com' and len(path) >= 2:
        return '/'.join(path[:2])
    value = item.get('project', '')
    return value if re.fullmatch(r'[\w.-]+/[\w.-]+', value) else ''


def contexts(item):
    result = []
    match item.get('source'):
        case 'github':
            repo = repository(item)
            if repo:
                result.append(dict(key='github:' + repo.casefold(), label='GitHub repo ' + repo))
        case 'sentry':
            result.append(dict(key='sentry-project:' + item['project'], label='Sentry project ' + item['project']))
        case 'linear':
            parent, seen = item.get('parent'), set()
            top = None
            while parent and parent.get('id') not in seen and len(seen) < 8:
                seen.add(parent['id']); top = parent; parent = parent.get('parent')
            if not top and item.get('children'):
                top = item
            if top:
                result.append(dict(key='linear-family:' + top['id'], label='Issue family ' + (top.get('reference') or top['title'])))
            workspace = urlsplit(item.get('url', '')).path.strip('/').split('/')[0]
            if item.get('projectId'):
                result.append(dict(key='linear-project:' + workspace + ':' + item['projectId'], label='Linear project ' + item['project']))
        case _:
            pass
    result.append(dict(key='item:' + item['id'], label='Only ' + (item.get('reference') or item['title'])))
    return result


def choose(item, choices, settings, mappings, payload, candidates):
    """Resolve only known folders; an absent context must open the browser picker."""
    base = root()
    chosen = payload.get('folderPath', '')
    new_name = payload.get('newFolder', '')
    if not isinstance(chosen, str) or not isinstance(new_name, str) or chosen and new_name:
        raise ValueError('Choose an existing folder or enter one new folder name.')
    if new_name:
        if not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9._-]{0,79}', new_name):
            raise ValueError('Use a folder name of 1–80 letters, numbers, dots, underscores, or hyphens. No paths.')
        path = base / new_name
        if path.exists() or path.is_symlink():
            raise ValueError('That folder already exists. Select it from Existing folders.')
        return str(path), new_name
    if chosen:
        allowed = {x['path'] for x in candidates}
        if chosen not in allowed:
            raise ValueError('Choose one of the existing folders under the work directory.')
        return chosen, None
    if not payload.get('chooseFolder'):
        if settings.get('explicit') and settings.get('cwd') and Path(settings['cwd']).is_dir() and Path(settings['cwd']).resolve() != base:
            return str(Path(settings['cwd']).resolve()), None
        for context in choices:
            saved = mappings.get(context['key'], {}).get('path')
            if saved and Path(saved).is_dir() and Path(saved).resolve() != base:
                return str(Path(saved).resolve()), None
        # Prefer the actual GitHub repo over an old launch-directory hint.
        if item and item.get('source') == 'github':
            repo = repository(item).split('/')[-1].casefold()
            match = next((x for x in candidates if x['name'].casefold() == repo), None)
            if match:
                return match['path'], None
        saved = settings.get('cwd') or (item or {}).get('cwd')
        if saved and Path(saved).is_dir() and Path(saved).resolve() != base:
            return str(Path(saved).resolve()), None
    suggested = repository(item).split('/')[-1] if item and item.get('source') == 'github' else ''
    raise FolderRequired(dict(root=str(base), folders=candidates, contexts=choices,
        suggestedName=suggested, title=(item or {}).get('title', 'New session'),
        reason='Choose where the agent should work. This choice can be remembered for related tickets.'))
