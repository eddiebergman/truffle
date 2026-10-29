# Technical decisions behind truffle

This is an implementation inventory for someone building a personal agent application. It describes the current Rust implementation, including the separately owned terminal daemon added on 2026-10-23. It excludes interface layout choices. Configuration, account names, and cloud identifiers should belong to each installation.

## Process boundaries

```text
Desktop webview ── restricted native IPC ── loopback HTTP service
Browser ── same-origin HTTP + CSRF ────────┘
                                          ├─ canonical local state + provider sync
                                          ├─ artifact snapshots + context sharing
                                          └─ private Unix socket ── terminal daemon ── agent PTYs
Agent CLI ── authenticated registration helper / lifecycle hooks ── HTTP service
```

The application uses Tauri 2 for its desktop shell and Rust for its backend. The presentation is local HTML/CSS/JavaScript with vendored xterm.js and Markdown rendering. It does not require a frontend framework, a CDN, a Python web server, or an embedded browser engine shipped by the application itself. The desktop uses the operating system webview.

The desktop and service are separate processes. The service can also serve the same static frontend in a browser. A small transport adapter keeps rendering code independent of whether requests arrive through native IPC or browser HTTP. The native layer accepts an explicit route allowlist, rather than exposing a general HTTP proxy or shell command executor.

The terminal daemon is a third process. It owns PTYs, child processes, resize/input handles, process generations, and bounded output buffers. It starts lazily on the first agent launch and detaches from the service's process group. App or HTTP-service exit does not close its terminals. A replacement service finds it using the canonical state-file path and reconnects. No PTY file descriptors are migrated between processes.

Existing terminals created by the older service cannot be transplanted into this daemon. The change applies to subsequent launches after activation. Computer restart or daemon termination still ends its terminal processes; saved native chat IDs support resume. The daemon remains on its existing binary while agents run, even when the app bundle is replaced.

## Persistence and data ownership

Canonical application state is one versioned JSON document at `data/state.json`, not SQLite. An exclusive OS file lock prevents two service processes from writing it concurrently. Mutations clone the state, validate/apply changes, write a private temporary file, sync the file, rename it over the canonical file, and only then publish the new in-memory state and revision. Failed transactions do not publish partial state.

The format retains unknown supported-version fields. Unsupported storage versions are rejected rather than rewritten. This makes later migrations possible without coupling stored records to a particular frontend. Data includes cached work items, local projects, item overrides, conversation associations, worktrees, schedules, and resource reports. Files such as artifacts, launch packets, view snapshots, logs, and sharing state live separately under the data directory.

JSON keeps local deployment and backup simple. Its tradeoff is whole-document cloning and writing on mutation. Larger installations or frequent concurrent updates could justify SQLite behind the same repository API. Revisions and cached projections reduce repeated serialization/rendering; they do not turn JSON into a multi-writer database. Atomic rename protects replacement visibility; filesystem and machine failure guarantees remain those of the local filesystem.

Visual preferences belong to local frontend preferences. Source records and domain relationships do not encode a specific page layout. Application domain code, provider adapters, repository transactions, session workflows, and presentation are separate modules.

## Concurrency and latency

Tauri's command layer uses its async runtime and moves blocking service work off the webview event loop. The HTTP backend currently uses bounded worker threads and blocking `reqwest`, not a fully Tokio-async server. Independent provider sync workers prevent one slow service from blocking another. Terminal polling waits are bounded; output readers and process reapers run independently.

Reading the dashboard uses cached local state and never waits for a fresh provider fetch. A normal sync interval is approximately five minutes, with per-source failure backoff, timeouts, and explicit force-refresh cooldown. One source failing retains its cached records and does not erase other sources. A successful sync clears that source's warning. Last-success time is distinct from last-attempt time.

The frontend avoids rebuilding unchanged projections, indexes associations, and paginates long lists. An opt-in performance harness records bounded numeric timing samples rather than work-item contents. These choices matter more than converting every function to async. Provider async I/O can be introduced behind the same interfaces if profiling warrants it.

## Service and terminal security boundaries

