# mydash

A private desktop workspace for agents and their GitHub, Linear, Sentry and local project work. Rust owns sync, storage and schedules; a separate persistent Rust daemon owns embedded PTYs. The replaceable frontend uses local HTML/CSS/JavaScript and xterm.js. No CDN or telemetry.

The share/inbox icon beside the title-bar `+` prepares a handover from the selected
chat, including selected prior conversation and attached results. Review it, enter
a recipient name such as `/eddie` (autocomplete discovers existing inbox paths and optional directory names), and Send. Inbox offers
a preview and Import into new chat with an agent/folder choice. A matching local
repo or branch worktree is suggested; otherwise choose a folder inline. Dismissed
shares are available under Archived, and importing a share again opens its existing
chat instead of creating duplicates.

Set your name and GCS root once in the popover settings. For example:
`gs://your-bucket/context-bundles/v1`. Existing `gcloud auth login` credentials supply
GCS access; no Slack access, public ACL, or new token file is required. Cloud checks
run every five minutes with backoff. Large existing cloud/public objects remain URI
references, rather than being copied into the sharing bucket. Nothing is sent until
you click Send.

Sharing uses the independent Context Kit workspace at `../context-kit`: JSON format,
Rust verification engine, and handoff/pickup/integration skills. Keep that workspace
beside mydash when building. The executable embeds the protocol and handover instructions; runtime use does not need that checkout. The app's GCS, native history,
and session adapters are separate from the protocol. Cached metadata lives under
`data/sharing/state.json`; manifests, raw deliveries, previews, and verified files
are stored separately beneath `data/sharing/`. Active refresh and imports do not
download large external URI references automatically. Source-agent preparation
requires a current Claude or Codex CLI with non-interactive fork support.

After installing a sharing update, restart the service and desktop window to load
both API and bundled UI changes. Restarting the updated service preserves daemon-owned embedded PTYs;
saved conversations remain resumable in History. The one-time upgrade from the older service still ends its service-owned terminals, so finish those before activating this update. Use an isolated `--demo` data file
for UI tests: its GCS adapter is a local mirror and it never launches real agents.

## Start

Install the built app at `~/Applications/mydash.app`, then start the service from a regular Terminal window outside coding-agent tool sandboxes:

```sh
~/code/mydash/start.command
```

Open mydash from Finder or Spotlight. The same UI is available at http://127.0.0.1:8765. Closing the app window keeps the service and agents running. With the terminal daemon active, stopping the HTTP service also preserves terminals. Older service-owned terminals close with their service; saved conversation UUIDs remain resumable in History. The service does not start at login automatically.

The desktop **Restart** button beside **GCP login** opens compact inline controls:

- **Service** replaces the HTTP service while daemon-owned terminals keep their processes, memory, and output. The selected agent remains selected.
- **Terminals** replaces the terminal daemon and service, then resumes the same saved Claude/Codex chats in their recorded working directories. The one-time migration from service-owned terminals uses the same resume path.

Any restart that closes live terminals requires confirmation. Finish running work first: chat history can resume, but in-flight commands and subagents may be interrupted and cannot be restored as live processes. Missing chat UUIDs or working directories block disruptive restarts. Reopen the desktop after installing this feature to load its native restart commands; the helper can then upgrade an older service directly.

The detached helper continues if the app window closes. Progress and failures persist in private `data/restarts/` job files; replacement-service output is in `data/logs/service.log`. Only verified same-user mydash processes for this installation are signalled, with no force-kill. Duplicate restarts are locked, and completed runs have a ten-second cooldown. These controls target the default `data/state.json` profile and do not replace saved conversations with new ones.

Agent tabs use a dot only when waiting for your reply or permission. Activity stays on the right. Claude's Stop/SubagentStop hooks supply background shell, monitor, other task, and scheduled-wakeup counts; a paused session with pending work displays **Background**, rather than asking for attention. These are snapshots updated by hook events, not continuous process polling. Missing metadata stays unknown; a fresh empty snapshot clears previous counts. The bridge retains counts only, without shell commands, monitor output, or scheduled prompts.

In an agent chat, **⌘R** edits its dashboard title directly in the title bar. **Enter** saves; **Escape** cancels. The saved conversation identity, folder, and linked work stay attached.

cmux integration is retired. Existing saved cmux conversations remain in History and resume embedded when opened. Close any external copy first; mydash no longer inspects or controls cmux.

## Workspace

**Ask agent** on All Items, Linear, GitHub and Sentry opens a compact inline question/task box with agent and folder choices. Sending creates a durable snapshot of every matching work item, including later pages, links, statuses, CI, hierarchy and the current filters. Hidden, snoozed and archived items stay outside the normal matching list. The agent reads this cached snapshot and starts your request immediately; current remote facts can be checked as needed. The view is reference context, so its entire list is not automatically attached as work owned by the agent. Snapshots live in `data/views/` and remain available when resuming the chat.

