const { test } = require("node:test");
const assert = require("node:assert/strict");
// app-chat.js is a classic script that uses the globals of documents.js.
Object.assign(globalThis, require("../static/documents.js"));
const {
  appChatCommand,
  appChatAssistant,
  appChatFailureMessage,
} = require("../static/app-chat.js");
test("assistant view commands preserve validated filters and reject arbitrary execution", () => {
  assert.deepEqual(
    appChatCommand({
      type: "show",
      source: "all",
      query: "latency",
      status: "all",
    }),
    { type: "show", source: "all", query: "latency", status: "all" },
  );
  for (const command of [
    { type: "eval", source: "all", query: "alert(1)", status: "all" },
    { type: "show", source: "javascript:alert(1)", query: "", status: "all" },
    { type: "show", source: "all", query: "x".repeat(501), status: "all" },
    { type: "show", source: "all", query: "", status: "bad" },
  ]) {
    assert.throws(() => appChatCommand(command), /unsupported view/);
  }
});

test("open-chat accepts an exact chat target and rejects malformed commands", () => {
  assert.deepEqual(
    appChatCommand({ type: "open-chat", id: "embedded:chat-1" }),
    { type: "open-chat", id: "embedded:chat-1" },
  );
  for (const id of ["", null, {}, "x".repeat(161)]) {
    assert.throws(
      () => appChatCommand({ type: "open-chat", id }),
      /unsupported view/,
    );
  }
});

test("chat results keep verified recommendations and reject unknown targets", () => {
  const result = {
    type: "chat-results",
    query: "thinkingv2",
    chats: [{ id: "one", title: "benchmark", cwd: "/thinking-v2" }],
    total: 1,
    recommendedId: "one",
    reason: "Folder matches",
  };
  assert.equal(appChatCommand(result).recommendedId, "one");
  assert.throws(
    () => appChatCommand({ ...result, recommendedId: "missing" }),
    /unsupported view/,
  );
  assert.throws(
    () =>
      appChatCommand({
        type: "open-chat",
        id: "one",
        results: { type: "show", source: "all", query: "", status: "all" },
      }),
    /unsupported view/,
  );
});
test("mixed display lists validate work and chat targets and recommendations", () => {
  const rows = [
    {
      type: "work",
      id: "linear:a",
      item: { id: "linear:a", title: "Latency" },
    },
    { type: "chat", id: "chat-b", item: { id: "chat-b", title: "Benchmark" } },
  ];
  const value = {
    type: "display-items",
    title: "Related work",
    items: rows,
    recommended: { type: "work", id: "linear:a" },
    reason: "Matches the goal",
  };
  assert.equal(appChatCommand(value).items.length, 2);
  assert.throws(
    () =>
      appChatCommand({
        ...value,
        recommended: { type: "work", id: "missing" },
      }),
    /unsupported view/,
  );
  assert.throws(
    () =>
      appChatCommand({
        ...value,
        items: [
          { type: "script", id: "bad", item: { id: "bad", title: "Bad" } },
        ],
      }),
    /unsupported view/,
  );
});

test("saved view navigation accepts verified IDs without interpreting code", () => {
  assert.deepEqual(
    appChatCommand({
      type: "saved-view",
      id: "github-review",
      name: "To Review",
    }),
    { type: "saved-view", id: "github-review", name: "To Review" },
  );
  for (const id of ["", "../other", "javascript:alert(1)", "x".repeat(81)]) {
    assert.throws(
      () => appChatCommand({ type: "saved-view", id, name: "View" }),
      /unsupported view/,
    );
  }
});

const { browserContext, loadScripts } = require("./helpers/static-script.cjs");
const {
  FakeDocument,
  FakeElement,
  FakeWindow,
} = require("./helpers/fake-dom.cjs");
// Load the whole App chat script. The panel nodes and its closure come from the real IIFE;
// `chat` holds the internals the script exposes when `module` exists. Test globals win after load.
function loadAppChat(globals = {}, { files = [], loading = {} } = {}) {
  const doc = new FakeDocument(),
    created = [],
    lookup = doc.querySelector.bind(doc);
  // No dialog is open until a test opens one.
  doc.querySelector = (selector) =>
    selector === "dialog[open]" ? null : lookup(selector);
  const el = (tag, cls, text) => {
    const node = doc.createElement(tag);
    if (cls) {
      node.className = cls;
    }
    if (text !== undefined) {
      node.textContent = text;
    }
    created.push(node);
    return node;
  };
  const c = browserContext({
    module: { exports: {} },
    el,
    registerRenderHook() {},
    source: "all",
    statusFilter: { value: "all" },
    scopePrefs: {},
    sourceNames: {},
    $: (selector) =>
      created.find((node) => `#${node.id}` === selector) ||
      doc.querySelector(selector),
    document: doc,
    window: new FakeWindow(doc),
    setTimeout: () => 0,
    clearTimeout() {},
    TextEncoder,
    structuredClone,
    ...loading,
  });
  loadScripts(c, ["static/documents.js", ...files, "static/app-chat.js"], {
    keepHost: ["window"],
  });
  Object.assign(c, globals);
  const find = (cls) => created.find((node) => node.className === cls),
    footer = find("app-chat-footer");
  const ui = {
    button: find("app-chat-launcher"),
    panel: find("app-chat-panel"),
    badge: find("app-chat-provider"),
    fresh: find("text-button app-chat-new"),
    close: created.find((node) => node.textContent === "×"),
    context: find("app-chat-context"),
    messages: find("app-chat-messages"),
    conversation: find("app-chat-conversation"),
    setup: find("app-chat-setup"),
    error: find("app-chat-error"),
    form: find("app-chat-form"),
    input: created.find((node) => node.tagName === "TEXTAREA"),
    hint: footer.children[0],
    stop: footer.children[1],
    send: footer.children[2],
  };
  return { c, chat: c.module.exports.appChatPanel, ui, doc };
}
// Record each value that the real closure writes to a node property.
function watch(node, key, record) {
  let value = node[key];
  Object.defineProperty(node, key, {
    configurable: true,
    get: () => value,
    set: (next) => {
      value = next;
      record(next);
    },
  });
}
// Each real renderDeferred call first removes the previous Show results button.
function recordDeferred(ui, calls) {
  const query = ui.panel.querySelector.bind(ui.panel);
  ui.panel.querySelector = (selector) => {
    if (selector === ".app-chat-show-results") {
      calls.push("deferred");
    }
    return query(selector);
  };
}
const showResults = (ui) =>
  ui.panel.children.filter(
    (node) => node.className === "text-button app-chat-show-results",
  );
