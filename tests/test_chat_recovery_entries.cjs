const { test } = require("node:test"),
  assert = require("node:assert/strict"),
  { join } = require("node:path");
const { evaluate } = require(join(__dirname, "helpers/static-script.cjs")),
  { scriptFixture: base, loadMore } = require(
    join(__dirname, "helpers/feature-fixture.cjs"),
  );
function fixture(context = { availability: "available", total: 2 }) {
  const f = base(),
    opened = [];
  let current = context;
  Object.assign(f.c, {
    source: "sessions",
    historyArchived: true,
    currentAgentRecord: () => null,
    candidateForRecord: () => null,
    workSummary: () => "",
    savedSessionOpenState: () => null,
    chatPreparationContext: () => current,
    shortcutLabel: (value) => value || "",
    setting: (key, fallback) => fallback,
    openSettings() {},
    liveCandidates: () => [],
    snoozedAgentTabs: {},
    closedAgentTabs: [],
    leftSidebarCollapsed: false,
    agentSidebarCollapsed: false,
    openChatRecovery: () => {
      opened.push("recovery");
      const dialog = f.$("#chat-recovery-list");
      dialog.tagName = "dialog";
      dialog.showModal();
      dialog.focus();
    },
    prepareSavedChat: () => assert.fail("entry must not prepare"),
    openSavedSession: () => assert.fail("entry must not open a saved chat"),
    api: () => assert.fail("entry must not make requests"),
  });
  f.$("#search").value = "Scoped search";
  f.c.scopePrefs.sessions = "preserved";
  f.c.state.sessions = [];
  // The fixture omits sessions.js, so stub what workspace.js reads from it.
  // The palette close handler that command-palette.js attaches also clears attributes.
  f.Node.prototype.removeAttribute = function (key) {
    delete this.attrs[key];
  };
  Object.assign(f.c, { prefs: {}, itemSessionChip() {} });
  loadMore(f, ["static/workspace.js", "static/command-palette.js"]);
  return {
    ...f,
    opened,
    setContext: (value) => (current = value),
    historyButton: () =>
      f
        .$("#feature-controls")
        .querySelectorAll("button")
        .find((node) => node.dataset.historyAction === "recovery"),
    archive: () =>
      f
        .$("#feature-controls")
        .querySelectorAll("button")
        .find((node) => node.dataset.historyAction === "archive"),
    entry: () =>
      f.c
        .commandEntries()
        .find((entry) => entry.title === "open: chat recovery"),
  };
}
function assertContext(f) {
  assert.equal(f.c.source, "sessions");
  assert.equal(f.$("#search").value, "Scoped search");
  assert.equal(f.c.historyArchived, true);
  assert.equal(f.c.scopePrefs.sessions, "preserved");
  assert.equal(f.calls.length, 0);
}
test("loaded History header exposes bounded pending count or unavailable state and only navigates", () => {
  for (const context of [
    { availability: "available", total: 64 },
    { availability: "unavailable" },
  ]) {
    const f = fixture(context);
    f.c.renderHistory();
    const button = f.historyButton();
    assert.ok(button);
    assert.equal(f.$("#feature-controls").hidden, false);
    assert.equal(
      f.$("#feature-controls").querySelector(".ui-actiongroup").attrs[
        "aria-label"
      ],
      "History actions",
    );
    assert.equal(button.className, "ui-control");
    assert.equal(button.dataset.variant, "secondary");
    assert.equal(
      button.textContent,
      context.total
        ? "Pending chat requests · 64"
        : "Pending chat requests · Unavailable",
    );
    button.onclick();
    assert.deepEqual(f.opened, ["recovery"]);
    assertContext(f);
  }
});
test("zero pending requests leave both History and palette entry absent without changing filters", () => {
  const f = fixture({ availability: "available", total: 0 });
  f.c.renderHistory();
  assert.equal(f.historyButton(), undefined);
  assert.equal(f.entry(), undefined);
  assert.ok(f.archive());
  assertContext(f);
});
test("History recovery count repaint restores owned focus and never takes outside focus", () => {
  const f = fixture();
  f.c.renderHistory();
  f.historyButton().focus();
  f.setContext({ availability: "available", total: 3 });
  f.c.renderHistory();
  assert.equal(f.document.activeElement, f.historyButton());
  assert.match(f.historyButton().textContent, /3$/);
  const outside = new f.Node("input");
  f.document.body.append(outside);
  outside.focus();
  f.setContext({ availability: "unavailable" });
  f.c.renderHistory();
  assert.equal(f.document.activeElement, outside);
  f.historyButton().focus();
  f.setContext({ availability: "available", total: 0 });
  f.c.renderHistory();
  assert.equal(f.document.activeElement, f.archive());
  assert.deepEqual(f.opened, []);
  assertContext(f);
});
test("History unrelated-modal veto reports owner error without closing or inspecting a request", () => {
  const f = fixture(),
    modal = f.$("#unrelated");
  modal.tagName = "dialog";
  modal.showModal();
  f.c.openChatRecovery = () => {
    throw Error("Close the current dialog before opening chat recovery.");
  };
  f.c.renderHistory();
  f.historyButton().onclick();
  assert.equal(modal.open, true);
  assert.deepEqual(f.messages, [
    "Close the current dialog before opening chat recovery.",
  ]);
  assert.deepEqual(f.opened, []);
  assertContext(f);
});
test("exact recovery palette command has no shortcut and closes its own palette before pure navigation", async () => {
  for (const context of [
    { availability: "available", total: 1 },
    { availability: "unavailable" },
  ]) {
    const f = fixture(context),
      dialog = f.$("#action-palette");
    dialog.tagName = "dialog";
    dialog.showModal();
    f.c.openChatRecovery = () => {
      assert.equal(dialog.open, false);
      f.opened.push("recovery");
    };
    const entry = f.entry();
    assert.ok(entry);
    assert.equal(entry.shortcut, undefined);
    assert.match(
      entry.detail,
      context.total ? /1 pending local chat request/ : /status unavailable/,
    );
    evaluate(f.c, "actionEntries=commandEntries()");
    const index = evaluate(
      f.c,
      "actionEntries.findIndex(entry=>entry.title==='open: chat recovery')",
    );
    await f.c.runPaletteCommand(index);
    assert.deepEqual(f.opened, ["recovery"]);
    assertContext(f);
  }
});
