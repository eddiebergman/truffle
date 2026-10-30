import { readdirSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

// Stylesheet checks that Biome does not have:
// - A selector must not follow a selector of higher specificity that ends in
//   the same compound, with pseudo-classes removed. Otherwise `.row:hover`
//   loses to a later `.row`. Biome compares only identical last compounds.
// - Media queries use min-/max- features. Range syntax needs WebKit 16.4, and
//   the desktop app still supports macOS 12.
// - Colors come from custom properties, so named colors are errors. Biome
//   reports hex colors.
// The input must be Prettier-formatted plain CSS without nesting.

const NAMED_COLORS = new Set(
  `aliceblue antiquewhite aqua aquamarine azure beige bisque black
  blanchedalmond blue blueviolet brown burlywood cadetblue chartreuse chocolate
  coral cornflowerblue cornsilk crimson cyan darkblue darkcyan darkgoldenrod
  darkgray darkgreen darkgrey darkkhaki darkmagenta darkolivegreen darkorange
  darkorchid darkred darksalmon darkseagreen darkslateblue darkslategray
  darkslategrey darkturquoise darkviolet deeppink deepskyblue dimgray dimgrey
  dodgerblue firebrick floralwhite forestgreen fuchsia gainsboro ghostwhite gold
  goldenrod gray green greenyellow grey honeydew hotpink indianred indigo ivory
  khaki lavender lavenderblush lawngreen lemonchiffon lightblue lightcoral
  lightcyan lightgoldenrodyellow lightgray lightgreen lightgrey lightpink
  lightsalmon lightseagreen lightskyblue lightslategray lightslategrey
  lightsteelblue lightyellow lime limegreen linen magenta maroon
  mediumaquamarine mediumblue mediumorchid mediumpurple mediumseagreen
  mediumslateblue mediumspringgreen mediumturquoise mediumvioletred
  midnightblue mintcream mistyrose moccasin navajowhite navy oldlace olive
  olivedrab orange orangered orchid palegoldenrod palegreen paleturquoise
  palevioletred papayawhip peachpuff peru pink plum powderblue purple
  rebeccapurple red rosybrown royalblue saddlebrown salmon sandybrown seagreen
  seashell sienna silver skyblue slateblue slategray slategrey snow springgreen
  steelblue tan teal thistle tomato turquoise violet wheat white whitesmoke
  yellow yellowgreen`.split(/\s+/),
);
const LEGACY_PSEUDO_ELEMENTS = new Set([
  "before",
  "after",
  "first-line",
  "first-letter",
]);

// Replaces comments and string contents with spaces, so offsets and line
// numbers stay the same and no brace, quote or comma inside them is seen.
function blank(text) {
  return text.replace(
    /\/\*[\s\S]*?\*\/|"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'/g,
    (match) =>
      match.startsWith("/*")
        ? match.replace(/[^\n]/g, " ")
        : match[0] + match.slice(1, -1).replace(/[^\n]/g, " ") + match[0],
  );
}

// Splits at `separator` outside parentheses and brackets.
function splitTopLevel(text, separator) {
  const parts = [];
  let depth = 0;
  let start = 0;
  for (let index = 0; index < text.length; index += 1) {
    const char = text[index];
    if (char === "(" || char === "[") {
      depth += 1;
    } else if (char === ")" || char === "]") {
      depth -= 1;
    } else if (char === separator && depth === 0) {
      parts.push(text.slice(start, index));
      start = index + 1;
    }
  }
  parts.push(text.slice(start));
  return parts.map((part) => part.trim());
}

// The compounds of one complex selector, without the combinators.
function compounds(selector) {
  const result = [];
  let depth = 0;
  let current = "";
  for (const char of selector) {
    if (char === "(" || char === "[") {
      depth += 1;
    } else if (char === ")" || char === "]") {
      depth -= 1;
    }
    if (depth === 0 && /[\s>+~]/.test(char)) {
      if (current !== "") {
        result.push(current);
      }
      current = "";
    } else {
      current += char;
    }
  }
  if (current !== "") {
    result.push(current);
  }
  return result;
}

// The simple selectors of one compound, such as `a`, `.b`, `[c]` or `:is(d)`.
function simpleSelectors(compound) {
  return (
    compound.match(
      /::?[\w-]+(?:\((?:[^()]|\((?:[^()]|\([^()]*\))*\))*\))?|[#.][\w-]+|\[[^\]]*\]|[\w-]+|\*|&/g,
    ) ?? []
  );
}

function isPseudoElement(simple) {
  return (
    simple.startsWith("::") ||
    (simple.startsWith(":") && LEGACY_PSEUDO_ELEMENTS.has(simple.slice(1)))
  );
}

function compare(left, right) {
  return left[0] - right[0] || left[1] - right[1] || left[2] - right[2];
}

function maxSpecificity(selectorList) {
  return splitTopLevel(selectorList, ",")
    .map(specificity)
    .reduce((max, next) => (compare(next, max) > 0 ? next : max), [0, 0, 0]);
}

function specificity(selector) {
  const total = [0, 0, 0];
  const add = ([a, b, c]) => {
    total[0] += a;
    total[1] += b;
    total[2] += c;
  };
  for (const simple of compounds(selector).flatMap(simpleSelectors)) {
    const pseudo = /^:([\w-]+)(?:\((.*)\))?$/s.exec(simple);
    if (simple.startsWith("#")) {
      add([1, 0, 0]);
    } else if (simple.startsWith(".") || simple.startsWith("[")) {
      add([0, 1, 0]);
    } else if (isPseudoElement(simple)) {
      add([0, 0, 1]);
    } else if (pseudo !== null) {
      const [, name, argument] = pseudo;
      switch (name) {
        case "where":
          break;
        case "is":
        case "not":
        case "has":
          add(maxSpecificity(argument));
          break;
        case "nth-child":
        case "nth-last-child": {
          add([0, 1, 0]);
          const of = /\sof\s(.*)$/s.exec(argument);
          if (of !== null) {
            add(maxSpecificity(of[1]));
          }
          break;
        }
        default:
          add([0, 1, 0]);
      }
    } else if (simple !== "*" && simple !== "&") {
      add([0, 0, 1]);
    }
  }
  return total;
}

// The last compound without its pseudo-classes. Selectors with the same key
// can match the same element in the same state.
function comparisonKey(selector) {
  const last = compounds(selector).at(-1);
  return simpleSelectors(last)
    .filter((simple) => !simple.startsWith(":") || isPseudoElement(simple))
    .join("");
}

const lineAt = (text, offset) => text.slice(0, offset).split("\n").length;

// Checks the selectors of one rule against the earlier rules in `seen`.
function checkSelectors({ prelude, line, seen, problems }) {
  for (const selector of splitTopLevel(prelude, ",")) {
    const key = comparisonKey(selector);
    if (key === "") {
      continue;
    }
    const value = specificity(selector);
    const prior = seen.get(key) ?? [];
    const higher = prior.find((entry) => compare(entry.specificity, value) > 0);
    if (higher !== undefined) {
      problems.push({
        line: line(selector),
        message: `Move "${selector}" before "${higher.selector}" (line ${higher.line}), which has higher specificity`,
      });
    }
    prior.push({ selector, specificity: value, line: line(selector) });
    seen.set(key, prior);
  }
}

function checkDeclaration({ declaration, line, problems }) {
  const match = /^[\w-]+\s*:([\s\S]*)$/.exec(declaration);
  if (match === null) {
    return;
  }
  const value = match[1].replace(/url\([^)]*\)/g, "");
  for (const [word] of value.matchAll(/(?<![\w-])[a-z]+(?![\w(-])/gi)) {
    if (NAMED_COLORS.has(word.toLowerCase())) {
      problems.push({
        line,
        message: `Use a color custom property, not the named color ${word}`,
      });
    }
  }
}

export function checkCss(source) {
  const text = blank(source);
  const problems = [];
  // One frame per open block. A rule is compared with the earlier rules of
  // its own at-rule block only.
  const stack = [{ kind: "root", seen: new Map() }];
  let start = 0;
  for (let index = 0; index < text.length; index += 1) {
    const char = text[index];
    if (char !== "{" && char !== "}" && char !== ";") {
      continue;
    }
    const chunk = text.slice(start, index);
    const chunkStart = start + Math.max(chunk.search(/\S/), 0);
    const statement = chunk.trim();
    const frame = stack.at(-1);
    start = index + 1;
    if (char === "{") {
      const atRule = /^@([\w-]+)/.exec(statement)?.[1];
      if (atRule === "media" && /[<>=]/.test(statement)) {
        problems.push({
          line: lineAt(text, chunkStart),
          message: `Use min-/max- media features, not range syntax: ${statement}`,
        });
      }
      if (atRule !== undefined) {
        stack.push({ kind: atRule, seen: new Map() });
        continue;
      }
      checkSelectors({
        prelude: statement,
        line: (selector) =>
          lineAt(text, chunkStart + statement.indexOf(selector)),
        seen: frame.seen,
        problems,
      });
      stack.push({ kind: "rule" });
      continue;
    }
    if (frame.kind === "rule") {
      checkDeclaration({
        declaration: statement,
        line: lineAt(text, chunkStart),
        problems,
      });
    }
    if (char === "}") {
      stack.pop();
    }
  }
  return problems;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const directory = new URL("../static/", import.meta.url);
  let failed = false;
  for (const name of readdirSync(directory).filter((file) =>
    file.endsWith(".css"),
  )) {
    for (const { line, message } of checkCss(
      readFileSync(new URL(name, directory), "utf8"),
    )) {
      console.error(`static/${name}:${line}: ${message}`);
      failed = true;
    }
  }
  if (failed) {
    process.exit(1);
  }
}
