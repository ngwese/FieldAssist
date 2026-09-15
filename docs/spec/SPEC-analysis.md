# FieldAssist analysis specification

**Document status:** Draft

### Revision history

| Revision | Date | Notes |
| --- | --- | --- |
| 1 | 2026-09-14 | Initial draft of the analysis layer |
| 2 | 2026-09-14 | Pull-based streams; Envelope Peak and Transient ops ship |

Overview **minmax peaks**, **Envelope Peak** (300 ms `dasp_envelope` follower),
and **Mark → Transients** (pink Transient markers) are implemented. Spectrum
representation, silence detection, correlation, and the Lua `c:analyze` surface
remain future work.

Related:

- [SPEC-application.md](SPEC-application.md) — as-built session, composition,
  waveform, markers, and regions
- [SPEC-processing.md](SPEC-processing.md) — future processing chains (transform
  for preview/export; not analysis)
- [SPEC-workflows.md](SPEC-workflows.md) — Lua review/ingest
- [SCRIPT.md](../SCRIPT.md) — Lua API reference (analysis methods land when
  implemented)

## Purpose

Reviewing field recordings benefits from **extra information** computed over
the edited timeline: peak envelopes for overview paint, silence or transient
regions for selection, spectral data for alternate views, and similar results.

That is the analysis layer. It exists so the user (and scripts) can gather
data that later guides selection, playhead movement, editing, and rendering —
without rewriting source media or folding listen DSP into a separate path.

## Relationship to what already ships

```
Source media  →  composition EDL (clip tree, markers, regions)
                      │
                      ├─ listen ──► monitor chain ──► output device
                      │
                      ├─ analyze (peaks today; more later)
                      │         └──► streams / markers / regions
                      │
                      └─ process (future)
                              │
                              ├─ preview listen ──► monitor chain ──► output device
                              └─ export ──► encoder (WAV / FLAC / Ogg, …)
```

| Layer | Role | Today |
| --- | --- | --- |
| **Composition EDL** | Non-destructive timeline over source media | Shipping |
| **Monitor chain** | Map whatever you are hearing onto the playback device; live Input/Output RMS | Shipping. Unchanged by this spec |
| **Analysis** | Gather extra information over the edited timeline | Minmax peaks (pull), Envelope Peak overlay, Transient markers; rest future |
| **Processing chain** | Ordered operations for preview and export | Future ([SPEC-processing.md](SPEC-processing.md)) |
| **Render** | Encode the current composition | Shipping one-shot |

Rules:

- Source media files remain read-only. Analysis never writes over them.
- Analysis reads the **edited** composition (clip tree / EDL), the same planar
  path render uses — not raw source files alone, and not monitor output.
- Monitor parameters and live VU meters stay on the listen path. They are not
  analysis operations.
- Processing transforms samples for preview/export. Analysis does not replace
  processing and does not rewrite PCM on the timeline.
- Plugin hosting and custom Lua DSP analyzers are out of scope for this spec.

## Analysis operation

An **analysis operation** is a named, parameterized job that:

1. Targets a **whole composition** or the union of regions in a **named
   collection** (including `"selection"`)
2. Applies a **channel scope** (`ChannelScope`: all channels, or a 0-based
   subset)
3. Consumes planar `f32` samples from the composition
4. Produces one or more of: **streams**, **markers**, or **regions**

Unlike a processing chain (which applies independently to each named region),
analysis walks the target collection **as a set**: overlapping regions are
one combined input range for the job, not separate export items.

### Target and scope

| Input | Rule |
| --- | --- |
| **Menu invoke** | If the session selection is non-empty, analyze its regions; otherwise the whole composition. Channel scope follows the selection’s lanes when the selection is channel-scoped; otherwise all channels |
| **Lua invoke** | May name any collection (`"selection"` or a persisted name) and any channel scope; omit collection to mean the whole composition |
| **Empty selection (menu)** | Whole composition, all channels |

Cross-channel operations (for example stereo correlation) still take a channel
scope: they consume those channels **together** and may emit one shared stream
rather than one series per lane.

### Incremental consume / produce

Analysis generalizes the already-shipping peak loop
(`Composition::build_next_peak_block`): read a planar block, fold or transform
it, append a typed output block, and let the UI paint while later blocks
decode.

For each pass of an operation:

1. **Begin pass** — allocate scratch, reset per-pass state
2. **For each planar block** in the target (block size chosen by the op; peaks
   fold at `PEAK_BLOCK` after a pager-sized read) — **consume** the input
   samples and **emit** a typed output block of **independent size** (including
   empty)
3. **End pass / finish** — flush remaining markers, regions, or summaries

| Kind | Passes | Incremental UI |
| --- | --- | --- |
| **Realtime-capable** | One; bounded work per block | Results appear as blocks complete (peaks today) |
| **Offline / multi-pass** | Two or more (for example noise-floor scan, then event detect) | Progress per pass; discrete results at the end of a later pass |

**Realtime-capable does not mean the CPAL output callback.** Jobs run on a
background thread with `ProgressHandle`, the same pattern as peak build. Enabled
stream layers **pull** analysis when paint (or Activate) needs data: if the layer
is on and bins are missing, the waveform requests an eager background job.
Live Input/Output RMS on the Monitor tab remain monitor DSP. Analysis must not
allocate or take blocking locks on the device callback.

