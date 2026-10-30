import { createHash } from "node:crypto";
import { readdirSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const repo = fileURLToPath(new URL("..", import.meta.url));
const inputs = [
  "desktop/frontend/diff-viewer.js",
  "desktop/diffs-build/build.mjs",
  "desktop/diffs-build/package-lock.json",
];
export const manifestPath = `${repo}desktop/diffs-build/bundle.sha256`;

// SHA-256 sums of the diff viewer bundle and the files it is built from, in
// `shasum -a 256 --check` format with paths relative to the repository.
export function diffsManifest() {
  const outputs = readdirSync(`${repo}static/vendor/diffs`)
    .sort()
    .map((name) => `static/vendor/diffs/${name}`);
  return [...inputs, ...outputs]
    .map(
      (path) =>
        `${createHash("sha256")
          .update(readFileSync(`${repo}${path}`))
          .digest("hex")}  ${path}\n`,
    )
    .join("");
}
