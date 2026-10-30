const { test } = require("node:test"),
  assert = require("node:assert/strict"),
  fs = require("node:fs");
const {
    browserContext,
    loadScripts,
    evaluate,
  } = require("./helpers/static-script.cjs"),
  { FakeDocument, browserHost } = require("./helpers/fake-dom.cjs");
const deferred = () => {
  let resolve, reject;
  const promise = new Promise((a, b) => {
    resolve = a;
    reject = b;
  });
  return { promise, resolve, reject };
};
// Static scripts in index.html order, up to and with `file`.
const pageScripts = [
  ...fs
    .readFileSync("static/index.html", "utf8")
    .matchAll(/<script src="\/([^"]+)"/g),
]
  .map((match) => `static/${match[1]}`)
  .filter(
    (file) =>
      !file.startsWith("static/vendor/xterm") &&
      file !== "static/vendor/addon-fit.js",
  );
const scriptsThrough = (file) =>
  pageScripts.slice(0, pageScripts.findIndex((name) => name === file) + 1);
test("context commands retain separate source links and checkout targets for multiple PRs", () => {
  const opened = [],
    reviews = [];
  const items = [
    {
      id: "a",
      source: "github",
      reference: "repo#1",
      title: "API change",
      url: "https://github.com/o/repo/pull/1",
    },
    {
      id: "b",
      source: "github",
      reference: "repo#2",
      title: "Worker change",
      url: "https://github.com/o/repo/pull/2",
    },
    {
      id: "c",
      source: "linear",
      reference: "ENG-1",
      title: "Implement API",
      url: "https://linear.app/team/issue/ENG-1",
    },
  ];
  const record = {
    id: "chat",
    title: "Two PRs",
    cwd: "/repo",
    items,
    resources: [],
  };
  const c = browserContext({
    chatPreparationContext: () => ({ availability: "available", total: 0 }),
    shortcutLabel: (value) => value || "",
    setting: (key, fallback) => fallback,
    openSettings() {},
    source: "agents",
    selectedChat: () => ({ key: "record:chat", candidate: null, record }),
    candidateForRecord: () => null,
    agentWork: () => items,
    workReference: (x) => x.reference,
    gitDiffDirectories: () => [
      { path: "/repo/first", branch: "api", itemIds: ["a"] },
      { path: "/repo/second", branch: "worker", itemIds: ["b"] },
    ],
    artifactLibrary: null,
    state: { artifacts: [] },
    liveCandidates: () => [],
    snoozedAgentTabs: {},
    closedAgentTabs: [],
    leftSidebarCollapsed: false,
    agentSidebarCollapsed: false,
    openCommandLink: (url) => opened.push(url),
    selectAgent() {},
    openDiffReview: (r, file, path) => reviews.push(path),
  });
  loadScripts(c, scriptsThrough("static/command-palette.js"));
  const entries = c.commandEntries(),
    github = entries.filter((x) => x.title.startsWith("open: github"));
  assert.equal(github.length, 2);
  github[1].run();
  assert.deepEqual(opened, [items[1].url]);
  assert.equal(
    entries.filter((x) => x.title.startsWith("open: linear")).length,
    1,
  );
  for (const entry of entries.filter((x) =>
    x.title.startsWith("review: changes"),
  )) {
    entry.run();
  }
  assert.deepEqual(reviews, ["/repo/first", "/repo/second"]);
});

