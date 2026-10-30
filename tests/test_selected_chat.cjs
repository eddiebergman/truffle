const { test } = require("node:test"),
  assert = require("node:assert/strict");
const { browserContext, loadScripts } = require("./helpers/static-script.cjs");
const uuid = "10000000-0000-7000-8000-000000000001";
const recordA = { id: "a", agent: "claude", sessionId: uuid },
  recordB = { id: "b", agent: "claude", sessionId: uuid };
function workspace({
  candidates,
  sessions = [recordA, recordB],
  files = [],
  ...globals
}) {
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
    ...files,
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
test("the selected chat follows the visible candidates, so a snoozed chat is not selected", () => {
  const shown = { host: "embedded", recordId: "a", agent: "claude" },
    snoozed = { host: "embedded", recordId: "b", agent: "claude" };
  const c = workspace({
    candidates: [shown, snoozed],
    snoozedAgentTabs: { "embedded:b": { at: 1, recordId: "b", title: "B" } },
    selectedAgentKey: "embedded:a",
  });
  const chat = c.selectedChat();
  assert.equal(chat.key, "embedded:a");
  assert.equal(chat.candidate.recordId, "a");
  assert.equal(chat.record, recordA);
  c.selectedAgentKey = "embedded:b";
  assert.deepEqual(
    { ...c.selectedChat() },
    { key: "embedded:b", candidate: null, record: null },
  );
});
test("a record key selects a saved chat without a candidate", () => {
  const c = workspace({ candidates: [], selectedAgentKey: "record:b" });
  assert.deepEqual(
    { ...c.selectedChat() },
    { key: "record:b", candidate: null, record: recordB },
  );
  c.selectedAgentKey = "record:gone";
  assert.equal(c.selectedChat().record, null);
});
test("the selected chat and the diff poll tolerate a missing state before the first load", () => {
  const c = workspace({
    candidates: [{ host: "embedded", recordId: "a", agent: "claude" }],
    sessions: null,
    source: "agents",
    selectedAgentKey: "record:a",
    files: ["static/git-diff.js", "static/diff-review.js"],
  });
  assert.equal(c.selectedChat().record, null);
  c.syncActiveDiff();
  c.selectedAgentKey = "embedded:a";
  assert.equal(c.selectedChat().record, null);
  assert.equal(c.selectedChat().candidate.recordId, "a");
});
