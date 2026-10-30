const { test } = require("node:test"),
  assert = require("node:assert/strict");
const { browserContext, loadScripts } = require("./helpers/static-script.cjs");

function fixture() {
  let clock = 0,
    next = 1;
  const timers = new Map(),
    errors = [];
  const c = browserContext({
    setTimeout: (fn, ms) => {
      timers.set(next, { at: clock + ms, fn });
      return next++;
    },
    clearTimeout: (id) => timers.delete(id),
    console: { error: (...args) => errors.push(args) },
  });
  loadScripts(c, "static/job-watch.js", {
    keepHost: ["setTimeout", "clearTimeout", "console"],
  });
  const settle = () => new Promise((resolve) => setImmediate(resolve));
  return {
    c,
    timers,
    errors,
    settle,
    async advance(ms) {
      clock += ms;
      for (const [id, timer] of [...timers]) {
        if (timer.at <= clock) {
          timers.delete(id);
          timer.fn();
        }
      }
      await settle();
    },
  };
}
// A step that waits until the test releases it.
function gate() {
  let open;
  const wait = new Promise((resolve) => (open = resolve));
  return { wait, open: (value) => open(value) };
}

test("a watch steps now and then after each delay its step returns until a step returns null", async () => {
  const f = fixture(),
    delays = [1000, 5000, null];
  let runs = 0;
  const watch = f.c.watchJob(() => {
    runs++;
    return delays.shift();
  });
  await watch.start();
  assert.equal(runs, 1);
  await f.advance(999);
  assert.equal(runs, 1);
  await f.advance(1);
  assert.equal(runs, 2);
  await f.advance(4999);
  assert.equal(runs, 2);
  await f.advance(1);
  assert.equal(runs, 3);
  assert.equal(f.timers.size, 0);
});

test("start during a step runs one more step after it, never two at once", async () => {
  const f = fixture(),
    gates = [gate(), gate()];
  let active = 0,
    most = 0,
    runs = 0;
  const watch = f.c.watchJob(async () => {
    active++;
    most = Math.max(most, active);
    const value = await gates[runs++].wait;
    active--;
    return value;
  });
  const first = watch.start(),
    second = watch.start(),
    third = watch.start();
  await f.settle();
  assert.equal(runs, 1);
  gates[0].open(750);
  await f.settle();
  assert.equal(runs, 2);
  let settled = false;
  void second.then(() => (settled = true));
  await f.settle();
  assert.equal(settled, false);
  gates[1].open(null);
  await Promise.all([first, second, third]);
  assert.equal(most, 1);
  assert.equal(runs, 2);
  assert.equal(f.timers.size, 0);
});

test("start while a step waits cancels the wait and steps now", async () => {
  const f = fixture();
  let runs = 0;
  const watch = f.c.watchJob(() => (++runs === 1 ? 2000 : null));
  await watch.start();
  await watch.start();
  assert.equal(runs, 2);
  assert.equal(f.timers.size, 0);
});

test("stop cancels the next step, a queued step, and the delay that a step in flight returns", async () => {
  const f = fixture(),
    step = gate();
  let runs = 0;
  const waiting = f.c.watchJob(() => {
    runs++;
    return 1000;
  });
  await waiting.start();
  waiting.stop();
  await f.advance(5000);
  assert.equal(runs, 1);

  let flights = 0;
  const flying = f.c.watchJob(() => {
    flights++;
    return step.wait;
  });
  const done = flying.start();
  void flying.start();
  flying.stop();
  step.open(1000);
  await done;
  assert.equal(flights, 1);
  assert.equal(f.timers.size, 0);
});

test("a step that throws or returns no delay stops the watch and logs the error", async () => {
  const f = fixture();
  let runs = 0;
  const failing = f.c.watchJob(() => {
    runs++;
    throw new Error("offline");
  });
  await failing.start();
  const forgetful = f.c.watchJob(() => {
    runs++;
  });
  await forgetful.start();
  assert.equal(runs, 2);
  assert.equal(f.timers.size, 0);
  assert.equal(f.errors.length, 2);
  assert.match(String(f.errors[1][1]), /returned undefined/);
});
