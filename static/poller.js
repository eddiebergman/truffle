"use strict";
// One clock for background refreshes. A task runs when its interval has passed
// since its last run finished, or after kick(). No task runs while the page is
// hidden, a task never overlaps itself, and a task with pauseInDialog waits
// while a dialog is open. A blocked task stays due and runs once it may.
function createPoller({ now, heartbeat, hidden, dialogOpen }) {
  const tasks = new Map();
  function due(task) {
    const every = typeof task.every === "function" ? task.every() : task.every;
    return task.kicked || now() - task.last >= every;
  }
  async function start(task) {
    task.running = true;
    task.kicked = false;
    try {
      await task.run();
    } catch (error) {
      console.error(`Poll task ${task.name} failed:`, error);
    } finally {
      task.running = false;
      task.last = now();
      if (task.kicked) {
        check(task);
      }
    }
  }
  function check(task) {
    if (
      task.running ||
      hidden() ||
      (task.pauseInDialog && dialogOpen()) ||
      !due(task) ||
      (task.when && !task.when())
    ) {
      return;
    }
    void start(task);
  }
  function register(name, { every, when, run, pauseInDialog = false }) {
    if (tasks.has(name)) {
      throw new Error(`Poll task ${name} is already registered.`);
    }
    tasks.set(name, {
      name,
      every,
      when,
      run,
      pauseInDialog,
      last: now(),
      kicked: false,
      running: false,
    });
  }
  // Run the task as soon as its guards allow, without waiting for its interval.
  function kick(name) {
    const task = tasks.get(name);
    if (!task) {
      throw new Error(`Unknown poll task ${name}.`);
    }
    task.kicked = true;
    void Promise.resolve().then(() => check(task));
  }
  heartbeat(() => {
    for (const task of tasks.values()) {
      check(task);
    }
  });
  return { register, kick };
}
const poller = createPoller({
  now: () => Date.now(),
  heartbeat: (tick) => setInterval(tick, 500),
  hidden: () => document.hidden,
  dialogOpen: () => document.querySelector("dialog[open]") !== null,
});
