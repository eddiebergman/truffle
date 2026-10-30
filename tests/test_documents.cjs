"use strict";
const { test } = require("node:test"),
  assert = require("node:assert/strict");
const {
  checkedPatch,
  documentStore,
  validDocumentIdentity,
} = require("../static/documents.js");

const hash = (letter) => letter.repeat(64);
function fixture(write) {
  let cached = { revision: 1, contentHash: hash("a"), value: "saved" },
    pendingRead;
  const writes = [];
  const store = documentStore({
    get: () => cached,
    set: (next) => {
      cached = next;
    },
    read: () =>
      new Promise((resolve) => {
        pendingRead = resolve;
      }),
    write: async (body) => {
      writes.push(body);
      return write(body);
    },
    validate: (value) => {
      if (!validDocumentIdentity(value)) {
        throw new Error("Unsupported document.");
      }
      return value;
    },
  });
  return {
    store,
    writes,
    cached: () => cached,
    resolveRead: (value) => pendingRead(value),
  };
}
const failure = (status) =>
  Object.assign(new Error(`Failed with ${status}.`), { status });

test("documentStore save outcomes and their effect on an earlier load", async () => {
  const saved = { revision: 2, contentHash: hash("b"), value: "new" };
  for (const [name, write, kind, earlierLoadCommits] of [
    ["accepted", () => saved, "accepted", false],
    ["conflict", () => Promise.reject(failure(409)), "conflict", true],
    ["rejected 400", () => Promise.reject(failure(400)), "rejected", true],
    ["rejected 403", () => Promise.reject(failure(403)), "rejected", true],
    ["rejected 404", () => Promise.reject(failure(404)), "rejected", true],
    [
      "unknown network",
      () => Promise.reject(new Error("Offline.")),
      "unknown",
      false,
    ],
    ["unknown 500", () => Promise.reject(failure(500)), "unknown", false],
    ["unknown 503", () => Promise.reject(failure(503)), "unknown", false],
    ["unknown invalid response", () => ({ value: "new" }), "unknown", false],
  ]) {
    const f = fixture(write),
      baseline = f.store.baseline(),
      before = f.cached(),
      load = f.store.load(),
      result = await f.store.save(baseline, { value: "new" });
    assert.equal(result.kind, kind, name);
    assert.deepEqual(
      f.writes,
      [{ revision: 1, contentHash: hash("a"), value: "new" }],
      name,
    );
    if (kind === "accepted") {
      assert.equal(result.document, saved, name);
      assert.equal(f.cached(), saved, name);
      assert.deepEqual(
        f.store.baseline(),
        { revision: 2, contentHash: hash("b") },
        name,
      );
    } else {
      assert.equal(typeof result.error, "string", name);
      assert.equal(f.cached(), before, name);
    }
    const latest = { revision: 3, contentHash: hash("c"), value: "read" };
    f.resolveRead(latest);
    assert.equal(await load, earlierLoadCommits ? latest : null, name);
    assert.equal(f.cached() === latest, earlierLoadCommits, name);
  }
});

test("documentStore load commits only a valid document that the caller still wants", async () => {
  const f = fixture(() => {
    throw new Error("No writes expected.");
  });
  const declined = f.store.load({ canApply: () => false });
  f.resolveRead({ revision: 2, contentHash: hash("b") });
  assert.equal(await declined, null);
  assert.equal(f.cached().revision, 1);
  const invalid = f.store.load();
  f.resolveRead({ revision: -1, contentHash: hash("b") });
  await assert.rejects(invalid, /Unsupported document/);
  assert.equal(f.cached().revision, 1);
});

test("document identity needs a non-negative revision and a 64-hex content hash", () => {
  assert.equal(
    validDocumentIdentity({ revision: 0, contentHash: hash("f") }),
    true,
  );
  for (const value of [
    null,
    { revision: -1, contentHash: hash("f") },
    { revision: 1.5, contentHash: hash("f") },
    { revision: 1, contentHash: hash("F") },
    { revision: 1, contentHash: "f".repeat(63) },
    { revision: 1 },
  ]) {
    assert.equal(validDocumentIdentity(value), false, JSON.stringify(value));
  }
});

test("checked patch sends changed fields with their read tokens and compares arrays by value", () => {
  const baseline = {
    values: { title: "Saved", enabled: true, count: 5, itemIds: ["a", "b"] },
    expected: {
      title: hash("1"),
      enabled: hash("2"),
      count: hash("3"),
      itemIds: hash("4"),
    },
  };
  const fields = ["title", "enabled", "count", "itemIds"];
  assert.deepEqual(
    checkedPatch(
      baseline,
      { title: "Saved", enabled: true, count: 5, itemIds: ["a", "b"] },
      fields,
    ),
    { patch: {}, expected: {} },
  );
  assert.deepEqual(
    checkedPatch(
      baseline,
      { title: "New", enabled: true, count: 5, itemIds: ["b", "a"] },
      fields,
    ),
    {
      patch: { title: "New", itemIds: ["b", "a"] },
      expected: { title: hash("1"), itemIds: hash("4") },
    },
  );
  assert.deepEqual(
    checkedPatch(
      baseline,
      { title: "New", enabled: false, count: 6, itemIds: ["a"] },
      ["enabled", "count", "itemIds"],
    ),
    {
      patch: { enabled: false, count: 6, itemIds: ["a"] },
      expected: { enabled: hash("2"), count: hash("3"), itemIds: hash("4") },
    },
  );
});