test("refresh Claude closes only the selected terminal generation and resumes the same saved conversation", async () => {
  const calls = [],
    record = {
      id: "chat",
      agent: "claude",
      sessionId: "saved-conversation",
      cwd: "/repo/.worktrees/task",
    };
  let candidate = {
    host: "embedded",
    recordId: "chat",
    generation: "old-process",
  };
  const c = browserContext({
    selectedAgentKey: "embedded:chat",
    loadInventory: async () => {},
    candidateForRecord: () => candidate,
    candidateKey: (entry) => `embedded:${entry.recordId}`,
    api: async (route, payload) => {
      calls.push({ route, payload });
      candidate = null;
    },
    openSavedSession: async (r) => {
      assert.equal(r, record);
      candidate = {
        host: "embedded",
        recordId: "chat",
        generation: "new-process",
      };
      calls.push({ resume: r.sessionId, cwd: r.cwd });
    },
    selectAgent: (r, selected) =>
      calls.push({ selected: r.id, generation: selected.generation }),
  });
  loadScripts(c, scriptsThrough("static/command-palette.js"));
  await c.refreshClaudeSession(record);
  assert.equal(calls[0].route, "/api/terminal/close");
  assert.equal(calls[0].payload.id, "chat");
  assert.equal(calls[0].payload.generation, "old-process");
  assert.equal(calls[1].resume, "saved-conversation");
  assert.equal(calls[1].cwd, record.cwd);
  assert.equal(calls[2].generation, "new-process");
  await assert.rejects(
    c.refreshClaudeSession({ ...record, sessionId: "" }),
    /register/,
  );
  assert.equal(calls.length, 3);
});

function paletteFixture({
  through = "static/command-palette.js",
  actualEntries = false,
} = {}) {
  const nodes = new Map(),
    capture = [],
    calls = [];
  let active = null,
    entryBuilds = 0;
  class Node {
    constructor(tag = "div", className = "", text = "") {
      this.tagName = tag.toUpperCase();
      this.className = className;
      this.textContent = text;
      this.children = [];
      this.attrs = {};
      this.listeners = {};
      this.value = "";
      this.scrollTop = 0;
      this.classList = {
        toggle: (name, on) => {
          const names = new Set(this.className.split(" ").filter(Boolean));
          if (on) {
            names.add(name);
          } else {
            names.delete(name);
          }
          this.className = [...names].join(" ");
        },
      };
    }
    append(...children) {
      this.children.push(...children);
    }
    replaceChildren(...children) {
      this.children = children;
    }
    setAttribute(name, value) {
      this.attrs[name] = value;
    }
    removeAttribute(name) {
      delete this.attrs[name];
    }
    querySelectorAll(selector) {
      return this.children.filter(
        (child) =>
          selector === ".action-command" &&
          child.className.split(" ").includes("action-command"),
      );
    }
    addEventListener(name, handler) {
      (this.listeners[name] ??= []).push(handler);
    }
    dispatchEvent(event) {
      for (const handler of this.listeners[event.type] || []) {
        handler(event);
      }
      return !event.defaultPrevented;
    }
    closest(selector) {
      if (selector === ".action-command") {
        return this.className.split(" ").includes("action-command")
          ? this
          : null;
      }
      if (selector === "dialog[open]") {
        return nodes.get("#action-palette").open
          ? nodes.get("#action-palette")
          : null;
      }
      return null;
    }
    showModal() {
      this.open = true;
    }
    close() {
      this.open = false;
      for (const handler of this.listeners.close || []) {
        handler();
      }
    }
    focus() {
      active = this;
      this.onfocus?.();
    }
    scrollIntoView(options) {
      this.scrolled = options;
    }
  }
  const $ = (selector) => {
    if (!nodes.has(selector)) {
      nodes.set(
        selector,
        new Node(selector === "#action-query" ? "input" : "div"),
      );
    }
    return nodes.get(selector);
  };
  const entries = [
    {
      title: "change: theme",
      detail: "Choose a light or dark theme",
      run: () => calls.push("theme"),
    },
    {
      title: "switch: agent",
      detail: "The host uses memory elsewhere",
      run: () => calls.push("agent"),
    },
    {
      title: "new: codex chat",
      detail: "Choose a local folder",
      run: () => calls.push("codex"),
    },
  ];
  const document = {
    querySelector: (selector) =>
      selector === "dialog[open]"
        ? $("#action-palette").open
          ? $("#action-palette")
          : null
        : $(selector),
    querySelectorAll: () => [...nodes.values()].filter((node) => node.open),
  };
  // Scripts load against a fake document that keeps only capturing keydown listeners, and attach the palette handlers to this fixture's nodes.
  const loadDocument = new FakeDocument();
  loadDocument.addEventListener = (name, handler, useCapture) => {
    if (name === "keydown" && useCapture) {
      capture.push(handler);
    }
  };
  for (const id of ["#action-query", "#action-palette"]) {
    loadDocument.elements.set(id, $(id));
  }
  const c = browserContext({
    Event,
    document: loadDocument,
    navigator: { platform: "MacIntel" },
    state: {},
    appSettings: {},
    source: "all",
    cycleAgentTab: () => false,
    selectedChat: () => ({ key: "", candidate: null, record: null }),
    ...(actualEntries
      ? {}
      : {
          commandEntries: () => {
            entryBuilds++;
            return entries;
          },
        }),
    featureButton: (text, run, cls) => {
      const node = new Node("button", cls, text);
      node.onclick = run;
      return node;
    },
    el: (tag, cls, text) => new Node(tag, cls, text),
    toast: (error) => {
      throw new Error(error);
    },
  });
  loadScripts(c, scriptsThrough(through), { keepHost: ["document"] });
  c.document = document;
  function key(name, extra = {}) {
    const event = {
      key: name,
      target: active,
      isComposing: false,
      preventDefault() {
        this.defaultPrevented = true;
      },
      stopPropagation() {
        this.stopped = true;
      },
      stopImmediatePropagation() {
        this.stopped = true;
      },
      ...extra,
    };
    for (const handler of capture) {
      handler(event);
      if (event.stopped) {
        return event;
      }
    }
    for (const handler of $("#action-palette").listeners.keydown || []) {
      handler(event);
      if (event.stopped) {
        break;
      }
    }
    return event;
  }
  function query(value, extra = {}) {
    $("#action-query").value = value;
    $("#action-query").oninput(extra);
  }
  return {
    c,
    $,
    key,
    query,
    calls,
    entries,
    get active() {
      return active;
    },
    get entryBuilds() {
      return entryBuilds;
    },
    rows: () => $("#action-results").querySelectorAll(".action-command"),
  };
}

