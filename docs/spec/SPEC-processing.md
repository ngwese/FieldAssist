# FieldAssist processing specification

**Document status:** Draft

### Revision history

| Revision | Date | Notes |
| --- | --- | --- |
| 1 | 2026-09-11 | Initial draft of the processing layer |

This is a future product layer. The as-built application already has
**monitor chains** (playback listen DSP) and **File → Render** (one-shot
export of the active composition). Processing does not replace either of
those.

Related:

- [SPEC-application.md](SPEC-application.md) — as-built session, composition,
  monitor, and render
- [SPEC-workflows.md](SPEC-workflows.md) — Lua review/ingest (how files enter
  and move through a session)

## Purpose

Reviewing field recordings is only half of the job. The other half is
applying a repeatable sequence of **corrective and creative operations**
(normalize, fades, gain, reverse, resample, metadata, …) to a file or to
named regions, validating that sequence by listening, then exporting many
items without doing the same work by hand.

That is the processing chain. It exists so the user can configure once, test
on a subset, and later batch-export.

## Relationship to what already ships

```
Source media  →  composition EDL (clip tree, markers, regions)
                      │
                      ├─ listen ──► monitor chain ──► output device
                      │
                      └─ process (future)
                              │
                              ├─ preview listen ──► monitor chain ──► output device
                              └─ export ──► encoder (WAV / FLAC / Ogg, …)
```

| Layer | Role | Today |
| --- | --- | --- |
| **Composition EDL** | Non-destructive timeline over source media | Shipping |
| **Monitor chain** | Map whatever you are hearing (channel count, M/S, FOA, …) onto the playback device | Shipping. Unchanged by this spec |
| **Processing chain** | Ordered operations on a file or region for preview and export | Future |
| **Render** | Encode the current composition (no chain, no monitor) | Shipping one-shot; batch export is future |

Rules:

- Source media files remain read-only. Processing never writes over them.
- Monitor parameters (`Output/Gain`, crossfeed, FOA yaw, …) stay listen-path
  controls.
  They are not processing-chain operations and are not burned into exports
  unless a future UI explicitly offers “render what I hear.”
- File → Render of the active composition stays valid without a chain.
- Plugin hosting is out of scope for this spec.

## Processing chain

A **processing chain** is an ordered list of operations applied to one
**target**: a whole composition or one named region (including overlapping
regions, each processed independently).

Operations run in list order. The user can reorder them (drag-and-drop).
Some operations set metadata; others are offline signal processing.

A chain can be:

- Bound explicitly to a target
- Left unbound, in which case the session **default chain** applies (if one
  is designated)
- Empty, meaning export is the composition (or region) as edited, same as
  today’s render of the timeline

### Built-in operations

The first shipping set should include at least:

| Operation | Kind |
| --- | --- |
| Fade in / fade out | Signal |
| Gain | Signal |
| Normalize | Signal |
| Invert phase | Signal |
| Reverse | Signal |
| Pitch shift | Signal |
| Resample | Signal (export-time rate may still be chosen on the encoder; this op is in-chain) |
| Metadata (name, tags, and similar fields applied to the export) | Metadata |

Parameters on an operation are editable. Changing a parameter re-applies the
chain to the current target for preview (see below).

Clip-level `fade_in` / `fade_out` already exist on the composition clip tree
and affect reads today, but they have no product UI. Chain fade operations
are the user-facing fades; they must not require rewriting source files.
Whether they compose with clip fades is an implementation choice that must
stay non-destructive and obvious in preview.

### Shelf and default

A session may keep a **shelf** of named chains. The user can save the
current chain to the shelf and recall it onto another target. One shelf
entry may be the **default** for that session: any target without an
explicit chain uses it.

Shelf state belongs on the session (`.fasession` or an adjacent file the
session references), not in a database. Compositions do not need to embed
the full chain if the session stores the binding; a composition that is
opened outside that session must still be exportable (unbound → no chain, or
a chain serialized on the `.facomp` if we choose to snapshot it). The
implementation must pick one rule and keep export reproducible.

## Preview

The user must be able to hear the chain on the **current target** without
committing an export.

- Parameter changes update the preview without blocking the rest of the UI.
- Selecting a different document or region previews that material through
  the same chain (or that target’s bound chain).
- Preview playback still uses the **monitor chain** so multi-channel and
  Ambisonic material remains listenable on a stereo (or other) device.
- Preview is not batch export and must not mutate source files or the EDL
  unless the user applies an edit operation separately.

Status-bar Preview (auto-play on document activate) is a transport convenience
and is unrelated to processing-chain preview, even though both are “preview”
in casual speech. Processing preview means “hear the chain.”

## Batch export

The user selects some or all session documents and/or named regions and
starts a batch. For each item the app applies that item’s chain (or the
captured default) and writes an export using encoder settings captured at
**start** (format, sample rate, bit depth, destination directory, naming).

Requirements carried from the original product intent:

- Work runs in the background; the UI stays usable (review, edit, adjust
  chains).
- The batch snapshots chain + export configuration at initiation. Later
  edits to the default chain or the shelf do not change in-flight items.
- Overlapping regions produce separate files.
- Failure on one item does not silently drop the rest; the user can see
  which items failed and why.
- Destination unavailable or read-only is a reported error, not a hang.
- Export still never overwrites source media unless the user has chosen that
  exact path (discouraged; default destinations are a distinct output
  location).

File → Render remains the interactive single-composition export. Batch is
the many-item path that uses processing chains.

## What this spec does not include

- Third-party plugins (VST, AU, CLAP, …)
- SQLite or a workspace directory with staged copies
- Replacing or folding monitor DSP into the processing chain
- Treating Faust monitor parameters as the chain’s gain/normalize
  operations

Monitor stays the listen path. Processing is the work-and-export path.
