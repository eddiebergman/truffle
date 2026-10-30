# Terminal renderer

Locally vendored @xterm/xterm 6.0.0 and @xterm/addon-fit 0.11.0, both MIT licensed (license files beside the assets). They are not npm dependencies. No CDN or runtime npm installation. The files are copied unchanged from the npm packages:

| File | Package file | SHA-256 |
| --- | --- | --- |
| xterm.js | @xterm/xterm 6.0.0 `lib/xterm.js` | `14903579ff54664cd72f8e8699e6961a6272c21863ec1c3b118cdc8af5d4a972` |
| xterm.css | @xterm/xterm 6.0.0 `css/xterm.css` | `854a7c0fb70e8b1a083c16797ab827299fb18744f5ad34f227b48337e33293c6` |
| addon-fit.js | @xterm/addon-fit 0.11.0 `lib/addon-fit.js` | `ba3ea256ce0620a0992a197d6c9baea64823fc93d8da07a9e366ca9943c18527` |

To update, download both new releases, copy these files and their licenses together, and update this table.

# Diff viewer

`diffs/` is generated from @pierre/diffs by `desktop/diffs-build/`. Do not edit it by hand. The root README explains how to regenerate it.

# Markdown renderer

markdown-it.min.js is `dist/browser/markdown-it.umd.min.js` from markdown-it 15.0.3 (MIT, license file beside it). It is not an npm dependency; to update it, download the new release and copy that file.
