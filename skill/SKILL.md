---
name: truffle
description: Register the current Claude or Codex conversation in truffle and link its tickets, PRs, errors, or local projects. Use when asked to attach or track this session, update its work links, replace stale resources, worktrees, or work links, report resources used by this session, save a dashboard handoff or Work review briefing, or contribute an inline review draft.
---

# truffle

Keep one conversation associated with all its related work. This skill changes only the local dashboard.

Truffle sets `TRUFFLE_BIN` to its executable and `TRUFFLE_HOME` to its local data profile when launching an agent. Use the exact helper command in the launch context. Outside such a session, set these variables to the installed executable and profile. No Python or uv runtime is needed.

## Register this conversation

Use the existing conversation ID. Never start or resume another conversation to obtain an ID.

- Claude exposes its current ID as `CLAUDE_CODE_SESSION_ID`.
- Codex exposes its current ID as `CODEX_THREAD_ID` when available.
- If the variable is absent, use an exact session ID supplied by the user or trusted session metadata.
- If no exact ID is available, ask for the chat UUID or a `codex://threads/<UUID>` link. Do not guess.

Use the native helper. The helper reads the local credential file. Never read, print, or paste that credential.

```sh
"$TRUFFLE_BIN" --home "$TRUFFLE_HOME" register \
  --agent claude \
  --title "OpenAPI implementation" \
  --cwd /absolute/path/to/repository \
  --url https://linear.app/workspace/issue/ENG-123/example \
  --url https://github.com/owner/repository/pull/123
```

Choose `--agent codex` for Codex. Use `--session <UUID>` when the helper cannot read the current ID from the environment.

For a conversation launched by truffle, include `--record <dashboard-record-ID>` from its context packet. Preserve that record ID.
Register a new dashboard-launched conversation before the first reply, even when it has no linked work yet.
Do not create a placeholder project. When the user supplies related issue or PR links in that conversation,
register them with the same record ID immediately. Do not ask the user to invoke this skill again.

To track work without an external issue, use `--project "Project name"` instead of `--url`.
Use both options when local work and external issues share a conversation.

The helper updates an existing association with the same agent and conversation ID. Repeating registration preserves existing links.
Repeat `--url` to add new tickets or PRs created during the work. GitHub and Linear access uses the dashboard's existing credentials.

The helper prints the dashboard link after successful registration. Return that link to the user.

## Worktrees and resume folders

After creating, selecting, or switching a worktree, report it immediately. Do not wait until the task is finished. Re-run `register` with the same dashboard record ID, agent and exact conversation UUID, set `--cwd` to the actual worktree root, and include the ticket or PR URLs for the work in that checkout. The server reads the real branch and main repository from Git; do not infer them from the folder name. Registration preserves previous worktree links and sets this checkout as the primary resume directory.

Start from the associated main repository under `~/code/<repo>`. Use `git worktree list` to reuse a checkout for the same ticket, PR or branch. Create a new checkout at `~/code/<repo>/.worktrees/<task>` when needed, with `.worktrees/` locally ignored. Do not move existing checkouts or use a shared `~/code/worktrees` folder. Follow the user's repository instructions when they differ. Do not create a worktree merely to register it.

For another checkout in the same conversation, first register any new issue/PR links, then associate only its related work with:

```sh
"$TRUFFLE_BIN" --home "$TRUFFLE_HOME" worktree \
  --record <dashboard-record-ID> \
  --path /absolute/path/to/worktree \
  --url https://github.com/owner/repository/pull/123
```

Repeat `--url` or use `--item` for existing dashboard IDs. These links must already belong to this conversation. Omit both to associate all its work. Add `--primary` only when this checkout should become the conversation's resume directory; otherwise the current primary directory stays unchanged. Re-report after switching branches. Never attach unrelated work, create a new conversation to report a checkout, or guess issue links.

The agent workspace shows these branches and their related work. Expand a worktree to select/copy its path. Existing deleted worktrees remain as saved context; choose a replacement and register the actual path before resuming work.

## Replace or remove stale context

Registration and worktree reporting add links. Resource reports update matching identities and preserve omitted fields. To replace a list or remove stale links, inspect this session first:

```sh
"$TRUFFLE_BIN" --home "$TRUFFLE_HOME" context --record <dashboard-record-ID> --show > /tmp/truffle-context.json
```

Edit that JSON, then submit it:

