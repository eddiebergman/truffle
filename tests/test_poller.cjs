const { test } = require("node:test"),
  assert = require("node:assert/strict");
const { browserContext, loadScripts } = require("./helpers/static-script.cjs");

function fixture() {
  const c = browserContext({});
  loadScripts(c, "static/poller.js");
  let clock = 0,
    beat = null;
  const page = { hidden: false, dialog: false },
    poller = c.createPoller({
      now: () => clock,
      heartbeat: (tick) => (beat = tick),
      hidden: () => page.hidden,
      dialogOpen: () => page.dialog,
    });
  const settle = () => new Promise((resolve) => setImmediate(resolve));
  return {
    c,
    poller,
    page,
    settle,
    async advance(ms) {
      clock += ms;
      beat();
      await settle();
    },
  };
}

test("a task first runs one interval after it registers and then one interval after each run finishes", async () => {
  const f = fixture(),
    runs = [];
  f.poller.register("state", { every: 1000, run: () => runs.push("state") });
  await f.advance(999);
  assert.deepEqual(runs, []);
  await f.advance(1);
  assert.deepEqual(runs, ["state"]);
  await f.advance(500);
  assert.deepEqual(runs, ["state"]);
  await f.advance(500);
  assert.deepEqual(runs, ["state", "state"]);
});

test("an interval function is read on every check", async () => {
  const f = fixture();
  let every = 60000,
    runs = 0;
  f.poller.register("sharing", { every: () => every, run: () => runs++ });
  await f.advance(1500);
  assert.equal(runs, 0);
  every = 1500;
  await f.advance(0);
  assert.equal(runs, 1);
});

test("no task runs while the page is hidden and an overdue task runs once the page shows", async () => {
  const f = fixture();
  let runs = 0;
  f.poller.register("state", { every: 1000, run: () => runs++ });
  f.page.hidden = true;
  await f.advance(5000);
  f.poller.kick("state");
  await f.settle();
  assert.equal(runs, 0);
  f.page.hidden = false;
  await f.advance(0);
  assert.equal(runs, 1);
});

test("a run never overlaps itself and a kick during a run starts one more run after it finishes", async () => {
  const f = fixture(),
    releases = [];
  let active = 0,
    most = 0,
    runs = 0;
  f.poller.register("review", {
    every: 1000,
    run: async () => {
      runs++;
      most = Math.max(most, ++active);
      await new Promise((resolve) => releases.push(resolve));
      active--;
    },
  });
  await f.advance(1000);
  await f.advance(5000);
  f.poller.kick("review");
  f.poller.kick("review");
  await f.settle();
  assert.equal(runs, 1);
  releases.shift()();
  await f.settle();
  assert.equal(runs, 2);
  releases.shift()();
  await f.settle();
  assert.equal(runs, 2);
  assert.equal(most, 1);
});

test("a failed run is reported, still finishes, and the task runs again on its interval", async () => {
  const f = fixture(),
    reports = [];
  let runs = 0;
  f.c.console = {
    error: (message, error) => reports.push([message, error.message]),
  };
  f.poller.register("agents", {
    every: 1000,
    run: () => {
      runs++;
      throw new Error("Offline");
    },
  });
  await f.advance(1000);
  await f.advance(1000);
  assert.equal(runs, 2);
  assert.deepEqual(reports, [
    ["Poll task agents failed:", "Offline"],
    ["Poll task agents failed:", "Offline"],
  ]);
});

test("a kick runs a task before its interval, after the caller returns", async () => {
  const f = fixture();
  let runs = 0;
  f.poller.register("git-diff", { every: 10000, run: () => runs++ });
  f.poller.kick("git-diff");
  assert.equal(runs, 0);
  await f.settle();
  assert.equal(runs, 1);
  await f.advance(9999);
  assert.equal(runs, 1);
  await f.advance(1);
  assert.equal(runs, 2);
  assert.throws(() => f.poller.kick("missing"), /Unknown poll task missing/);
  assert.throws(
    () => f.poller.register("git-diff", { every: 1, run() {} }),
    /already registered/,
  );
});

test("a task whose condition fails stays due and runs as soon as the condition holds", async () => {
  const f = fixture();
  let open = false,
    runs = 0;
  f.poller.register("git-diff", {
    every: 10000,
    when: () => open,
    run: () => runs++,
  });
  await f.advance(30000);
  f.poller.kick("git-diff");
  await f.settle();
  assert.equal(runs, 0);
  open = true;
  await f.advance(0);
  assert.equal(runs, 1);
  await f.advance(0);
  assert.equal(runs, 1);
});

test("only tasks that opt in wait while a dialog is open", async () => {
  const f = fixture(),
    runs = [];
  f.poller.register("state", {
    every: 1000,
    pauseInDialog: true,
    run: () => runs.push("state"),
  });
  f.poller.register("review", { every: 1000, run: () => runs.push("review") });
  f.page.dialog = true;
  await f.advance(1000);
  assert.deepEqual(runs, ["review"]);
  f.page.dialog = false;
  await f.advance(0);
  assert.deepEqual(runs, ["review", "state"]);
});

test("the page poller checks its tasks every 500 ms against the document", () => {
  const intervals = [],
    document = { hidden: false, querySelector: () => null };
  const c = browserContext({
    document,
    setInterval: (fn, ms) => intervals.push(ms),
  });
  loadScripts(c, "static/poller.js", { keepHost: ["document", "setInterval"] });
  assert.deepEqual(intervals, [500]);
});
