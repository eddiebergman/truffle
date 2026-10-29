# Engine profiling

Use a narrow recording of the reported interaction. Profilers add overhead, so keep a separate uninstrumented baseline.

## Native macOS app

WKWebView's Web Inspector **Timelines** separates JavaScript, network, layout, rendering, and CPU activity. Use an optimized local profiling build with Tauri's `devtools` Cargo feature if the installed release does not expose inspection. Enable it only in that build; do not silently change the shipped app's inspection policy. Use the actual app and copied data for repeatability.

- [WebKit Timelines reference](https://webkit.org/web-inspector/timelines-tab/)
- [WebKit CPU timeline](https://webkit.org/blog/8993/cpu-timeline-in-web-inspector/)
- [Tauri inspector availability](https://v2.tauri.app/reference/javascript/api/namespacewebview/): release inspection requires the `devtools` feature.

Find `truffle:render` and inspect adjacent layout/paint work. Look for long main-thread tasks, repeated style/layout cycles, large paint regions, and CPU activity while idle. A `sample <verified-pid> 3 -file <local-output>` capture can distinguish a blocked native host from an idle host waiting on its WebContent process. Verify PID and executable first; WebContent has its own process. Sampling only the Rust host cannot attribute JavaScript cost.

## Browser comparison

Open the same service in a browser and reproduce the same view and search. [Chrome's Performance panel](https://developer.chrome.com/docs/devtools/performance/reference) provides interaction, JavaScript, rendering, and paint tracks. This helps separate shared frontend problems from WKWebView-specific behavior. Do not equate a Node function microbenchmark with end-to-end interaction latency.

Use available supported UI automation for native and browser actions. If it cannot control the inspector, the built-in Logs timing report remains useful and directly testable; describe the missing engine trace rather than guessing the bottleneck.