```sh
"$TRUFFLE_BIN" --home "$TRUFFLE_HOME" context --record <dashboard-record-ID> --file /tmp/truffle-context.json
```

The command prints the context object directly: `resources`, `worktrees`, `itemIds`, `artifactIds`, and `revision`. Every replacement requires the exact `revision` captured by this read. Keep it unchanged while editing. Each supplied list becomes the complete replacement for that list; omitted lists stay unchanged. Use `[]` to clear dashboard links only. All lists commit together, or none do. On conflict, read the latest context, inspect what changed, and reapply your intended edits to that snapshot. Do not copy a fresh revision into an old replacement file or retry the stale file unchanged. Do not replace a full list based only on a partial account of the session's work.

- `resources`: full resource objects with the same required identity fields as resource reporting. Replacement removes omitted records and omitted metadata fields. An ordinary report with the same identity remains the way to update a resource's state while retaining its other metadata.
- `worktrees`: objects with `path`, `itemIds`, and `primary`. Paths must be existing Git checkouts. Include at most one `primary: true` to change the saved resume directory. Other tree metadata is read from Git. An empty list removes worktree links but preserves the saved resume directory. This does not change the running terminal's directory.
- `itemIds`: existing dashboard work IDs. Register unfamiliar URLs first. Removed work IDs are also removed from worktree associations.
- `artifactIds`: IDs of already attached artifacts. Removing an association keeps the stored file and its links to other sessions.

For a resource-only replacement, first run `context --record ID --show` as above. Create a report object with only `revision` and `resources`: copy the captured context `revision` unchanged and edit the complete `resources` array from that same snapshot. Preserve metadata you intend to keep, including fields you do not recognize. Then submit it:

```sh
"$TRUFFLE_BIN" --home "$TRUFFLE_HOME" resource --record <dashboard-record-ID> --file /absolute/path/resources.json --replace
```

`resource --replace` requires the captured revision in the report JSON; there is no separate revision flag. The revision covers the whole session context, so another context edit can cause a conflict even when it changes a different list. On conflict, re-read context and reapply your intended resource edits. `{"resources":[]}` alone is not a valid replacement; clearing resource links also requires the captured `revision`. Without `--replace`, resource reports keep their existing update behavior and do not require a revision. Replacement changes dashboard metadata only. It does not stop cloud resources, delete worktrees or files, or close conversations. Report a known finished resource as completed/stopped when that history remains useful; detach it when it no longer belongs in the session's context.

## Boundaries

- Use only links related to the work that the user wants tracked.
- Registration does not create or edit GitHub or Linear records.
- Registration does not send messages to other conversations.
- Keep agent permissions and approval settings unchanged.
- If the sandbox blocks loopback access, the helper uses a signed private file inbox. Keep sandbox settings unchanged.
- If truffle is unavailable, report the error. Ask the user to start it from any terminal with `~/code/mydash/start.command`.
- Retry once after the server is available. Do not repeatedly launch servers or agent sessions.
- Done and Archive are dashboard actions. They do not close an agent conversation.


## Save a handoff or work review

For a registered record, use the same helper to save a local checkpoint. Write a UTF-8 JSON file with string fields `done`, `blocked`, and `next`, then run:

```sh
"$TRUFFLE_BIN" --home "$TRUFFLE_HOME" checkpoint --record <dashboard-record-ID> --file /absolute/path/checkpoint.json
```

This updates only local dashboard notes. Do not create or resume another chat. New agent launch packets include related checkpoints as reference data. Continue does not inject notes into an existing chat.

For a dashboard Work review, make every Linear ticket ID and GitHub PR/issue reference a Markdown link to its verified source URL. Include links in the top actions, bold text, repeated mentions, and tables. Use repository-qualified labels such as `api#1419`. A link in another section does not replace a link in the current action. Resolve URLs from the dashboard snapshot or source; do not guess ambiguous repository names. Code examples can remain literal.

Save the completed report as UTF-8 text and run:

```sh
"$TRUFFLE_BIN" --home "$TRUFFLE_HOME" briefing --record <work-review-record-ID> --file /absolute/path/report.md
```

Only Work review records accept briefings. Do not attach every reviewed ticket to the review session. Do not change source records while preparing a read-only review.

## Save newsletter outputs and artifacts

For a registered session, attach a final report or other display file:

```sh
"$TRUFFLE_BIN" --home "$TRUFFLE_HOME" artifact --record <dashboard-record-ID> --file /absolute/path/report.html --title "Production daily"
```