function navigationFixture(options) {
  const calls = [],
    nodes = new Map([
      ["#settings-dialog", { open: false }],
      ["#app-chat-setup", { hidden: true }],
      ["#app-chat-panel", { hidden: false }],
      ["#settings-search", { value: "old setting query" }],
      ["#settings-body", { scrollTop: 50 }],
      ["#search", { value: "retained work query" }],
      ["#group-by", { value: "status" }],
    ]);
  const { c, chat, ui } = loadAppChat({}, options);
  Object.assign(c, {
    source: "github",
    scopePrefs: { github: "retained-scope" },
    githubView: "review",
    statusFilter: { value: "blocked" },
    settingsCategory: "appearance",
    settingsModifiedOnly: true,
    $: (selector) => nodes.get(selector),
    persist: () => calls.push("persist"),
    render: () => calls.push("render"),
    savedViewCapturePresentation: (page) => calls.push(["capture", page]),
    openPreferences: async ({ category } = {}) => {
      if (!nodes.get("#settings-dialog").open) {
        nodes.get("#settings-dialog").open = true;
        calls.push("settings");
      }
      if (category !== undefined) {
        c.settingsCategory = category;
        c.resetSettingsFilter();
        c.filterSettings();
        nodes.get("#settings-body").scrollTop = 0;
      }
      return true;
    },
    resetSettingsFilter: () => {
      nodes.get("#settings-search").value = "";
      c.settingsModifiedOnly = false;
      calls.push("clear-filters");
    },
    filterSettings: () => calls.push("filter"),
  });
  return { c, calls, nodes, chat, ui, error: () => ui.error.textContent };
}

test("known navigation intents reject extra actions, arbitrary targets and category misuse", () => {
  for (const destination of [
    "setup",
    "skills",
    "artifacts",
    "logs",
    "settings",
    "sharing-inbox",
  ]) {
    assert.deepEqual(appChatCommand({ type: "navigate", destination }), {
      type: "navigate",
      destination,
    });
  }
  assert.deepEqual(
    appChatCommand({
      type: "navigate",
      destination: "settings",
      category: "connections",
    }),
    { type: "navigate", destination: "settings", category: "connections" },
  );
  for (const command of [
    { type: "navigate", destination: "setup", category: "appearance" },
    { type: "navigate", destination: "setup", step: "theme" },
    { type: "navigate", destination: "setup", reset: true },
    { type: "navigate", destination: "run-skill" },
    { type: "navigate", destination: "javascript:alert(1)" },
    { type: "navigate", destination: "settings", category: "invented" },
    { type: "navigate", destination: "logs", category: "data" },
    { type: "navigate", destination: "settings", category: null },
    { type: "navigate", destination: "settings", token: "benign-fixture" },
  ]) {
    assert.throws(() => appChatCommand(command), /unsupported view/);
  }
});

test("auxiliary page navigation captures work presentation and preserves every work filter", async () => {
  for (const destination of ["skills", "artifacts", "logs"]) {
    const { c, calls, nodes } = navigationFixture();
    await c.appChatNavigate({ type: "navigate", destination });
    assert.equal(c.source, destination);
    assert.deepEqual(calls, [["capture", "github"], "persist", "render"]);
    assert.equal(nodes.get("#search").value, "retained work query");
    assert.equal(c.statusFilter.value, "blocked");
    assert.equal(c.scopePrefs.github, "retained-scope");
    assert.equal(c.githubView, "review");
  }
});

test("settings navigation opens only the requested category and keeps the underlying work page", async () => {
  const { c, calls, nodes } = navigationFixture();
  await c.appChatNavigate({
    type: "navigate",
    destination: "settings",
    category: "keyboard",
  });
  assert.equal(c.settingsCategory, "keyboard");
  assert.equal(nodes.get("#settings-dialog").open, true);
  assert.equal(nodes.get("#settings-search").value, "");
  assert.equal(c.settingsModifiedOnly, false);
  assert.equal(nodes.get("#settings-body").scrollTop, 0);
  assert.equal(c.source, "github");
  assert.deepEqual(calls, ["settings", "clear-filters", "filter"]);
  c.settingsModifiedOnly = true;
  await c.appChatNavigate({
    type: "navigate",
    destination: "settings",
    category: "sharing",
  });
  assert.equal(c.settingsCategory, "sharing");
  assert.equal(c.settingsModifiedOnly, false);
  assert.deepEqual(calls, [
    "settings",
    "clear-filters",
    "filter",
    "clear-filters",
    "filter",
  ]);
});

test("setup navigation opens saved setup once and preserves the work page and filters", async () => {
  const { c, calls, nodes } = navigationFixture();
  nodes.set("#onboarding-dialog", { open: false });
  c.openOnboarding = async ({ host }) => {
    assert.equal(host, nodes.get("#app-chat-setup"));
    calls.push("setup");
    host.hidden = false;
  };
  await c.appChatNavigate({ type: "navigate", destination: "setup" });
  await c.appChatNavigate({ type: "navigate", destination: "setup" });
  assert.deepEqual(calls, ["setup"]);
  assert.equal(c.source, "github");
  assert.equal(c.githubView, "review");
  assert.equal(c.scopePrefs.github, "retained-scope");
  assert.equal(c.statusFilter.value, "blocked");
  assert.equal(nodes.get("#search").value, "retained work query");
});

function setupApplyFixture() {
  const fixture = navigationFixture(),
    { c } = fixture;
  Object.assign(c, {
    settingsDirty: false,
    document: { querySelector: () => null, querySelectorAll: () => [] },
  });
  return fixture;
}

test("setup failures remain deferred when opener is missing or returns without an open dialog", async () => {
  for (const opener of [undefined, async () => {}]) {
    const { c, nodes, chat, error } = setupApplyFixture();
    nodes.set("#onboarding-dialog", { open: false });
    if (opener) {
      c.openOnboarding = opener;
    }
    await chat.apply([{ type: "navigate", destination: "setup" }]);
    assert.equal(chat.deferred[0].destination, "setup");
    assert.match(error(), /Setup (?:is unavailable|could not open)/);
    assert.equal(c.source, "github");
  }
});

test("setup navigation obeys dirty Settings and unrelated modal guards and accepts already-open setup", async () => {
  for (const mode of ["dirty", "unrelated", "setup"]) {
    const { c, calls, nodes, chat, error } = setupApplyFixture();
    nodes.set("#onboarding-dialog", { open: false });
    nodes.get("#app-chat-setup").hidden = mode !== "setup";
    c.settingsDirty = mode === "dirty";
    c.document.querySelector = () =>
      mode === "unrelated" ? { id: "other-dialog" } : null;
    c.openOnboarding = async () => calls.push("unexpected reopen");
    await chat.apply([{ type: "navigate", destination: "setup" }]);
    assert.deepEqual(calls, []);
    assert.equal(c.source, "github");
    if (mode === "setup") {
      assert.equal(chat.deferred.length, 0);
      assert.equal(error(), "");
    } else {
      assert.equal(chat.deferred[0].destination, "setup");
      assert.match(
        error(),
        mode === "dirty" ? /Save or discard/ : /Close the open dialog/,
      );
    }
  }
});

test("sharing navigation opens the existing Inbox tab without importing or changing the work view", async () => {
  const { c, calls, nodes } = navigationFixture();
  let open = false,
    selected = false,
    toggle;
  const inbox = {
    textContent: "Inbox",
    getAttribute: () => String(selected),
    click: () => {
      selected = true;
      calls.push("inbox");
    },
  };
  const menu = {
    get open() {
      return open;
    },
    set open(value) {
      open = value;
      toggle?.();
    },
    addEventListener: (event, callback) => {
      assert.equal(event, "toggle");
      toggle = callback;
    },
    querySelectorAll: () => [inbox],
  };
  nodes.set("#sharing-menu", menu);
  await c.appChatNavigate({ type: "navigate", destination: "sharing-inbox" });
  assert.equal(open, true);
  assert.equal(selected, true);
  assert.equal(c.source, "github");
  assert.deepEqual(calls, ["inbox"]);
  await c.appChatNavigate({ type: "navigate", destination: "sharing-inbox" });
  assert.deepEqual(calls, ["inbox"]);
});

