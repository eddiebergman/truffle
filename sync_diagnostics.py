"""Small private JSON-lines logs. Never log exception messages, headers, or payloads."""
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import threading

_LOCK = threading.Lock()
_ALLOWED = {'source','operation','outcome','exceptionType','reasonType','kind','errno',
            'tlsVerifyCode','httpStatus','host','phase','page','receivedItems',
            'requestMs','elapsedMs','itemCount','retrySeconds','transport','timeoutSeconds','exitCode','scope'}


class DiagnosticLog:
    def __init__(self, directory, max_bytes=512*1024):
        self.path=Path(directory)/'logs'/'sync.jsonl'
        self.max_bytes=max_bytes

    def write(self, **fields):
        entry={'time':datetime.now(timezone.utc).isoformat(timespec='seconds')}
        for key,value in fields.items():
            if key in _ALLOWED and (value is None or type(value) in (str,int,float,bool)):
                entry[key]=value[:160] if isinstance(value,str) else value
        try:
            with _LOCK:
                self.path.parent.mkdir(parents=True,exist_ok=True,mode=0o700)
                if self.path.exists() and self.path.stat().st_size>=self.max_bytes:
                    older=self.path.with_name('sync.jsonl.1')
                    if older.exists():os.replace(older,self.path.with_name('sync.jsonl.2'))
                    os.replace(self.path,older)
                descriptor=os.open(self.path,os.O_WRONLY|os.O_APPEND|os.O_CREAT,0o600)
                with os.fdopen(descriptor,'a',encoding='utf-8') as output:
                    os.fchmod(output.fileno(),0o600)
                    output.write(json.dumps(entry,ensure_ascii=True)+'\n')
        except OSError:
            pass  # Diagnostics must never break syncing or hide the original error.


    def read(self, limit=500):
        limit=max(1,min(500,int(limit)))
        entries=[]
        with _LOCK:
            for name in ('sync.jsonl','sync.jsonl.1','sync.jsonl.2'):
                path=self.path.with_name(name)
                try:
                    with path.open('rb') as source:
                        source.seek(0,2);size=source.tell();start=max(0,size-(512*1024+2048));source.seek(start)
                        lines=source.read(512*1024+2048).decode('utf-8',errors='replace').splitlines()
                    if start:lines=lines[1:]
                except FileNotFoundError:
                    continue
                for line in reversed(lines):
                    try:row=json.loads(line)
                    except (ValueError,TypeError):continue
                    if not isinstance(row,dict):continue
                    entry={key:value for key,value in row.items() if key in _ALLOWED|{'time'} and (value is None or type(value) in (str,int,float,bool))}
                    if 'time' not in entry:continue
                    entries.append(entry)
                    if len(entries)>=limit:return entries
        return entries


def failure_fields(exc):
    fields={'exceptionType':type(exc).__name__}
    fields.update({key:value for key,value in (getattr(exc,'diagnostic',None) or {}).items() if key in _ALLOWED})
    return fields


def network_fields(exc):
    import socket
    import ssl
    from urllib.error import URLError
    reason=exc.reason if isinstance(exc,URLError) else exc
    match reason:
        case socket.gaierror(): kind='dns'
        case ssl.SSLCertVerificationError(): kind='tls-certificate'
        case ssl.SSLError(): kind='tls'
        case TimeoutError(): kind='timeout'
        case ConnectionError(): kind='connection'
        case _: kind='network'
    return dict(kind=kind,reasonType=type(reason).__name__,errno=getattr(reason,'errno',None),tlsVerifyCode=getattr(reason,'verify_code',None))


def cli_failure_fields(stderr):
    """Classify a few known diagnostics; never retain raw command output."""
    import re
    text=(stderr or '').lower()
    fields={'kind':'cli-exit'}
    status=re.search(r'\bhttp\s+(\d{3})\b',text)
    if status:fields['httpStatus']=int(status[1])
    match fields.get('httpStatus'):
        case 401: fields['kind']='authentication'
        case 429: fields['kind']='rate-limit'
        case _: pass
    if 'rate limit' in text:fields['kind']='rate-limit'
    elif any(value in text for value in ('context deadline exceeded','i/o timeout','timed out')):fields['kind']='timeout'
    elif any(value in text for value in ('no such host','temporary failure in name resolution','nodename nor servname')):fields['kind']='dns'
    elif any(value in text for value in ('certificate signed by unknown authority','certificate verify failed','x509:')):fields['kind']='tls-certificate'
    return fields
