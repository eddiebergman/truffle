# mydash: Rust service and desktop host

The active backend is Rust. It implements the existing loopback API and version-1 JSON storage; the old Python server remains in the source tree as migration reference/fallback, not the app's runtime. The small `register.py` helper remains a client for existing Claude/Codex skills. It does not own or write work state.

## Boundaries

- `src/backend/domain.rs`: work records, local edits, snoozes, related-work graph, source changes, and read projections. No HTTP, window, DOM, CSS, or terminal implementation.
- `src/backend/repository.rs`: the sole canonical JSON writer, atomic saves, file lock, sync scheduling/backoff, transient revision/cache, and diagnostic log. No layout or UI preference state is stored in domain records.
- `src/backend/providers.rs`: provider reads, pagination, CI status, source URL identities, targeted refreshes, and explicit fixed GitHub PR mutations. `src/backend/github.rs` validates cached PR identities, gates writes, updates the cache, and tracks versioned review-request attention cycles.
- `src/backend/workflows.rs`: agent/session associations, work folders, review/newsletter tasks, context packets, recovery, and local registration. Conversation UUIDs remain independent from terminal surfaces.
- `src/backend/terminal.rs`: fixed local command utilities and optional desktop launch. `terminal_daemon::Host` is the PTY presentation port. `terminal_daemon.rs` owns an independent private-socket process; `embedded.rs` manages its PTYs, independent of the app and HTTP service; local xterm.js renders their byte streams. A future Ghostty renderer can replace xterm without changing source records or agent identity. Ghostty is not included.
- `src/backend/scheduler.rs`: persisted skill schedules, bounded run history, missed-run policy and overlap protection. Schedules are opt-in and only run while the service is awake.
- `src/backend/artifacts.rs`: skill presets and discovered/registered display artifacts, including session associations and safe local file access.
- `src/backend/http.rs`: loopback HTTP, Host/Origin/CSRF checks, narrow signed registration inbox, and existing API compatibility.
- `src/service.rs`: UI-independent `DashboardService` client interface and its loopback adapter. No dependency on Tauri, component types, or board columns.
- `src/main.rs`: desktop IPC, isolated artifact protocol, native window, menus, browser links, single-instance behavior, and `--serve` / `--terminal-daemon` entry points.
- `static/client.js`: shared browser/desktop client adapter. Browser uses same-origin HTTP; the desktop uses narrow native IPC. Frontend presentation remains plain HTML/CSS/JS and can move to a TypeScript framework without rewriting persistence.

Canonical state is stored in `~/code/mydash/data/state.json`. Only one backend process can acquire its `.lock` file; Rust and the old Python implementation use the same OS file lock. Switching implementations does not copy or migrate records. Unknown version-1 fields are retained. An incompatible storage version is rejected and requires an explicit migration with a backup. The API has an independent version handshake.

Native window geometry and browser-style view preferences are presentation state. They are separate from work records. API-derived fields such as action reasons or grouped work are read projections, not a required persisted UI schema.

## Runtime and performance

Desktop IPC commands use Tauri's Tokio runtime and move blocking service calls off the UI event loop. The current backend uses bounded HTTP workers (32 maximum) and three independent provider-sync threads with blocking reqwest I/O. It is not async throughout. No provider fetch occurs in the cached-state read path. Provider async I/O can be introduced behind the same service interfaces later.

Read projections are cached by a transient per-process revision. Commits and sync-status changes advance it; revision metadata is not persisted into work records. The frontend uses revisions to skip unchanged redraws and bounds initial DOM rendering while keeping search across the full cached dataset.

## Security

The backend binds only `127.0.0.1`. It validates Host, requires same-origin CSRF for UI mutations, and provides no CORS wildcard. The native service adapter rejects remote addresses, proxies, and redirects, keeps CSRF in Rust, and refreshes authorization once after a service restart rejects a mutation. Native IPC accepts only fixed API routes. The agent bearer credential and signed inbox are not exposed through native IPC.

Artifacts use opaque IDs. The backend validates display-file formats, size, roots, and resolved symlinks. HTML previews are sandboxed without same-origin privileges, network access, forms, or dashboard IPC. External links open through macOS in the default browser. The main app window never navigates to a remote origin.

## Current deployment

Run `~/code/mydash/start.command` from a regular Terminal window outside coding-agent tool sandboxes to start the Rust service. The native app connects to it on port 8765 and does not stop it when its window closes. The service does not connect to cmux.

`start.command` runs the installed app's Rust binary in `--serve` mode. No Python server or uv process is needed for sync, storage, or desktop presentation. Existing agent registration commands still use the lightweight Python client through uv for compatibility.

## Build and run

Requires Rust stable, Apple's Command Line Tools, and Node/npm for the Tauri build CLI. Node is a build dependency, not a runtime.

```sh
cd ~/code/mydash/desktop
npm ci
npm run build
```

The bundle is `desktop/target/release/bundle/macos/mydash.app`. Install it under `~/Applications/`. Build dependencies are locked by `Cargo.lock` and `package-lock.json`.