The HTTP service binds to `127.0.0.1`, validates Host, and requires the configured same-origin header plus CSRF for browser mutations. It does not expose wildcard CORS. The native adapter restricts origins and paths, rejects redirects/proxy traversal, and retains service authorization in Rust. Credentials for source services stay in the backend.

Agent registration uses a separate rotating bearer credential in a private runtime file. A signed, bounded private file inbox supports agent sandboxes that can write local files but cannot reach loopback. This bridge is limited to association/reporting operations; it is not a browser-accessible credential bridge. Helpers read the credential without requiring the agent to paste it into a conversation.

The terminal daemon uses a deterministic short Unix-socket directory derived from user ID and the canonical state-file path. It validates directory ownership and private permissions, rejects symlink directories, uses private lock/socket files, and holds a single-owner lock. Its framed JSON protocol has a version, bounded frame size, connection timeouts, and bounded concurrent handlers. Startup is serialized, and an unreachable daemon holding its ownership lock is not replaced with a duplicate agent host. Protocol mismatch requires an intentional upgrade after existing agents finish.

Terminal operations are fixed: inventory, launch a supported agent, poll, input, resize, and close. Launch packets must belong to the installation's private context directory. Commands are constructed as argument arrays. The service does not accept arbitrary executable strings. Of course, an authenticated terminal user can type commands into their own shell or agent; this is a trusted local application, not a sandbox service for remote users.

Start the service from a normal terminal. Child processes inherit the service's operating-system restrictions. Changing an agent's approval mode cannot repair a host sandbox that prevents transcript writes. A startup probe checks that the agent's storage directory is writable before launching it. The application does not change global agent permission settings.

## Provider integration

Provider adapters normalize GitHub, Linear, and Sentry into a stable local work-item model. Remote identity and URL, normalized status, original provider status, hierarchy, links, and relevant metadata remain distinct. Local overrides do not modify remote records.

GitHub collects relevant authored, assigned, and review-requested work; authorship and a requested review are separate roles. PR review state, mergeability, and CI are separate signals. A PR is ready to merge only when the evidence meets the configured rule; ambiguous or missing CI is not treated as passing. Pagination and request bounds are explicit.

Linear relevance does not depend solely on membership of a particular project. Relevant work can be outside a project or in Triage. Parent/subissue structure and linked source URLs are retained. Sentry errors are separate records from Linear issues; a linked error does not prove that automatic Linear issue creation is enabled.

Source queries and credentials belong behind provider adapters. The UI passes identities or filters, not arbitrary fetch URLs. Prefer bounded pagination and targeted incremental queries to repeated full scans. Keep source-specific errors and timing information available in logs without logging tokens or whole private response bodies.

Explicit GitHub PR controls are fixed provider mutations, separate from read-only sync and local status overrides. The service resolves the selected cached node identity, checks the live PR and viewer permissions, and supports only marking a draft ready or requesting a reviewer. Review requests use union semantics to preserve existing people; already-ready/already-requested actions are idempotent. Per-PR locking and cooldowns prevent overlapping actions. A failed or uncertain remote response is not recorded as success. Demo mode exercises local projections without sending remote writes. Reviewer suggestions reuse cached identities rather than polling a user directory.

Review attention is stored as versioned request cycles. A successfully observed transition into ready-for-my-review creates a cycle; viewing the actual row acknowledges that version. Late acknowledgements cannot clear a newer cycle, and refreshes alone do not create duplicates. Work ownership uses direct session attachments. New launches attach only the selected work item; related-work graphs remain a separate reference mechanism. Co-mentions in newsletter reports do not establish work-group edges or agent ownership.

## Conversation identity and work context

A dashboard association ID is distinct from the agent's native conversation UUID and the terminal process generation. One conversation can be linked to multiple tickets, PRs, errors, or local projects. A new terminal generation may resume the same native conversation; it is not the same running process.

Launching an agent writes a private Markdown context packet containing the requested task, actual working directory, exact association ID, registration command, relevant linked work, and reporting instructions. The normal Claude/Codex launch retains global configuration and skills. Launch-scoped lifecycle hooks report working/waiting/needs-input and known subagent counts. Missing or stale observations stay unknown rather than becoming invented activity.

