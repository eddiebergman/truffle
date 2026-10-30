const { test } = require("node:test"),
  assert = require("node:assert/strict");
const { browserContext, loadScripts } = require("./helpers/static-script.cjs");
const uuid = "10000000-0000-7000-8000-000000000001";
const recordA = { id: "a", agent: "claude", sessionId: uuid },
  recordB = { id: "b", agent: "claude", sessionId: uuid };
function workspace({ candidates, sessions = [recordA, recordB], ...globals }) {
  const c = browserContext({
    inventory: { candidates: candidates.map((x) => ({ live: true, ...x })) },
    state: sessions === null ? null : { sessions },
    ...globals,
  });
  loadScripts(c, [
    "static/client.js",
    "static/app.js",
    "static/sessions.js",
    "static/features.js",
    "static/workspace.js",
  ]);
  return c;
}
test("a candidate with a recordId joins only that record, even when another shares its session", () => {
  const c = workspace({
    candidates: [
      { host: "embedded", recordId: "b", agent: "claude", sessionId: uuid },
    ],
  });
  const [candidate] = c.liveCandidates();
  assert.equal(c.recordForCandidate(candidate)?.id, "b");
  assert.equal(c.candidateForRecord(recordA), undefined);
  assert.equal(c.candidateForRecord(recordB), candidate);
});
test("agent and sessionId join only candidates without a recordId", () => {
  const c = workspace({
    candidates: [{ host: "terminal", agent: "claude", sessionId: uuid }],
    sessions: [
      { id: "n", agent: "claude" },
      { id: "x", agent: "codex", sessionId: uuid },
      recordA,
    ],
  });
  const [candidate] = c.liveCandidates();
  assert.equal(c.recordForCandidate(candidate)?.id, "a");
  assert.equal(c.candidateForRecord(recordA), candidate);
  assert.equal(
    c.recordForCandidate({ host: "terminal", agent: "claude" }),
    undefined,
  );
});
test("a record prefers its exact candidate over a session match in any order", () => {
  const exact = { host: "embedded", recordId: "a", agent: "claude" },
    loose = { host: "terminal", agent: "claude", sessionId: uuid };
  for (const candidates of [
    [exact, loose],
    [loose, exact],
  ]) {
    const c = workspace({ candidates });
    assert.equal(c.candidateForRecord(recordA)?.host, "embedded");
  }
});
