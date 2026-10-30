const { test } = require("node:test"),
  assert = require("node:assert/strict");
const { browserContext, loadScripts } = require("./helpers/static-script.cjs"),
  { FakeDocument, FakeStorage } = require("./helpers/fake-dom.cjs");
// Load app.js with recording stubs for render and the saved-view owner. The fake
// document only serves the form controls whose values persist() reads.
function navigation({ savedViewId = "" } = {}) {
  const calls = [],
    window = new EventTarget(),
    localStorage = new FakeStorage();
  const c = browserContext({
    window,
    CustomEvent,
    document: new FakeDocument(),
    localStorage,
    source: "agents",
    render: () => calls.push(["render", c.source]),
    currentSavedViewState: (page) => {
      calls.push(["state", page]);
      return { id: savedViewId };
    },
    resetSavedViewState: (page) => calls.push(["reset", page]),
    savedViewUsePage: (page) => calls.push(["use", page]),
  });
  loadScripts(c, ["static/client.js", "static/documents.js", "static/app.js"]);
  window.addEventListener("truffle-navigate", (event) =>
    calls.push(["event", event.detail.from, event.detail.to]),
  );
  const stored = () => JSON.parse(localStorage.getItem("truffle-view"));
  return { c, calls, stored };
}
test("navigate resets an active saved view, persists, renders and then reports the page change", () => {
  const f = navigation({ savedViewId: "standup" });
  f.c.navigate("github", { savedView: "reset" });
  assert.deepEqual(f.calls, [
    ["state", "github"],
    ["reset", "github"],
    ["use", "github"],
    ["render", "github"],
    ["event", "agents", "github"],
  ]);
  assert.equal(f.stored().source, "github");
});
test("navigate keeps the saved view unless asked to reset it and rejects unknown policies", () => {
  const f = navigation({ savedViewId: "standup" });
  f.c.navigate("linear", { savedView: "keep" });
  assert.deepEqual(f.calls, [
    ["use", "linear"],
    ["render", "linear"],
    ["event", "agents", "linear"],
  ]);
  f.calls.length = 0;
  assert.throws(
    () => f.c.navigate("github", { savedView: "clear" }),
    /Unknown saved view policy: clear/,
  );
  assert.equal(f.c.source, "linear");
  assert.deepEqual(f.calls, []);
});
test("reset leaves a page without an active saved view untouched", () => {
  const f = navigation();
  f.c.navigate("all", { savedView: "reset" });
  assert.deepEqual(f.calls, [
    ["state", "all"],
    ["use", "all"],
    ["render", "all"],
    ["event", "agents", "all"],
  ]);
});
test("persist stores the preferences that each owner registers", () => {
  const f = navigation();
  let collapsed = true;
  f.c.registerPersistedPrefs(() => ({ ownerCollapsed: collapsed }));
  f.c.source = "sessions";
  f.c.persist();
  assert.equal(f.stored().source, "sessions");
  assert.equal(f.stored().ownerCollapsed, true);
  collapsed = false;
  f.c.persist();
  assert.equal(f.stored().ownerCollapsed, false);
  assert.equal(Object.hasOwn(f.stored(), "selectedAgentKey"), false);
});