Use `--url https://...` instead of `--file` to attach an existing external artifact. Do not invent artifact URLs.
Local files are copied to a private immutable snapshot in truffle data/artifacts; original paths are kept as provenance. HTTPS links remain links. Registration does not upload or publish the file.
Files must be inside the registered session folder or a configured newsletter reports folder.
Supported files: HTML, Markdown, text, CSV, truffle.table/v1 JSON, PDF, PNG, JPEG, WebP, and Python scripts (`.py`). The size limit is 10 MiB.
Attach Python scripts directly with `artifact --record ... --file /absolute/path/script.py --title ...`; do not wrap the source in Markdown just to attach it. The preview preserves indentation as read-only text and never runs the script. Use a separate Markdown artifact for run instructions or results when useful.
Register final reports and display outputs. Do not register raw telemetry or credential files.
Reuse the same record for every output from one conversation. Repeated registration preserves other session associations.
Newsletter launch packets supply the skill path and output folder. Start the assigned workflow after registration.
Follow the launch packet's delivery instructions. A local newsletter run does not authorize external publication or messages.

## Communicate results in the agent workspace

Prefer a dashboard result over a large table, plot, or report printed in the terminal. Write Markdown for narrative reports, UTF-8 CSV with a header row for tables, and PNG/WebP for plots. Register each finished output with `artifact --record ... --file ... --title ...`, then give the user a brief conclusion and the dashboard link. Keep original source links in reports. Only register outputs relevant to this conversation.

For typed tables, use a UTF-8 JSON file with this exact schema (up to 100 columns and 10,000 rows; row cells are strings, numbers, booleans, or null):

```json
{"schema":"truffle.table/v1","columns":["Metric","Value"],"rows":[["p50_ms",12.4],["p95_ms",20.1]]}
```

The drawer renders CSV and table JSON as a table, Markdown as a report, and images as plots. It displays the first 1,000 table rows; the stored file retains all rows. HTML is displayed without scripts beside terminal controls. Do not depend on inline JavaScript for the primary result; static HTML or images are preferable.

Agents use embedded PTYs. cmux integration has been retired; saved external conversations can be resumed in truffle after closing their external terminal. Launch-scoped lifecycle hooks report working/waiting/needs-input and subagent counts automatically when supported. They preserve global instructions and skills. If a hook is unavailable, report state explicitly at meaningful boundaries using:

```sh
"$TRUFFLE_BIN" --home "$TRUFFLE_HOME" status --record <dashboard-record-ID> --state working --subagents 2
```

Use `waiting` when returning control to the user and `needs-input` when blocked on user input. Report only counts you know. Add `--reason finished`, `question`, `approval`, `failed`, or `ready` when known, so the tab explains why it needs attention. Do not infer a failure or question merely from an idle terminal. Do not create new sessions, alter permission settings, or install global hooks to report state.

## Report ephemeral resources

After creating or reusing an ephemeral VM, GPU pool, dev environment, deploy job, or monitor, report it with the same dashboard record ID. Update it when its status, expiry, ownership or cleanup plan changes. Use existing knowledge or cheap checks relevant to the task; reporting does not require broad cloud discovery.

Write a JSON object and submit it:

```json
{
  "resources": [{
    "provider": "gcp",
    "kind": "vm",
    "id": "projects/example/zones/europe-west4-a/instances/bench-1",
    "name": "bench-1",
    "state": "running",
    "purpose": "Latency benchmark",
    "observedAt": "2026-10-23T10:00:00Z",
    "verification": "checked",
    "expiresAt": "2026-10-23T18:00:00Z",
    "cleanup": "Ask the user after the benchmark; do not delete automatically",
    "workUrls": ["https://github.com/owner/repository/pull/123"],
    "uris": {"console": ["https://console.cloud.google.com/compute/instancesDetail/zones/europe-west4-a/instances/bench-1?project=example"]},
    "data": {"machineType": "c4d-highmem-48"}
  }]
}
```

```sh
"$TRUFFLE_BIN" --home "$TRUFFLE_HOME" resource --record <dashboard-record-ID> --file /absolute/path/resources.json
```

`provider`, `kind`, and `id` are required strings. Keep the same `id` for updates; use the fully qualified provider ID when possible. An optional `scope` distinguishes otherwise identical IDs across projects or regions. Other fields are optional, and extra metadata is retained. `uris` groups reference strings or arrays by purpose; use real URIs when available. Command text is displayed for copying and is never executed by the dashboard. Include no credentials, tokens, signed URLs, or secret environment values.

