import {build} from 'esbuild';
import {fileURLToPath} from 'node:url';
import {rm,mkdir,copyFile} from 'node:fs/promises';
const root=fileURLToPath(new URL('.',import.meta.url));
await rm(root+'../static/vendor/diffs',{recursive:true,force:true});
await mkdir(root+'../static/vendor/diffs',{recursive:true});
await copyFile(root+'node_modules/@pierre/diffs/LICENSE.md',root+'../static/vendor/diffs/LICENSE.pierre-diffs.md');
await build({entryPoints:[root+'frontend/diff-viewer.js'],outdir:root+'../static/vendor/diffs',entryNames:'diff-viewer',chunkNames:'chunk-[hash]',bundle:true,splitting:true,format:'esm',target:'safari16',minify:true,legalComments:'eof'});