test("command arrows keep query focus and row identities through document capture handlers", () => {
  const f = paletteFixture();
  f.c.openCommands();
  const rows = f.rows();
  assert.equal(rows[0].attrs["aria-selected"], "true");
  assert.equal(f.key("ArrowDown").defaultPrevented, true);
  assert.equal(f.rows()[1], rows[1]);
  assert.equal(f.rows()[1].attrs["aria-selected"], "true");
  assert.equal(f.$("#action-query").attrs["aria-activedescendant"], rows[1].id);
  assert.equal(f.active, f.$("#action-query"));
  assert.equal(rows[1].scrolled.block, "nearest");
  assert.equal(f.entryBuilds, 1);
  f.key("ArrowUp");
  assert.equal(rows[0].attrs["aria-selected"], "true");
  f.key("ArrowUp");
  assert.equal(rows[2].attrs["aria-selected"], "true");
  f.key("ArrowDown");
  assert.equal(rows[0].attrs["aria-selected"], "true");
});

test("exact command words exclude unrelated fuzzy matches and editing resets selection", () => {
  const f = paletteFixture();
  f.c.openCommands();
  f.key("ArrowDown");
  f.$("#action-results").scrollTop = 100;
  f.query("theme");
  assert.equal(f.rows().length, 1);
  assert.equal(
    f.rows()[0].children[0].children[0].textContent,
    "change: theme",
  );
  assert.equal(f.rows()[0].attrs["aria-selected"], "true");
  assert.equal(f.$("#action-results").scrollTop, 0);
  f.query("ch thm");
  assert.ok(f.rows().length > 0);
  assert.equal(
    f.rows()[0].children[0].children[0].textContent,
    "change: theme",
  );
  f.query("zzzz");
  assert.equal(f.rows().length, 0);
  assert.equal(f.$("#action-query").attrs["aria-activedescendant"], undefined);
  f.key("ArrowDown");
  f.key("ArrowUp");
  f.key("Enter");
  assert.deepEqual(f.calls, []);
  assert.equal(f.$("#action-palette").open, true);
  f.query("codex");
  f.key("Enter");
  assert.deepEqual(f.calls, ["codex"]);
  assert.equal(f.$("#action-palette").open, false);
  assert.equal(f.$("#action-query").value, "");
  assert.equal(f.$("#action-query").attrs["aria-expanded"], "false");
  f.c.openCommands();
  assert.equal(f.rows()[0].attrs["aria-selected"], "true");
  assert.equal(f.rows().length, 3);
});