test("navigation is deferred while settings are dirty or another dialog is open", async () => {
  for (const dirty of [false, true]) {
    const { c, chat, error } = navigationFixture();
    let applied = 0;
    Object.assign(c, {
      settingsDirty: dirty,
      document: { querySelector: () => (dirty ? null : {}) },
      appChatNavigate: async () => applied++,
    });
    await chat.apply([{ type: "navigate", destination: "skills" }]);
    assert.equal(applied, 0);
    assert.equal(chat.deferred[0].destination, "skills");
    assert.match(error(), dirty ? /Save or discard/ : /Close the open dialog/);
  }
});

test("a reply received after user navigation stays queued until Show results is chosen", async () => {
  let applied = 0,
    job = null;
  const { c, chat, ui } = loadAppChat({
    api: async (path) =>
      path === "/api/app-chat"
        ? { assistant: { id: "claude" }, messages: [], job }
        : {},
    crypto: { randomUUID: () => "request-1" },
    appChatNavigate: async () => applied++,
    appChatRefreshSavedViews: async () => {},
    loadAppSettings: async () => {},
    settingsDirty: false,
  });
  // Send from the original page, then navigate before the reply arrives.
  await chat.poll();
  ui.input.value = "Show the logs";
  await ui.form.onsubmit({ preventDefault() {} });
  c.$("#search").value = "changed-page";
  job = {
    id: "request-1",
    state: "succeeded",
    commands: [{ type: "navigate", destination: "logs" }],
  };
  await chat.poll();
  const offered = showResults(ui).length;
  assert.equal(applied, 0);
  assert.equal(offered, 1);
  assert.equal(chat.deferred[0].destination, "logs");
});

test("sharing open and tab changes count as navigation while private input values stay absent", () => {
  const { c, nodes, chat } = navigationFixture();
  let inbox = false;
  const menu = {
    open: false,
    querySelectorAll: () => [
      { textContent: "Inbox", getAttribute: () => String(inbox) },
    ],
  };
  nodes.set("#sharing-menu", menu);
  nodes.set("#group-by", { value: "project" });
  nodes.set("#page-title", { textContent: "GitHub" });
  c.document = { querySelector: () => null, querySelectorAll: () => [] };
  const closed = chat.stamp();
  menu.open = true;
  const shared = chat.stamp();
  inbox = true;
  const received = chat.stamp();
  assert.notEqual(closed, shared);
  assert.notEqual(shared, received);
  assert.equal(chat.snapshot().ui.sharingInbox, true);
  assert.equal(chat.snapshot().query, "retained work query");
  assert.equal(chat.snapshot().filters.scope, "retained-scope");
});

test("top Setup and Settings dialogs suppress obscured chat context while Review keeps its own record and file", () => {
  const { c, nodes, chat } = navigationFixture();
  let dialogs = [];
  nodes.set("#group-by", { value: "project" });
  nodes.set("#page-title", { textContent: "Chats" });
  Object.assign(c, {
    source: "agents",
    sourceNames: { agents: "Chats" },
    currentAgentRecord: () => ({ id: "underlying-chat", title: "Hidden chat" }),
    selectedGitDirectory: () => "/underlying-worktree",
    reviewState: {
      record: { id: "review-chat", title: "Reviewed chat" },
      directory: "/review-worktree",
      file: { path: "src/review.rs" },
    },
    document: {
      querySelectorAll: (selector) =>
        selector === "dialog[open]" ? dialogs : [],
    },
  });
  const dialog = (id, title) => ({
    id,
    querySelector: (selector) => {
      assert.equal(selector, "h1,h2,h3");
      return { textContent: title };
    },
    getAttribute: () => null,
  });
  const review = dialog("diff-review-dialog", "Review"),
    setup = dialog("onboarding-dialog", "Set up truffle"),
    settings = dialog("settings-dialog", "Settings");
  let page = chat.snapshot();
  assert.equal(page.recordId, "underlying-chat");
  assert.equal(chat.displayContext(page), "Context · Chats · Hidden chat");
  dialogs = [review];
  page = chat.snapshot();
  assert.equal(page.recordId, "review-chat");
  assert.equal(page.worktree, "/review-worktree");
  assert.equal(page.ui.reviewFile, "src/review.rs");
  assert.equal(chat.displayContext(page), "Context · Review");
  dialogs = [review, setup];
  page = chat.snapshot();
  assert.equal(page.ui.dialog, "Set up truffle");
  assert.equal(page.recordId, "");
  assert.equal(page.worktree, "");
  assert.equal(page.ui.reviewFile, "");
  assert.equal(chat.displayContext(page), "Context · Set up truffle");
  dialogs = [settings];
  page = chat.snapshot();
  assert.equal(page.source, "settings");
  assert.equal(page.ui.settingsCategory, "appearance");
  assert.equal(page.recordId, "");
  assert.equal(page.ui.reviewFile, "");
  assert.equal(chat.displayContext(page), "Context · Settings");
  dialogs = [settings, setup];
  page = chat.snapshot();
  assert.equal(page.source, "agents");
  assert.equal(page.ui.settingsCategory, "");
  assert.equal(page.ui.dialog, "Set up truffle");
  assert.equal(page.recordId, "");
});