- **Agents:** persistent live-agent strip, embedded terminal, related work and a results drawer. Working/waiting/permission state and subagent counts come from launch-scoped hooks; unavailable metadata is shown as unknown.
- **Work sources:** compact searchable lists. Start an agent or jump to its workspace from an issue. Repository/project folder choices can be remembered. Hide is reversible. Local projects support Done and Archive.
- **History:** saved conversations whose terminals are no longer in memory. Resume preserves the conversation ID, work links and working directory.
- **Skills:** on-demand presets and opt-in daily/interval schedules. Missed runs either catch up once or skip; no backlog replay. Schedules run only while the service is awake. The clock button shows new results and schedule failures/skips.
- **Results:** Markdown, images, sandboxed HTML, CSV and `mydash.table/v1` JSON. Registered local outputs become private immutable snapshots; tables display the first 1,000 rows while preserving the full file.
- **Changes and Logs:** source changes, sync errors and optional UI performance recording.

Choose Claude or Codex and click **+** for an embedded session. Issue/PR buttons supply context automatically. The global [mydash skill](skill/SKILL.md) explains work attribution, checkpoints and result registration. It is available to Claude and Codex through their global skill directories. The same Rust executable handles registration, lifecycle hooks and the signed-file fallback. New sessions do not need Python or uv. The retained Python server, clients and tests are migration reference and support previously launched sessions until they are resumed; they are not included in the app bundle.

## Connections

The bottom-left **GCP login** button runs Google Cloud CLI login and Application Default Credentials login concurrently in the background, opening their normal browser flows. Complete both browser tabs. CLI login uses `--force` to refresh even cached credentials; both commands use `--quiet --launch-browser` so terminal confirmation prompts do not block. Separate statuses show completion/failure, with Cancel and a ten-minute timeout. Repeated clicks reuse the active run; a 30-second cooldown prevents duplicate launches. Credentials remain in gcloud’s own storage; mydash stores no tokens or raw authentication output.

GitHub uses `gh auth login`, or an optional `GITHUB_TOKEN` in `.env`. Linear uses `LINEAR_API_KEY`. Sentry uses `SENTRY_ORG` plus credentials from `~/.sentryclirc`, or `SENTRY_AUTH_TOKEN`; optional `SENTRY_STATS_PERIOD` is 1–90 days. Sync needs read access. Optional GitHub PR actions need repository write permissions for pull requests; tokens with read-only permissions can keep syncing. Copy `.env.example` to `.env`, fill it locally, set mode 600, and restart the service. Never commit credentials.

Each GitHub PR row has a reviewer chip. It shows pending reviewers, the latest submitted review when none are pending, or **None** when there are no reviewers. Open it to mark a draft ready, request a reviewer by GitHub username, or refresh that PR. Suggestions use cached coworkers' usernames. Requests add to existing reviewers. Every write checks the live PR and GitHub permissions first; errors leave the action unsuccessful locally, with an explicit refresh option for uncertain responses. No real PR mutations run in demo mode.

A dot beside GitHub indicates unseen PRs that are ready and request your review. Successful sync observes new request/draft-to-ready cycles; showing a matching PR row in the GitHub view acknowledges its cycle. Seen markers are durable and versioned, so a late acknowledgement cannot clear a newer request. Drafts, completed, hidden, and snoozed items do not raise the dot. Updates follow the normal five-minute sync interval.

The right-hand agent sidebar starts collapsed. Use the small sidebar button beside the terminal controls to show work, resources, worktrees, and results. Sections also start collapsed, and your expansion choices are remembered locally.

Starting from a work item attaches only that selected item. Agents can explicitly register more items as they take them on. The agent work list and Continue buttons use direct attachments; related groups and newsletter co-mentions do not implicitly assign the whole group. Existing saved attachments are preserved for manual review.

GitHub, Linear and Sentry sync independently with cooldown/backoff. Cached data renders immediately. Force refresh respects source backoff. Sentry errors and Linear tickets are separate records; automatic ticket creation depends on external alert rules.

## Storage and boundaries

Canonical work, conversation links, schedules and run history live in `data/state.json`. Writes are atomic with a single-writer file lock. Provider refresh does not replace local projects or agent links. Credentials stay server-side. Terminal scrollback is bounded in memory, not stored as a conversation transcript. Agent CLIs own their conversation history.

The service binds only loopback and validates Host/Origin/CSRF. Agent registration uses a private bearer credential or signed inbox. Embedded sessions are limited to 12 live terminals, each with 2 MiB retained output. HTML results cannot run scripts or access terminal IPC. Artifacts use opaque IDs, validated roots and private snapshots. Launches keep normal agent approval settings and global guidance.

## Build and verify

```sh
cd ~/code/mydash/desktop
npm ci
npm run build
cargo test -- --test-threads=1
cd ..
node --test tests/*.cjs
```

Rust stable, Apple Command Line Tools and Node/npm are build dependencies. Production builds leave `MYDASH_SERVICE_URL` unset. Install `desktop/target/release/bundle/macos/mydash.app`, quit the old window and relaunch it. Restarting only the service does not replace bundled frontend assets.

