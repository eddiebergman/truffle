# Terminal renderer

Locally vendored @xterm/xterm 6.0.0 and @xterm/addon-fit 0.11.0, both MIT licensed (license files beside the assets). Exact build dependencies are pinned in desktop/package-lock.json. No CDN or runtime npm installation. Update the packages and copy their distributed JavaScript/CSS together.

# Markdown renderer

markdown-it.min.js is `dist/browser/markdown-it.umd.min.js` from markdown-it 15.0.3 (MIT, license file beside it). It is not an npm dependency; to update it, download the new release and copy that file.