test("IME and modified arrows keep editing behavior, Escape uses the shared dialog handler", () => {
  const f = paletteFixture();
  f.c.openCommands();
  const rows = f.rows();
  for (const extra of [
    { isComposing: true },
    { keyCode: 229 },
    { shiftKey: true },
    { altKey: true },
    { metaKey: true },
    { ctrlKey: true },
  ]) {
    assert.equal(f.key("ArrowDown", extra).defaultPrevented, undefined);
    assert.equal(rows[0].attrs["aria-selected"], "true");
  }
  f.key("Enter", { isComposing: true });
  assert.deepEqual(f.calls, []);
  f.query("theme", { isComposing: true });
  assert.equal(f.rows()[0], rows[0]);
  for (const handler of f.$("#action-query").listeners.compositionend) {
    handler();
  }
  assert.equal(f.rows().length, 1);
  f.key("Escape", { isComposing: true });
  assert.equal(f.$("#action-palette").open, true);
  let blockCancel = true;
  f.$("#action-palette").addEventListener("cancel", (event) => {
    if (blockCancel) {
      event.preventDefault();
    }
  });
  f.key("Escape");
  assert.equal(f.$("#action-palette").open, true);
  blockCancel = false;
  f.key("Escape");
  assert.equal(f.$("#action-palette").open, false);
  assert.equal(f.$("#action-query").value, "");
  assert.equal(f.$("#action-query").attrs["aria-activedescendant"], undefined);
});

test("focused command Enter runs that row while close control Enter keeps its native action", () => {
  const f = paletteFixture();
  f.c.openCommands();
  f.rows()[2].focus();
  f.key("Enter");
  assert.deepEqual(f.calls, ["codex"]);
  f.c.openCommands();
  const close = f.$("#close");
  close.focus();
  assert.equal(f.key("Enter").defaultPrevented, undefined);
  assert.equal(f.$("#action-palette").open, true);
  assert.deepEqual(f.calls, ["codex"]);
});

test("configured Cmd+K honors the actual saving filter-editor cancel guard and opens only after cancellation is allowed", async () => {
  const f = paletteFixture({ through: "static/settings.js" }),
    dialog = f.$("#saved-view-editor"),
    field = f.$("#filter-field"),
    reload = deferred(),
    doc = new FakeDocument(),
    el = f.c.el;
  const viewState = {
    id: "",
    draft: {
      parent: "github",
      lifecycle: "active",
      filter: { op: "all", conditions: [] },
      query: "",
      groupBy: "status",
    },
  };
  Object.assign(f.c, {
    settingsDirty: false,
    settingsSaving: false,
    chordFromEvent: (event) =>
      event.metaKey && event.key === "k" ? "Primary+K" : "",
    appSettings: { values: { keybindings: { commands: "Primary+K" } } },
    currentSavedViewState: () => viewState,
    api: () => reload.promise,
    Option: browserHost().Option,
  });
  loadScripts(f.c, "static/views.js");
  // The real editor opens on the fixture dialog, so the palette sees it open; its contents are tolerant fake elements.
  Object.assign(dialog, { isConnected: true, remove() {} });
  f.c.document.body = doc.body;
  f.c.el = (tag, cls, text) =>
    tag === "dialog"
      ? dialog
      : Object.assign(doc.createElement(tag), {
          className: cls || "",
          textContent: text ?? "",
        });
  try {
    f.c.openSavedViewEditor();
  } finally {
    f.c.el = el;
  }
  // A pending Reload views holds the editor's saving guard.
  dialog.children
    .find((node) =>
      String(node.className).includes("saved-view-editor-actions"),
    )
    .children.find((node) => node.textContent === "Reload views")
    .onclick();
  field.focus();
  f.$("#action-query").value = "Preserve pending query";
  f.key("k", { metaKey: true });
  assert.equal(dialog.open, true);
  assert.notEqual(f.$("#action-palette").open, true);
  assert.equal(f.entryBuilds, 0);
  assert.equal(f.active, field);
  assert.equal(f.$("#action-query").value, "Preserve pending query");
  reload.reject(Error("Reload failed"));
  await new Promise((resolve) => setImmediate(resolve));
  f.key("k", { metaKey: true });
  assert.equal(dialog.open, false);
  assert.equal(f.$("#action-palette").open, true);
  assert.equal(f.entryBuilds, 1);
  assert.equal(f.active, f.$("#action-query"));
  assert.equal(f.$("#action-query").value, "");
});

