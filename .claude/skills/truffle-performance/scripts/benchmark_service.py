#!/usr/bin/env python3
"""Read-only cache benchmark; does not print work records or credentials."""
import argparse
import json
import math
import statistics
import time
from urllib.parse import urlparse
from urllib.request import ProxyHandler, build_opener

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--url', default='http://127.0.0.1:8765')
parser.add_argument('--samples', type=int, default=10)
args = parser.parse_args()
url = urlparse(args.url)
if url.scheme != 'http' or url.hostname != '127.0.0.1' or url.username or url.password or url.path not in ('', '/') or url.query or url.fragment:
    parser.error('Use a plain 127.0.0.1 HTTP origin.')
if not 2 <= args.samples <= 100:
    parser.error('Use 2–100 samples.')
opener = build_opener(ProxyHandler({}))
elapsed, parsing = [], []
state, data = {}, b''
for _ in range(args.samples):
    start = time.perf_counter()
    with opener.open(args.url.rstrip('/')+'/api/state', timeout=10) as response:
        data = response.read(20*1024*1024+1)
    if len(data) > 20*1024*1024:
        raise SystemExit('State exceeds the benchmark size limit.')
    elapsed.append((time.perf_counter()-start)*1000)
    start = time.perf_counter()
    state = json.loads(data)
    parsing.append((time.perf_counter()-start)*1000)
warm = sorted(elapsed[1:])
print(json.dumps({'url':args.url, 'samples':len(elapsed), 'bytes':len(data),
    'items':len(state.get('items',[])), 'sessions':len(state.get('sessions',[])),
    'firstRequestMs':round(elapsed[0],2), 'warmMedianMs':round(statistics.median(warm),2),
    'warmP95Ms':round(warm[math.ceil(len(warm)*.95)-1],2),
    'jsonParseMedianMs':round(statistics.median(parsing),2)}, indent=2))
