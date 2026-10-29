const fs = require("node:fs");
// Read a stylesheet with comments and formatting whitespace removed, so tests can match rules regardless of formatter output.
module.exports = function readCompactCss(path) {
  return fs
    .readFileSync(path, "utf8")
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .replace(/\s+/g, " ")
    .replace(/\s*([{};:,>])\s*/g, "$1")
    .replace(/\(\s+/g, "(")
    .replace(/\s+\)/g, ")")
    .replace(/;}/g, "}")
    .trim();
};
