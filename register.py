#!/usr/bin/env python3
"""Register this agent conversation with the local mydash server. No remote writes."""
import argparse
import json
import os

from pathlib import Path
from urllib.error import HTTPError, URLError
from urllib.parse import urlsplit
from urllib.request import Request, urlopen

import agent_inbox


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='action', required=True)
    register = sub.add_parser('register', help='Attach the current conversation to local work')
    register.add_argument('--agent', choices=['claude', 'codex'], required=True)
    register.add_argument('--session', help='Exact UUID; defaults to the current agent environment variable')
    register.add_argument('--record', help='Existing mydash association ID from a launch context packet')
    register.add_argument('--title', help='Display name for this conversation')
    register.add_argument('--cwd', default=os.getcwd(), help='Existing repository or worktree path')
    register.add_argument('--url', action='append', default=[], help='GitHub issue/PR or Linear issue URL; repeatable')
    register.add_argument('--item', action='append', default=[], help='Existing mydash item ID; repeatable')
    register.add_argument('--project', help='Create or reuse a local project by name')
    register.add_argument('--destination', choices=['embedded', 'cmux', 'desktop'], default=None)
    artifact = sub.add_parser('artifact',help='Register a local display file or an existing HTTPS artifact URL')
    artifact.add_argument('--record',required=True)
    artifact.add_argument('--title')
    target=artifact.add_mutually_exclusive_group(required=True)
    target.add_argument('--file')
    target.add_argument('--url')
    for command in ('checkpoint','briefing'):
        child = sub.add_parser(command, help='Save a local agent checkpoint or work review report')
        child.add_argument('--record', required=True)
        child.add_argument('--file', required=True, help='UTF-8 report text, or checkpoint JSON with done/blocked/next')
    status = sub.add_parser('status', help='Report current agent state and active subagent count')
    status.add_argument('--record', required=True)
    status.add_argument('--state', choices=['working','waiting','needs-input'], required=True)
    status.add_argument('--subagents', type=int)
    status.add_argument('--reason', choices=['ready','finished','question','approval','failed'])
    worktree = sub.add_parser('worktree', help='Link an existing Git worktree to this conversation and its registered work')
    worktree.add_argument('--record', required=True)
    worktree.add_argument('--path', default=os.getcwd())
    worktree.add_argument('--item', action='append', default=[])
    worktree.add_argument('--url', action='append', default=[])
    worktree.add_argument('--primary', action='store_true', help='Use this worktree when resuming the conversation')
    resource = sub.add_parser('resource', help='Report ephemeral resources used by this conversation')
    resource.add_argument('--replace', action='store_true', help='Replace the full resource list; omitted resources are detached, not stopped')
    resource.add_argument('--record', required=True)
    resource.add_argument('--file', required=True, help='JSON object with a resources array; up to 64 KiB')
    context = sub.add_parser('context', help='Read or replace the session dashboard links')
    context.add_argument('--record', required=True)
    context_mode = context.add_mutually_exclusive_group(required=True)
    context_mode.add_argument('--show', action='store_true', help='Print editable context JSON with a revision guard')
    context_mode.add_argument('--file', help='JSON with resources, worktrees, itemIds or artifactIds to replace')
    review = sub.add_parser('review', help='Read or append a batch to a session review draft; does not send it')
    review.add_argument('--record', required=True)
    review_mode = review.add_mutually_exclusive_group()
    review_mode.add_argument('--show', action='store_true')
    review_mode.add_argument('--file', help='JSON with comments array; up to 96 KiB')
    review.add_argument('--request', help='Review request ID from the dashboard prompt')
    review.add_argument('--state', choices=['reviewing', 'complete', 'failed'])
    review.add_argument('--pass', dest='review_pass', help='Stable reviewer identifier, such as correctness or security')
    ci = sub.add_parser('ci', help='Report progress for a dashboard-requested CI investigation')
    ci.add_argument('--record', required=True)
    ci.add_argument('--item', required=True)
    ci.add_argument('--request', required=True)
    ci.add_argument('--state', required=True, choices=['investigating', 'complete', 'failed'])
    args = parser.parse_args()
    if args.action == 'review' and not (args.show or args.file or args.state):
        parser.error('Review requires --show, --file, or --state.')
    if args.action == 'review' and (args.state or args.review_pass) and not args.request:
        parser.error('Review progress requires --request.')
    if args.action == 'review' and args.show and (args.state or args.review_pass):
        parser.error('Use --show separately from progress updates.')
    home = Path(os.environ.get('MYDASH_HOME', str(Path.home() / 'code/mydash')))
    try:
        runtime = json.loads((home / 'data/runtime.json').read_text())
        parsed = urlsplit(runtime['url'])
        if parsed.scheme != 'http' or parsed.hostname != '127.0.0.1' or parsed.path or parsed.query or parsed.fragment or parsed.username:
            raise ValueError('Invalid dashboard runtime address.')
        match args.action:
            case 'register':
                sid = args.session or os.environ.get('CLAUDE_CODE_SESSION_ID' if args.agent == 'claude' else 'CODEX_THREAD_ID')
                if not sid:
                    parser.error('Current session ID is unavailable. Pass --session with the exact UUID. Do not create a new session.')
                payload = dict(agent=args.agent, sessionId=sid, cwd=args.cwd, urls=args.url, itemIds=args.item)
                for key, value in [('id', args.record), ('title', args.title), ('project', args.project), ('destination', args.destination)]:
                    if value:
                        payload[key] = value
            case 'ci':
                payload = dict(id=args.record, operation='ci-investigation', itemId=args.item, requestId=args.request, state=args.state)
            case 'review':
                payload = dict(id=args.record, operation='review')
                if args.request: payload['requestId'] = args.request
                if args.state: payload['state'] = args.state
                if args.review_pass: payload['pass'] = args.review_pass
                if args.file:
                    path = Path(args.file).expanduser()
                    if path.stat().st_size > 96 * 1024:
                        raise ValueError('Review batch exceeds 96 KiB.')
                    payload['report'] = json.loads(path.read_text())
            case 'context':
                payload = dict(id=args.record, operation='context-read' if args.show else 'context-replace')
                if args.file:
                    path = Path(args.file).expanduser()
                    if path.stat().st_size > 96 * 1024:
                        raise ValueError('Context exceeds 96 KiB.')
                    payload['context'] = json.loads(path.read_text())
            case 'resource':
                path = Path(args.file).expanduser()
                if path.stat().st_size > 65536:
                    raise ValueError('Resource report exceeds 64 KiB.')
                report = json.loads(path.read_text())
                match report:
                    case {'resources': list()}:
                        report['replace'] = args.replace
                        payload = dict(id=args.record, operation='resources', report=report)
                    case _:
                        raise ValueError('Resource report must be an object with a resources array.')
            case 'worktree':
                payload=dict(id=args.record,operation='worktree',path=str(Path(args.path).expanduser().resolve()),itemIds=args.item,urls=args.url,primary=args.primary)
            case 'status':
                payload=dict(id=args.record,operation='runtime',state=args.state)
                if args.subagents is not None: payload['subagents']=args.subagents
                if args.reason: payload['reason']=args.reason
            case 'artifact':
                payload=dict(id=args.record,operation='save-artifact')
                if args.file:payload['path']=str(Path(args.file).expanduser().resolve())
                else:payload['url']=args.url
                if args.title:payload['title']=args.title
            case _:
                path=Path(args.file).expanduser()
                if path.stat().st_size>96000:
                    raise ValueError('Report file is too large.')
                body=path.read_text()
                payload=dict(id=args.record,operation='save-briefing' if args.action=='briefing' else 'checkpoint')
                if args.action=='briefing':
                    payload['body']=body
                else:
                    data=json.loads(body)
                    if not isinstance(data,dict) or set(data)-{'done','blocked','next'}:
                        raise ValueError('Checkpoint JSON must contain only done, blocked, and next.')
                    payload.update(data)
        request = Request(runtime['url'] + '/api/agent/register', data=json.dumps(payload).encode(),
            headers={'Content-Type': 'application/json', 'Authorization': 'Bearer ' + runtime['token']})
        try:
            with urlopen(request, timeout=120) as response:
                result = json.load(response)
        except HTTPError:
            raise
        except URLError:
            # Codex sandboxes may deny loopback sockets while permitting private temporary files.
            # The signed inbox has the same association-only scope as the HTTP endpoint.
            if not runtime.get('inbox'):
                raise
            result = agent_inbox.submit(runtime, payload)
        if args.action == 'review' and args.show:
            print(json.dumps(result['review'], indent=2))
            return
        if args.action == 'context' and args.show:
            print(json.dumps(result['context'], indent=2))
            return
        message = f"Registered {result['linkedItems']} linked items." if args.action=='register' else "Saved to mydash."
        print(f"{message} {runtime['url']}/#session={result['id']}")
    except FileNotFoundError:
        parser.exit(1, 'mydash is not running, or MYDASH_HOME points to the wrong installation. Start it from a regular Terminal with ~/code/mydash/start.command.\n')
    except HTTPError as exc:
        try:
            message = json.load(exc).get('error', 'Registration failed.')
        except (ValueError, OSError):
            message = 'Registration failed.'
        parser.exit(1, message + '\n')
    except ValueError as exc:
        parser.exit(1, str(exc) + '\n')
    except (URLError, OSError, KeyError):
        parser.exit(1, 'Cannot reach mydash. Run ~/code/mydash/start.command from a regular Terminal and retry once.\n')


if __name__ == '__main__':
    main()