### Outputs

An operation may produce any mix of the following:

| Kind | Description | Persistence |
| --- | --- | --- |
| **Streams** | Dense, time-aligned, typed series (per-channel or shared). Hop / bin size is chosen by the op | **Not** written to `.facomp`. Recompute when missing, cancelled, or invalidated |
| **Markers** | Composition markers on a dedicated type (created if needed). Re-run **replaces** markers of that type inside the target range | Persisted with the composition (existing marker model) |
| **Regions** | Written to a named collection (default from the op, overridable). Re-run **replaces** that collection’s regions | Named collections persist on `.facomp`; `"selection"` stays session-only |

Peaks today live on in-memory `ClipCache` and split with clips. That is the
shipping **stream cache** for overview paint, not a second on-disk format.

**Invalidation:** timeline edits that change samples in an analyzed range drop
derived streams (rebuild peaks as today). Analysis-owned markers and regions
are **not** auto-rewritten; the user (or a script) re-runs the operation.

## Built-in operations

The catalog is non-exhaustive. The first product set should include at least
the following as intent; not every row must ship in one release.

| Operation | Kind | Typical output |
| --- | --- | --- |
| Gather peaks | Realtime-capable; **pull when overview layer needs paint** | Per-channel `(min, max)` stream |
| Envelope Peak | Realtime-capable; Analyze → Envelope → Peak or View overlay | Per-channel smoothed peak stream (300 ms) |
| Stereo correlation | Realtime-capable; needs ≥2 channels in scope | Shared stream |
| Transient detection | Realtime-capable; Analyze → Mark → Transients | Pink **Transient** markers |
| Silence | Often needs a floor estimate (multi-pass) | Named region collection |
| Speech / sound labeling | Offline; later | Labeled regions (no ML/plugin commitment in v1) |
| Spectral data | Realtime-capable, heavier | Per-channel spectra; drives spectrum **representation** |

Parameters are per operation (window, threshold, hop, output collection or
marker type). Changing a parameter re-runs that job for the current target.

**Gather peaks** starts when the waveform needs overview data (document activate
/ paint pull). **Envelope Peak** and **Transients** run from the Analyze menu
(or when the envelope overlay is enabled and data is missing).

## User interface

### Analyze menu

A new **Analyze** menu runs operations on the active composition. **Selection
Only** (toggle at the top of the menu) limits Envelope Peak and Mark →
Transients to the current selection; overview minmax peaks always cover the
full timeline. Default channel scope follows the menu rules above. Each item
corresponds to a built-in operation (and later, parameters via a sheet or
submenu where needed).

| Item | Behavior |
| --- | --- |
| Selection Only | When checked, scoped ops use selection spans only |
| Envelope → Peak | Peak envelope stream (300 ms) |
| Mark → Transients | Pink Transient markers |

### View: representation and overlays

**Waveform representation** (View menu):

| Mode | Behavior |
| --- | --- |
| **Peaks** (default) | As-built overview: peak bins; sample-accurate zoom still reads PCM |
| **Spectrum** | Lanes show spectral analysis. Missing spectral streams trigger that analysis job |

**Overlays** (View menu): independent toggles for extra streams (RMS,
correlation, and similar). Region and marker results use the existing lane
overlays and the Detail Regions / Markers panels.

Overlay and representation state are **per-document UI**, not fields in
`.facomp`. They may later ride along with session `capture_ui` the same way
dock visibility does.

Analysis does **not** add a fourth dock. Streams are visual unless a later UI
lists them; markers and regions appear where they already do.

### Progress

The status bar continues to show background job text (`building peaks` today).
Other analysis jobs use the same progress pattern and cancel semantics.

## Lua

Scripts **invoke** built-in analysis; they do not register custom analyzers or
consume PCM buffers from Lua (same plugin stance as processing). Full method
tables belong in [SCRIPT.md](../SCRIPT.md) when implemented. Product contract:

```lua
c:analyze("peaks")
c:analyze("silence", {
  collection = "selection",  -- target; omit = whole composition
  channels = {0, 1},         -- omit / "all"
  output = "silence",        -- region collection to replace
})
c:analyze("transients", { threshold = 0.4, type = "Transient" })
```

View and overlay switches use `app:command` ids shared with menus, for example:

- `view.waveform_peaks` / `view.waveform_spectrum`
- `view.overlay_rms` (and similar per overlay)

An optional host hook `analyzed(composition, name)` may be added later; it is
not required for the first implementation.

Workflows may later call `analyze` the same way they may later assign
processing chains ([SPEC-workflows.md](SPEC-workflows.md)). Nothing in the
current host does that.

## What this spec does not include

- Third-party plugins (VST, AU, CLAP, …) or Faust graphs as analyzers
- Lua that registers custom DSP or reads planar PCM buffers for analysis
- Running analysis on the CPAL device callback
- Persisting dense streams in `.facomp` or a sidecar database
- Auto-running every operation on open (peaks remain pull-on-need; Envelope /
  Transients are menu- or overlay-driven)
- Folding monitor VU into analysis, or analysis into the processing chain
- Learned speech / sound models as a shipping requirement (listed only as a
  future example)

Monitor stays the listen path. Processing stays the work-and-export path.
Analysis is the gather-information path.
