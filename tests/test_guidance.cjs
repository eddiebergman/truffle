const test = require("node:test");
const assert = require("node:assert/strict");
const { guidanceEligible } = require("../static/guidance.js");
test("completed, dismissed and disabled tips do not repeat across contexts", () => {
  const progress = {
    enabled: true,
    tips: { "ci.investigate": "completed", "chat.shell": "dismissed" },
  };
  assert.equal(guidanceEligible(progress, "ci.investigate"), false);
  assert.equal(guidanceEligible(progress, "chat.shell"), false);
  assert.equal(guidanceEligible(progress, "review.agent-scope"), true);
  assert.equal(
    guidanceEligible({ ...progress, enabled: false }, "review.agent-scope"),
    false,
  );
});
const { guidanceReadyToSnooze } = require("../static/guidance.js");
test("snooze lesson requires three hours of known readiness with no background work", () => {
  const now = 20000,
    ready = {
      state: "waiting",
      reason: "finished",
      at: 9000,
      backgroundCounts: {},
    };
  assert.equal(guidanceReadyToSnooze(ready, 8000, now), true);
  assert.equal(
    guidanceReadyToSnooze({ ...ready, at: now - 10799 }, 8000, now),
    false,
  );
  for (const patch of [
    { state: "working" },
    { reason: "question" },
    { reason: "approval" },
    { backgroundCounts: { task: 1 } },
    { subagents: 1 },
    { scheduledWakeups: 1 },
    { at: 21000 },
  ])
    assert.equal(
      guidanceReadyToSnooze({ ...ready, ...patch }, 8000, now),
      false,
    );
  assert.equal(guidanceReadyToSnooze(ready, 10000, now), false);
});

const { browserContext, loadScripts } = require("./helpers/static-script.cjs");
// Load the whole script; its first guidance read resolves to `data`.
async function loadGuidance(globals, data) {
  const mod = { exports: {} },
    c = browserContext({
      module: mod,
      $: () => null,
      api: async () => data,
      ...globals,
    }),
    host = loadScripts(c, "static/guidance.js");
  await new Promise((resolve) => setImmediate(resolve));
  return { c, tip: mod.exports.guidanceTip, host };
}
async function guidanceFixture({
  shadow = false,
  globals = {},
  data = {},
} = {}) {
  const node = (tag, cls, text) => ({
    tag,
    cls,
    textContent: text || "",
    style: {},
    attrs: {},
    children: [],
    contains(target) {
      return (
        this === target || this.children.some((child) => child.contains(target))
      );
    },
    append(...children) {
      for (const child of children) {
        child.parent = this;
        this.children.push(child);
      }
    },
    setAttribute(key, value) {
      this.attrs[key] = value;
    },
    getAttribute(key) {
      return this.attrs[key] || null;
    },
    removeAttribute(key) {
      delete this.attrs[key];
    },
    remove() {
      this.removed = true;
      this.parent.children = this.parent.children.filter(
        (child) => child !== this,
      );
    },
    getBoundingClientRect: () => ({ width: 260, height: 120 }),
  });
  const body = node("body"),
    dialog = node("dialog");
  class ShadowRoot {}
  let rect = {
    left: 500,
    top: 100,
    width: 20,
    height: 20,
    right: 520,
    bottom: 120,
  };
  const anchor = node("button");
  anchor.isConnected = true;
  anchor.focus = () => {
    context.document.activeElement = anchor;
  };
  anchor.attrs["aria-describedby"] = "existing-help";
  anchor.closest = () => null;
  anchor.getBoundingClientRect = () => rect;
  const root = shadow
    ? Object.assign(new ShadowRoot(), {
        host: { closest: () => dialog, getBoundingClientRect: () => rect },
      })
    : {};
  anchor.getRootNode = () => root;
  const {
    c: context,
    tip,
    host,
  } = await loadGuidance(
    {
      document: { body },
      ShadowRoot,
      el: node,
      featureButton: (text, onclick) =>
        Object.assign(node("button", "", text), { onclick }),
      innerWidth: 1000,
      innerHeight: 700,
      getComputedStyle: () => ({ zoom: "1.25" }),
      setting: () => "",
      shortcutLabel: () => "",
      toast() {},
      ...globals,
    },
    {
      definitions: {
        tips: [
          {
            id: "ci.investigate",
            title: "CI failed",
            text: "Use this icon to investigate.",
          },
        ],
      },
      ...data,
    },
  );
  return {
    context,
    tip,
    host,
    anchor,
    body,
    dialog,
    move: (value) => {
      rect = value;
    },
  };
}
test("active guidance marks the exact target and follows it at the current scale", async () => {
  const { tip, anchor, move } = await guidanceFixture();
  tip.show("ci.investigate", anchor);
  const { highlight, node } = tip.active;
  assert.equal(highlight.cls, "guidance-target-ring");
  assert.equal(highlight.attrs["aria-hidden"], "true");
  assert.equal(highlight.style.left, "396.8px");
  assert.equal(highlight.style.top, "76.8px");
  assert.equal(highlight.style.width, "22.4px");
  assert.equal(anchor.attrs["aria-describedby"], "existing-help " + node.id);
  move({ left: 600, top: 200, width: 24, height: 24, right: 624, bottom: 224 });
  tip.position();
  assert.equal(highlight.style.left, "476.8px");
  assert.equal(highlight.style.height, "25.6px");
  tip.remove();
  assert.equal(highlight.removed, true);
  assert.equal(node.removed, true);
  assert.equal(anchor.attrs["aria-describedby"], "existing-help");
  assert.equal(tip.active, null);
});
test("guidance removes its target highlight when the control disappears", async () => {
  const { tip, anchor, body } = await guidanceFixture();
  tip.show("ci.investigate", anchor);
  anchor.isConnected = false;
  tip.position();
  assert.equal(tip.active, null);
  assert.equal(body.children.length, 0);
});
test("shadow-root controls get the same highlight in the owning dialog without invalid ARIA references", async () => {
  const { tip, anchor, body, dialog } = await guidanceFixture({ shadow: true });
  tip.show("ci.investigate", anchor);
  assert.equal(body.children.length, 0);
  assert.equal(tip.active.highlight.parent, dialog);
  assert.equal(tip.active.node.parent, dialog);
  assert.equal(anchor.attrs["aria-describedby"], "existing-help");
  tip.remove();
  assert.equal(dialog.children.length, 0);
});

