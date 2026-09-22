# FieldAssist analysis specification

**Document status:** Draft

### Revision history

| Revision | Date | Notes |
| --- | --- | --- |
| 1 | 2026-09-14 | Initial draft of the analysis layer |
| 2 | 2026-09-14 | Pull-based streams; Envelope Peak and Transient ops ship |
| 3 | 2026-09-15 | Spectral stream + Spectrum waveform representation |
| 4 | 2026-09-15 | Progressive stream coverage; spectrum tile textures |
| 5 | 2026-09-15 | MinMaxOp; analysis ops split into per-file module |
| 6 | 2026-09-15 | Peaks + Spectrum combined representation |
| 7 | 2026-09-16 | Waveform representation is app-global |
| 8 | 2026-09-16 | Regional stream invalidation; op-declared recompute radius |
| 9 | 2026-09-16 | History jumps splice streams via EDL snapshots |
| 10 | 2026-09-17 | Shared multi-kind analysis pass (one planar read, many ops) |

Overview **minmax peaks**, **Envelope Peak** (5 ms attack / 300 ms release
`dasp_envelope` follower), **Mark → Transients** (pink Transient markers), and
**Spectrum** representation (pull-based spectral stream) are implemented.
Silence detection, correlation, and the Lua `c:analyze` surface remain future
work.

Related:

- [SPEC-application.md](SPEC-application.md) — as-built session, composition,
  waveform, markers, and regions
- [SPEC-processing.md](SPEC-processing.md) — future processing chains (transform
  for preview/export; not analysis)
