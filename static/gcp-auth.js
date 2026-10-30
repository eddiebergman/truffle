/* Background Google login. Only an explicit click starts the fixed CLI commands. */
"use strict";
(() => {
  const button = $("#gcp-login"),
    panel = $("#gcp-login-status"),
    cancel = $("#gcp-login-cancel");
  let current = null,
    pending = false;
  function paint() {
    const running = Boolean(current?.running),
      cooling = (current?.cooldownUntil || 0) * 1000 > Date.now();
    button.disabled = pending || running || cooling;
    button.setAttribute("aria-busy", String(running));
    button.querySelector("span:last-child").textContent = running
      ? "GCP login…"
      : "GCP login";
    panel.hidden = !current?.runId;
    for (const key of ["cli", "adc"]) {
      const row = $(`#gcp-login-${key}`),
        task = current?.[key];
      const labels = {
        running: "Browser…",
        succeeded: "Signed in",
        failed: "Failed",
        cancelled: "Cancelled",
        timed_out: "Timed out",
      };
      row.textContent = `${key === "cli" ? "CLI" : "ADC"} · ${
        labels[task?.status] || "Waiting"
      }`;
      row.dataset.status = task?.status || "";
      row.title = task?.message || "";
      row.setAttribute(
        "aria-label",
        `${
          key === "cli" ? "Google Cloud CLI" : "Application Default Credentials"
        }: ${task?.message || "Waiting"}`,
      );
    }
    cancel.hidden = !running;
    cancel.disabled = Boolean(current?.cancelling);
    cancel.textContent = current?.cancelling ? "Cancelling…" : "Cancel login";
  }
  async function load() {
    try {
      current = await api("/api/gcp-auth");
      paint();
      return current.running || (current.cooldownUntil || 0) * 1000 > Date.now()
        ? 1000
        : null;
    } catch (error) {
      if (current?.running) {
        return 5000;
      }
      if (error.message.includes("Not found")) {
        button.disabled = true;
        button.title =
          "Restart the updated Truffle service to enable GCP login.";
      }
      return null;
    }
  }
  const watch = watchJob(load);
  button.onclick = async () => {
    if (pending || current?.running) {
      return;
    }
    pending = true;
    paint();
    try {
      current = await api("/api/gcp-auth/start", {});
      toast(
        typeof state !== "undefined" && state?.demo
          ? "Demo login completed; no credentials changed."
          : "Complete both GCP login tabs in your browser.",
      );
    } catch (error) {
      toast(error.message);
    } finally {
      pending = false;
      paint();
      void watch.start();
    }
  };
  cancel.onclick = async () => {
    cancel.disabled = true;
    try {
      current = await api("/api/gcp-auth/cancel", {});
      paint();
      void watch.start();
    } catch (error) {
      toast(error.message);
      cancel.disabled = false;
    }
  };
  window.addEventListener("truffle-service-restarted", () => {
    void watch.start();
  });
  void watch.start();
})();