const { appChatDiagnosticRows } = require("../static/app-chat.js");
test("failure details allowlist bounded metadata and never expose raw output, prompts or arbitrary fields", () => {
  const job = {
    id: "request-1",
    error: "Visible classified failure",
    diagnostics: {
      requestId: "request-1",
      stage: "cli-envelope",
      code: "missing_result",
      exitCode: 0,
      durationMs: 1234,
      stdoutBytes: 456,
      stderrBytes: 12,
      stdoutShape: "object",
      responseShape: "empty",
      jsonErrorCategory: "syntax",
      jsonErrorLine: 2,
      jsonErrorColumn: 9,
      stdout: "private-output-marker",
      stderr: "private-error-marker",
      prompt: "private-prompt-marker",
      message: "private-message-marker",
      credentials: { token: "private-token-marker" },
    },
  };
  const rows = Object.fromEntries(appChatDiagnosticRows(job));
  assert.equal(rows["Request ID"], "request-1");
  assert.equal(rows.Stage, "cli-envelope");
  assert.equal(rows.Code, "missing_result");
  assert.equal(rows["Exit code"], "0");
  assert.equal(rows.Duration, "1.23 s");
  assert.equal(rows["JSON error line"], "2");
  assert.ok(!JSON.stringify(rows).includes("private-"));
  assert.ok(!JSON.stringify(rows).includes("Visible classified"));
  const invalid = Object.fromEntries(
    appChatDiagnosticRows({
      id: "safe-request",
      diagnostics: {
        requestId: "<script>",
        stage: "<img>",
        code: "arbitrary unsafe text",
        exitCode: "1",
        durationMs: Infinity,
        stdoutBytes: -1,
        stdoutShape: "secret",
      },
    }),
  );
  assert.deepEqual(invalid, { "Request ID": "safe-request" });
  assert.deepEqual(appChatDiagnosticRows(null), []);
});
test("failure controls expose an accessible disclosure and log navigation without any retry action", async () => {
  class Node {
    constructor(tag, cls = "", text = "") {
      this.tag = tag;
      this.className = cls;
      this.textContent = text;
      this.dataset = {};
      this.attrs = {};
      this.children = [];
    }
    append(...items) {
      this.children.push(...items);
    }
    setAttribute(key, value) {
      this.attrs[key] = value;
    }
  }
  let opened = 0;
  const { c } = loadAppChat({ el: (...args) => new Node(...args) });
  const node = c.appChatFailureActions(
      {
        id: "request-1",
        diagnostics: {
          stage: "process",
          code: "timed_out",
          stderr: "private-marker",
        },
      },
      async () => opened++,
    ),
    [actions, details] = node.children,
    [toggle, logs] = actions.children;
  assert.equal(actions.className, "ui-actiongroup app-chat-failure-actions");
  assert.equal(actions.attrs.role, "group");
  assert.equal(actions.attrs["aria-label"], "App chat failure actions");
  assert.deepEqual(
    actions.children.map((n) => n.textContent),
    ["View details", "Open logs"],
  );
  assert.equal(toggle.attrs["aria-expanded"], "false");
  assert.equal(toggle.attrs["aria-controls"], details.id);
  assert.equal(details.hidden, true);
  toggle.onclick();
  assert.equal(details.hidden, false);
  assert.equal(toggle.attrs["aria-expanded"], "true");
  assert.equal(toggle.textContent, "Hide details");
  toggle.onclick();
  assert.equal(details.hidden, true);
  await logs.onclick();
  assert.equal(opened, 1);
  assert.ok(!JSON.stringify(node).includes("private-marker"));
});
test("Open logs captures the previous work view and filters failures to the exact request", async () => {
  const { c, calls, nodes } = navigationFixture();
  Object.assign(c, {
    document: { querySelector: () => null },
    logFilters: { source: "sentry", outcome: "success" },
  });
  await c.appChatOpenFailureLogs({
    id: "request-1",
    diagnostics: { requestId: "request-1" },
  });
  assert.equal(c.source, "logs");
  assert.equal(nodes.get("#search").value, "request-1");
  assert.deepEqual(c.logFilters, { source: "", outcome: "failure" });
  assert.deepEqual(calls[0], ["capture", "github"]);
  assert.equal(c.scopePrefs.github, "retained-scope");
});
test("Open logs preserves dirty Settings and open dialogs, and older failures use the App chat query", async () => {
  for (const dirty of [false, true]) {
    const { c, nodes, calls } = navigationFixture();
    Object.assign(c, {
      settingsDirty: dirty,
      document: { querySelector: () => (dirty ? null : {}) },
    });
    await assert.rejects(
      c.appChatOpenFailureLogs({ id: "request-1" }),
      dirty ? /Save or discard/ : /Close the open dialog/,
    );
    assert.equal(c.source, "github");
    assert.equal(nodes.get("#search").value, "retained work query");
    assert.deepEqual(calls, []);
  }
  const { c, nodes } = navigationFixture();
  c.document = { querySelector: () => null };
  await c.appChatOpenFailureLogs({});
  assert.equal(nodes.get("#search").value, "app-chat");
});
test("failed polling retains the saved-message explanation and diagnostics while Settings has unsaved edits", async () => {
  const job = {
      id: "request-1",
      state: "failed",
      error: "Claude returned no result.",
      diagnostics: {
        requestId: "request-1",
        stage: "cli-envelope",
        code: "missing_result",
      },
    },
    errors = [];
  let applied = 0;
  // Any applied command reaches one of these owners.
  const { chat, ui } = loadAppChat({
    api: async () => ({
      messages: [{ role: "user", text: "Saved question" }],
      job,
    }),
    persist: () => applied++,
    appChatNavigate: async () => applied++,
    applyResultPresentation: async () => applied++,
    appChatOpenSettingsProposal: () => applied++,
    loadAppSettings: async () => {
      throw new Error("Must preserve draft");
    },
    appChatFailureActions: (details) => {
      errors.at(-1).details = details;
      return new FakeElement("div");
    },
    settingsDirty: true,
  });
  watch(ui.error, "textContent", (message) => errors.push({ message }));
  await chat.poll();
  assert.equal(applied, 0);
  assert.equal(errors.length, 1);
  assert.match(errors[0].message, /Claude returned no result/);
  assert.match(errors[0].message, /Your message is saved in this chat/);
  assert.equal(errors[0].details, job);
  await chat.poll();
  assert.equal(errors.length, 1);
});

test("settings proposals require a bounded patch and an exact revision, and copy their input", () => {
  const input = {
    type: "settings-proposal",
    revision: 7,
    contentHash: "a".repeat(64),
    patch: { editor: "zed", theme: "light" },
  };
  const command = appChatCommand(input);
  input.patch.editor = "changed";
  assert.equal(command.patch.editor, "zed");
  assert.equal(command.revision, 7);
  for (const change of [
    { revision: -1 },
    { revision: 1.5 },
    { revision: null },
    { contentHash: null },
    { contentHash: "bad" },
    { patch: [] },
    { patch: null },
    { patch: "text" },
    { patch: { editor: "x".repeat(65536) } },
    { extra: "unexpected" },
  ]) {
    assert.throws(
      () => appChatCommand({ ...input, ...change }),
      /invalid settings proposal/,
    );
  }
});
function proposalFixture() {
  class Node {
    constructor(tag, cls = "", text = "") {
      Object.assign(this, {
        tag,
        className: cls,
        textContent: text,
        children: [],
        dataset: {},
        listeners: {},
        attrs: {},
      });
    }
    append(...nodes) {
      this.children.push(...nodes);
    }
    setAttribute(k, v) {
      this.attrs[k] = v;
    }
    addEventListener(k, fn) {
      this.listeners[k] = fn;
    }
    showModal() {
      this.open = true;
    }
    focus() {
      this.focused = true;
    }
    close() {
      this.open = false;
      this.listeners.close();
    }
    remove() {
      this.removed = true;
    }
  }
  let disposed = 0;
  const calls = [],
    body = new Node("body");
  const { c } = loadAppChat({
    TextEncoder,
    structuredClone,
    settingsDirty: false,
    document: { body, querySelector: () => null },
    el: (...args) => new Node(...args),
    renderSettingsProfilePreview: (container, options) => {
      calls.push({ container, options });
      return {
        disposeProfileImport() {
          disposed++;
        },
      };
    },
  });
  return { c, calls, body, disposed: () => disposed };
}
test("settings proposal opens the shared local approval preview without applying settings", () => {
  const f = proposalFixture(),
    dialog = f.c.appChatOpenSettingsProposal({
      type: "settings-proposal",
      revision: 7,
      contentHash: "a".repeat(64),
      patch: { editor: "zed" },
    });
  assert.equal(dialog.open, true);
  assert.equal(f.body.children[0], dialog);
  assert.equal(f.calls.length, 1);
  assert.equal(f.calls[0].options.expectedRevision, 7);
  assert.equal(f.calls[0].options.expectedContentHash, "a".repeat(64));
  assert.equal(f.calls[0].options.initialProfile.format, "truffle-settings");
  assert.equal(f.calls[0].options.initialProfile.values.editor, "zed");
  assert.equal(dialog.children[0].children[1].focused, true);
  dialog.children[0].children[1].onclick();
  assert.equal(f.disposed(), 1);
  assert.equal(dialog.removed, true);
});
test("settings proposals preserve dirty drafts and existing dialogs", () => {
  for (const dirty of [false, true]) {
    const f = proposalFixture();
    f.c.settingsDirty = dirty;
    f.c.document.querySelector = () => (dirty ? null : {});
    assert.throws(
      () =>
        f.c.appChatOpenSettingsProposal({
          type: "settings-proposal",
          revision: 0,
          contentHash: "a".repeat(64),
          patch: { editor: "zed" },
        }),
      dirty ? /Save or discard/ : /Close the open dialog/,
    );
    assert.equal(f.calls.length, 0);
    assert.equal(f.body.children.length, 0);
  }
});