test("palette dismisses topmost dialogs first and a veto leaves lower dialogs untouched", () => {
  const f = paletteFixture(),
    parent = f.$("#parent-dialog"),
    child = f.$("#child-dialog"),
    order = [];
  parent.open = child.open = true;
  parent.addEventListener("cancel", () => order.push("parent"));
  child.addEventListener("cancel", (event) => {
    order.push("child");
    event.preventDefault();
  });
  f.c.openCommands();
  assert.deepEqual(order, ["child"]);
  assert.equal(parent.open, true);
  assert.equal(child.open, true);
  assert.notEqual(f.$("#action-palette").open, true);
});

test("an allowed child dismissal still aborts palette opening when its parent vetoes", () => {
  const f = paletteFixture(),
    parent = f.$("#parent-dialog"),
    child = f.$("#child-dialog"),
    order = [];
  parent.open = child.open = true;
  let block = true;
  parent.addEventListener("cancel", (event) => {
    order.push("parent");
    if (block) {
      event.preventDefault();
    }
  });
  child.addEventListener("cancel", () => order.push("child"));
  f.c.openCommands();
  assert.deepEqual(order, ["child", "parent"]);
  assert.equal(child.open, false);
  assert.equal(parent.open, true);
  assert.notEqual(f.$("#action-palette").open, true);
  assert.equal(f.entryBuilds, 0);
  block = false;
  f.c.openCommands();
  assert.equal(parent.open, false);
  assert.equal(f.$("#action-palette").open, true);
  assert.equal(f.entryBuilds, 1);
});

test("go Agents uses the explicit workspace owner while other pages reset their saved view", () => {
  const events = [],
    state = { id: "captured-view" },
    c = browserContext({
      window: new EventTarget(),
      CustomEvent,
      source: "github",
      selectedAgentKey: "",
      chatPreparationContext: () => ({ availability: "available", total: 0 }),
      selectedChat: () => ({ key: "", candidate: null, record: null }),
      candidateForRecord: () => null,
      liveCandidates: () => [],
      snoozedAgentTabs: {},
      closedAgentTabs: [],
      leftSidebarCollapsed: false,
      agentSidebarCollapsed: false,
      state: { artifacts: [] },
      shortcutLabel: (value) => value || "",
      setting: (key, fallback) => fallback,
      openSettings() {},
      currentSavedViewState: () => state,
      resetSavedViewState: () => {
        events.push("reset");
        state.id = null;
      },
      savedViewUsePage: (view) => events.push(`page:${view}`),
      showAgentWorkspace: () => {
        events.push("workspace-owner");
        c.selectedAgentKey = "embedded:first-live";
      },
      persist: () => events.push("persist"),
      render: () => events.push("render"),
    });
  loadScripts(c, scriptsThrough("static/command-palette.js"));
  c.commandEntries()
    .find((entry) => entry.title === "go: agents")
    .run();
  assert.deepEqual(events, ["workspace-owner"]);
  assert.equal(c.selectedAgentKey, "embedded:first-live");
  events.length = 0;
  state.id = "another-view";
  c.commandEntries()
    .find((entry) => entry.title === "go: github")
    .run();
  assert.deepEqual(events, ["reset", "page:github", "persist", "render"]);
  assert.equal(c.source, "github");
  assert.equal(c.selectedAgentKey, "embedded:first-live");
});