test("keyboard focus keeps a tip alive during the idle pause and dismissal returns focus to its anchor", async () => {
  let now = 20000;
  const checks = new Map([["run", { at: now / 1000 }]]);
  const {
    context: c,
    tip,
    host,
    anchor,
  } = await guidanceFixture({
    globals: {
      Date: { now: () => now },
      window: { getSelection: () => null },
      source: "agents",
      selectedAgentKey: "one",
      currentAgentRecord: () => ({}),
      candidateForRecord: () => ({ host: "embedded" }),
      chatCIChecks: checks,
    },
    data: { progress: { enabled: true, tips: {} } },
  });
  anchor.closest = (selector) =>
    selector === ".context-work-ci"
      ? { querySelector: () => ({ dataset: { chatCi: "run" } }) }
      : null;
  Object.assign(c.document, {
    querySelector: () => null,
    querySelectorAll: (selector) =>
      selector === ".investigate-ci" ? [anchor] : [],
  });
  // The first tick enters the page epoch; the next one after the pause shows the tip.
  tip.tick();
  now = 30000;
  tip.tick();
  const node = tip.active.node;
  node.children[2].closest = () => null;
  c.document.activeElement = node.children[2];
  host.document.listeners.pointerdown[0]({ composedPath: () => [{}] });
  now = 30001;
  tip.tick();
  assert.equal(tip.active?.node, node);
  tip.remove();
  assert.equal(c.document.activeElement, anchor);
});