test("successful replies refresh saved views and guidance even when no navigation was requested", async () => {
  const calls = [],
    job = { id: "view-write", state: "succeeded", commands: [] };
  const { c, chat, ui } = loadAppChat({
    api: async () => ({ messages: [], job }),
    appChatRefreshSavedViews: async () => calls.push("views"),
    appChatNavigate: async () => calls.push("apply"),
    persist: () => calls.push("apply"),
    loadAppSettings: async () => calls.push("settings"),
    settingsDirty: false,
  });
  c.window.truffleGuidance = { refresh: async () => calls.push("guidance") };
  recordDeferred(ui, calls);
  watch(ui.error, "textContent", (message) => calls.push(message));
  await chat.poll();
  assert.deepEqual(calls, ["deferred", "views", "settings", "guidance"]);
});
test("saved-view refresh errors preserve deferred navigation and never activate cached definitions", async () => {
  const commands = [{ type: "saved-view", id: "standup", name: "Standup" }],
    calls = [];
  // Only the work filters that the context snapshot reads exist; every other lookup misses.
  const { c, chat, ui } = loadAppChat(
    {
      source: "github",
      githubView: "all",
      settingsDirty: false,
      $: (selector) =>
        ["#search", "#group-by"].includes(selector) ? { value: "" } : null,
      document: { querySelector: () => null, querySelectorAll: () => [] },
      loadSavedViews: async () => {
        throw new Error("Saved views could not refresh. Offline.");
      },
      savedViewsError: "",
      savedViewsDocument: { views: [{ id: "standup" }] },
      activateSavedView: () => calls.push("activate"),
    },
    { files: ["static/presentation.js"] },
  );
  recordDeferred(ui, calls);
  watch(ui.error, "textContent", (message) => calls.push(message));
  await chat.apply(commands);
  assert.equal(chat.deferred, commands);
  assert.ok(!calls.includes("activate"));
  assert.match(calls.at(-1), /could not refresh/);
  c.loadSavedViews = async () => {};
  await chat.apply(commands);
  assert.ok(calls.includes("activate"));
  assert.equal(chat.deferred.length, 0);
});
test("saved-view refresh rejects retained documents when loading failed", async () => {
  const { c } = loadAppChat({
    loadSavedViews: async () => ({ views: [{ id: "cached" }] }),
    savedViewsError: "Invalid view document",
  });
  await assert.rejects(c.appChatRefreshSavedViews(), /Invalid view document/);
});

test("inline setup context suppresses underlying chat details without reading setup inputs", () => {
  const { c, nodes, chat } = navigationFixture();
  nodes.set("#app-chat-setup", { hidden: false });
  nodes.set("#group-by", { value: "source" });
  nodes.set("#page-title", { textContent: "Agents" });
  Object.assign(c, {
    source: "agents",
    sourceNames: { agents: "Chats" },
    currentAgentRecord: () => ({ id: "underlying", title: "Hidden chat" }),
    selectedGitDirectory: () => "/private-worktree",
    document: { querySelectorAll: () => [] },
  });
  const page = chat.snapshot();
  assert.equal(page.ui.dialog, "Set up truffle");
  assert.equal(page.recordId, "");
  assert.equal(page.worktree, "");
  assert.equal(chat.displayContext(page), "Context · Set up truffle");
  nodes.get("#app-chat-panel").hidden = true;
  assert.equal(chat.snapshot().ui.dialog, "");
  assert.equal(chat.snapshot().recordId, "underlying");
});

test("changed chat history preserves inline setup nodes and its scroll position", () => {
  const {
      chat,
      ui: { messages, setup, conversation },
      doc,
    } = loadAppChat(),
    field = doc.createElement("input");
  field.value = "private unsaved setup field";
  setup.append(field);
  setup.hidden = false;
  Object.assign(conversation, { scrollTop: 47, scrollHeight: 900 });
  const saved = setup.children[0];
  chat.draw([{ role: "user", text: "new question" }]);
  assert.equal(messages.children.length, 1);
  assert.equal(setup.children[0], saved);
  assert.equal(conversation.scrollTop, 47);
  setup.hidden = true;
  chat.draw([{ role: "user", text: "another question" }]);
  assert.equal(conversation.scrollTop, 900);
});

test("closing or replacing App chat disposes only its own setup host", async () => {
  for (const action of ["close", "launcher", "new"]) {
    const calls = [],
      {
        c,
        ui: { button, close, fresh, setup: host },
      } = loadAppChat({ api: async () => {} });
    button.onclick();
    c.window.closeOnboarding = (options) => calls.push(options.host);
    if (action === "close") {
      close.onclick();
    } else if (action === "launcher") {
      button.onclick();
    } else {
      await fresh.onclick();
    }
    assert.deepEqual(calls, [host]);
  }
});

test("modal rehosting never appends App chat into its own descendant", () => {
  const calls = [],
    {
      c,
      chat,
      ui: { panel },
      doc,
    } = loadAppChat(),
    dialog = (name) =>
      Object.assign(doc.createElement("dialog"), {
        append: () => calls.push(name),
      });
  const descendant = dialog("descendant"),
    external = dialog("external"),
    body = { append: () => calls.push("body") };
  panel.children.push(descendant);
  let dialogs = [external, descendant];
  c.document = { querySelectorAll: () => dialogs, body };
  chat.placeChat();
  dialogs = [descendant];
  chat.placeChat();
  assert.deepEqual(calls, ["external", "body"]);
});

test("inline setup defers unrelated navigation until its controls are closed", async () => {
  const { c, nodes, chat, error } = setupApplyFixture();
  nodes.get("#app-chat-setup").hidden = false;
  let applied = 0;
  c.appChatNavigate = async () => applied++;
  await chat.apply([{ type: "navigate", destination: "logs" }]);
  assert.equal(applied, 0);
  assert.equal(chat.deferred[0].destination, "logs");
  assert.match(error(), /Close setup/);
  nodes.get("#app-chat-setup").hidden = true;
  await chat.apply(chat.deferred);
  assert.equal(applied, 1);
  assert.equal(chat.deferred.length, 0);
});

