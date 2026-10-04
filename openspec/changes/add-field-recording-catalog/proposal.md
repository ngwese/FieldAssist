# Proposal

## Why

Ingest and Review can stage and triage field recordings, but Catalog is still
a stub. Kept compositions cannot be batch-exported into a Sononym-visible
library with review metadata and stereo mixdowns for Ambisonic/M/S material.
That blocks finishing the field-recording pipeline.

## What Changes

- Add a stateful **Catalog** workflow (sheet UX, Ingest-like) that exports
  every session document with `group == "keep"`.
- Export the **composition edit** (regions, channel subset, child comps), not
  raw staging bytes. “Identity” means source-equivalent rate/format/quality
  via an export profile (default: `FLAC (Source Equivalent)`).
- Resolve library root from `${catalog.library_root}` with fallback to
  `user.catalog.library_root`; resolve default profile from
  `catalog.export_profile` (sheet pulldown can override).
- Mirror staging event directories under the library root; output basename =
  composition name; event directory from `source.event.name`.
- Add a multi-output **render-with-chain** API: one source read amortized
  across N outputs; optional shared DSP later; per-output DSP now for stereo
  mixdown via offline Faust monitor chains (`foa`, `foa_fuma`, `ms`) using
  fixed defaults, with optional DSP params on the API for future presets/UX.
- Emit a stereo sidecar when playback would use those chains; rename by
  replacing the layout **code** with `st` (`ambix`/`fuma`/`ms` → `st`).
  Mono, stereo, and 2OA stay identity-only (no mixdown).
- Map known bext + iXML leaves to stable FLAC Vorbis keys; write
  `user.artist` and `user.copyright` into the catalog format’s artist /
  copyright fields.
- **Preflight** the full output plan and **fail before Run** on existing
  destinations or colliding planned paths; during render re-check, skip
  offenders, and report them in the summary.
- After Run, a confirm page deletes **all** session-referenced files under
  staging (keep, drop, never-exported); paths outside staging are untouched.
  Cleanup is offered even after partial export failure, with a warning.
- Update the field-recording Catalog example to use the new APIs.

## Non-goals

- Sononym API integration or auto-registering a Library (user adds the folder).
- Image/notes association for Sononym (future).
- Full processing-chain product (normalize, fades, …) beyond FOA/M/S stereo
  mixdown and the multi-output render shape that can host shared DSP later.
- Baking Review’s live monitor knobs into Catalog mixdowns (fixed defaults;
  params only via render API).
- SQLite ingest/catalog ledger, workflow `finish({ next })` handoff, or
  C2PA signing.
- Changing the CPAL realtime callback / listen path (offline render only).

## Capabilities

### New Capabilities

- `catalog-workflow`: Catalog sheet, variables, preflight, batch export of
  `keep` compositions, summary, and staging cleanup confirm.
- `composition-render`: Multi-output render-with-chain (amortized source
  read, per-output DSP via offline Faust, optional DSP params).
- `export-tags`: Catalog tag write path — bext/iXML → stable Vorbis keys,
  plus `user.artist` / `user.copyright`.

### Modified Capabilities

- (none — `openspec/specs/` is empty; product docs in `docs/spec/` remain
  background until archive merges these deltas)

## Impact

- **Hosts:** FieldAssist (Catalog workflow UI/sheet); `field-batch` may call
  render APIs later but is not required for v1 UX. `field-play` unchanged.
- **Crates:** `field-audio-monitor` (offline Faust apply), `field-audio-io`
  / `field-composition` (encode + tags), `field-scripting` (Lua render +
  workflow surface), `field-assist` (sheet host, Catalog example wiring).
  Prefer leaf traits + app adapters; avoid new sibling-crate edges.
- **Realtime risk:** Offline render must not run on the CPAL callback.
  Reuse Faust DSP off the realtime path; listen-path monitor behavior stays
  unchanged.
- **Docs/examples:** `docs/examples/workflows/field-recording/workflow_catalog.lua`,
  related notes in `SPEC-field-recording.md` / `SCRIPTING.md` as follow-ons.
- **External:** Sononym consumes the library directory as a manual Library
  location ([Sononym docs](https://www.sononym.net/docs/manual/getting-started/)).
