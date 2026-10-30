import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

// Rebuilds the committed diff viewer bundle and fails when the result differs
// from the committed files.
const root = fileURLToPath(new URL(".", import.meta.url));
execFileSync(process.execPath, [`${root}build.mjs`], { stdio: "inherit" });
const changes = execFileSync(
  "git",
  [
    "status",
    "--porcelain",
    "--untracked-files=all",
    "--",
    "static/vendor/diffs",
    "desktop/diffs-build/bundle.sha256",
  ],
  { cwd: `${root}../..`, encoding: "utf8" },
);
if (changes !== "") {
  console.error(
    `The rebuilt diff viewer bundle differs from the committed files. Commit the result if the change is intended:\n${changes}`,
  );
  process.exit(1);
}