test("the public setup entry reveals chat before opening its inline component without launching a model", async () => {
  const calls = [],
    { c, ui } = loadAppChat({
      appChatNavigate: async (command) => calls.push(command.destination),
      api: async () => {
        calls.push("history");
        return { messages: [] };
      },
    });
  // Each real update shows or hides the panel; history loading ends with one more update.
  watch(ui.panel, "hidden", (hidden) => calls.push(["visible", !hidden]));
  await c.window.openAppChatSetup();
  assert.deepEqual(calls, [
    ["visible", true],
    "setup",
    "history",
    ["visible", true],
  ]);
});

test("first-run setup uses App chat once and leaves existing or completed workspaces alone", async () => {
  for (const [state, progress, expected] of [
    [{ items: [], sessions: [] }, { completed: false, steps: {} }, 1],
    [
      { items: [{ id: "work" }], sessions: [] },
      { completed: false, steps: {} },
      0,
    ],
    [{ items: [], sessions: [] }, { completed: true, steps: {} }, 0],
    [
      { items: [], sessions: [] },
      { completed: false, steps: { theme: "skipped" } },
      0,
    ],
  ]) {
    let opened = 0;
    const c = browserContext({
      state,
      api: async () => ({ progress }),
      window: { openAppChatSetup: async () => opened++ },
      console,
    });
    loadScripts(c, ["static/documents.js", "static/product-tools.js"]);
    await c.maybeOfferOnboarding();
    await c.maybeOfferOnboarding();
    assert.equal(opened, expected);
  }
});

async function preparedMessageFixture({
  pending = false,
  draft = "",
  images = 0,
} = {}) {
  const calls = [],
    { c, chat, ui } = loadAppChat({
      api: async () => ({
        messages: [],
        job: { id: "running-reply", state: "running" },
      }),
    }),
    { input } = ui;
  if (pending) {
    await chat.poll();
  }
  // A poll reaches the server through api, so this also rejects polling.
  c.api = () => {
    throw new Error("Draft preparation must not call the server");
  };
  c.window.truffleChatImages = { count: () => images };
  input.value = draft;
  input.focus = () => calls.push("focus");
  watch(ui.error, "textContent", (message) => calls.push(["error", message]));
  watch(ui.panel, "hidden", () => calls.push("update"));
  return { c, chat, input, calls };
}

test("preparing a setup message reveals an editable draft without sending or polling", async () => {
  const { c, chat, input, calls } = await preparedMessageFixture();
  const guide = "Help configure Truffle.\nCompany guide: use a shared folder.";
  c.window.prepareAppChatMessage(guide);
  assert.equal(input.value, guide);
  assert.equal(chat.open, true);
  assert.deepEqual(calls, [["error", ""], "update", "focus"]);
});

test("setup draft preparation preserves existing text and attachments and waits for a running reply", async () => {
  for (const options of [
    { pending: true },
    { draft: "My unsent question" },
    { images: 1 },
  ]) {
    const { c, chat, input, calls } = await preparedMessageFixture(options);
    assert.throws(
      () => c.window.prepareAppChatMessage("Team setup instructions"),
      /Wait for|Send or clear/,
    );
    assert.equal(input.value, options.draft || "");
    assert.equal(chat.open, false);
    assert.deepEqual(calls, []);
  }
});

test("setup draft preparation rejects empty or oversized UTF-8 text without truncation", async () => {
  for (const guide of [
    null,
    {},
    "",
    "  ",
    "x".repeat(4001),
    "😀".repeat(1001),
  ]) {
    const { c, input, calls } = await preparedMessageFixture();
    assert.throws(
      () => c.window.prepareAppChatMessage(guide),
      /Enter setup|too long/,
    );
    assert.equal(input.value, "");
    assert.deepEqual(calls, []);
  }
  const { c, input } = await preparedMessageFixture();
  const text = "😀".repeat(1000);
  c.window.prepareAppChatMessage(text);
  assert.equal(input.value, text);
});

test("named render hook refreshes App chat visible context on results, saved chat and Back without polling", async () => {
  const nodes = new Map([
      ["#settings-dialog", { open: false }],
      ["#app-chat-setup", { hidden: true }],
      ["#app-chat-panel", { hidden: false }],
      ["#search", { value: "retained work query" }],
    ]),
    record = {
      id: "saved-thinking",
      title: "Thinking-v2 investigation",
      cwd: "/work/thinking-v2",
    };
  let writes = 0,
    label = "",
    renders = 0;
  nodes.set("#group-by", { value: "none" });
  nodes.set("#page-title", { textContent: "Selected results" });
  nodes.set("#chat-results-back", {});
  const c = browserContext({
    persist() {},
    state: { sessions: [record] },
    document: {
      querySelector: (selector) => nodes.get(selector),
      querySelectorAll: () => [],
    },
    currentAgentRecord: () => (c.source === "agents" ? record : null),
    selectedGitDirectory: () => record.cwd,
    loadInventory: async () => {},
    candidateForRecord: () => null,
    sessionStorage: { getItem: () => null, setItem() {} },
    selectAgent: () => {
      c.source = "agents";
      c.render();
    },
    renderWorkspacePage: () => {
      renders++;
      nodes.get("#page-title").textContent =
        c.source === "agents" ? "Chats" : "Selected results";
    },
  });
  const host = loadScripts(c, [
    "static/client.js",
    "static/documents.js",
    "static/app.js",
    "static/app-chat.js",
    "static/chat-results.js",
  ]);
  assert.ok(c.renderHooks.after.has("app-chat:context"));
  Object.assign(c, { source: "github", githubView: "review" });
  Object.assign(c.sourceNames, {
    "chat-results": "Chat search",
    agents: "Chats",
  });
  c.statusFilter.value = "blocked";
  c.scopePrefs.github = "retained-scope";
  const context = host.document.body.children
    .find((node) => node.id === "app-chat-panel")
    .children.find((node) => node.className === "app-chat-context");
  Object.defineProperty(context, "textContent", {
    get() {
      return label;
    },
    set(value) {
      writes++;
      label = value;
    },
  });
  // Any accidental read beyond page metadata or added network request fails this test.
  c.selectedAgentKey = "";
  c.api = () => {
    throw new Error("Context refresh must not poll or send a model request.");
  };
  c.showChatResults({
    type: "display-items",
    title: "Selected results",
    items: [{ type: "chat", id: record.id, item: record }],
    reason: "",
  });
  assert.equal(label, "Context · Chat search");
  await c.openChatSearchTarget(record.id);
  assert.equal(label, "Context · Chats · Thinking-v2 investigation");
  c.showChatResults();
  assert.equal(label, "Context · Chat search");
  assert.equal(writes, 3);
  assert.equal(renders, 3);
  c.render();
  assert.equal(writes, 3);
  assert.equal(renders, 4);
});