Only use `verification: "checked"` with an actual observation timestamp. Otherwise use `state: "unknown"` or a description that explains what is last known. Use RFC3339 timestamps for known observations and expiry; do not invent an expiry. `reportedAt` is added by the server and is not evidence that the cloud state was checked. Report `stopped`, `deleted`, `completed`, or `expired` only when known; these remain accessible under past resources. Omitted resources are not removed, and an empty report does not imply cleanup.

Resource lifetime is independent of terminal lifetime. Closing a chat does not authorize stopping, deleting, or taking ownership of resources. Preserve explicit keep/delete constraints and identify user-owned resources you merely reused. The dashboard stores private metadata; it does not provision, poll, or clean up cloud resources for you.


## Contribute a review draft

Use this when asked to review work for another truffle agent. Use the target dashboard record ID, not your own conversation UUID. This adds comments for the user to inspect. It does not send terminal input or post a GitHub review.

Write a JSON file with a `comments` array. Each comment has a stable unique `id`, `author` (for example `Codex · correctness`), absolute registered worktree `root`, relative file `path`, `side` (`additions` or `deletions`), one-based inclusive `start` and `end`, and `text`. Omit `fingerprint` unless you obtained it from truffle's current diff API. The service verifies the checkout and line range, records the current file version and excerpt, and saves the whole batch atomically. Verify the code immediately before submitting. Do not claim that a comment reviewed a version you did not read.

```sh
"$TRUFFLE_BIN" --home "$TRUFFLE_HOME" review --record TARGET_RECORD_ID --file /absolute/path/review.json
"$TRUFFLE_BIN" --home "$TRUFFLE_HOME" review --record TARGET_RECORD_ID --show
```

Repeat an unchanged batch with the same comment IDs to retry safely. Different reviewers must use different IDs. Batches append to the shared draft; they do not replace other comments. Supply at most 100 comments and 96 KiB per batch. The user sees authors and inline comments in the diff and chooses when to Send review as one batch to the target agent. Later comments stay in the draft if they arrive during a send. Draft comments persist across app restarts.

For a dashboard-requested review, preserve the target record ID and review request ID. Follow the invoked skill for reviewer roles, count and completion requirements. Truffle does not prescribe them. Report `review --record ID --request REQUEST --state reviewing`. Give every reviewer both IDs, the shared goal, relevant checkouts and reporting steps. Use a stable identifier with `--pass NAME` for reviewer progress and assessments. The parent verifies the required reports and marks the whole request complete with `--state complete`. Use `--state failed` when blocked. These updates do not send the draft or authorize applying findings.

Each dashboard review pass must include an `overall` object beside `comments` in its JSON report: `{"comments": [], "overall": {"author": "Codex · Performance", "text": "Verdict, scope checked, general findings and limits."}}`. Supply the exact `--request` and `--pass`. The author is at most 120 bytes; the assessment is at most 8000 bytes. Submit one assessment per pass, even with no inline findings. Keep unanchored findings here. The drawer displays each assessment separately and retains it after sending. Only unsent assessments join the next batch. Use the same content when retrying a submission.

Dashboard Agent review covers the conversation's active work across repositories. A selected diff checkout is a viewer location, not the review boundary. Pass the shared goal, acceptance criteria and relevant source links to every reviewer chosen by the skill. Register additional existing checkouts with `worktree --record RECORD_ID --path /absolute/checkout`; this leaves the primary resume folder unchanged. Each inline comment names its real `root` and relative `path`. Findings can span any checkout linked to this conversation, including checkouts linked after the review starts. Each overall assessment describes the actual scope and any limits. Freshness is checked against recorded checkout versions; unavailable versions remain unverified.

When the dashboard asks for a CI investigation, use the supplied record, PR item and request IDs with `ci --record ID --item ITEM --request REQUEST --state investigating`. Finish with `--state complete` after reporting the diagnosis, or `--state failed` if blocked. These updates record progress only. They do not rerun CI or authorize code changes.

When the parent has consumed a completed review round, read `review --record ID --show` and finish with `review --record ID --request REQUEST --state complete --reviewed-revision REVISION`. If the revision changed, read the new feedback before retrying. Completed comments remain visible but are not sent again. Later feedback stays pending. This state records a completed review, not completed implementation.