Worktree metadata comes from Git, not directory-name inference. Sessions record the actual checkout, branch, main repository, and related work items. Continue uses the actual primary directory. Creating or switching a worktree triggers an association update; additional secondary checkouts can be retained without changing the primary one.

Closing a terminal is explicit and targets its process generation, preventing an old close action from terminating a replacement process. Saved conversation metadata is retained. PTY output is a bounded in-memory replay buffer, not a durable chat transcript: each terminal retains at most 2 MiB, returned in sequential chunks of at most 32 KiB. Frontend write callbacks provide backpressure. Native agent transcripts remain the source for conversation history and resume.

A task launched from a filtered work view receives an immutable JSON snapshot of the matching item IDs and cached facts, including matches beyond the first rendered page. The snapshot records filters, capture time, source sync times, links, statuses, and truncated-description markers. It is reference context, not an instruction to act on every item. Passing many items to an agent does not attach all of them as that agent's assigned work. Requests carry retry IDs to avoid duplicate sessions after an uncertain response.

## Artifacts and ephemeral resources

Artifacts are separate from terminal text. A local file is copied into a private immutable snapshot with provenance and conversation associations. Existing HTTPS artifacts remain references. The registration layer checks allowed roots, size limits, and supported formats. Opaque artifact IDs prevent path traversal through native or HTTP routes.

Markdown is rendered with source HTML escaped and unsafe links rejected. Tables use headered UTF-8 CSV or the small `truffle.table/v1` schema. Stored files retain the full table; the preview is bounded. Images, PDFs, and static HTML are supported. HTML previews are sandboxed without scripts or network requests. Unknown file types need an explicit fallback rather than executable rendering.

Ephemeral resources are agent-reported descriptors, not a cloud control plane. Required fields are `provider`, `kind`, and `id`; optional fields include `scope`, `name`, `state`, `purpose`, observation/expiry timestamps, cleanup responsibility, related work URLs, grouped `uris`, and arbitrary `data`. Unknown metadata is retained. Same-resource reports upsert; omitting a resource does not declare it stopped or deleted.

Receipt time and observed state time are distinct. An old cloud observation must not become fresh just because the app receives it now. Free-form state descriptions remain descriptions; only explicit terminal states move a resource into past records. Connection references can be copied; command text is never executed by the resource viewer. Resource lifetime is independent of chat/terminal lifetime, and reporting never authorizes automatic cloud cleanup. Secrets and signed URLs do not belong in descriptors.

Clipboard operations are explicit viewer actions. Text files copy their source text; tables offer CSV and Markdown exports of the full saved data, independent of preview pagination. Images copy image data, never a pathname. The desktop uses narrow native IPC and the existing AppKit bindings to write text or decoded images; image IPC accepts an opaque artifact ID, not an arbitrary filesystem path. A named private pasteboard permits integration checks without replacing the user's clipboard. Browser image copy uses the Clipboard API with a PNG payload and preserves the user gesture through a promise. Paths are displayed separately from file contents, with source and immutable snapshot locations distinguished. Python scripts are displayable as inert, whitespace-preserving text and remain Python files in portable handoffs.

## Portable handoff protocol

Context sharing is implemented in a separate Rust workspace, Context Kit. Its Context Bundle JSON format and handoff, pickup, and integration skills are application-independent. A recipient does not need truffle. Basic rendering metadata is explicit, while open items, resource descriptors, `uris` groups, and arbitrary extension data allow other applications and agents to make sense of the content.

A bundle includes selected handover context, references, optional artifacts, and a bounded previous-conversation excerpt. It is not automatically the entire transcript. Tool calls, machine notifications, credentials, and irrelevant history are excluded from the natural-language excerpt. The source agent prepares the useful summary; pickup reports unavailable or unsupported entries instead of crashing the whole import. An independent preparation fork can avoid modifying the source conversation.

Content hashing identifies immutable resources and manifests. Large objects already available in other buckets or publicly accessible locations are referenced by URI rather than uploaded again. Local files chosen for transfer can be uploaded and verified. A URI is a reference, not a promise that the recipient has access; missing access is visible and recoverable.

