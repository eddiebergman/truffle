import { readdirSync, readFileSync } from "node:fs";
import js from "@eslint/js";
import * as eslintScope from "eslint-scope";
import * as espree from "espree";
import globals from "globals";

const root = new URL(".", import.meta.url);
const read = (path) => readFileSync(new URL(path, root), "utf8");

// index.html loads these classic scripts in order. They share one global scope,
// so each script may use the top-level names that the others declare.
const loadedScripts = [
  ...read("static/index.html").matchAll(/<script src="\/([^"]+)"/g),
].map((match) => `static/${match[1]}`);
const vendorScripts = loadedScripts.filter((path) =>
  path.startsWith("static/vendor/"),
);
const ownScripts = loadedScripts.filter(
  (path) => !path.startsWith("static/vendor/"),
);
// Static scripts that index.html does not load. The tests load them next to the others.
const unloadedScripts = readdirSync(new URL("static", root))
  .filter((name) => name.endsWith(".js"))
  .map((name) => `static/${name}`)
  .filter((path) => !ownScripts.includes(path));
const testFiles = ["tests", "tests/helpers"].flatMap((directory) =>
  readdirSync(new URL(directory, root))
    .filter((name) => name.endsWith(".cjs"))
    .map((name) => `${directory}/${name}`),
);

// Vendored bundles are minified UMD code, so name their globals here and check
// that each bundle still defines its name.
const vendorGlobals = {
  "static/vendor/xterm.js": "Terminal",
  "static/vendor/addon-fit.js": "FitAddon",
  "static/vendor/markdown-it.min.js": "markdownit",
};
for (const path of vendorScripts) {
  const name = vendorGlobals[path];
  if (name === undefined) {
    throw new Error(
      `index.html loads ${path}; add its global to eslint.config.mjs`,
    );
  }
  if (!read(path).includes(name)) {
    throw new Error(`${path} no longer mentions its global ${name}`);
  }
}

function parse(path, sourceType) {
  return espree.parse(read(path), {
    ecmaVersion: "latest",
    sourceType,
    range: true,
  });
}

function* walk(node) {
  if (Array.isArray(node)) {
    for (const child of node) {
      yield* walk(child);
    }
  } else if (
    node !== null &&
    typeof node === "object" &&
    typeof node.type === "string"
  ) {
    yield node;
    for (const [key, child] of Object.entries(node)) {
      if (key !== "parent") {
        yield* walk(child);
      }
    }
  }
}

// The globals that one classic script defines and uses.
function scriptGlobals(path) {
  const program = parse(path, "script");
  const { globalScope } = eslintScope.analyze(program, {
    ecmaVersion: 2026,
    sourceType: "script",
  });
  const declared = {};
  for (const variable of globalScope.variables) {
    const [definition] = variable.defs;
    if (definition !== undefined) {
      const constant =
        definition.type === "ClassName" ||
        (definition.type === "Variable" && definition.parent.kind === "const");
      declared[variable.name] = constant ? "readonly" : "writable";
    }
  }
  // `window.name = ...` defines a global at any depth.
  const assigned = {};
  for (const node of walk(program)) {
    const { left } = node;
    if (
      node.type === "AssignmentExpression" &&
      left.type === "MemberExpression" &&
      !left.computed &&
      left.object.type === "Identifier" &&
      left.object.name === "window"
    ) {
      assigned[left.property.name] = "writable";
    }
  }
  const used = new Set(
    globalScope.through.map((reference) => reference.identifier.name),
  );
  return { declared, assigned, used };
}

const scripts = new Map(
  [...ownScripts, ...unloadedScripts].map((path) => [
    path,
    scriptGlobals(path),
  ]),
);
const vendorGlobalSettings = Object.fromEntries(
  vendorScripts.map((path) => [vendorGlobals[path], "readonly"]),
);

// The globals that a script can use. Only scripts that index.html loads provide
// them. A script's own declarations stay out, so `no-redeclare` does not report them.
function sharedGlobals(path) {
  const shared = { ...vendorGlobalSettings };
  for (const other of ownScripts) {
    const { declared, assigned } = scripts.get(other);
    Object.assign(shared, assigned, other === path ? {} : declared);
  }
  for (const name of Object.keys(scripts.get(path).declared)) {
    delete shared[name];
  }
  return shared;
}

// Names that the tests can reach on a script context, such as `c.renderRow`.
const testNames = new Set(
  testFiles.flatMap((path) =>
    [...walk(parse(path, "script"))]
      .filter((node) => node.type === "Identifier")
      .map((node) => node.name),
  ),
);

// A top-level declaration is in use when another script or a test uses it.
// `no-unused-vars` sees one file only, so this rule tells it about those uses.
const sharedGlobalsAreUsed = {
  meta: {
    type: "problem",
    docs: {
      description:
        "Mark globals that other static scripts or tests use as used",
    },
    schema: [],
  },
  create(context) {
    const path = context.filename.slice(new URL(root).pathname.length);
    const others = [...scripts].filter(([other]) => other !== path);
    return {
      Program(node) {
        for (const name of Object.keys(scripts.get(path).declared)) {
          if (
            testNames.has(name) ||
            others.some(([, { used }]) => used.has(name))
          ) {
            context.sourceCode.markVariableAsUsed(name, node);
          }
        }
      },
    };
  },
};

const rules = {
  ...js.configs.recommended.rules,
  eqeqeq: "error",
  "no-var": "error",
  "prefer-const": "error",
};

export default [
  {
    // Third-party and generated code.
    ignores: [
      "static/vendor/**",
      "desktop/node_modules/**",
      "desktop/target/**",
      "desktop/gen/**",
    ],
  },
  {
    linterOptions: {
      noInlineConfig: true,
      reportUnusedDisableDirectives: "error",
      reportUnusedInlineConfigs: "error",
    },
  },
  {
    files: ["**/*.js", "**/*.cjs", "**/*.mjs"],
    rules,
  },
  ...[...ownScripts, ...unloadedScripts].map((path) => ({
    files: [path],
    plugins: {
      scripts: { rules: { "shared-globals-are-used": sharedGlobalsAreUsed } },
    },
    languageOptions: {
      sourceType: "script",
      globals: {
        ...globals.browser,
        // Scripts export their helpers to the node tests when `module` exists.
        module: "readonly",
        ...sharedGlobals(path),
      },
    },
    rules: { "scripts/shared-globals-are-used": "error" },
  })),
  {
    files: ["desktop/frontend/*.js"],
    languageOptions: { sourceType: "module", globals: globals.browser },
  },
  {
    files: ["tests/**/*.cjs"],
    languageOptions: { sourceType: "commonjs", globals: globals.node },
  },
  {
    files: ["desktop/build-diffs.mjs", "*.config.mjs"],
    languageOptions: { sourceType: "module", globals: globals.node },
  },
];
