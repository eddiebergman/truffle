---
name: handoff-context
description: Prepare and share an application-independent context handoff with selected artifacts and references using the shared Context Bundle JSON format. Use when asked to hand off work or publish context for another person or agent.
---

# Handoff context

Use the shared [Context Bundle format](../../protocol/FORMAT.md). The format is the contract; no chat application, dashboard, UI, provider API, or native session-resume state is required. Any conforming implementation can write it. The commands below use the optional `context-kit` reference CLI on PATH.

Read [the shared format](../../protocol/FORMAT.md) when authoring or extending a manifest. Read [the GCS transport guide](../../protocol/GCS.md) when transferring a bundle with GCS.

For named team recipients, use [the independent inbox protocol](../../protocol/INBOX.md).
Accept direct names/handles such as `/eddie`; existing inbox paths supply name suggestions, and the optional directory can add
display names, never a registration requirement or email/alias lookup.
Normalize and validate the handle using INBOX.md, publish the bundle first, then
create immutable per-recipient delivery records. Preserve retry IDs in a local
outbox and report each recipient's result. A plain manifest URI is still enough
when no delivery adapter is available; say that no inbox notification was sent.

## Prepare a handoff

- Capture the objective, current conclusion, decisions, unresolved questions, next steps, and relevant verified links. Include a short chronological excerpt of prior user/assistant conversation by default, alongside the summary, so the recipient understands the framing and tone. Prefer original wording with roles and timestamps when known; label paraphrases and truncation. Exclude tool calls/results and hidden reasoning. The full readable conversation or raw transcript is an optional explicit export.
- Supply human-readable authors/sender provenance and timestamps when known. Put
  external references in an open `uris` map (for example arbitrary categories
  containing URI strings or objects with `uri`, `title`, and free-form `data`).
  Do not add domain-specific top-level fields or impose a fixed category list.
  Keep structured details in generic `data` or items for recipient agents to use.
- Select relevant artifacts explicitly. Tables use UTF-8 CSV or the protocol's columns/rows JSON; plots use PNG/WebP; reports use Markdown, PDF, or HTML. Keep a plain-text explanation for every result that might not render elsewhere.
- Reference large objects that already exist in accessible buckets or public storage
  by their existing URI. Do not download or re-upload those bytes into the sharing
  bucket. Prefer an entry in the open `uris` map, especially above the attachment
  limit, with purpose and known size/media type/checksum/version in `data`. A
  `link` item can provide additional discussion. Include a small preview only when
  useful. Do not fetch a large object just
  to compute its checksum; distinguish known metadata from unverified claims.
  Note required access and moving references, and omit embedded credentials or
  expiring signed URLs. Existing bucket/public URIs need no blob copy in `misc`.
- Check repository URLs separately from branch availability: a local-only branch
  does not imply that the repository has no remote. Verify exact commits and flag
  commits unavailable remotely. For reports with relative image/file links, keep
  the original and provide a portable rendering or explicit artifact mapping;
  expose unresolved links rather than silently presenting a broken report.
- Check selected file contents as well as existence. Label full-study tables as
  such rather than describing them as a selected subset. Separate facts checked
  now from historical agent reports (for example cloud shutdown or remote doc
  state). Use evidence from the current tool environment or an explicitly labeled
  driver verification; do not infer remote availability from a local Git commit.
- For code, include the repository URL and exact commit, plus branch, file path, line range, PR/ticket links, or an optional patch as appropriate. A branch is a moving reference; an exact commit gives the recipient a stable baseline. Local absolute paths and native resume IDs are optional private provenance, not portable code references.
- Do not collect the whole working directory or agent home. Include only content covered by the sharing request. Do not put credentials, environment files, or authentication tokens in a bundle.

Create `context.json`, then add structured items from JSON files or attach selected files:

```sh
context-kit new context.json 'Investigation handoff'
context-kit add context.json handoff-item.json
context-kit attach context.json latency.csv table 'Latency measurements' text/csv
context-kit attach context.json latency.png plot 'Latency comparison' image/png
context-kit check context.json
context-kit pack context.json ./shareable-context
context-kit inspect ./shareable-context/context.json
```

`attach` snapshots bytes immediately and omits the original source path. `pack` makes a new folder with deduplicated, hashed attachments. Use `pack ... --single-json` for a small, self-contained JSON file. All output paths must be new; an existing output is left intact.

## Verify the handover

A selection plan is a draft until its local files are actually snapshotted and
checked. If tools cannot read files or run an engine, return the draft and explain
what a conforming host must materialize; do not claim a finished bundle or invent
integrity metadata. A host can perform byte snapshots and verification separately
from the source agent's content selection.

Inspect the packed bundle from its new location, not from the original worktree.
Check attachment hashes and relative links. A report with figures needs a portable
rendering (for example static HTML with embedded images) or a viewer that resolves
its bundled artifacts. Preserve the original source alongside a derived rendering
and label the derivation. Keep large existing remote objects as links throughout.

When a handover test is requested, give a fresh recipient only the finished bundle
and pickup instructions, without the source conversation or sender's worktree.
Ask it to identify the objective, findings, uncertainty, completed/unresolved work,
and evidence, and inspect an included table and plot when present. Compare its
understanding against the supplied evidence. Check that missing/unknown parts
stay visible without blocking useful context. Correct problems and repeat the
affected steps before calling the handover usable; state which paths were tested.

If the request is only to prepare or review a context, stop at the local bundle. A request to share/publish permits transfer of the selected content to the requested destination; this skill does not expand that selection or change bucket permissions. After transfer, return the immutable manifest URI and a brief description of the included material.


The recipient uses the independent `pickup-context` skill or any conforming reader. Include the manifest URI/path and a short readable summary in the handoff; do not require the recipient to use the sender's application.
