import { build } from "esbuild";
import { fileURLToPath } from "node:url";
import { rm, mkdir, copyFile } from "node:fs/promises";
import { bundledLanguagesInfo } from "shiki/langs";
const root = fileURLToPath(new URL(".", import.meta.url));
// Shiki grammars that the diff viewer can load. A file in another language shows as plain text.
const languages = new Set([
  "c",
  "cpp",
  "css",
  "csv",
  "diff",
  "docker",
  "dotenv",
  "go",
  "graphql",
  "html",
  "ini",
  "java",
  "javascript",
  "json",
  "jsonc",
  "jsonl",
  "jsx",
  "kotlin",
  "make",
  "markdown",
  "objective-cpp",
  "python",
  "ruby",
  "rust",
  "scss",
  "shellscript",
  "sql",
  "swift",
  "toml",
  "tsx",
  "typescript",
  "xml",
  "yaml",
]);
const included = bundledLanguagesInfo.filter(({ id }) => languages.has(id));
if (included.length !== languages.size) {
  throw new Error("A diff viewer language is not a Shiki language id");
}
const languageModule = `export const bundledLanguages = {${included
  .flatMap(({ id, aliases = [] }) =>
    [id, ...aliases].map(
      (name) => `${JSON.stringify(name)}: () => import("@shikijs/langs/${id}")`,
    ),
  )
  .join(",")}};`;
// The viewer uses the JavaScript regex engine and registers each theme it
// shows, so it never loads the WebAssembly engine, the Highlights backend or
// the theme catalogs.
const unused = new Set([
  "./backends/highlights.js",
  "@pierre/highlights/themes/loader",
  "@pierre/theming/themes",
  "shiki/engine/oniguruma",
  "shiki/wasm",
]);
const trimHighlighter = {
  name: "trim-highlighter",
  setup(context) {
    context.onResolve({ filter: /^shiki\/langs$/ }, () => ({
      path: "languages",
      namespace: "trim-highlighter",
    }));
    context.onResolve({ filter: /./ }, ({ path, kind }) =>
      kind === "dynamic-import" && unused.has(path)
        ? { path, namespace: "trim-highlighter" }
        : undefined,
    );
    context.onLoad(
      { filter: /./, namespace: "trim-highlighter" },
      ({ path }) =>
        path === "languages"
          ? { contents: languageModule, resolveDir: root }
          : {
              contents: `throw new Error(${JSON.stringify(`${path} is not bundled`)});`,
            },
    );
  },
};
await rm(`${root}../static/vendor/diffs`, { recursive: true, force: true });
await mkdir(`${root}../static/vendor/diffs`, { recursive: true });
await copyFile(
  `${root}node_modules/@pierre/diffs/LICENSE.md`,
  `${root}../static/vendor/diffs/LICENSE.pierre-diffs.md`,
);
await build({
  entryPoints: [`${root}frontend/diff-viewer.js`],
  absWorkingDir: root,
  outdir: `${root}../static/vendor/diffs`,
  entryNames: "diff-viewer",
  chunkNames: "chunk-[hash]",
  bundle: true,
  splitting: true,
  format: "esm",
  target: "safari16",
  minify: true,
  legalComments: "eof",
  plugins: [trimHighlighter],
});
