import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import { fileURLToPath } from "node:url";

// Rebuilds the committed diff viewer bundle and fails when the result differs
// from the committed files.
const desktop = fileURLToPath(new URL(".", import.meta.url));
if (!existsSync(`${desktop}node_modules`)) {
  console.error(
    "desktop/node_modules is missing. Run `npm ci` in desktop/ to check the diff viewer bundle.",
  );
  process.exit(1);
}
execFileSync(process.execPath, [`${desktop}build-diffs.mjs`], {
  stdio: "inherit",
});
const changes = execFileSync(
  "git",
  [
    "status",
    "--porcelain",
    "--untracked-files=all",
    "--",
    "static/vendor/diffs",
  ],
  { cwd: `${desktop}..`, encoding: "utf8" },
);
if (changes !== "") {
  console.error(
    `static/vendor/diffs does not match desktop/frontend/diff-viewer.js. Run \`node desktop/build-diffs.mjs\` and commit the result:\n${changes}`,
  );
  process.exit(1);
}