```sh
# In any terminal:
~/code/mydash/start.command
# Launch the installed app from Finder, Spotlight, or:
open ~/Applications/mydash.app
```

For an isolated service test, run the binary with `--serve --demo --home ~/code/mydash --data /tmp/mydash-preview/data/state.json --port 8766`. Demo mode uses the given cache and suppresses all provider sync and agent launches. `MYDASH_SERVICE_URL` selects another loopback origin for development; it is read at runtime or at build time. Production builds leave it unset.

Close hides the native window. Clicking the Dock icon restores it. Cmd-Q quits the app. The independently running service continues. The app's reconnect screen explains how to start it if unavailable.

## Embedded agents and results

A separate terminal daemon owns up to 12 live PTYs. Closing the app or restarting the HTTP service does not stop them. The daemon keeps process generations and replay buffers so a replacement service can reconnect. The daemon itself and laptop shutdown still end its PTYs. Existing terminals owned by the older service cannot be migrated; this applies to launches after activation. Each PTY retains at most 2 MiB of output. Sequential 32 KiB polling and xterm write callbacks provide bounded replay/backpressure. Input and resize use authenticated fixed routes, never an arbitrary command execution API. History stores conversation identity and work links, not terminal transcripts. Existing external processes cannot be transplanted; after closing them, Resume in mydash starts their saved conversation in an embedded terminal.

Launch-local Claude/Codex hooks report waiting, working, permission requests and subagent counts through runtime_hook.py. Global instructions and skill directories remain unchanged. Unknown state/counts are shown explicitly. Lifecycle metadata is advisory; live PTY/surface checks determine the active strip.

Registered local results become private immutable snapshots under data/artifacts. Supported results include Markdown, sandboxed HTML without scripts, PNG/WebP/JPEG/SVG, CSV and mydash.table/v1 JSON tables. The skill documents registration. Schedules support daily local time or intervals, with one catch-up run or skipping missed intervals; they never replay a backlog. A completed turn means the agent stopped producing output for that turn, not proof that its task succeeded.

## cmux retirement

cmux inventory, focus and launch integration has been removed. Only backend-owned embedded PTYs appear as live agents. Legacy cmux destination records remain intact; opening one resumes its native conversation UUID in an embedded PTY and updates its destination. Close an external copy yourself before resuming to avoid concurrent agent work. No cmux socket is used and no cmux ancestry policy is required.

Embedded agents inherit the service’s operating-system restrictions. Deploy the service from a regular user Terminal, never a sandboxed coding-tool process. Before launch, mydash checks that agent storage is writable; it does not alter sandbox or agent permissions. Interactive PTYs set true color and remove inherited NO_COLOR/CI hints.


## Persistent terminal host

The production repository delegates PTY operations over a private Unix socket in `/tmp/mydash-pty-<uid>-<state-path-hash>/terminal.sock`. The directory is user-owned mode 0700, with exclusive daemon and startup locks. Framed JSON has protocol version 1, a 512 KiB limit and bounded handlers/timeouts. The daemon starts lazily through `--terminal-daemon --home ROOT --data STATE`, detaches from the service process group, and retains at most 12 live terminals. The app/service must not kill it during routine restart or bundle replacement. Upgrade it only after its agents finish or after explicit permission to end them. Do not use broad `pkill mydash` commands. An unreachable locked daemon is an error, not permission to launch duplicate agents. Demo mode retains an in-process fixture host; the process regression uses an explicitly allowed shell fixture.

`restart.rs` supplies native preview/start/status commands and a detached `--restart-dashboard` worker. It checks the reviewed live-agent snapshot again, verifies process ownership/executable/profile, and uses SIGTERM without force-kill. A service restart preserves daemon PTYs; a terminal restart or legacy migration explicitly closes them and resumes the original saved UUIDs/cwds. Missing resume metadata blocks disruptive operations. Exclusive worker locks and private atomic `data/restarts/` jobs prevent concurrent restarts and retain diagnostics across UI closure. New service output is bounded through `data/logs/service.log` rotation. The desktop can invoke the helper even if the running service predates these controls. `--restart-control` exposes the same fixed operations for isolated process regression tests; it is not an arbitrary shell interface.

## Agent resource metadata and work-view tasks

`resources.rs` stores bounded open resource descriptors on the associated session; `register.py resource --record ID --file REPORT` accepts a resources array with provider/kind/id identity and optional status, timestamps, grouped URIs and metadata. Receipt time is separate from observed time. Metadata does not provision or clean up cloud resources, and closing a terminal never implies cleanup. Agent launch packets and the mydash skill describe reporting at lifecycle changes.

`view_context.rs` prepares an immutable bounded JSON snapshot from the exact filtered work IDs sent by the frontend, including matches beyond pagination. It records source sync times, local status overrides and reference links. Request IDs make retries reuse the saved task. Snapshot items are context; they are not all attached as assigned work.
