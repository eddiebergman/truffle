const { test } = require("node:test");
const assert = require("node:assert/strict");

const lines = async (css) => {
  const { checkCss } = await import("../desktop/check-css.mjs");
  return checkCss(css).map(({ line }) => line);
};

test("a base rule after its pseudo-class state is reported", async () => {
  assert.deepEqual(
    await lines(".a:hover {\n  margin: 0;\n}\n.a {\n  margin: 0;\n}\n"),
    [4],
  );
  assert.deepEqual(
    await lines(".a {\n  margin: 0;\n}\n.a:hover {\n  margin: 0;\n}\n"),
    [],
  );
});

test("only selectors that end in the same compound are compared", async () => {
  assert.deepEqual(
    await lines(".x .a {\n  margin: 0;\n}\n.a,\n.b {\n  margin: 0;\n}\n"),
    [4],
  );
  assert.deepEqual(
    await lines(".x .a {\n  margin: 0;\n}\n.b {\n  margin: 0;\n}\n"),
    [],
  );
  assert.deepEqual(
    await lines(".x .a::before {\n  margin: 0;\n}\n.a {\n  margin: 0;\n}\n"),
    [],
  );
});

test("specificity follows :is, :not, :where and ids", async () => {
  assert.deepEqual(
    await lines(
      ":is(#x, .y) .a {\n  margin: 0;\n}\n.z .w .a {\n  margin: 0;\n}\n",
    ),
    [4],
  );
  assert.deepEqual(
    await lines(":where(#x) .a {\n  margin: 0;\n}\n.a {\n  margin: 0;\n}\n"),
    [],
  );
  assert.deepEqual(
    await lines(
      ".a:not(.b, #c) {\n  margin: 0;\n}\n.x .y .a {\n  margin: 0;\n}\n",
    ),
    [4],
  );
});

test("rules in an at-rule block are compared on their own", async () => {
  assert.deepEqual(
    await lines(
      ".x .a {\n  margin: 0;\n}\n@media (min-width: 1px) {\n  .a {\n    margin: 0;\n  }\n}\n",
    ),
    [],
  );
});

test("media queries use min- and max- features", async () => {
  assert.deepEqual(
    await lines("@media (width >= 600px) {\n  .a {\n    margin: 0;\n  }\n}\n"),
    [1],
  );
  assert.deepEqual(
    await lines(
      "@media (min-width: 600px) {\n  .a {\n    margin: 0;\n  }\n}\n",
    ),
    [],
  );
});

test("named colors are reported outside names, strings and URLs", async () => {
  assert.deepEqual(
    await lines(".a {\n  margin: 0;\n  border: 1px solid Red;\n}\n"),
    [3],
  );
  assert.deepEqual(
    await lines(
      '.red {\n  --red: transparent;\n  color: var(--red);\n  content: "white";\n  background: url(black.png);\n}\n',
    ),
    [],
  );
});
