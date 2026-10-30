import { spawnSync } from "node:child_process";
import { readdirSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

// Script checks that Biome cannot make alone:
// - static/*.js are classic scripts that share one global scope, but Biome
//   checks each file on its own. For them biome.json reports undeclared names,
//   unused names and `let`s that could be `const` as info. This check keeps
//   such a report unless another loaded script declares the name, or another
//   script or a test uses or assigns the top-level name.
// - Two loaded scripts must not declare the same top-level name. The browser
//   refuses a second `let`, `const` or `class`, and a second function silently
//   replaces the first.
// - Biome reports empty blocks and empty function bodies as info. Only empty
//   blocks are errors.
// - Comments cannot suppress Biome diagnostics.

const root = new URL("..", import.meta.url);
const read = (path) => readFileSync(new URL(path, root), "utf8");
const IDENTIFIER = /^[A-Za-z_$][\w$]*$/;
// Vendored bundles are minified UMD code, so their globals are named here.
const VENDOR_GLOBALS = {
  "static/vendor/xterm.js": "Terminal",
  "static/vendor/addon-fit.js": "FitAddon",
  "static/vendor/markdown-it.min.js": "markdownit",
};
const BIOME_RULES = [
  "correctness/noUndeclaredVariables",
  "correctness/noUnusedVariables",
  "style/useConst",
  "suspicious/noEmptyBlockStatements",
];

const globalNames = (context) =>
  vm.runInContext("Object.getOwnPropertyNames(globalThis)", context);
const builtins = new Set(globalNames(vm.createContext()));

// Instantiates the top-level declarations of a script in `context` and runs
// none of its code: the inserted `throw` stops it first.
function declare(source, context) {
  const prologue = /^\s*(["'])use strict\1;?/.exec(source)?.[0] ?? "";
  try {
    vm.runInContext(
      `${prologue}throw 0;${source.slice(prologue.length)}`,
      context,
    );
  } catch (error) {
    if (error !== 0) {
      throw error;
    }
  }
}

class Scope {
  constructor(source) {
    this.context = vm.createContext();
    declare(source, this.context);
    // Lexical names are not properties of the global object.
    this.functions = globalNames(this.context).filter(
      (name) => !builtins.has(name),
    );
  }

  declares(name) {
    if (this.functions.includes(name)) {
      return true;
    }
    // `typeof` throws only for an uninitialized `let`, `const` or `class`.
    try {
      vm.runInContext(`typeof ${name}`, this.context);
      return false;
    } catch {
      return true;
    }
  }
}

function offsetOf(source, { line, column }) {
  let offset = 0;
  for (let index = 1; index < line; index += 1) {
    offset = source.indexOf("\n", offset) + 1;
  }
  return offset + column - 1;
}

function isFunctionBody(source, offset) {
  const before = source.slice(0, offset).trimEnd();
  if (before.endsWith("=>")) {
    return true;
  }
  if (!before.endsWith(")")) {
    return false;
  }
  let depth = 0;
  for (let index = before.length - 1; index >= 0; index -= 1) {
    if (before[index] === ")") {
      depth += 1;
    } else if (before[index] === "(") {
      depth -= 1;
      if (depth === 0) {
        return !/\b(?:if|for|for\s+await|while|switch|catch|with)\s*$/.test(
          before.slice(0, index),
        );
      }
    }
  }
  return false;
}

// `sources` maps each linted path to its text. `loaded` lists the static
// scripts that index.html loads, in order, and `unloaded` the other static
// scripts. `diagnostics` are Biome's JSON diagnostics for BIOME_RULES.
export function checkScripts({
  sources,
  loaded,
  unloaded,
  vendorGlobals,
  testSource,
  diagnostics,
}) {
  const problems = [];
  const scopes = new Map(
    [...loaded, ...unloaded].map((path) => [
      path,
      new Scope(sources.get(path)),
    ]),
  );

  const shared = new Set(vendorGlobals);
  const owners = new Map();
  const context = vm.createContext();
  for (const path of loaded) {
    for (const [, name] of sources
      .get(path)
      .matchAll(/\bwindow\.([A-Za-z_$][\w$]*)\s*=(?!=)/g)) {
      shared.add(name);
    }
    try {
      declare(sources.get(path), context);
    } catch (error) {
      // The error comes from the context's realm, so `instanceof` fails.
      if (error?.name !== "SyntaxError") {
        throw error;
      }
      problems.push({ path, message: error.message });
    }
    for (const name of scopes.get(path).functions) {
      if (owners.has(name)) {
        problems.push({
          path,
          message: `${name} is already declared in ${owners.get(name)}`,
        });
      }
      owners.set(name, path);
    }
  }

  const found = diagnostics
    .filter(({ severity }) => severity === "info")
    .map(({ category, message, location }) => {
      const source = sources.get(location.path);
      const start = offsetOf(source, location.start);
      const end = offsetOf(source, location.end);
      return {
        rule: category.slice(category.lastIndexOf("/") + 1),
        message,
        path: location.path,
        line: location.start.line,
        source,
        start,
        end,
        span: source.slice(start, end),
      };
    });
  const mismatch = ({ path, line }) =>
    new Error(
      `Biome reported a location that check-js misreads: ${path}:${line}`,
    );

  // The names that each script uses without declaring them.
  const uses = found
    .filter(({ rule }) => rule === "noUndeclaredVariables")
    .map(({ path, line, source, start, end, span }) => {
      if (!IDENTIFIER.test(span)) {
        throw mismatch({ path, line });
      }
      return {
        path,
        name: span,
        assigns:
          /^\s*(?:\*\*|<<|>>>?|&&|\|\||\?\?|[-+*/%&|^])?=(?!=)|^\s*(?:\+\+|--)/.test(
            source.slice(end),
          ) || /(?:\+\+|--)\s*$/.test(source.slice(0, start)),
      };
    });
  const testNames = new Set(testSource.match(/[A-Za-z_$][\w$]*/g));
  const usedElsewhere = (path, name, { assigned }) =>
    testNames.has(name) ||
    uses.some(
      (use) =>
        use.path !== path && use.name === name && (use.assigns || !assigned),
    );
  const declaredTopLevel = (path, name) => scopes.get(path)?.declares(name);

  for (const diagnostic of found) {
    const { rule, path, span } = diagnostic;
    let explained;
    switch (rule) {
      case "noUndeclaredVariables":
        explained =
          shared.has(span) ||
          loaded.some((other) => scopes.get(other).declares(span));
        break;
      case "noUnusedVariables":
        explained =
          declaredTopLevel(path, span) &&
          usedElsewhere(path, span, { assigned: false });
        break;
      case "useConst": {
        const name = /^let\s+([A-Za-z_$][\w$]*)/.exec(
          diagnostic.source.slice(diagnostic.start),
        )?.[1];
        if (name === undefined) {
          throw mismatch(diagnostic);
        }
        explained =
          declaredTopLevel(path, name) &&
          usedElsewhere(path, name, { assigned: true });
        break;
      }
      case "noEmptyBlockStatements":
        if (span !== "{}") {
          throw mismatch(diagnostic);
        }
        explained = isFunctionBody(diagnostic.source, diagnostic.start);
        break;
      default:
        throw new Error(`check-js does not handle Biome rule ${rule}`);
    }
    if (!explained) {
      problems.push({
        path,
        line: diagnostic.line,
        message: diagnostic.message,
      });
    }
  }
  return problems;
}

export function findSuppressions(path, source) {
  return source
    .split("\n")
    .flatMap((text, index) =>
      /\/[/*]\s*biome-ignore/.test(text)
        ? [{ path, line: index + 1, message: "Fix the code; do not suppress" }]
        : [],
    );
}

function biomeDiagnostics() {
  const result = spawnSync(
    fileURLToPath(new URL("node_modules/.bin/biome", root)),
    [
      "lint",
      "--reporter=json",
      "--max-diagnostics=none",
      ...BIOME_RULES.map((rule) => `--only=${rule}`),
    ],
    { cwd: fileURLToPath(root), encoding: "utf8", maxBuffer: 1 << 28 },
  );
  if (result.status !== 0) {
    throw new Error(`biome lint failed:\n${result.stderr}${result.stdout}`);
  }
  // The JSON report is the first line. The rest is a summary for people.
  return JSON.parse(result.stdout.slice(0, result.stdout.indexOf("\n")))
    .diagnostics;
}

const list = (directory, extension) =>
  readdirSync(new URL(directory, root))
    .filter((name) => name.endsWith(extension))
    .map((name) => `${directory}/${name}`);

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const scripts = [
    ...read("static/index.html").matchAll(/<script src="\/([^"]+)"/g),
  ].map((match) => `static/${match[1]}`);
  const vendor = scripts.filter((path) => path.startsWith("static/vendor/"));
  const loaded = scripts.filter((path) => !path.startsWith("static/vendor/"));
  const unloaded = list("static", ".js").filter(
    (path) => !loaded.includes(path),
  );
  for (const path of vendor) {
    const name = VENDOR_GLOBALS[path];
    if (name === undefined) {
      throw new Error(
        `index.html loads ${path}; add its global to desktop/check-js.mjs`,
      );
    }
    if (!read(path).includes(name)) {
      throw new Error(`${path} no longer mentions its global ${name}`);
    }
  }
  const tests = [...list("tests", ".cjs"), ...list("tests/helpers", ".cjs")];
  const paths = [
    ...loaded,
    ...unloaded,
    ...tests,
    ...list("static", ".css"),
    ...list("desktop/frontend", ".js"),
    ...list("desktop", ".mjs"),
    ...list("desktop/diffs-build", ".mjs"),
  ];
  const sources = new Map(paths.map((path) => [path, read(path)]));
  const problems = [
    ...checkScripts({
      sources,
      loaded,
      unloaded,
      vendorGlobals: vendor.map((path) => VENDOR_GLOBALS[path]),
      testSource: tests.map((path) => sources.get(path)).join("\n"),
      diagnostics: biomeDiagnostics(),
    }),
    ...paths.flatMap((path) => findSuppressions(path, sources.get(path))),
  ];
  for (const { path, line, message } of problems) {
    console.error(
      `${line === undefined ? path : `${path}:${line}`}: ${message}`,
    );
  }
  if (problems.length > 0) {
    process.exit(1);
  }
}