function providerComposerFixture() {
  const calls = [],
    errors = [],
    images = [{ mediaType: "image/png", data: "private-image-fixture" }];
  let metadata = { messages: [] };
  // The host metadata arrives through the real poll; every other request is a model call.
  const { c, chat, ui } = loadAppChat({
    settingsDirty: false,
    crypto: { randomUUID: () => "request-provider" },
    appChatRefreshSavedViews: async () => {},
    loadAppSettings: async () => {},
    api: async (path, payload) => {
      if (path === "/api/app-chat") {
        return metadata;
      }
      calls.push({ path, payload });
      return {};
    },
  });
  c.window.truffleChatImages = {
    count: () => images.length,
    payload: () => images.map((image) => ({ ...image })),
    clear: () => images.splice(0),
  };
  watch(ui.error, "textContent", (message) => errors.push(message));
  return {
    c,
    chat,
    ...ui,
    calls,
    errors,
    images,
    receive(value) {
      metadata = value;
      return chat.poll();
    },
    submit: () => ui.form.onsubmit({ preventDefault() {} }),
  };
}
const assistantMetadata = (id, job = null) => ({
  assistant: id ? { id, label: id === "codex" ? "Codex" : "Claude" } : null,
  assistants: [
    { id: "claude", label: "Claude", installed: id === "claude" },
    { id: "codex", label: "Codex", installed: id === "codex" },
  ],
  job,
});
test("provider badge follows host-selected installed assistant and captured running job without model calls", async () => {
  const f = providerComposerFixture();
  f.input.value = "Unsent draft";
  f.input.oninput();
  assert.equal(f.send.disabled, true);
  assert.equal(f.badge.textContent, "Checking…");
  await f.receive(assistantMetadata("codex"));
  assert.equal(f.badge.textContent, "Codex");
  assert.equal(f.send.disabled, false);
  assert.equal(f.calls.length, 0);
  await f.receive(
    assistantMetadata("claude", {
      id: "reply-codex",
      state: "running",
      assistant: { id: "codex", label: "Codex" },
    }),
  );
  assert.equal(f.badge.textContent, "Codex");
  assert.equal(f.hint.textContent, "Codex is replying…");
  assert.equal(f.send.disabled, true);
  await f.receive(
    assistantMetadata("claude", {
      id: "reply-codex",
      state: "succeeded",
      assistant: { id: "codex", label: "Codex" },
    }),
  );
  assert.equal(f.badge.textContent, "Claude");
  assert.equal(f.send.disabled, false);
  assert.equal(f.calls.length, 0);
  assert.equal(f.input.value, "Unsent draft");
});
test("no installed assistant blocks even programmatic Send, keeps draft/images, and leaves setup navigation available", async () => {
  const f = providerComposerFixture();
  f.input.value = "Keep this question";
  await f.receive(assistantMetadata(null));
  assert.equal(f.send.disabled, true);
  assert.equal(f.badge.textContent, "Unavailable");
  assert.match(f.hint.textContent, /continue setup without one/);
  await f.submit();
  assert.equal(f.input.value, "Keep this question");
  assert.equal(f.images.length, 1);
  assert.equal(f.calls.length, 0);
  assert.match(f.errors.at(-1), /draft and attachments are kept/);
  // Exercise the same setup opening function; capability gating belongs only to Send.
  Object.assign(f.c, {
    appChatNavigate: async (command) => {
      assert.equal(command.destination, "setup");
      f.setup.hidden = false;
    },
  });
  await f.c.window.openAppChatSetup();
  assert.equal(f.setup.hidden, false);
  assert.equal(f.input.value, "Keep this question");
  assert.equal(f.images.length, 1);
});
test("Claude and Codex both send the existing private image payload only on explicit submit", async () => {
  for (const id of ["claude", "codex"]) {
    const f = providerComposerFixture();
    await f.receive(assistantMetadata(id));
    f.input.value = "Explain screenshot";
    assert.equal(f.calls.length, 0);
    await f.submit();
    assert.equal(f.calls.length, 1);
    assert.equal(f.calls[0].path, "/api/app-chat/send");
    assert.equal(f.calls[0].payload.message, "Explain screenshot");
    assert.deepEqual(JSON.parse(JSON.stringify(f.calls[0].payload.images)), [
      { mediaType: "image/png", data: "private-image-fixture" },
    ]);
    assert.equal(f.input.value, "");
    assert.equal(f.images.length, 0);
  }
});
test("send rejection never retries another provider or clears unsent text and attachments", async () => {
  const f = providerComposerFixture();
  await f.receive(assistantMetadata("codex"));
  f.input.value = "Keep failed send";
  f.c.api = async (path, payload) => {
    f.calls.push({ path, payload });
    throw new Error("Codex unavailable before job creation.");
  };
  await f.submit();
  assert.equal(f.calls.length, 1);
  assert.equal(f.input.value, "Keep failed send");
  assert.equal(f.images.length, 1);
  assert.equal(f.chat.pending, false);
  assert.match(f.errors.at(-1), /Codex unavailable/);
});
test("failed replies identify their captured provider rather than current preference and keep diagnostics allowlisted", () => {
  const job = {
    id: "request-codex",
    assistant: { id: "codex", label: "private-provider-label" },
    error: "Reply could not finish.",
    diagnostics: {
      stage: "process",
      code: "process_failed",
      prompt: "private-prompt",
      stdout: "private-output",
    },
  };
  assert.equal(
    appChatFailureMessage(job),
    "Codex · Reply could not finish. Your message is saved in this chat.",
  );
  assert.equal(
    Object.fromEntries(appChatDiagnosticRows(job)).Assistant,
    "Codex",
  );
  assert.ok(!JSON.stringify(appChatDiagnosticRows(job)).includes("private-"));
  assert.equal(appChatAssistant({ id: "unknown", label: "Claude" }), null);
  assert.equal(appChatAssistant({ id: {}, label: "private" }), null);
});

test("actual polling adopts host selection and job-captured provider before updating failure context", async () => {
  const f = providerComposerFixture(),
    job = {
      id: "failed-claude",
      state: "failed",
      assistant: { id: "claude", label: "Claude" },
      error: "Reply failed.",
      diagnostics: { code: "process_failed" },
    };
  await f.receive({ ...assistantMetadata("codex"), job, messages: [] });
  assert.equal(f.badge.textContent, "Codex");
  assert.match(f.errors.at(-1), /^Claude · Reply failed/);
  assert.equal(f.chat.jobAssistant.id, "claude");
  assert.equal(f.chat.assistant.id, "codex");
});

