#!/usr/bin/env node
// Read-only cache benchmark; does not print work records or credentials.
import { parseArgs } from 'node:util';
import { performance } from 'node:perf_hooks';

const MAX_STATE_BYTES = 20 * 1024 * 1024;
const MIN_SAMPLES = 2;
const MAX_SAMPLES = 100;
const REQUEST_TIMEOUT_MS = 10_000;
const P95 = 0.95;

function fail(message) {
  process.stderr.write(`benchmark_service: error: ${message}\n`);
  process.exit(2);
}

async function readCapped(response) {
  const chunks = [];
  let size = 0;
  for await (const chunk of response.body) {
    size += chunk.length;
    if (size > MAX_STATE_BYTES) {
      process.stderr.write('State exceeds the benchmark size limit.\n');
      process.exit(1);
    }
    chunks.push(chunk);
  }
  return Buffer.concat(chunks);
}

function median(sorted) {
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 1 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2;
}

const round = (value) => Math.round(value * 100) / 100;

const { values } = parseArgs({
  options: {
    url: { type: 'string', default: 'http://127.0.0.1:8765' },
    samples: { type: 'string', default: '10' },
  },
});
let origin;
try {
  origin = new URL(values.url);
} catch {
  fail('Use a plain 127.0.0.1 HTTP origin.');
}
if (
  origin.protocol !== 'http:' ||
  origin.hostname !== '127.0.0.1' ||
  origin.username !== '' ||
  origin.password !== '' ||
  origin.pathname !== '/' ||
  origin.search !== '' ||
  origin.hash !== ''
) {
  fail('Use a plain 127.0.0.1 HTTP origin.');
}
const samples = Number(values.samples);
if (!Number.isInteger(samples) || samples < MIN_SAMPLES || samples > MAX_SAMPLES) {
  fail(`Use ${MIN_SAMPLES}-${MAX_SAMPLES} samples.`);
}

const endpoint = `${values.url.replace(/\/+$/, '')}/api/state`;
const elapsed = [];
const parsing = [];
let state = {};
let data = Buffer.alloc(0);
for (let sample = 0; sample < samples; sample += 1) {
  let start = performance.now();
  const response = await fetch(endpoint, { signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS) });
  if (!response.ok) fail(`HTTP ${response.status} from ${endpoint}`);
  data = await readCapped(response);
  elapsed.push(performance.now() - start);
  start = performance.now();
  state = JSON.parse(data.toString('utf8'));
  parsing.push(performance.now() - start);
}
const warm = elapsed.slice(1).sort((a, b) => a - b);
const report = {
  url: values.url,
  samples: elapsed.length,
  bytes: data.length,
  items: (state.items ?? []).length,
  sessions: (state.sessions ?? []).length,
  firstRequestMs: round(elapsed[0]),
  warmMedianMs: round(median(warm)),
  warmP95Ms: round(warm[Math.ceil(warm.length * P95) - 1]),
  jsonParseMedianMs: round(median([...parsing].sort((a, b) => a - b))),
};
process.stdout.write(`${JSON.stringify(report, null, 2)}\n`);
