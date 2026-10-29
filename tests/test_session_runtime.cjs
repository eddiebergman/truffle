const { test } = require("node:test"),
  assert = require("node:assert/strict");
const { browserContext, loadScripts } = require("./helpers/static-script.cjs");
const uuid = "10000000-0000-7000-8000-000000000001",
  otherUUID = "20000000-0000-4000-8000-000000000002";
function runtime(candidates, session) {
  const c = browserContext({ inventory: { candidates } });
  loadScripts(c, ["static/client.js", "static/app.js", "static/sessions.js"]);
  return JSON.parse(
    JSON.stringify(
      c.sessionRuntime({
        id: "a",
        agent: "codex",
        destination: "embedded",
        launchNew: false,
        ...session,
      }),
    ),
  );
}
test("unregistered same-agent chats report only the lifecycle of their own stable record", () => {
  const candidates = [
    { recordId: "a", agent: "codex", sessionId: null, lifecycle: "running" },
    { recordId: "b", agent: "codex", sessionId: null, lifecycle: "needsInput" },
  ];
  assert.deepEqual(runtime(candidates, { id: "a", sessionId: null }), {
    label: "Running",
    value: "running",
  });
  assert.deepEqual(runtime(candidates, { id: "b", sessionId: null }), {
    label: "Needs input",
    value: "needsInput",
  });
  assert.deepEqual(runtime(candidates, { id: "unrelated", sessionId: null }), {
    label: "Awaiting chat link",
    value: "unknown",
  });
});
test("null undefined empty and malformed conversation identities never correlate unrelated candidates", () => {
  for (const sessionId of [null, undefined, "", "not-a-uuid"]) {
    for (const candidateId of [null, undefined, "", "not-a-uuid"]) {
      const result = runtime(
        [{ agent: "codex", sessionId: candidateId, lifecycle: "running" }],
        { sessionId },
      );
      assert.equal(result.value, "unknown");
      assert.notEqual(result.label, "Running");
    }
  }
});
test("exact record identity outranks stale UUID and legacy fallback in any inventory ordering", () => {
  const exact = {
      recordId: "a",
      agent: "codex",
      sessionId: otherUUID,
      lifecycle: "idle",
    },
    fallback = { agent: "codex", sessionId: uuid, lifecycle: "running" };
  for (const candidates of [
    [exact, fallback],
    [fallback, exact],
  ]) {
    assert.deepEqual(runtime(candidates, { sessionId: uuid }), {
      label: "Idle",
      value: "idle",
    });
  }
});
test("unassociated external candidates can match a valid UUID and agent but conflicting record ownership cannot", () => {
  for (const recordId of [null, undefined, ""]) {
    assert.deepEqual(
      runtime(
        [{ recordId, agent: "codex", sessionId: uuid, lifecycle: "closed" }],
        { sessionId: uuid },
      ),
      { label: "Closed", value: "closed" },
    );
  }
  for (const candidate of [
    { recordId: "b", agent: "codex", sessionId: uuid, lifecycle: "running" },
    { agent: "claude", sessionId: uuid, lifecycle: "running" },
  ]) {
    assert.deepEqual(runtime([candidate], { sessionId: uuid }), {
      label: "Saved session",
      value: "unknown",
    });
  }
});
test("ambiguous exact or fallback candidates and malformed lifecycle report Unknown instead of the first status", () => {
  for (const candidates of [
    [
      { recordId: "a", lifecycle: "running" },
      { recordId: "a", lifecycle: "closed" },
    ],
    [
      { agent: "codex", sessionId: uuid, lifecycle: "idle" },
      { agent: "codex", sessionId: uuid, lifecycle: "needsInput" },
    ],
    [{ recordId: "a", lifecycle: "unexpected" }],
    [{ recordId: "a" }],
  ]) {
    assert.deepEqual(runtime(candidates, { sessionId: uuid }), {
      label: "Unknown",
      value: "unknown",
    });
  }
});
test("absence preserves saved launch state without using unrelated runtime evidence", () => {
  assert.deepEqual(runtime([], { sessionId: null, launchNew: true }), {
    label: "Not started",
    value: "unknown",
  });
  assert.deepEqual(runtime([], { sessionId: uuid }), {
    label: "Saved session",
    value: "unknown",
  });
  assert.deepEqual(runtime([], { sessionId: null, destination: "cmux" }), {
    label: "Linking chat",
    value: "unknown",
  });
});

test("exact record with contradictory agent is Unknown and cannot borrow an otherwise valid UUID fallback", () => {
  const contradictory = {
      recordId: "a",
      agent: "claude",
      sessionId: otherUUID,
      lifecycle: "running",
    },
    fallback = { agent: "codex", sessionId: uuid, lifecycle: "idle" };
  for (const candidates of [
    [contradictory],
    [contradictory, fallback],
    [fallback, contradictory],
  ]) {
    assert.deepEqual(runtime(candidates, { sessionId: uuid }), {
      label: "Unknown",
      value: "unknown",
    });
  }
});
