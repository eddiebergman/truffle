"use strict";
// These two forms edit preset creation and schedule metadata, never run a skill.
function skillEditorMutable(current = featureForms.get($("#skill-editor"))) {
  return featureFormMutable(current) && !current.loading;
}
function skillScheduleMutable(current) {
  return featureFormMutable(current) && !current.loading && !current.loadFailed;
}
function openSkillEditor({
  skill = null,
  source = "installed",
  cwd = "",
  onSaved = null,
  returnLabel = "Close and inspect Skills",
} = {}) {
  const dialog = $("#skill-editor"),
    form = $("#skill-editor-form");
  if (!requestDialogNavigation()) {
    return false;
  }
  disposeFeatureForm(dialog);
  form.reset();
  const agent = skill
    ? skill.agents.includes(setting("defaultAgent", "claude"))
      ? setting("defaultAgent", "claude")
      : skill.agents[0]
    : setting("defaultAgent", "claude");
  form.elements.agent.value = agent;
  form.elements.cwd.value = cwd;
  if (skill) {
    form.elements.title.value = skill.name;
    form.elements.skill.value = skill.path;
    form.elements.command.value =
      (agent === "codex" ? "$" : "/") + (skill.commandName || skill.name);
    if (form.elements.description) {
      form.elements.description.value = skill.description || "";
    }
  }
  $("#skill-editor-error").hidden = true;
  dialog.dataset.skillSource = skill ? "installed" : source;
  const current = createFeatureForm(dialog, form, null);
  current.kind = "preset";
  current.onSaved = onSaved;
  current.acceptedText = "Skill saved.";
  current.ui.inspect.textContent = returnLabel;
  dialog.showModal();
  return current;
}
function updateScheduleFields() {
  const form = $("#schedule-form");
  $("#schedule-time").hidden = form.elements.mode.value !== "daily";
  $("#schedule-interval").hidden = form.elements.mode.value !== "interval";
}
function paintSkillSchedule(current, metadata) {
  const form = current.host,
    schedule = metadata.schedule;
  current.baseline = metadata;
  form.elements.presetId.value = current.target;
  for (const field of ["mode", "time", "everyMinutes", "missed"]) {
    form.elements[field].value = schedule[field];
  }
  form.elements.enabled.checked = schedule.enabled;
  updateScheduleFields();
}
async function readSkillSchedule(current) {
  if (!current.isCurrent() || current.loading || current.pending) {
    return;
  }
  current.loading = true;
  current.loadFailed = false;
  holdFormFocus(current.host, current);
  freezeFeatureForm(current, true);
  current.ui.message.textContent = "Loading current schedule…";
  current.ui.refresh.disabled = true;
  try {
    const metadata = await api("/api/commands/invoke", {
      name: "skills.preset.read",
      args: { presetId: current.target },
    });
    if (!current.isCurrent()) {
      return;
    }
    if (
      metadata.presetId !== current.target ||
      !metadata.schedule ||
      !metadata.scheduleExpected
    ) {
      throw new Error("Current schedule is unavailable.");
    }
    paintSkillSchedule(current, metadata);
    current.result = null;
    current.conflict = false;
    current.ui.message.textContent = "";
    current.ui.refresh.textContent = "Refresh view";
    current.ui.refresh.onclick = () => refreshFeatureForm(current);
    current.ui.refresh.hidden = true;
  } catch (error) {
    if (current.isCurrent()) {
      current.loadFailed = true;
      current.ui.message.textContent = error.message;
      current.ui.refresh.textContent = "Reload schedule";
      current.ui.refresh.hidden = false;
      current.ui.refresh.onclick = () => readSkillSchedule(current);
    }
  } finally {
    current.loading = false;
    if (current.isCurrent()) {
      freezeFeatureForm(current, current.loadFailed);
      current.ui.refresh.disabled = false;
      restoreFormFocus(
        current,
        current.loadFailed ? current.ui.refresh : current.host.elements.mode,
      );
    }
  }
}
function openSkillSchedule(preset) {
  const dialog = $("#schedule-editor"),
    form = $("#schedule-form");
  if (!requestDialogNavigation()) {
    return false;
  }
  disposeFeatureForm(dialog);
  form.reset();
  $("#schedule-title").textContent = `${preset.title} schedule`;
  $("#schedule-error").hidden = true;
  const current = createFeatureForm(dialog, form, preset.id);
  current.kind = "schedule";
  current.acceptedText = "Schedule saved.";
  dialog.showModal();
  return readSkillSchedule(current);
}
async function saveSkillEditor(current, name, args) {
  let invalidated = false,
    savedError = null;
  const invalidateLibrary = () => {
    if (current.kind === "preset" && !invalidated) {
      libraryGeneration++;
      libraryFetched = 0;
      invalidated = true;
    }
  };
  await submitFeatureForm(current, {
    write: async () => {
      try {
        const receipt = await api("/api/commands/invoke", { name, args });
        invalidateLibrary();
        if (
          receipt.saved !== true ||
          typeof receipt.presetId !== "string" ||
          !receipt.presetId
        ) {
          throw new Error("The save identity could not be confirmed.");
        }
        if (current.kind === "preset" && current.isCurrent()) {
          current.acceptedPresetId = receipt.presetId;
          // The caller learns the accepted ID even when the view cannot refresh.
          try {
            current.onSaved?.(receipt.presetId);
          } catch (error) {
            savedError = error;
          }
        }
      } catch (error) {
        if (writeFailureKind(error) === "unknown") {
          invalidateLibrary();
        }
        throw error;
      }
    },
    acceptedText: current.acceptedText,
    afterReady: async () => {
      // The caller's failure is reported once; Refresh view then closes normally.
      if (savedError !== null) {
        const error = savedError;
        savedError = null;
        throw error;
      }
      if (current.kind === "preset") {
        const library = await loadArtifactLibrary(true);
        if (!library.accepted) {
          throw new Error("The Skills library could not refresh.");
        }
      }
      current.dialog.close();
    },
    reloadUnknown:
      current.kind === "preset" ? () => loadArtifactLibrary(true) : null,
    onConflict:
      current.kind === "schedule"
        ? (result) => {
            current.ui.message.textContent = `${
              result.error
            } Your draft remains. Reload schedule replaces it with the current schedule before saving.`;
            current.ui.refresh.textContent = "Reload schedule";
            current.ui.refresh.hidden = false;
            current.ui.refresh.onclick = () => readSkillSchedule(current);
            restoreFormFocus(current, current.ui.refresh);
          }
        : null,
  });
}
$("#skill-editor-form").onsubmit = async (event) => {
  event.preventDefault();
  const current = featureForms.get($("#skill-editor"));
  if (!skillEditorMutable(current)) {
    return;
  }
  const args = {};
  for (const field of [
    "title",
    "cwd",
    "skill",
    "command",
    "agent",
    "description",
  ]) {
    if (current.host.elements[field]) {
      args[field] = current.host.elements[field].value;
    }
  }
  await saveSkillEditor(current, "skills.save", args);
};
$("#schedule-form").elements.mode.onchange = updateScheduleFields;
$("#schedule-form").onsubmit = async (event) => {
  event.preventDefault();
  const current = featureForms.get($("#schedule-editor"));
  if (!skillScheduleMutable(current)) {
    return;
  }
  const form = current.host,
    values = {
      mode: form.elements.mode.value,
      time: form.elements.time.value,
      everyMinutes: Number(form.elements.everyMinutes.value),
      missed: form.elements.missed.value,
      enabled: form.elements.enabled.checked,
    },
    { patch, expected } = checkedPatch(
      {
        values: current.baseline.schedule,
        expected: current.baseline.scheduleExpected,
      },
      values,
      Object.keys(values),
    );
  if (!Object.keys(patch).length) {
    current.dialog.close();
    return;
  }
  await saveSkillEditor(current, "skills.schedule", {
    presetId: current.target,
    patch,
    expected,
  });
};
