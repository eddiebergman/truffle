#!/usr/bin/env python3
"""Non-blocking lifecycle metadata bridge. Never retain hook bodies or change hook decisions."""
import argparse
import json
import os
from pathlib import Path
import sys
from urllib.request import Request, urlopen
from urllib.parse import urlsplit
import agent_inbox

def activity_snapshot(data):
    """Keep only bounded counts, never commands, prompts, or monitor output."""
    result = {}
    if isinstance(data.get('background_tasks'), list):
        counts, seen = {}, set()
        for task in data['background_tasks'][:1000]:
            if not isinstance(task, dict):
                continue
            if task.get('status') in ('completed', 'failed', 'cancelled', 'killed', 'stopped', 'done'):
                continue
            kind = task.get('type')
            if not isinstance(kind, str) or not kind or len(kind) > 40:
                kind = 'other'
            identity = (kind, str(task.get('id', len(seen))))
            if identity in seen:
                continue
            seen.add(identity)
            counts[kind] = counts.get(kind, 0) + 1
        result['backgroundCounts'] = counts
    if isinstance(data.get('session_crons'), list):
        result['scheduledWakeups'] = len(data['session_crons'][:1000])
    return result

def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--record',required=True)
    parser.add_argument('--event',required=True)
    args=parser.parse_args()
    data=json.loads(sys.stdin.buffer.read(128*1024))
    # Subagent turn hooks must not overwrite the parent's turn state.
    if data.get('agent_id') and args.event in ('SessionStart','Stop','UserPromptSubmit'):
        return
    root=Path(os.environ.get('MYDASH_HOME',str(Path.home()/'code/mydash')))
    runtime=json.loads((root/'data/runtime.json').read_text())
    parsed=urlsplit(runtime['url'])
    if parsed.scheme!='http' or parsed.hostname!='127.0.0.1' or parsed.username or parsed.password or parsed.path not in ('','/') or parsed.query or parsed.fragment:
        return
    payload={'id':args.record,'operation':'runtime-event','event':args.event,
             'sessionId':data.get('session_id',''),
             'subagentId':data.get('agent_id') or data.get('subagent_id') or ''}
    if args.event in ('Stop', 'SubagentStop'):
        payload.update(activity_snapshot(data))
    request=Request(runtime['url']+'/api/agent/register',data=json.dumps(payload).encode(),headers={'Content-Type':'application/json','Authorization':'Bearer '+runtime['token']})
    try:
        with urlopen(request,timeout=1.5) as response:response.read(4096)
    except OSError:
        agent_inbox.submit(runtime,payload)

if __name__=='__main__':
    try:main()
    except Exception:pass
