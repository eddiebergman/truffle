import { readFileSync } from "node:fs";
import { diffsManifest, manifestPath } from "./diffs-manifest.mjs";

// Fails when the diff viewer bundle or its sources differ from the hashes that
// the last bundle build recorded. `npm run check:diffs` rebuilds the bundle.
if (diffsManifest() !== readFileSync(manifestPath, "utf8")) {
  console.error(
    "static/vendor/diffs or its sources changed after the last bundle build. Run `node desktop/diffs-build/build.mjs` after `npm ci` in desktop/diffs-build/, then commit the result.",
  );
  process.exit(1);
}
