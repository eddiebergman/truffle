"""Authenticated file-based registration for agent sandboxes without loopback access."""
import hashlib
import hmac
import json
import os
from pathlib import Path
import stat
import tempfile
import time
import uuid

MAX_BYTES = 128 * 1024


def private_write(path, data):
    content = json.dumps(data).encode()
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, 'wb') as output:
        output.write(content)


def signature(token, data):
    encoded = json.dumps(data, sort_keys=True, separators=(',', ':')).encode()
    return hmac.new(token.encode(), encoded, hashlib.sha256).hexdigest()


class Inbox:
    def __init__(self, token):
        self.token = token
        self.path = Path(tempfile.mkdtemp(prefix='truffle-agent-', dir='/tmp'))
        self.path.chmod(0o700)

    def process(self, register):
        # One registration per pass bounds work. Only authenticated requests can call the callback.
        for path in self.path.glob('*.request.json'):
            result_path = self.path / (path.name.removesuffix('.request.json') + '.result.json')
            try:
                info = path.lstat()
                if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid() or info.st_size > MAX_BYTES:
                    path.unlink(missing_ok=True)
                    continue
                envelope = json.loads(path.read_text())
                data = envelope['data']
                if (not hmac.compare_digest(envelope.get('signature', ''), signature(self.token, data))
                        or abs(time.time() - data['time']) > 180
                        or str(uuid.UUID(data['nonce'])) != path.name.removesuffix('.request.json')):
                    path.unlink(missing_ok=True)
                    continue
                record = register(data['payload'])
                result = {'id': record['id'], 'linkedItems': len(record['items'])}
            except Exception as exc:
                result = {'error': str(exc) if isinstance(exc, ValueError) else 'Registration failed. Check source access and retry once.'}
            try:
                if not result_path.exists():
                    staging = result_path.with_suffix('.tmp')
                    private_write(staging, result)
                    os.replace(staging, result_path)
            finally:
                path.unlink(missing_ok=True)
            break
        # Remove stale acknowledgements and abandoned envelopes from this private inbox.
        now = time.time()
        for path in self.path.iterdir():
            try:
                if now - path.lstat().st_mtime > 300:
                    path.unlink(missing_ok=True)
            except FileNotFoundError:
                continue


def submit(runtime, payload, timeout=90):
    path = Path(runtime.get('inbox', ''))
    info = path.lstat()
    if (not stat.S_ISDIR(info.st_mode) or stat.S_ISLNK(info.st_mode) or info.st_uid != os.getuid()
            or info.st_mode & 0o077 or path.parent.resolve() != Path('/tmp').resolve()
            or not path.name.startswith('truffle-agent-')):
        raise ValueError('Invalid local registration inbox. Restart truffle.')
    nonce = str(uuid.uuid4())
    data = {'nonce': nonce, 'time': time.time(), 'payload': payload}
    envelope = {'data': data, 'signature': signature(runtime['token'], data)}
    if len(json.dumps(envelope).encode()) > MAX_BYTES:
        raise ValueError('Registration request is too large.')
    staging = path / (nonce + '.tmp')
    request = path / (nonce + '.request.json')
    result_path = path / (nonce + '.result.json')
    private_write(staging, envelope)
    os.replace(staging, request)
    deadline = time.monotonic() + timeout
    try:
        while time.monotonic() < deadline:
            if result_path.exists():
                result = json.loads(result_path.read_text())
                if result.get('error'):
                    raise ValueError(result['error'])
                return result
            time.sleep(0.1)
        raise ValueError('truffle did not acknowledge registration. Check that its server is running and retry once.')
    finally:
        request.unlink(missing_ok=True)
        result_path.unlink(missing_ok=True)