test("Escape dismisses guidance before Review and honors Review cancellation before closing", () => {
  let active = true,
    closed = 0,
    dismissed = 0;
  const dialog = Object.assign(new EventTarget(), {
    id: "diff-review-dialog",
    close: () => closed++,
  });
  let saving = true;
  dialog.addEventListener("cancel", (event) => {
    if (saving) event.preventDefault();
  });
  const c = browserContext({
    Event,
    truffleGuidance: {
      dismiss: () => {
        if (!active) return false;
        active = false;
        dismissed++;
        return true;
      },
    },
    document: { querySelectorAll: () => [dialog] },
  });
  loadScripts(c, ["static/client.js", "static/app.js"]);
  const event = () => ({
    key: "Escape",
    target: { closest: () => dialog },
    preventDefault() {
      this.prevented = true;
    },
    stopImmediatePropagation() {
      this.stopped = true;
    },
  });
  const first = event();
  c.dismissOpenDialog(first);
  assert.equal(dismissed, 1);
  assert.equal(closed, 0);
  assert.equal(first.prevented, true);
  assert.equal(first.stopped, true);
  c.dismissOpenDialog(event());
  assert.equal(closed, 0);
  saving = false;
  c.dismissOpenDialog(event());
  assert.equal(closed, 1);
});

test("visibility crosses shadow hosts and rejects gutters clipped by scroll containers", async () => {
  const rect = (top, bottom) => ({
    left: 100,
    right: 130,
    top,
    bottom,
    width: 30,
    height: bottom - top,
  });
  const scroller = {
    getBoundingClientRect: () => ({
      left: 50,
      right: 800,
      top: 150,
      bottom: 600,
    }),
    style: { overflowX: "auto", overflowY: "auto" },
  };
  const host = {
    parentElement: scroller,
    getBoundingClientRect: () => rect(100, 500),
    style: {},
  };
  const gutter = {
    isConnected: true,
    getBoundingClientRect: () => rect(100, 120),
    getRootNode: () => ({ host }),
    style: {},
  };
  const { tip } = await loadGuidance(
    {
      innerWidth: 1000,
      innerHeight: 700,
      getComputedStyle: (node) => node.style,
    },
    null,
  );
  assert.equal(tip.visible(gutter), false);
  gutter.getBoundingClientRect = () => rect(200, 220);
  assert.equal(tip.visible(gutter), true);
  host.hidden = true;
  assert.equal(tip.visible(gutter), false);
});

test("review guidance finds a later visible gutter when the first line is clipped", async () => {
  const rect = (top, bottom) => ({
    left: 100,
    right: 130,
    top,
    bottom,
    width: 30,
    height: bottom - top,
  });
  const first = {
      isConnected: true,
      getBoundingClientRect: () => rect(-20, -5),
    },
    second = {
      isConnected: true,
      getBoundingClientRect: () => rect(200, 220),
      getRootNode: () => ({}),
    },
    host = {
      shadowRoot: {
        querySelector: () => first,
        querySelectorAll: () => [first, second],
      },
    };
  const dialog = { open: true, querySelectorAll: () => [host] },
    nodes = {
      "#diff-review-dialog": dialog,
      "#diff-review-composer": { hidden: true },
    };
  const { tip } = await loadGuidance(
    {
      guidanceEligible: () => true,
      $: (selector) => nodes[selector],
      reviewState: { draft: { comments: [] } },
      innerWidth: 1000,
      innerHeight: 700,
      getComputedStyle: () => ({}),
    },
    { progress: {} },
  );
  assert.equal(tip.candidate()[1], second);
});

test("tip width is constrained before measurement at browser UI zoom", async () => {
  const { context: c, tip, anchor, move } = await guidanceFixture();
  c.innerWidth = 320;
  move({ left: 200, top: 100, width: 20, height: 20, right: 220, bottom: 120 });
  tip.show("ci.investigate", anchor);
  const node = tip.active.node;
  node.getBoundingClientRect = () => ({
    width: Math.min(290, parseFloat(node.style.maxWidth) || 290) * 1.25,
    height: 120,
  });
  tip.position();
  assert.equal(node.style.maxWidth, "236.8px");
  const left = parseFloat(node.style.left) * 1.25;
  assert.ok(left >= 12);
  assert.ok(left + node.getBoundingClientRect().width <= c.innerWidth - 12);
});
