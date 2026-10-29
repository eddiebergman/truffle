# Context Bundle draft 0.1 — wire version 1.0

Status: local prototype, not an established public standard. The protocol has no dependency on mydash, Claude, Codex, GCS, a database, or a UI toolkit.

## Envelope

A UTF-8 JSON object has `format: "context-bundle"`, `version: "1.0"`, `title`, `summary`, `items: []`, and `resources: []`. Optional fields are `created_at` (RFC 3339), `authors` (self-declared provenance), `parents` (immutable manifest URIs), and `extensions` (namespaced JSON).

For useful list/preview rendering, writers should provide `created_at` and
`authors: [{"name": "Eddie", "handle": "eddie"}]`; `updated_at` is an optional
content-revision timestamp. Delivery records separately identify who sent a copy
and when. These are self-declared display metadata, not authentication. Display
missing metadata as unknown rather than making context unreadable.

Optional `data` is arbitrary JSON for an agent to interpret. Optional `uris` groups
external references by any producer-chosen category. Category names have no fixed
meaning or required vocabulary:

```json
{
  "uris": {
    "datasets": ["gs://example-study/data/run-42.parquet"],
    "models": [{"uri": "gs://example-study/checkpoints/run-42.bin", "title": "Reference checkpoint", "data": {"generation": "1234567890"}}],
    "reports": ["https://example.org/study/report"]
  }
}
```

Each category is an array of URI strings or objects with required `uri`, optional
`title`, `text`, and free-form `data`. Preserve other fields and unfamiliar
categories. There are no special top-level `datasets` or `models` fields. A generic
viewer can show category, title and URI; agents interpret remaining data in their
own tool. URI groups are references, not resource downloads or upload jobs. Do not
follow arbitrary schemes automatically. Item-level `href` remains useful when a
reference needs its own discussion or relationships.

The portable state is the manifest and its attachment bytes. A reader does not need the sender's machine, session, or installed tools.

The default is curated handover context, not the full chat. Selected normalized
messages may be included as `conversation` items. A full conversation is an
optional explicit export: readable normalized messages or a selected raw vendor
export as a file/reference with a useful summary. Raw session logs can contain
tool outputs, embedded media and private data; they are not required for pickup.
Writers should include a short chronological conversation excerpt by default to
retain framing and tone. Preserve original wording where possible and identify
paraphrases/truncation; a normalized representation does not require paraphrasing.

## Items

Each item has a unique string `id`, a string `kind`, and a human `title`. Optional `text` gives a readable fallback. `resources` is an array of resource IDs, `href` is a reference link, `relations` can link other item IDs, and `extensions` carries namespaced additions. `data` is kind-specific JSON.

| Kind | Data or reference |
| --- | --- |
| `note` | Markdown/plain text in `text` |
| `link` | `href` plus a readable explanation |
| `table` | CSV resource, or `data: {"columns": ["Name", "Value"], "rows": [["p50_ms", 12.4]]}` |
| `plot` | Image resource plus explanatory `text` |
| `file` | Generic resource; unknown media types remain downloadable |
| `code` | `data: {"repository": "https://github.com/org/repo", "commit": "exact commit", "branch": "feature-name", "path": "src/file.rs", "start_line": 20, "end_line": 35}` |
| `conversation` | `data: {"messages": [{"role": "user", "text": "Question"}, {"role": "assistant", "text": "Result"}]}`; optional `speaker`, `at`, and namespaced extensions |

Table cells are strings, finite numbers, booleans, or null. Typed tables support at most 100 columns and 10,000 rows. Larger tables should be file resources with a small preview/summary. Raw vendor chat exports can be file resources; they are not required to interpret normalized messages.

Kinds are open strings. An extension such as `org.example/volume-plot` includes `title`, fallback `text`, and its own `data`. Consumers with no matching renderer show an unsupported-type notice and preserve the raw item.

## Resources

Every resource has a unique `id`, `media_type`, `size` in bytes, and a hexadecimal `sha256`. It has either `inline_base64`, or `href`:

- A relative path inside the bundle, conventionally `objects/<sha256>`.
- A `gs://` or `https://` URI. A link is not proof that bytes are present or verified.

Resources identify bytes; items explain their meaning. Several items may reference the same resource. File extensions and display names are optional; a hash and media type suffice. A manifest does not require a filename that reveals its original source path.

Optional `name` can suggest a logical relative filename, including subfolders for
compound reports with image/file links. A recipient may restore that layout or
provide equivalent link resolution. Treat it as an advisory name, never an absolute
sender path: reject traversal and collisions, keep writes inside the import root,
and verify bytes before use. A missing layout/renderer gets a visible fallback;
it must not make other artifacts unusable.

The reader is offline. It verifies local/inline bytes and never fetches links automatically. Unknown formats remain generic files. An unavailable resource does not remove other items. Duplicate IDs are ambiguous and must be reported; all resources sharing a duplicate ID are unavailable.

### Existing external objects

Large datasets, checkpoints, or artifacts already available in another bucket or
public storage remain references at their existing URI in `uris`, or as a `link`
item when a separate explanation is useful. They are not copied into the handover
bucket. One item-level representation is:

```json
{
  "id": "source-data",
  "kind": "link",
  "title": "Existing study dataset",
  "href": "gs://example-study/data/run-42.parquet",
  "text": "Required to reproduce the analysis; existing bucket access is needed. Not included in this bundle.",
  "data": {"media_type": "application/vnd.apache.parquet", "size": 30000000000, "generation": "1234567890"}
}
```

The URI and generation above are illustrative. Known `sha256`, object `generation`,
or version metadata may be included; omit unknown metadata rather than inventing
it or downloading a large object to hash it. A link does not count toward bundled
attachment sizes and does not claim verified bytes are present. Readers display
the reference and its access requirements without automatically fetching it.
An unversioned URI may change; say so. Never embed access tokens or signed URLs.
Small selected local artifacts can still be snapshotted as resources. Referencing
an unavailable external object must not prevent other context from being read.

## Compatibility and notices

- Readers preserve unknown fields at every level and expose import diagnostics with a stable code and JSON pointer.
- Unknown kinds and media types get a visible fallback; an optional renderer must not be necessary to read the summary.
- An unsupported major version gets best-effort generic display and an explicit notice. A writer must not rewrite a version whose semantics it cannot interpret.
- Malformed individual items remain raw JSON with notices. Invalid JSON, a non-object envelope, or the document/resource limits produce a controlled error instead of a crash.
- Checksums and sizes must match before attachment bytes are treated as verified. A mismatch makes that attachment unavailable. Checksums detect corruption; they do not authenticate a sender.
- Context is reference data. Import performs no commands, hooks, code execution, branch checkouts, credential installation, or access-policy changes. Viewers choose how to sandbox HTML and other active media.

Engine limits: 16 MiB JSON; 4,096 items and 4,096 resources; 32 MiB per attachment; 128 MiB declared attachments per bundle. Readers may have lower limits but must identify skipped content. Handoff output is bounded and announces truncation. Raw manifests remain available.

## Contributions and revisions

Published bundles are immutable snapshots. A contribution creates a new manifest and can name earlier immutable manifests in `parents`. Branches, URLs, and authors remain provenance, not merge commands. Conflicts stay explicit; this prototype does not perform collaborative merging.

Start additions under a reverse-domain namespace in `extensions` or a namespaced `kind`. Keep fallback text useful. To promote a common field, add examples and compatibility fixtures before changing the engine. Additive optional fields can use a minor version; incompatible meanings require a new major version. Readers must not download schemas or JSON-LD contexts to inspect a bundle.