test("Codex parser failure codes remain visible while arbitrary diagnostic content stays excluded", () => {
  for (const code of [
    "invalid_codex_jsonl",
    "codex_turn_failed",
    "unsupported_codex_item",
    "unsupported_codex_event",
    "incomplete_codex_turn",
    "invalid_codex_wrapper",
  ]) {
    const rows = Object.fromEntries(
      appChatDiagnosticRows({
        id: "codex-request",
        assistant: { id: "codex", label: "private-label" },
        diagnostics: {
          code,
          stage: "cli-envelope",
          stdout: "private-output",
          stderr: "private-error",
          prompt: "private-prompt",
          other: "private-extension",
        },
      }),
    );
    assert.equal(rows.Code, code);
    assert.equal(rows.Assistant, "Codex");
    assert.equal(rows.Stage, "cli-envelope");
    assert.ok(!JSON.stringify(rows).includes("private-"));
  }
  assert.equal(
    Object.fromEntries(
      appChatDiagnosticRows({ diagnostics: { code: "unknown-private-code" } }),
    ).Code,
    undefined,
  );
});
test("actual App chat snapshot obtains recovery metadata only through the pure owner API and omits it for older clients", () => {
  const { c, nodes, chat } = navigationFixture();
  nodes.set("#group-by", { value: "project" });
  nodes.set("#page-title", { textContent: "GitHub" });
  c.document = { querySelector: () => null, querySelectorAll: () => [] };
  assert.equal(Object.hasOwn(chat.snapshot(), "chatRecovery"), false);
  let calls = 0;
  const recovery = {
    availability: "available",
    total: 1,
    entries: [
      {
        requestId: "30000000-0000-4000-8000-000000000001",
        kind: "quick",
        pending: false,
        openAttempted: false,
      },
    ],
  };
  c.chatPreparationContext = () => {
    calls++;
    return recovery;
  };
  c.chatPreparations = {
    get() {
      throw Error("App chat must not access owner map");
    },
  };
  assert.deepEqual(
    JSON.parse(JSON.stringify(chat.snapshot().chatRecovery)),
    recovery,
  );
  assert.equal(calls, 1);
  c.chatPreparationContext = () => ({ availability: "unavailable" });
  assert.deepEqual(JSON.parse(JSON.stringify(chat.snapshot().chatRecovery)), {
    availability: "unavailable",
  });
  assert.deepEqual(nodes.get("#search").value, "retained work query");
});
test("chat-recovery navigation is strictly view-only and reports an unavailable or blocked owner", async () => {
  assert.deepEqual(
    appChatCommand({ type: "navigate", destination: "chat-recovery" }),
    { type: "navigate", destination: "chat-recovery" },
  );
  for (const extra of [
    { requestId: "50000000-0000-4000-8000-000000000001" },
    { open: true },
    { clear: true },
    { category: "data" },
  ]) {
    assert.throws(
      () =>
        appChatCommand({
          type: "navigate",
          destination: "chat-recovery",
          ...extra,
        }),
      /unsupported view/,
    );
  }
  const { c, nodes } = navigationFixture();
  let calls = 0;
  c.openChatRecovery = () => {
    calls++;
    return true;
  };
  await c.appChatNavigate({ type: "navigate", destination: "chat-recovery" });
  assert.equal(calls, 1);
  assert.equal(c.source, "github");
  assert.equal(nodes.get("#search").value, "retained work query");
  for (const owner of [undefined, () => false]) {
    c.openChatRecovery = owner;
    await assert.rejects(
      c.appChatNavigate({ type: "navigate", destination: "chat-recovery" }),
      /could not open/,
    );
  }
});

test("App chat recovery navigation respects dirty/modal guards and defers a blocked owner instead of claiming success", async () => {
  for (const mode of ["dirty", "modal", "blocked"]) {
    const { c, chat, error } = setupApplyFixture();
    let opened = 0;
    c.openChatRecovery = () => {
      opened++;
      return false;
    };
    c.settingsDirty = mode === "dirty";
    c.document.querySelector = () =>
      mode === "modal" ? { id: "other-dialog" } : null;
    await chat.apply([{ type: "navigate", destination: "chat-recovery" }]);
    assert.equal(opened, mode === "blocked" ? 1 : 0);
    assert.equal(chat.deferred[0].destination, "chat-recovery");
    assert.match(
      error(),
      mode === "blocked"
        ? /could not open/
        : mode === "dirty"
          ? /Save or discard/
          : /Close the open dialog/,
    );
    assert.equal(c.source, "github");
  }
});

test("App chat includes actual Settings filter context only for topmost Settings and stamps each filter flag", () => {
  const { c, nodes, chat } = navigationFixture({
    files: ["static/settings.js"],
    loading: {
      openSettings: null,
      featureButton: (label, onclick, cls) =>
        Object.assign(new FakeElement("button"), {
          textContent: label,
          onclick,
          className: cls,
        }),
    },
  });
  nodes.set("#group-by", { value: "project" });
  nodes.set("#page-title", { textContent: "GitHub" });
  nodes.set("#settings-modified", { getAttribute: () => "false" });
  c.settingsDirty = false;
  const settings = {
      id: "settings-dialog",
      querySelector: () => ({ textContent: "Settings" }),
    },
    other = {
      id: "other-dialog",
      querySelector: () => ({ textContent: "Other dialog" }),
    };
  let dialogs = [settings];
  c.document = { querySelectorAll: () => dialogs };
  const actual = c.settingsFilterContext;
  let reads = 0;
  c.settingsFilterContext = () => {
    reads++;
    return actual();
  };
  let page = chat.snapshot();
  assert.deepEqual(JSON.parse(JSON.stringify(page.ui.settingsFilter)), {
    query: "old setting query",
    queryTruncated: false,
    modifiedOnly: false,
    preferencesDirty: false,
  });
  assert.equal(page.source, "settings");
  assert.equal(page.query, "retained work query");
  assert.equal(page.filters.scope, "retained-scope");
  let stamp = chat.stamp();
  nodes.get("#settings-search").value = "Theme";
  assert.notEqual(chat.stamp(), stamp);
  stamp = chat.stamp();
  nodes.get("#settings-modified").getAttribute = () => "true";
  assert.notEqual(chat.stamp(), stamp);
  stamp = chat.stamp();
  c.settingsDirty = true;
  assert.notEqual(chat.stamp(), stamp);
  nodes.get("#settings-search").value = "😀".repeat(200);
  stamp = chat.stamp();
  nodes.get("#settings-search").value = "😀".repeat(201);
  assert.notEqual(
    chat.stamp(),
    stamp,
    "truncation flag alone changes the context stamp",
  );
  page = chat.snapshot();
  assert.equal(page.ui.settingsFilter.query, "😀".repeat(200));
  assert.equal(page.ui.settingsFilter.queryTruncated, true);
  assert.equal(
    new TextEncoder().encode(page.ui.settingsFilter.query).length,
    800,
  );
  for (const obscured of [[settings, other], []]) {
    dialogs = obscured;
    const before = reads;
    page = chat.snapshot();
    assert.equal(Object.hasOwn(page.ui, "settingsFilter"), false);
    assert.equal(reads, before);
  }
  dialogs = [other, settings];
  assert.equal(chat.snapshot().ui.settingsFilter.modifiedOnly, true);
  nodes.delete("#settings-modified");
  assert.equal(Object.hasOwn(chat.snapshot().ui, "settingsFilter"), false);
  c.settingsFilterContext = undefined;
  assert.equal(Object.hasOwn(chat.snapshot().ui, "settingsFilter"), false);
});

test("refused Settings owner stays deferred even when the shared Settings dialog was already open", async () => {
  const { c, nodes, chat, error } = setupApplyFixture();
  nodes.get("#settings-dialog").open = true;
  let options;
  c.openPreferences = async (value) => {
    options = value;
    return false;
  };
  await chat.apply([
    { type: "navigate", destination: "settings", category: "sharing" },
  ]);
  assert.equal(options.category, "sharing");
  assert.equal(c.settingsCategory, "appearance");
  assert.equal(chat.deferred[0].category, "sharing");
  assert.match(error(), /Settings could not open/);
  assert.equal(c.settingsModifiedOnly, true);
  assert.equal(nodes.get("#settings-search").value, "old setting query");
});
