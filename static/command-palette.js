"use strict";
// Step 1 lists commands and variant groups; choosing a group opens step 2 with
// only its variants. `actionStep` is null in step 1.
let actionRows = [],
  actionSelected = 0,
  actionStep = null;
function openCommandLink(url) {
  const link = el("a");
  link.href = url;
  link.target = "_blank";
  link.rel = "noopener noreferrer";
  document.body.append(link);
  link.click();
  link.remove();
}
const refreshingClaudeSessions = new Set();
async function refreshClaudeSession(record) {
  if (refreshingClaudeSessions.has(record.id)) {
    return;
  }
  if (record.agent !== "claude" || !record.sessionId) {
    throw new Error(
      "Wait for this Claude chat to register its conversation before refreshing it.",
    );
  }
  refreshingClaudeSessions.add(record.id);
  try {
    await loadInventory();
    const candidate = candidateForRecord(record);
    if (candidate?.host !== "embedded" || !candidate.generation) {
      throw new Error("Select a running embedded Claude chat to refresh.");
    }
    const focused = selectedAgentKey === candidateKey(candidate);
    await api("/api/terminal/close", {
      id: record.id,
      generation: candidate.generation,
    });
    const terminal = terminalViews.get(record.id);
    if (terminal) {
      terminal.stopped = true;
    }
    if (focused) {
      selectedAgentKey = `record:${record.id}`;
    }
    await loadInventory();
    await openSavedSession(record);
    await loadInventory();
    const resumed = candidateForRecord(record);
    if (focused && selectedAgentKey === `record:${record.id}` && resumed) {
      selectAgent(record, resumed);
    }
  } finally {
    refreshingClaudeSessions.delete(record.id);
  }
}
function commandEntries() {
  const entries = [],
    { record } = selectedChat(),
    candidate = record && candidateForRecord(record);
  // Entries that share verb and noun and differ by `target` are variants of one group.
  const add = (verb, noun, detail, run, { target, ...extra } = {}) =>
    entries.push({
      title:
        target === undefined
          ? `${verb}: ${noun}`
          : `${verb}: ${noun} · ${target}`,
      ...(target === undefined ? {} : { group: `${verb}: ${noun}`, target }),
      detail,
      run,
      ...extra,
    });
  if (record) {
    for (const item of agentWork(record)) {
      if (item.url) {
        add("open", item.source, item.title, () => openCommandLink(item.url), {
          target: workReference(item) || item.title,
        });
      }
    }
    for (const tree of gitDiffDirectories(record)) {
      const label = tree.branch || tree.path.split("/").pop(),
        refs = (tree.itemIds || [])
          .map((id) => (record.items || []).find((x) => x.id === id))
          .filter(Boolean)
          .map(workReference)
          .join(" · ");
      add(
        "review",
        "changes",
        [refs, tree.path].filter(Boolean).join(" · "),
        () => {
          selectAgent(record, candidate);
          openDiffReview(record, undefined, tree.path);
        },
        { target: label },
      );
      if (serviceClient.isDesktop) {
        add(
          "open",
          "editor diff",
          tree.path,
          () => serviceClient.openDiff(tree.path),
          { target: label },
        );
      }
    }
    for (const result of (
      artifactLibrary?.artifacts ||
      state.artifacts ||
      []
    ).filter((x) => (x.sessionIds || []).includes(record.id))) {
      add(
        "open",
        "result",
        [result.format, result.path || result.url].filter(Boolean).join(" · "),
        () => viewArtifact(result),
        { target: result.title },
      );
    }
    const addResourceLink = (resource, ref) => {
      try {
        const url = new URL(ref.value);
        if (
          !["https:", "http:"].includes(url.protocol) ||
          url.username ||
          url.password
        ) {
          return;
        }
        add(
          "open",
          "resource",
          resource.state || "Unknown",
          () => openCommandLink(url.href),
          { target: `${resource.name || resource.id} · ${ref.category}` },
        );
      } catch {
        // Not a URL; offer no link for this reference.
      }
    };
    for (const resource of record.resources || []) {
      for (const ref of resourceLinks(resource)) {
        addResourceLink(resource, ref);
      }
    }
    add("copy", "work links", record.title, () =>
      serviceClient.copyText(
        agentWork(record)
          .filter((x) => x.url)
          .map((x) => `[${workReference(x) || x.title}](${x.url})`)
          .join("\n"),
      ),
    );
    if (candidate) {
      if (record.agent === "claude" && candidate.host === "embedded") {
        add(
          "refresh",
          "claude session",
          `Close and resume this chat in ${
            record.cwd
          } · interrupts running commands and subagents`,
          () => refreshClaudeSession(record),
        );
      }
      add(
        "rename",
        "chat",
        record.title,
        () => {
          selectAgent(record, candidate);
          beginAgentRename();
        },
        { shortcut: "⌘R" },
      );
      add("snooze", "chat", record.title, () => snoozeAgentTab(candidate));
      add(
        "close",
        "chat",
        record.title,
        () => {
          selectAgent(record, candidate);
          return runTabCommand("close");
        },
        { shortcut: "⌘W" },
      );
      add(
        "toggle",
        "terminal focus",
        terminalFocus ? "Exit focus" : "Expand terminal",
        () => {
          selectAgent(record, candidate);
          terminalFocus = !terminalFocus;
          render();
        },
        { shortcut: "⌘⇧↵" },
      );
    }
  }
  for (const entry of liveCandidates()) {
    const linked = recordForCandidate(entry),
      snoozed = isAgentSnoozed(entry);
    add(
      snoozed ? "restore" : "switch",
      "agent",
      `${runtimeInfo(entry, linked).summary} · ${
        linked?.cwd || entry.cwd || ""
      }`,
      () => selectAgent(linked, entry),
      { target: linked?.title || entry.title },
    );
  }
  for (const [key, entry] of Object.entries(snoozedAgentTabs)) {
    if (!liveCandidates().some((c) => candidateKey(c) === key)) {
      add(
        "restore",
        "agent",
        "Saved conversation",
        () => restoreSnoozedTab(key, { disabled: false }),
        { target: entry.title },
      );
    }
  }
  for (const agent of ["claude", "codex"]) {
    add("new", `${agent} chat`, "Choose a local folder", () => {
      $("#quick-agent").value = agent;
      updateQuickAgent();
      return runTabCommand("new");
    });
  }
  if (closedAgentTabs.length) {
    add(
      "reopen",
      "closed chat",
      "Restore the last closed tab",
      () => runTabCommand("reopen"),
      { shortcut: "⌘⇧T" },
    );
  }
  for (const [view, label] of [
    ["agents", "agents"],
    ["all", "all items"],
    ["linear", "linear"],
    ["github", "github"],
    ["sentry", "sentry"],
    ["notes", "projects"],
    ["skills", "skills"],
    ["sessions", "activity"],
  ]) {
    add("go", label, "Dashboard view", () => openPage(view), {
      shortcut: shortcutLabel(setting("keybindings", {})[`view-${view}`]),
    });
  }
  for (const view of typeof savedViewsDocument !== "undefined"
    ? savedViewsDocument?.views || []
    : []) {
    add(
      "go",
      view.name,
      `${
        sourceNames[view.parent] || view.parent
      } saved view · ${savedViewSummary(view.filter)}`,
      () => activateSavedView(view.id),
    );
  }
  add(
    "toggle",
    "left sidebar",
    leftSidebarCollapsed ? "Show navigation" : "Hide navigation",
    () => toggleSidebar("left"),
    { shortcut: "⌘H" },
  );
  add(
    "toggle",
    "right sidebar",
    agentSidebarCollapsed ? "Show agent context" : "Hide agent context",
    () => toggleSidebar("right"),
    { shortcut: "⌘L" },
  );
  add("change", "theme", "Choose a light or dark theme", () =>
    openThemeChooser(),
  );
  if (source === "agents" && record) {
    add(
      "toggle",
      "chat terminal",
      "Shell in this chat’s working folder",
      () => toggleChatTerminal(),
      { shortcut: "⌘J" },
    );
  }
  add("find", "chats", "Search live, snoozed and saved chats", () =>
    showChatResults(),
  );
  add(
    "find",
    "work items",
    "Search tickets, PRs, projects, and saved chats",
    () => openPalette(),
    { shortcut: "⌘P" },
  );
  add("refresh", "work", "Sync connected sources; respects cooldowns", () =>
    $("#force-refresh").click(),
  );
  add("open", "sharing and inbox", "Share a chat or import a context", () => {
    $("#sharing-menu").open = true;
  });
  const recovery = chatPreparationContext();
  if (recovery.availability === "unavailable" || recovery.total > 0) {
    add(
      "open",
      "chat recovery",
      recovery.availability === "unavailable"
        ? "Local request status unavailable. Inspect saved recovery data."
        : `${recovery.total} pending local chat ${
            recovery.total === 1 ? "request" : "requests"
          }`,
      () => openChatRecovery(),
    );
  }
  if (typeof presentationCommands === "function") {
    for (const entry of presentationCommands()) {
      const [noun, ...target] = entry.noun.split(" · ");
      add(entry.verb, noun, entry.detail, entry.run, {
        target: target.length ? target.join(" · ") : undefined,
      });
    }
  }
  if (typeof setting === "function") {
    add(
      "open",
      "settings",
      "Appearance, shortcuts, agents, quick actions and connections",
      openSettings,
      { shortcut: shortcutLabel(setting("keybindings", {}).settings) },
    );
    for (const action of setting("quickActions", [])) {
      const identity = settingsStore.baseline();
      add("run", action.title, action.command, () =>
        chooseQuickAction(action, identity),
      );
    }
    const ids = {
      "toggle: left sidebar": "toggle-left",
      "toggle: right sidebar": "toggle-right",
      "find: work items": "find-work",
      "rename: chat": "agent-rename",
      "close: chat": "agent-close",
      "reopen: closed chat": "agent-reopen",
      "toggle: terminal focus": "terminal-focus",
    };
    const views = {
      agents: "agents",
      "all items": "all",
      linear: "linear",
      github: "github",
      sentry: "sentry",
      projects: "notes",
      skills: "skills",
      activity: "sessions",
    };
    for (const entry of entries) {
      const id =
        ids[entry.title] ||
        (entry.title.startsWith("go: ")
          ? `view-${views[entry.title.slice(4)]}`
          : null);
      if (id) {
        entry.shortcut = shortcutLabel(setting("keybindings", {})[id]);
      }
    }
  }
  return entries;
}
function openCommands() {
  if (!state) {
    return;
  }
  if ($("#action-palette").open) {
    requestDialogClose($("#action-palette"));
    return;
  }
  if (!requestDialogNavigation()) {
    return false;
  }
  $("#action-query").value = "";
  $("#action-palette").showModal();
  $("#action-query").setAttribute("aria-expanded", "true");
  paintCommands(true);
  $("#action-query").focus();
}
function paintCommandHints() {
  const row = actionRows[actionSelected],
    hints = [];
  if (actionStep !== null) {
    hints.push(["Back", "esc"]);
  }
  if (row !== undefined) {
    hints.push([row.kind === "group" ? "Choose" : "Run", "↵"]);
  }
  $("#action-hint").replaceChildren(
    ...hints.map(([label, key]) => {
      const hint = el("span", "", label);
      hint.append(el("kbd", "", key));
      return hint;
    }),
  );
}
function paintCommandSelection({ scroll = false } = {}) {
  const rows = $("#action-results").querySelectorAll(".action-command");
  for (const [index, row] of rows.entries()) {
    const selected = index === actionSelected;
    row.classList.toggle("selected", selected);
    row.setAttribute("aria-selected", String(selected));
  }
  const query = $("#action-query"),
    selected = rows[actionSelected];
  if (selected) {
    query.setAttribute("aria-activedescendant", selected.id);
    if (scroll) {
      selected.scrollIntoView({ block: "nearest" });
    }
  } else {
    query.removeAttribute("aria-activedescendant");
  }
  paintCommandHints();
}
// Rows for step 1. A group with one variant stays a plain command. A search
// also lists every variant, so a variant is found without opening its group.
function commandRows(entries, { searching }) {
  const sizes = new Map();
  for (const entry of entries) {
    if (entry.group !== undefined) {
      sizes.set(entry.group, (sizes.get(entry.group) || 0) + 1);
    }
  }
  const rows = [],
    emitted = new Set();
  for (const entry of entries) {
    if (entry.group === undefined || sizes.get(entry.group) === 1) {
      rows.push({
        kind: "command",
        entry,
        text: entry.title,
        detail: entry.detail,
      });
      continue;
    }
    if (!emitted.has(entry.group)) {
      emitted.add(entry.group);
      rows.push({
        kind: "group",
        title: entry.group,
        entries: entries.filter((x) => x.group === entry.group),
        text: entry.group,
        detail: "",
      });
    }
    if (searching) {
      rows.push({
        kind: "command",
        entry,
        text: entry.title,
        detail: entry.detail,
      });
    }
  }
  return rows;
}
function paintCommandRow(row, index) {
  const button = featureButton(
    "",
    () => runPaletteCommand(index),
    "action-command",
  );
  button.id = `action-command-${index}`;
  button.tabIndex = -1;
  button.setAttribute("role", "option");
  button.onmousedown = (event) => event.preventDefault();
  button.onfocus = () => {
    actionSelected = index;
    paintCommandSelection();
  };
  switch (row.kind) {
    case "command":
      button.append(el("span", "action-command-label", row.entry.title));
      break;
    case "group":
      button.setAttribute(
        "aria-label",
        `${row.title}, ${row.entries.length} choices`,
      );
      button.append(
        el("span", "action-command-label", row.title),
        el("small", "action-command-suffix", `${row.entries.length} ›`),
      );
      break;
    case "variant": {
      button.append(el("span", "action-command-label", row.entry.target));
      if (row.entry.detail) {
        const suffix = el("small", "action-command-suffix", row.entry.detail);
        suffix.title = row.entry.detail;
        button.append(suffix);
      }
      break;
    }
    default:
      throw new Error(`Unknown command row: ${row.kind}`);
  }
  const shortcut = row.kind === "group" ? "" : row.entry.shortcut;
  if (shortcut) {
    button.append(el("kbd", "", shortcut));
  }
  return button;
}
function paintCommands(reset, { select } = {}) {
  const query = $("#action-query")
      .value.trim()
      .toLowerCase()
      .replaceAll(":", " "),
    words = query.split(/\s+/).filter(Boolean);
  const rows =
    actionStep === null
      ? commandRows(commandEntries(), { searching: words.length > 0 })
      : actionStep.entries.map((entry) => ({
          kind: "variant",
          entry,
          text: entry.target,
          detail: entry.detail,
        }));
  const matches = rows
    .map((row, index) => {
      const text = `${row.text} ${row.detail}`
        .toLowerCase()
        .replaceAll(":", " ");
      return {
        row,
        index,
        direct: words.every((word) => text.includes(word)),
        score: query ? fuzzyScore(query, text) : 0,
      };
    })
    .filter((x) => Number.isFinite(x.score));
  const direct = matches.filter((x) => x.direct);
  actionRows = (direct.length ? direct : matches)
    .sort((a, b) => b.score - a.score || a.index - b.index)
    .map((x) => x.row);
  if (reset) {
    actionSelected = 0;
  }
  if (select !== undefined) {
    actionSelected = Math.max(0, actionRows.findIndex(select));
  }
  actionSelected = Math.max(0, Math.min(actionSelected, actionRows.length - 1));
  const list = $("#action-results");
  list.replaceChildren(...actionRows.map(paintCommandRow));
  if (!actionRows.length) {
    list.append(el("p", "form-hint", "No matching commands."));
  }
  if (reset) {
    list.scrollTop = 0;
  }
  paintCommandSelection();
}
function paintCommandStep() {
  const step = $("#action-step"),
    input = $("#action-query");
  step.hidden = actionStep === null;
  step.textContent = actionStep === null ? "" : actionStep.title;
  input.placeholder =
    actionStep === null ? "Execute a command…" : `Choose ${actionStep.title}…`;
  input.setAttribute(
    "aria-label",
    actionStep === null ? "Search commands" : `Choose ${actionStep.title}`,
  );
}
function enterCommandGroup(group) {
  actionStep = { title: group.title, entries: group.entries };
  $("#action-query").value = "";
  paintCommandStep();
  paintCommands(true);
  $("#action-query").focus({ preventScroll: true });
}
function leaveCommandGroup() {
  const title = actionStep.title;
  actionStep = null;
  $("#action-query").value = "";
  paintCommandStep();
  paintCommands(true, {
    select: (row) => row.kind === "group" && row.title === title,
  });
  $("#action-query").focus({ preventScroll: true });
}
async function runPaletteCommand(index) {
  const row = actionRows[index];
  if (row === undefined) {
    return;
  }
  if (row.kind === "group") {
    enterCommandGroup(row);
    return;
  }
  $("#action-palette").close();
  try {
    await row.entry.run();
  } catch (error) {
    toast(error.message || String(error));
  }
}
$("#action-query").oninput = (event) => {
  if (!event.isComposing) {
    paintCommands(true);
  }
};
$("#action-query").addEventListener("compositionend", () =>
  paintCommands(true),
);
// The shared Escape handler asks this first, so Escape in step 2 goes back to step 1.
$("#action-palette").dismissLayer = () => {
  if (actionStep === null) {
    return false;
  }
  leaveCommandGroup();
  return true;
};
$("#action-palette").addEventListener("close", () => {
  $("#action-query").value = "";
  $("#action-query").setAttribute("aria-expanded", "false");
  $("#action-query").removeAttribute("aria-activedescendant");
  actionRows = [];
  actionSelected = 0;
  actionStep = null;
  paintCommandStep();
});
$("#action-palette").addEventListener("keydown", (event) => {
  if (
    event.isComposing ||
    event.keyCode === 229 ||
    event.metaKey ||
    event.ctrlKey ||
    event.altKey ||
    event.shiftKey
  ) {
    return;
  }
  if (
    event.target !== $("#action-query") &&
    !event.target.closest(".action-command")
  ) {
    return;
  }
  if (["ArrowDown", "ArrowUp"].includes(event.key)) {
    event.preventDefault();
    event.stopPropagation();
    if (!actionRows.length) {
      return;
    }
    actionSelected =
      (actionSelected +
        (event.key === "ArrowDown" ? 1 : -1) +
        actionRows.length) %
      actionRows.length;
    $("#action-query").focus({ preventScroll: true });
    paintCommandSelection({ scroll: true });
  } else if (event.key === "Enter") {
    event.preventDefault();
    event.stopPropagation();
    void runPaletteCommand(actionSelected);
  } else if (
    event.key === "Backspace" &&
    actionStep !== null &&
    $("#action-query").value === ""
  ) {
    event.preventDefault();
    event.stopPropagation();
    leaveCommandGroup();
  }
});
$("#command-open").onclick = openCommands;
window.addEventListener("truffle-ui-command", (event) => {
  if (event.detail === "settings") {
    openSettings();
  } else if (event.detail === "commands") {
    openCommands();
  } else if (event.detail === "find-work") {
    if ($("#command-dialog").open) {
      requestDialogClose($("#command-dialog"));
    } else {
      openPalette();
    }
  } else if (event.detail === "toggle-left") {
    toggleSidebar("left");
  } else if (event.detail === "toggle-right") {
    toggleSidebar("right");
  }
});
document.addEventListener(
  "keydown",
  (event) => {
    if (
      !(typeof appSettings !== "undefined" && appSettings) &&
      (/Mac|iPhone|iPad/.test(navigator.platform)
        ? event.metaKey
        : event.ctrlKey) &&
      !event.shiftKey &&
      !event.altKey &&
      !event.isComposing &&
      event.key.toLowerCase() === "k"
    ) {
      event.preventDefault();
      event.stopImmediatePropagation();
      openCommands();
    }
  },
  true,
);