- [SPEC-workflows.md](SPEC-workflows.md) — Lua review/ingest
- [SCRIPTING.md](../SCRIPTING.md) — Lua API reference (analysis methods land when
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
| **Analysis** | Gather extra information over the edited timeline | Minmax peaks (pull), Envelope Peak overlay, Transient markers, Spectral stream / Spectrum view; rest future |
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

When several kinds are needed at once (Peaks + Spectrum paint, or a timer
drain that coalesced queued `ensure_*` requests), the host runs a **shared
analysis pass**:

1. **Plan** — collect each kind’s needed ranges (dirty neighborhood, uncovered
   peak spans, selection target, or full timeline), expand each by that kind’s
   `RecomputeScope` radius/warmup, then **merge** into one non-overlapping
   target using the **maximum** radius across participating kinds
2. **For each pager-sized chunk** — `read_planar` **once**, then fan the same
   PCM into every participating op (`MinMaxOp`, `SpectralOp`, …)
3. **Finish** — flush ops, clear dirty holes, update progress

Single-kind jobs keep the prior per-kind steppers. Mid-pass new kinds are
**re-queued** for the next pass (the live plan is not mutated). Progress labels
combine when multiple kinds run together (for example
`building peaks + spectrum`).

Disk decode is expected to dominate cold builds; pass timings
(`AnalysisPassStats`, or `FIELDASSIST_ANALYSIS_TIMING=1`) report read vs
consume vs pager decode. If consume saturates a core after shared I/O lands, a
follow-up may partition channel/hop work over the filled planar buffer on a
worker pool — without parallelizing pager fills.

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

**Invalidation:** sample-changing edits splice hop-aligned composition streams
(envelope, spectral) through the edit and mark a **radius-padded dirty
neighborhood** that the next pull job rebuilds. Ops declare a
`RecomputeScope`: regional ops (Spectral, Envelope Peak, MinMax) keep valid
hops outside the hole; full-timeline ops drop their stream. Peaks remain
clip-local (split/reuse on the clip tree; no composition-stream splice).
Analysis-owned markers and regions are **not** auto-rewritten; the user (or a
script) re-runs the operation. Multi-edit history jumps splice streams through
each EDL step (using per-edit snapshots for intermediate lengths); they clear
only when an inverse cannot be expressed (for example undoing Trim) or the hop
buffer shape no longer matches the destination timeline.

### Regional recompute

Each built-in op exposes how far beyond an edited range it must rewrite for
continuity with a previous pass:

| Kind | Scope | Radius / warmup |
| --- | --- | --- |
| MinMax | Regional (clip caches) | 0 — non-overlapping fold |
| Spectral | Regional | `FFT − hop` (768 frames) lookback and output pad |
| Envelope Peak | Regional | ~300 ms release window at the composition rate |
| Transients | FullTimeline (no auto rewrite) | — |

Dirty seeds come from the edit’s landing/join ranges; the job reads
`warmup_frames` of lookback to prime op state, then rewrites only the padded
output spans. Unaligned hop splices reuse suffix bins with at most one hop of
phase error (same tradeoff as clip peak caches).

Stream ops advance coverage so the UI can **progressively consume** partial
results: first builds expose a shrinking dirty prefix; later edits leave holes
without resetting `covered_frames` to zero. Paint skips dirty hops (floor /
zero) while valid hops outside the hole stay painted. `ensure_*` keeps
requesting work until dirty ranges are empty.

## Built-in operations

The catalog is non-exhaustive. The first product set should include at least
the following as intent; not every row must ship in one release.

| Operation | Kind | Typical output |
| --- | --- | --- |
| Gather peaks | Realtime-capable; **pull when overview layer needs paint** | Per-channel `(min, max)` stream |
| Envelope Peak | Realtime-capable; Analyze → Envelope → Peak or View → Overlay → Envelope Peak | Per-channel smoothed peak stream (5 ms attack / 300 ms release) |
| Stereo correlation | Realtime-capable; needs ≥2 channels in scope | Shared stream |
| Transient detection | Realtime-capable; Analyze → Mark → Transients | Pink **Transient** markers |
| Silence | Often needs a floor estimate (multi-pass) | Named region collection |
| Speech / sound labeling | Offline; later | Labeled regions (no ML/plugin commitment in v1) |
| Spectral data | Realtime-capable, heavier | Per-channel log-band spectrogram; drives Spectrum **representation** |

Parameters are per operation (window, threshold, hop, output collection or
marker type). Changing a parameter re-runs that job for the current target.

**Gather peaks** starts when the waveform needs overview data (document activate
/ paint pull) and paints as soon as the first chunk has bins (`can_paint_overview`).
**Envelope Peak** and **Transients** run from the Analyze menu (or when the
envelope overlay is enabled and data is missing). **Spectral** runs when the
Spectrum or Peaks + Spectrum representation is selected and bins are missing
(full timeline, all channels — Selection Only does not apply). Peaks + Spectrum
coalesces MinMax and Spectral into one shared pass. After an edit, Spectral and
Envelope Peak rebuild only the radius-padded dirty neighborhood when a prior
stream exists (a shared pass uses the max radius across kinds).

Stream ops clear dirty ranges as pager blocks complete so the UI can
**progressively consume** partial results: paint gates use “has data” while
`ensure_*` keeps requesting work until dirty holes are gone.

### Spectral defaults

Full-timeline float STFTs at overview hop would be multi‑GB per hour, so the
shipping spectral stream is a compact spectrogram:

| Parameter | Value |
| --- | --- |
| Window / FFT | Hann, **N = 1024** |
| Hop | **256** (`PEAK_BLOCK`, time-aligned with peaks) |
| Bands | **64 log-spaced** magnitude bands from ~20 Hz to Nyquist (average linear FFT bins into each band) |
| Value | Magnitude → **dB**, clamped (−80…0 dBFS) as `f32` |
| Scope | Full timeline, all channels |
| Regional recompute | Continuity radius = `FFT − hop` (768); unaligned splice ≤1 hop |

Streams are not written to `.facomp`. Sample-changing edits splice them and
mark dirty neighborhoods rather than wiping the whole series.

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
| Envelope → Peak | Peak envelope stream (5 ms attack / 300 ms release) |
| Mark → Transients | Pink Transient markers |

### View: representation and overlays

**Waveform representation** (View menu):

| Mode | Behavior |
| --- | --- |
| **Peaks** (default) | As-built overview: peak bins; sample-accurate zoom still reads PCM. A left gutter labels the linear amplitude axis in dBFS (`-3` near full scale, `-∞` at the zero line). Hover reports dB |
| **Spectrum** | Lanes show a time × frequency heatmap from the spectral stream. Missing data triggers the Spectral job. Zoomed sample-accurate PCM paint is Peaks-only. The left gutter labels log Hz (`1k`-style); hover reports frequency |
| **Peaks + Spectrum** | Each lane is split vertically: peaks on top, spectrum below. A shared drag handle sets the peaks/spectrum height ratio for all lanes (default 20% / 80%). Missing data queues MinMax and Spectral together so one shared pass feeds both. The left gutter stacks both scales (shared across channels); hover follows the pane under the pointer |

**Overlays** (View → Overlay): independent toggles for extra streams (RMS,
correlation, and similar). Region and marker results use the existing lane
overlays and the Detail Regions / Markers panels.

**Representation** (Peaks / Spectrum / Peaks + Spectrum) is **app-global UI**
so switching compositions keeps the same waveform body. **Overlay** toggles
remain **per-document UI**. Neither is stored in `.facomp`; they may later
ride along with session `capture_ui` the same way dock visibility does.

Analysis does **not** add a fourth dock. Streams are visual unless a later UI
lists them; markers and regions appear where they already do.

### Progress

The status bar continues to show background job text (`building peaks` today).
Other analysis jobs use the same progress pattern and cancel semantics.

## Lua

Scripts **invoke** built-in analysis; they do not register custom analyzers or
consume PCM buffers from Lua (same plugin stance as processing). Full method
tables belong in [SCRIPTING.md](../SCRIPTING.md) when implemented. Product contract:

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
