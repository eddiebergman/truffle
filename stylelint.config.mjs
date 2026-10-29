/** @type {import("stylelint").Config} */
export default {
  extends: ["stylelint-config-standard"],
  rules: {
    "no-duplicate-selectors": true,
    "declaration-block-no-duplicate-properties": true,
    "no-descending-specificity": true,
    "color-no-hex": true,
    "color-named": "never",
    "selector-max-specificity": "2,2,0",
    // Range syntax needs WebKit 16.4; the desktop app still supports macOS 12.
    "media-feature-range-notation": "prefix",
    // Classes such as .needsInput come from runtime state values in JS.
    "selector-class-pattern": null,
  },
};