For a trusted team, GCS holds immutable blobs/manifests and small inbox deliveries:

```text
<root>/blobs/sha256/<hash>
<root>/manifests/sha256/<hash>/context.json
<root>/inboxes/<person>/deliveries/<delivery-id>.json
```

Publish dependencies first, then the manifest, then the inbox delivery. Use conditional creation and stable delivery IDs for safe retries. Hashes verify exact manifest/resource bytes. An optional directory supplies display names; direct names remain usable without central registration or aliases. Recipient suggestions also come from immediate inbox prefixes using delimiter-based listing, without reading other people's message bodies.

Trusted-team bucket access is the authorization boundary. A display name or `from` field is self-declared metadata, not cryptographic sender authentication. Inbox paths are routing conventions, not per-recipient access controls. A public bucket or hostile multi-tenant service needs a different security model.

The receiving integration needs a notification surface, preview, and explicit import into a new agent conversation or equivalent local context. A delivery itself is not authorization to run shell commands, fetch every referenced object, or modify repositories. Onboarding should ask the receiving user where this flow belongs when the host application has no obvious equivalent.

## Scheduled work and utility processes

Skills can run on demand or on a durable local schedule. Missed runs can be skipped or caught up once; the scheduler does not replay an unbounded backlog after sleep. It runs while the service is awake, not as a guarantee of cloud execution or an operating-system wakeup service.

GCP reauthentication is a separate fixed utility workflow. Explicit user action starts `gcloud auth login` and `gcloud auth application-default login` concurrently in background child processes. Their browser flows remain Google's. Each result is tracked independently, with cancellation, timeout, and cooldown. The app does not collect their raw output or persist returned tokens. These short-lived utility jobs are service-owned; persistent agent PTYs are daemon-owned.

## Testing, deployment, and compatibility

Rust tests cover repository transactions, provider query syntax/normalization, artifact boundaries, agent identity, lifecycle behavior, scheduling, sharing, and terminal transport. JavaScript tests cover transport, filtering/search, bounded rendering, shortcuts, Markdown, and artifact parsing. An end-to-end process test starts an isolated terminal daemon, stores a value inside a shell, replaces the whole HTTP process, then checks the same terminal generation, shell memory, and replay buffer. It uses only fixture processes and no real cloud resources.

Demo mode uses isolated state and suppresses provider sync and real agent launches. It permits visual verification of the same frontend. The terminal-daemon test shell requires a separate explicit test flag. Debugging and performance verification should not restart or inject commands into a user's active agents without authorization.

The Rust executable is used as desktop shell, HTTP service, or terminal daemon through separate CLI modes. Bundle replacement must preserve the inode of any running executable: prepare a new bundle and rename/swap directories, rather than overwrite a live binary. Restarting the frontend/service is independent of upgrading the daemon that owns running agents.

Restart orchestration is a fourth detached executable mode, invoked through fixed native commands rather than a generic shell API. It can bootstrap an older HTTP service that lacks restart routes. A reviewed snapshot binds the operation to service/daemon PIDs and live terminal generations; the helper checks it again before stopping anything. Same-user executable/profile checks precede SIGTERM. A filesystem lock prevents overlapping workers, private atomic JSON job files preserve progress across UI closure, and a cooldown prevents repeated launches. A service-only restart leaves daemon-owned PTYs untouched. A terminal-host restart, or migration from an older service-owned PTY, reopens each saved conversation UUID and working directory; it does not preserve process memory, commands, or subagents. Missing resume metadata blocks that path. Regression tests use temporary profiles and a mock agent to verify legacy migration, preserved shell memory, and explicit terminal restart/resume.

The Python backend is no longer the runtime. Small existing Python registration/hook clients remain and run through `uv`; legacy migration/reference tests can remain separate from the Rust runtime checks. Removing these clients would require replacing their agent-facing interface, not reintroducing a Python service.

The principal reusable boundaries are the repository API, provider adapters, restricted service transport, terminal-host protocol, agent registration/reporting interface, artifact registry, and independent Context Bundle protocol. Keep those independent of any particular arrangement of panels or pages.