See [desktop/ARCHITECTURE.md](desktop/ARCHITECTURE.md) for module boundaries. Use [the performance skill](.agents/skills/mydash-performance/SKILL.md) for measured diagnostics. In Logs, start UI performance recording, reproduce the interaction and inspect durations; stop recording afterward. Samples are bounded numeric data in memory.

Embedded agents inherit the service’s operating-system restrictions. Deploy the service from a regular user Terminal, never a sandboxed coding-tool process. Before launch, mydash checks that agent storage is writable; it does not alter sandbox or agent permissions. Interactive PTYs set true color and remove inherited NO_COLOR/CI hints.

The compact UI uses Rosé Pine main, including the terminal ANSI palette. The global + starts an agent immediately; its arrow holds remembered agent and folder choices. Work links and results attach inline. The terminal’s expand button hides the surrounding panels and leaves Exit focus visible. Cmd+Shift+Enter (Ctrl+Shift+Enter on other platforms) toggles focus; Escape remains available to the agent CLI.

Agents report worktrees through registration. The service records Git branch, main repo, checkout path and linked work; `register.py worktree --record … --path … --url … [--primary]` links additional checkouts without changing the resume directory unless requested. Git metadata is read on explicit registration only. See the global mydash skill for the reporting protocol.

Keyboard navigation: ⌘1–9 selects live agents; ⌘⇧Enter toggles terminal focus.
⌘N opens the inline fuzzy folder picker for the selected agent (Enter starts it).
Sidebar shortcuts are displayed beside each view. On All Items, Linear, GitHub,
Sentry, and Projects, `/` focuses fuzzy search. Type to filter the complete cache;
↑/↓ selects a row, O opens its source, and Enter opens details. Escape or Enter
leaves the search field on the highlighted row. Ordinary typing stays in inputs.

Sidebar keys follow the view nouns: ⌘G GitHub, ⌘P Projects, ⌘L Linear,
⌘S Sentry, ⌘⇧A Agents, ⌘I All Items, ⌘⇧S Skills, ⌘⇧H History,
⌘⇧C Changes, and ⌘⇧L Logs. Shift resolves collisions while keeping standard
Select All, Copy, and Hide shortcuts available.

The agent heading includes a Zed button. It opens the registered working directory
(including its worktree) in Zed. Missing directories show an error rather than
opening a different repository. This action is available in the desktop app.

On macOS, live agents and the + launcher share a compact native title bar.
Tabs shrink and truncate titles as more agents open; the notification button
hides at the tightest widths. The + launcher always keeps its space. Drag the
empty title-bar background to move the window. A tab's × closes its terminal
with a hangup signal; its saved conversation and work links remain in History.
The code icon beside terminal controls opens the current working folder in Zed.

Browser-style tab keys: ⌘T opens the new-agent folder picker, ⌘W closes the
selected agent terminal, and ⌘⇧T resumes the last closed conversation. The
recently closed list survives app restarts and retains up to 20 entries.
All Items uses ⌘I (items).

GitHub expands into Mine (authored PRs) and To Review (PRs requesting your review).
Click GitHub to return to all tracked GitHub work. Linear expands into its project
filters, including No project; click Linear to clear the project filter. These
filters combine with search, repository, and status filters without fetching.
History, Changes, and Logs remain aligned at the bottom of the sidebar.

Artifact viewers offer content-copy buttons: source text for text artifacts, separate CSV and Markdown buttons for tables, and actual image data for PNG/JPEG/WebP. Tables copy all saved rows. Source and saved-copy paths are selectable and have their own copy controls. Python scripts can be attached directly as `.py` files and are previewed without executing them.

GitHub’s **Group: Stack** view uses cached base/head branch metadata to show dependent PRs base-first, with compact indentation. Mine, To Review, search, repository and status filters still apply; a hidden/filtered parent appears as a context link without adding it to the result set. Closed parents, fork ambiguities and missing branch data are left ungrouped. Sync refreshes the branch metadata; no local Git checkout or extra per-PR fetch is needed.

## Native agent commands

The installed `mydash-desktop` executable also runs `register`, `artifact`, `checkpoint`, `briefing`, `status`, `worktree`, `resource`, `context`, `review`, `ci`, and `hook`. Run a command with `--help` for options. Launch packets supply the exact executable and profile. New agent sessions receive `MYDASH_BIN` and `MYDASH_HOME`. The client reads its private runtime credential internally; do not print that file.

```sh
"$MYDASH_BIN" --home "$MYDASH_HOME" review --record RECORD_ID --show
"$MYDASH_BIN" --home "$MYDASH_HOME" status --record RECORD_ID --state working
```

The binary embeds browser assets and reporting/protocol instructions. `--serve --home /absolute/data-profile` can run without this source tree. Git, installed agent CLIs and selected provider tools remain external dependencies. Switching an existing terminal daemon to native hooks requires restarting that daemon; service-only replacement preserves existing processes and their original hook commands.

Review completion records which feedback the coordinator consumed. Those comments remain visible, and later feedback stays pending. Completing a review does not claim the findings were fixed.