function installActualPaletteEntries(f) {
  Object.assign(f.c, {
    candidateForRecord: () => null,
    liveCandidates: () => [],
    snoozedAgentTabs: {},
    closedAgentTabs: [],
    leftSidebarCollapsed: false,
    agentSidebarCollapsed: false,
    shortcutLabel: (value) => value || "",
    setting: (key, fallback) => fallback,
    openSettings() {},
    chatPreparationContext: () => ({ availability: "available", total: 0 }),
    api: () => {
      throw Error("Command enumeration must not make requests.");
    },
  });
}
test("actual commandEntries retains every owner presentation label/detail/exact callback without performing actions", () => {
  const f = paletteFixture({ actualEntries: true });
  installActualPaletteEntries(f);
  let enumerated = 0,
    effects = 0;
  const descriptors = Array.from({ length: 8 }, (_, i) => ({
    verb: i % 3 === 0 ? "show" : i % 3 === 1 ? "retry" : "dismiss",
    noun: `agent results · Result ${i}`,
    detail: `Exact result status ${i}`,
    run: () => effects++,
  }));
  f.c.presentationCommands = () => {
    enumerated++;
    return descriptors;
  };
  const entries = f.c.commandEntries();
  assert.equal(enumerated, 1);
  assert.equal(effects, 0);
  for (const descriptor of descriptors) {
    const found = entries.find(
      (entry) => entry.title === `${descriptor.verb}: ${descriptor.noun}`,
    );
    assert.ok(found);
    assert.equal(found.detail, descriptor.detail);
    assert.equal(found.run, descriptor.run);
  }
  assert.equal(effects, 0);
  evaluate(f.c, "delete presentationCommands");
  assert.doesNotThrow(() => f.c.commandEntries());
  assert.equal(
    f.c.commandEntries().some((entry) => entry.title.includes("agent results")),
    false,
  );
  assert.equal(effects, 0);
});
test("actual palette runner closes before awaiting a presentation owner and preserves its rejection in the existing toast route", async () => {
  const f = paletteFixture({ actualEntries: true });
  installActualPaletteEntries(f);
  const order = [],
    errors = [];
  let reject;
  const run = () => {
    assert.equal(f.$("#action-palette").open, false);
    order.push("owner-after-close");
    return new Promise((resolve, fail) => (reject = fail));
  };
  f.c.presentationCommands = () => [
    {
      verb: "show",
      noun: "agent results · Investigation",
      detail: "Explicit display only",
      run,
    },
  ];
  f.c.toast = (error) => errors.push(error);
  f.c.openCommands();
  f.query("show agent results");
  assert.equal(f.rows().length, 1);
  assert.equal(
    f.rows()[0].children[0].children[0].textContent,
    "show: agent results · Investigation",
  );
  assert.deepEqual(order, []);
  const pending = f.c.runPaletteCommand(0);
  assert.equal(f.$("#action-palette").open, false);
  assert.deepEqual(order, ["owner-after-close"]);
  assert.deepEqual(errors, []);
  reject(Error("Close setup, then show these results."));
  await pending;
  assert.deepEqual(errors, ["Close setup, then show these results."]);
  assert.equal(f.$("#action-query").attrs["aria-expanded"], "false");
});

test("command palette self-toggle honors its cancel owner instead of forcibly closing", () => {
  const f = paletteFixture();
  f.c.openCommands();
  f.$("#action-palette").addEventListener("cancel", (event) =>
    event.preventDefault(),
  );
  f.c.openCommands();
  assert.equal(f.$("#action-palette").open, true);
  assert.equal(f.entryBuilds, 1);
});
