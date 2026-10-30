const { test } = require("node:test");
const assert = require("node:assert/strict");

// A Biome info diagnostic for the first `text` after `after` in `source`.
function diagnostic(rule, path, source, text, after = "") {
  const offset = source.indexOf(text, source.indexOf(after) + after.length);
  assert.notEqual(offset, -1, text);
  const position = (index) => {
    const before = source.slice(0, index).split("\n");
    return { line: before.length, column: before.at(-1).length + 1 };
  };
  return {
    severity: "info",
    category: `lint/${rule}`,
    message: `${rule} ${text}`,
    location: {
      path,
      start: position(offset),
      end: position(offset + text.length),
    },
  };
}

async function check({
  loaded = {},
  unloaded = {},
  vendorGlobals = [],
  testSource = "",
  diagnostics,
}) {
  const { checkScripts } = await import("../desktop/check-js.mjs");
  return checkScripts({
    sources: new Map(Object.entries({ ...loaded, ...unloaded })),
    loaded: Object.keys(loaded),
    unloaded: Object.keys(unloaded),
    vendorGlobals,
    testSource,
    diagnostics: diagnostics(loaded, unloaded),
  }).map(({ path, message }) => `${path}: ${message}`);
}

test("a name is declared by a loaded script, window or a vendor bundle", async () => {
  const user = "a();\nb();\nc();\nd();\ne();\nf();\n";
  assert.deepEqual(
    await check({
      loaded: {
        "a.js": "function a() {}\nconst b = 1;\nwindow.c = 2;\n",
        // A strict script keeps a function in a block out of the global scope.
        "b.js": '"use strict";\nif (a) {\n  function f() {}\n}\n',
        "user.js": user,
      },
      unloaded: { "other.js": "function e() {}\n" },
      vendorGlobals: ["d"],
      diagnostics: () =>
        ["a", "b", "c", "d", "e", "f"].map((name) =>
          diagnostic(
            "correctness/noUndeclaredVariables",
            "user.js",
            user,
            name,
          ),
        ),
    }),
    [
      "user.js: correctness/noUndeclaredVariables e",
      "user.js: correctness/noUndeclaredVariables f",
    ],
  );
});

test("an unused top-level name is used by another script or a test", async () => {
  const own =
    "function a() {}\nfunction b() {}\nfunction c() {\n  const d = 1;\n}\nfunction e() {}\n";
  assert.deepEqual(
    await check({
      loaded: { "own.js": own, "user.js": "a();\n" },
      testSource: "c.b(); d();",
      diagnostics: ({ "user.js": user }) => [
        diagnostic("correctness/noUndeclaredVariables", "user.js", user, "a"),
        ...["a", "b", "d", "e"].map((name) =>
          diagnostic(
            "correctness/noUnusedVariables",
            "own.js",
            own,
            name,
            "function ",
          ),
        ),
      ],
    }),
    [
      "own.js: correctness/noUnusedVariables d",
      "own.js: correctness/noUnusedVariables e",
    ],
  );
});

test("a top-level let is assigned by another script or a test", async () => {
  const own = "let a = 1;\nlet b = 1;\nlet c = 1;\n";
  assert.deepEqual(
    await check({
      loaded: { "own.js": own, "user.js": "a = 2;\nb + 1;\n" },
      testSource: "c",
      diagnostics: ({ "user.js": user }) => [
        diagnostic("correctness/noUndeclaredVariables", "user.js", user, "a"),
        diagnostic("correctness/noUndeclaredVariables", "user.js", user, "b"),
        ...["let a", "let b", "let c"].map((text) =>
          diagnostic("style/useConst", "own.js", own, text),
        ),
      ],
    }),
    ["own.js: style/useConst let b"],
  );
});

test("loaded scripts declare each top-level name once", async () => {
  assert.deepEqual(
    await check({
      loaded: {
        "a.js": '"use strict";\nconst a = 1;\nfunction b() {}\n',
        "b.js": '"use strict";\nfunction b() {}\n',
        "c.js": "let a = 2;\n",
      },
      unloaded: { "d.js": "const a = 3;\n" },
      diagnostics: () => [],
    }),
    [
      "b.js: b is already declared in a.js",
      "c.js: Identifier 'a' has already been declared",
    ],
  );
});

test("empty function bodies are allowed and empty blocks are not", async () => {
  const source =
    "function a() {}\nconst b = () => {};\nconst c = { d() {} };\nif (a) {}\ntry {\n  a();\n} catch (error) {}\nfor (const e of []) {}\n";
  const blocks = [];
  for (
    let index = source.indexOf("{}");
    index !== -1;
    index = source.indexOf("{}", index + 1)
  ) {
    blocks.push(source.slice(0, index));
  }
  assert.deepEqual(
    await check({
      loaded: { "a.js": source },
      diagnostics: () =>
        blocks.map((before) =>
          diagnostic(
            "suspicious/noEmptyBlockStatements",
            "a.js",
            source,
            "{}",
            before,
          ),
        ),
    }),
    [
      "a.js: suspicious/noEmptyBlockStatements {}",
      "a.js: suspicious/noEmptyBlockStatements {}",
      "a.js: suspicious/noEmptyBlockStatements {}",
    ],
  );
});

test("suppression comments are reported", async () => {
  const { findSuppressions } = await import("../desktop/check-js.mjs");
  assert.deepEqual(
    findSuppressions(
      "a.js",
      // `\x2F` is `/`, so check-js does not report this file.
      'a();\n\x2F\x2F biome-ignore lint: reason\n\x2F* biome-ignore-all */\nconst b = "biome-ignore";\n',
    ).map(({ line }) => line),
    [2, 3],
  );
});
