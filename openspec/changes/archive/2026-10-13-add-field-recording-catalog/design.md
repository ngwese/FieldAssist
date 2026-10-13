# Design

## Context

See proposal.md for motivation. Today:

- Ingest example uses `field.media.begin_transcode` (identity encode; tags
  often empty). Review ships as an embedded workflow.
- Catalog example under `docs/examples/workflows/field-recording/` is a
  stub that logs intended exports.
- FOA/M/S → stereo exists only on the listen path (`field-audio-monitor`
  Faust chains via CPAL callback).
- `composition:export` encodes composition audio with tags but no DSP.
- Product background: `docs/spec/SPEC-field-recording.md`,
  `SPEC-processing.md`, `SPEC-metadata.md`.

Offline Catalog work must not touch the CPAL callback. Monitor listen
behavior stays as-is.

## Goals / Non-Goals

**Goals:**

- A reusable multi-output render API that Catalog (and later batch) can call.
- Offline reuse of existing Faust monitor chains for mixdown outputs.
- Catalog sheet that preflights paths, batches keep exports, summarizes,
  and confirms staging wipe.
- Tag path that actually writes bext/iXML-derived + user artist/copyright
  onto FLAC library files.

**Non-Goals:**

- Full processing-chain editor / normalize ops (only the render plan shape
  that can host a shared stage later).
- SQLite ledger, Sononym deep integration, image/notes.
- Sibling-crate dependency shortcuts that break the DAG.

## Decisions

### 1. New offline render in `field-composition` + monitor adapter

**Choice:** Add a multi-output render entry point that reads composition
planar audio once, optionally runs a shared DSP stage (noop in v1), then
runs per-output DSP → SRC/format → encode **in parallel** across outputs
(shared read still amortized). Async jobs expose frame-based progress
(`done`/`total` = composition frames). Put orchestration next to existing
`export_to_path` in `field-composition`. Offline Faust execution lives
behind a trait owned by the composition/export consumer (e.g.
`OfflineMonitorProcess` or reuse `MonitorProcess` with an offline driver)
implemented in `field-assist` / scripting host using `field-audio-monitor`
— **not** a new edge from `field-composition` → `field-audio-monitor` if
that would create a sibling or layering violation.

**Alternatives considered:**

- Extend `field.media.transcode` only — rejects composition edits /
  children; wrong unit of work.
- Burn “render what I hear” through the playback engine — couples Catalog
  to device/callback; violates realtime isolation.

### 2. Lua surface: `composition:render({ outputs = { ... } })`

**Choice:** Script-facing API on the composition/document that accepts a
plan:

```lua
doc:render({
  -- optional shared = { chain = "...", params = { ... } },  -- future
  outputs = {
    {
      path = dest_identity,
      profile = "FLAC (Source Equivalent)",
      -- no chain = identity encode of composition edit
    },
    {
      path = dest_stereo,
      profile = "FLAC (Source Equivalent)",
      chain = "foa",           -- or foa_fuma / ms
      params = { yaw = 0 },    -- optional; defaults if omitted
    },
  },
})
```

Async variant (`begin_render` + job poll) mirrors `begin_transcode` for
sheet `:defer` loops.

**Alternatives considered:** One Lua call per output — simpler but loses
amortized read; rejected for the stated API goal.

### 3. Mixdown trigger mirrors playback layout

**Choice:** Catalog plans a stereo sidecar when the composition’s effective
monitor chain is `foa`, `foa_fuma`, or `ms` (same outcome as
`detect_layout` / chosen layout). 2OA and mono/stereo → identity only.
Sidecar basename: replace layout **code** (`ambix`/`fuma`/`ms`) with `st`
in the composition name string used for the identity basename.

**Defaults:** Faust chain defaults; Catalog does not read live Review knobs.
Params only when the render plan supplies them.

### 4. Tags via existing export TagMap path

**Choice:** Build tags with the Export-site resolver + profile metadata,
ensuring `source.bwf` / `source.ixml` leaves map into Vorbis via a
documented stable key table (extend `build_tag_map` / FLAC writer as
needed). Force `ARTIST`/`COPYRIGHT` from `user.artist` /
`user.copyright` when non-empty. Prefer the composition export/render tag
path over `media.transcode`’s empty TagMap.

### 5. Catalog workflow as copyable example (+ optional embed later)

**Choice:** Implement Catalog as an updated example workflow
(`workflow_catalog.lua`) using sheet pages like Ingest, against the new
render/fs/confirm APIs. Embed only if host gaps are closed enough for a
built-in; otherwise ship example + SCRIPTING docs. Variables:
`catalog.library_root`, `catalog.export_profile`, staging from session
props / ingest.

### 6. Preflight as pure plan validation

**Choice:** Catalog Lua (or a small host helper) computes all destination
URLs, checks `field.fs.exists`, checks uniqueness in the plan, fails the
sheet before starting jobs. Render re-checks exists-or-collide before each
write; skip + collect into summary.

### 7. Cleanup confirm

**Choice:** FieldAssist `app:confirm` opens a native alert dialog. Catalog
puts the staging wipe in `on_confirm` (`ok = "Cleanup"`, `danger = true`)
because the dialog is async (sync return is `false` once shown). Headless /
tests still use the queued / default-true path. Delete only paths under
`staging_root` that appear as session document/media URLs.

## Risks / Trade-offs

- **[Risk] Offline Faust parity drift vs listen path** → Mitigation: call
  the same compiled DSP graphs; share default param tables; unit-test FOA
  and M/S offline against known fixtures.
- **[Risk] Large compositions × N encoders still memory-heavy** →
  Mitigation: stream block-wise through DSP into encode streams where
  encoders allow; document if first cut buffers planar like
  `export_to_path`.
- **[Risk] Layout code not present in composition name** → Mitigation:
  define fallback: if code token absent, append `-st` before extension
  rather than silently colliding with identity path; cover in Catalog
  helper tests.
- **[Risk] Confirm API missing blocks cleanup UX** → Mitigation: include
  confirm in this change’s tasks if not already shipped.
- **[Trade-off] Example vs embedded Catalog** → Example ships faster and
  matches Ingest; users copy into config. Embed can follow once stable.

## Migration Plan

- No migration of existing library files.
- Users set `user.catalog.library_root` and optional
  `user.catalog.export_profile`, add that folder as a Sononym Library.
- Copy updated `workflow_catalog.lua` (+ `shared.lua`) into the config
  directory when ready.
- Rollback: omit the workflow file; render API unused leaves prior export
  behavior intact.

## Open Questions

- Exact Vorbis key spellings for each bext/iXML leaf (implement against
  `CANONICAL_KEYS` + a small explicit table in code/docs; adjust if Sononym
  shows a preferred alias later).
- Whether Catalog v1 embeds in the binary or remains example-only after
  APIs land (default: example-first).
