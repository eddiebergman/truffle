"""Read-only Sentry issue feed using server configuration or sentry-cli credentials."""
import configparser
import json
import os
from pathlib import Path
import re
import socket
import ssl
import time
from http.client import HTTPException
from urllib.error import HTTPError, URLError
from urllib.parse import urlencode, urlsplit, parse_qs
from urllib.request import Request, HTTPRedirectHandler, build_opener


def settings():
    config = configparser.ConfigParser(interpolation=None)
    config.read(Path.home() / '.sentryclirc')
    org = os.environ.get('SENTRY_ORG') or config.get('defaults', 'org', fallback='')
    token = os.environ.get('SENTRY_AUTH_TOKEN') or config.get('auth', 'token', fallback='')
    base = (os.environ.get('SENTRY_URL') or config.get('defaults', 'url', fallback='https://sentry.io')).rstrip('/')
    period = os.environ.get('SENTRY_STATS_PERIOD', '90d')
    if org and not re.fullmatch(r'[A-Za-z0-9_-]{1,100}', org):
        raise ValueError('SENTRY_ORG must be an organization slug.')
    if base not in ('https://sentry.io', 'https://us.sentry.io', 'https://de.sentry.io'):
        raise ValueError('SENTRY_URL must use a supported Sentry cloud API host.')
    if not re.fullmatch(r'(?:[1-9]|[1-8][0-9]|90)d', period):
        raise ValueError('SENTRY_STATS_PERIOD must be between 1d and 90d.')
    return org, token, base, period


def configured():
    try:
        org, token, _, _ = settings()
        return bool(org and token)
    except (ValueError, OSError, configparser.Error):
        return False


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def get(path, params, api):
    org, token, base, _ = settings()
    if not org or not token:
        raise api.SyncError('Configure SENTRY_ORG and sign in with sentry-cli, or set SENTRY_AUTH_TOKEN.')
    req = Request(base + path + ('?' + urlencode(params) if params else ''),
                  headers={'Authorization': 'Bearer ' + token, 'User-Agent': 'mydash-local/1'})
    started=time.monotonic()
    diagnostic=dict(host=urlsplit(base).hostname,phase='connect')
    try:
        with build_opener(NoRedirect).open(req, timeout=25) as response:
            diagnostic.update(phase='read',httpStatus=response.status)
            return json.load(response), response.headers.get('Link', '')
    except HTTPError as exc:
        if exc.code == 404:
            return None, ''
        diagnostic.update(kind='http',httpStatus=exc.code,requestMs=round((time.monotonic()-started)*1000))
        match exc.code:
            case 401 | 403: message=f'Sentry returned HTTP {exc.code}. Check token access.'
            case 429: message='Sentry rate limit reached (HTTP 429). Cached issues are still available.'
            case _: message=f'Sentry returned HTTP {exc.code}. Cached issues are still available.'
        retry = exc.headers.get('Retry-After', '0')
        raise api.SyncError(message,int(retry) if retry.isdigit() else 0,diagnostic=diagnostic) from None
    except (URLError, TimeoutError, OSError, HTTPException) as exc:
        reason=exc.reason if isinstance(exc,URLError) else exc
        match reason:
            case socket.gaierror(): kind='dns'; message='Sentry DNS lookup failed.'
            case ssl.SSLCertVerificationError(): kind='tls-certificate'; message='Sentry TLS certificate verification failed.'
            case ssl.SSLError(): kind='tls'; message='Sentry TLS connection failed.'
            case TimeoutError(): kind='timeout'; message='Sentry request timed out.'
            case ConnectionError(): kind='connection'; message='Sentry connection was interrupted.'
            case HTTPException(): kind='protocol'; message='Sentry response was interrupted.'
            case _: kind='network'; message='Cannot reach Sentry.'
        diagnostic.update(kind=kind,reasonType=type(reason).__name__,errno=getattr(reason,'errno',None),
            tlsVerifyCode=getattr(reason,'verify_code',None),requestMs=round((time.monotonic()-started)*1000))
        raise api.SyncError(message+' Cached issues are still available.',diagnostic=diagnostic) from None
    except (json.JSONDecodeError,UnicodeDecodeError):
        diagnostic.update(kind='invalid-json',requestMs=round((time.monotonic()-started)*1000))
        raise api.SyncError('Sentry returned an invalid JSON response. Cached issues are still available.',diagnostic=diagnostic) from None


def next_cursor(header):
    for part in header.split(','):
        if 'rel="next"' not in part or 'results="true"' not in part:
            continue
        explicit = re.search(r'cursor="([^"]+)"', part)
        if explicit:
            return explicit[1]
        url = re.search(r'<([^>]+)>', part)
        if url:
            return parse_qs(urlsplit(url[1]).query).get('cursor', [None])[0]
    return None


def item(node, org, api):
    issue_id = str(node['id'])
    if not issue_id.isdigit():
        raise api.SyncError('Sentry returned an invalid issue identity.')
    url = api.safe_url(node['permalink'])
    host = urlsplit(url).hostname or ''
    if host != 'sentry.io' and not host.endswith('.sentry.io'):
        raise api.SyncError('Sentry returned an unsupported issue URL.')
    native = node.get('status', 'unresolved')
    return dict(id=f'sentry:{org}:{issue_id}', source='sentry', title=node['title'],
        reference=node.get('shortId') or issue_id, url=url,
        project=org+'/'+node['project']['slug'], status='done' if native in ('resolved', 'ignored') else 'todo',
        sourceStatus=native.title(), kind='Error', detail=(node.get('culprit') or '')[:1500],
        updatedAt=node.get('lastSeen'), firstSeen=node.get('firstSeen'), lastSeen=node.get('lastSeen'),
        eventCount=node.get('count'), userCount=node.get('userCount'), level=node.get('level'),
        priority=node.get('priority'), category=node.get('issueCategory') or 'error', relations=['sentry'])


def fetch(api, environment=None):
    org, _, _, period = settings()
    path = f'/api/0/organizations/{org}/issues/'
    params = dict(query='is:unresolved', statsPeriod=period, limit=100, sort='freq')
    if environment:
        params['environment'] = environment
    items = {}
    for page in range(1,21):
        try:
            nodes, links = get(path, params, api)
        except api.SyncError as exc:
            exc.diagnostic.update(page=page,receivedItems=len(items))
            raise
        if nodes is None:
            raise api.SyncError('Sentry organization not found. Check SENTRY_ORG and token access.')
        for node in nodes:
            converted = item(node, org, api)
            items[converted['id']] = converted
        cursor = next_cursor(links)
        if not cursor:
            return list(items.values())
        params['cursor'] = cursor
    raise api.SyncError('More than 2,000 unresolved Sentry issues in the time window. Reduce SENTRY_STATS_PERIOD.')


def refresh(references, api):
    org, _, _, _ = settings()
    result = []
    for ref in references:
        prefix = f'sentry:{org}:'
        issue_id = ref['id'].removeprefix(prefix)
        if not ref['id'].startswith(prefix) or not issue_id.isdigit():
            raise api.SyncError('A saved Sentry issue belongs to another organization.')
        node, _ = get(f'/api/0/organizations/{org}/issues/{issue_id}/', {}, api)
        result.append(item(node, org, api) if node else {**ref, 'sourceUnavailable': True})
    return result
