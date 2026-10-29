---
name: mydash-performance
description: Diagnose and fix slow startup, navigation, search, scrolling, refresh, or agent actions in the mydash browser and Rust/Tauri desktop app. Use measured service, JavaScript, and native rendering costs to isolate regressions.
---

# Profile mydash

Establish which installed build and service the user is actually running before changing code. The frontend is bundled into `mydash.app`: editing `static/` or restarting the backend does not update a running desktop window. Release builds leave `MYDASH_SERVICE_URL` unset; preview builds can point at a copied cache. Inspect the exact executable path when several bundles share `local.mydash.desktop`.

## Measure the reported interaction

- Open **Logs → UI performance → Start recording**. Reproduce the slow action several times, then return to Logs and refresh the report. It identifies the native build profile, item count, CPU render times, service round trips, and approximate input-to-next-frame times. Stop recording afterwards. It stores only bounded numeric samples in memory. `mydash:render` also appears in the engine's User Timing timeline.
- Use `uv run --no-project .agents/skills/mydash-performance/scripts/benchmark_service.py` from the repo root for a read-only cache baseline. It reads state without syncing or launching an agent. It reports bytes, counts, JSON parsing, and warm/cold latency. It does not print work titles or credentials.
- A tool's click/screenshot/accessibility call duration includes automation and accessibility traversal; it is not an app latency measurement. Use runtime timings and visible results. Avoid dumping thousands of accessibility rows while profiling.

Separate CPU work from waiting. A fast HTTP read with a slow native IPC request points to IPC/serialization/queueing; a fast JavaScript render with slow presentation points to layout, paint, or native frame scheduling. Agent actions have separate cmux inventory or embedded PTY launch costs and should not block hover or navigation. Do not launch real agents merely to benchmark the dashboard.

For a stall that the built-in timings do not explain, read [references/engine-profiling.md](references/engine-profiling.md). Use the actual engine: macOS Tauri uses WKWebView. Browser results are useful comparisons, not proof of native performance.

## Fix the measured cause

Current code is plain DOM JavaScript. Inspect `static/app.js`, `static/features.js`, and `static/sessions.js` for projection rebuilds, nested whole-dataset scans, closed-section DOM construction, and full redraws. Initial work rendering is paged to 100 rows; search still covers all records. Item projections and session associations are cached per immutable snapshot. Unchanged revision polling should perform no board redraw. Preserve Linear hierarchy, grouping counts, hidden/snoozed restoration, keyboard focus, and scroll position.

For embedded terminal stalls, inspect `static/workspace.js` and `desktop/src/backend/embedded.rs`. Verify sequential polling and xterm write completion, bounded output (2 MiB/PTY), stable terminal DOM across navigation, and no full-state payload for keystrokes. Use the demo-only fixed shell (`--demo --terminal-demo <existing-record-UUID>`) for input, ANSI, resize, reconnect, exit and output-pressure checks. Never benchmark by launching real review/newsletter work.

Service projections live in `desktop/src/backend/repository.rs`; providers and terminal adapters run outside the UI event loop. Cached reads must not fetch a provider or scan terminal history. Revision caches must expire timed snoozes and change on process restart. Do not replace the canonical state with a UI-specific schema to reduce payload size.

Use a copied `data/state.json` and `--serve --demo --data <copy> --port <free-port>` for mutations or stress tests. The single-writer lock protects live data. Start the production service from a regular Terminal, outside coding-agent tool sandboxes. Embedded children inherit the service’s OS restrictions. A restricted tool launch can break ~/.claude transcript/session-env writes and inherit NO_COLOR=1. Use --check-agent-storage claude (or codex) to verify actual access; do not change agent permissions to repair a service deployment. No credentials or raw provider payloads belong in profiling reports.

## Verify and deploy

Compare the same dataset, filters, build profile, foreground state, and interaction before/after. Report cold/warm values and median/p95 where sample count supports them; distinguish CPU timing from approximate presentation. Keep hover feedback immediate and free of layout-changing effects.

Run relevant Rust tests and `node --test tests/test_client.cjs tests/test_markdown.cjs tests/test_render_performance.cjs tests/test_performance.cjs`. Rebuild the release bundle with `npm --prefix desktop run build`, install it, quit the previous window, and launch the exact installed path through the available native UI tool. Verify the build and behavior visibly; a successful compile is not installation. Save a screenshot of the verified UI and include remaining limitations in the handoff.
