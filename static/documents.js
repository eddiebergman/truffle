/* Revisioned documents that the service saves by compare-and-swap. */
"use strict";
// The service refused these writes, so nothing changed. Any other failure,
// such as a lost connection or a 5xx, leaves the outcome unknown.
function writeFailureKind(error) {
  switch (error?.status) {
    case 409:
      return "conflict";
    case 400:
    case 401:
    case 403:
    case 404:
      return "rejected";
    default:
      return "unknown";
  }
}
function validDocumentIdentity(value) {
  return (
    Number.isSafeInteger(value?.revision) &&
    value.revision >= 0 &&
    typeof value.contentHash === "string" &&
    /^[a-f0-9]{64}$/.test(value.contentHash)
  );
}
// The owner keeps the cached document in its own binding; `get` and `set` reach it.
// A load that started before a save with a possibly changed document does not commit.
function documentStore({ get, set, read, write, validate = (value) => value }) {
  let generation = 0;
  return {
    baseline() {
      const document = get();
      return document
        ? { revision: document.revision, contentHash: document.contentHash }
        : null;
    },
    async load({ canApply = () => true } = {}) {
      const started = generation,
        next = validate(await read());
      if (generation !== started || !canApply()) {
        return null;
      }
      set(next);
      return next;
    },
    async save(baseline, patch) {
      let response;
      try {
        response = await write({
          revision: baseline.revision,
          contentHash: baseline.contentHash,
          ...patch,
        });
      } catch (error) {
        const kind = writeFailureKind(error);
        if (kind === "unknown") {
          generation++;
        }
        return {
          kind,
          error: error.message || "The save could not be confirmed.",
        };
      }
      generation++;
      let document;
      try {
        document = validate(response);
      } catch (error) {
        return { kind: "unknown", error: error.message };
      }
      set(document);
      return { kind: "accepted", document };
    },
  };
}
if (typeof module !== "undefined") {
  module.exports = { writeFailureKind, validDocumentIdentity, documentStore };
}
