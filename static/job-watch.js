"use strict";
// Follow a job that a user action started. `step` fetches and shows the job,
// then returns the delay in ms before the next step, or null to stop. Steps
// never overlap: start() during a step runs one more step after it, and the
// returned promise settles after that step. stop() cancels the next step and
// drops the delay that a step in flight returns.
function watchJob(step) {
  let timer = null,
    current = null,
    again = false,
    epoch = 0;
  async function loop() {
    do {
      again = false;
      const mine = epoch;
      let delay;
      try {
        delay = await step();
        if (delay !== null && !Number.isFinite(delay)) {
          throw new TypeError(`A job watch step returned ${delay}.`);
        }
      } catch (error) {
        delay = null;
        console.error("Job watch step failed:", error);
      }
      if (!again && mine === epoch && delay !== null) {
        timer = setTimeout(start, delay);
      }
    } while (again);
    current = null;
  }
  function start() {
    clearTimeout(timer);
    timer = null;
    if (current === null) {
      current = loop();
    } else {
      again = true;
    }
    return current;
  }
  function stop() {
    clearTimeout(timer);
    timer = null;
    again = false;
    epoch += 1;
  }
  return { start, stop };
}
