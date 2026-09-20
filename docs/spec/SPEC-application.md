# FieldAssist application specification

**Document status:** Provisional

### Revision history

| Revision | Date | Notes |
| --- | --- | --- |
| 1 | 2026-09-11 | Initial as-built specification |

This document describes the shipping desktop application. Where an earlier
FieldAssist spec used different names or features, this file follows the
implementation.

Related:

- [SPEC-workflows.md](SPEC-workflows.md) — Lua scripts, workflows, and session
  grouping
- [SPEC-analysis.md](SPEC-analysis.md) — future analysis layer (streams,
  markers, regions). Overview peaks already ship; overlays and spectrum view
  are future
- [SPEC-processing.md](SPEC-processing.md) — future processing chains, preview,
  and batch export. Additive to monitor DSP; not a replacement.
- [SCRIPT.md](../SCRIPT.md) — Lua API reference
- [BUILDING.md](../BUILDING.md) — build and packaging

## Purpose

Field recordings arrive from phones, dedicated recorders, and other capture
devices. They may be mono, stereo, mid-side, or Ambisonic; they may use awkward
file names, sample rates, or containers; only parts of a take may be useful.

FieldAssist is a desktop editor for **reviewing, marking, editing, and
exporting** that material without modifying the source files. Work is organized
in a **session** that can be saved and reopened so review can span multiple
sittings.

Named **processing chains** and background batch export are a future layer,
specified in [SPEC-processing.md](SPEC-processing.md). They sit **in addition
to** the as-built **monitor chain** (playback downmix / listen DSP) and the
current File → Render of the active composition. **Analysis** beyond overview
peaks (overlays, spectrum view, silence/transients, …) is specified in
[SPEC-analysis.md](SPEC-analysis.md). Lua workflows (especially Review) cover
incremental pass-through of a set of files.

## Terminology

| This document | Earlier spec | Meaning in the as-built app |
| --- | --- | --- |
| **Session** (`.fasession`) | Workspace | The open set of media and composition documents, recorded composition ids, workflow binding, grouping, optional UI layout, and a `media` descriptor array |
| **Composition** (`.facomp`) | File + EDL | A non-destructive timeline over referenced media (`media:<blake3>` ids), plus markers and named region collections |
| **Document** | Collection item | A session row (`doc:` id) whose target is either media (audio URL + recorded `comp:` id) or a `.facomp` (project URL + recorded `comp:` id) |
| **Media** | Shared pool | Content-addressed by basename + audio/file stats (not mtime); session and compositions intern into a shared `MediaStore` |
| **Explorer** | Collection list | Left dock listing session documents, grouped by `group` |
| **Monitor chain** | Panner | Faust DSP that maps composition channels to the playback device. Shipping now; remains when processing lands |
| **Processing chain** | Processing chain | Future ordered DSP/metadata applied for preview and export. Does not replace the monitor |
| **Render** | (partial) Batch / export | As-built: one-shot encode of the active composition. Future batch is in SPEC-processing |
| **Workflow** | (not in that spec) | Lua-declared ingest or review procedure; see [SPEC-workflows.md](SPEC-workflows.md) |

Internal UI type `WorkspacePanel` is the center editor tab. It is not a
user-facing “workspace”.

## Product surface

FieldAssist is a single-window GPUI desktop app for macOS, Windows, and Linux.

Launch:

```text
FieldAssist [FILE]
  --list-devices
  --dump-init
  --output <name|index>
```

`FILE` may be audio, a `.facomp` composition, or a `.fasession`. A session path
replaces the empty launch session. Other paths open as a document in a new
session. `--output` selects the cpal playback device (name, index, or substring)
the same way Lua `app.output_device` does.

**Decode (open):** WAV, FLAC, MP3, OGG, M4A, and other formats enabled by
Symphonia.

**Encode (render):** WAV, FLAC, and Ogg Vorbis only.

Default window is 1280×760, dark theme. macOS uses the system application menu;
Windows and Linux put menus in the title bar and Quit on File. Settings exists
in the menu and is disabled.

FieldAssist remains a single-editor-window product. On macOS, closing that
window leaves the process alive (`QuitMode::Default`). Dock reopen and
File → Open recreate the editor with a **new empty session**, then load any
chosen paths (audio / `.facomp` / `.fasession`) into it. About is an
independent utility window (not parented to the editor). Quit with no editor
window exits immediately. With no editor window, only the application menu
(About / Quit) and File → Open stay enabled; other menu items are disabled.
Windows and Linux still quit when the last window closes.

## Session

There is always one UI-active session. It starts empty (untitled). Documents
are added by File → Open, the CLI, OS open-file events, Lua `session:open`, or
drag-drop **workflows** (not File → Open’s type rules). Opening a `.fasession`
from File → Open, the CLI, or `session:open` **replaces** the active session
after unsaved compositions (and, if needed, the saved session itself) are
handled.

A session holds:

- A stable id (`session:<uuid>`)
- Optional path of the `.fasession` file
- Ordered named `groups` registry (may include empty sections; order is the
  explorer section order)
- Ordered documents, each with a `doc:<uuid>` row id
- A `media` array of descriptors for open media-backed documents
- Which document is active
- Optional bound workflow name
- String→string `properties` (session-level)
- `capture_ui` (default true): when saving, persist window size and which docks
  are open
- Dirty flag

Each **document** is a tagged target:

- **Media**: `media_id`, recorded `comp:<uuid>`, and optional `name` (defaults to
  the media basename). The source URL lives only on the matching entry in the
  session `media` array — documents do not repeat `url`.
- **Composition**: `.facomp` project `url`, recorded `comp:<uuid>`, and optional
  `name` (defaults to the composition file basename)
- Whether its editor tab is open and pinned
- Optional `group` (explorer section; Review uses `todo` / `reviewed` / `drop`)
- Optional `state` (persisted; unused by built-in Review)
- String→string `properties` (not written into `.facomp`)

The document id (`doc:`) is the session-row / tab / explorer identity. It is
not the composition id. Reloading a media document applies the **recorded**
`comp:` id so break-out children that stored `parent` still lineage correctly.

Closing a **tab** hides the editor and clears pin; the document stays in the
session. Closing a **document** (File → Close or explorer Close) removes it
from the session after a modified-composition prompt. Nested `.fasession`
documents are rejected.

**Close Session** (File → Close Session / `file.close_session`) prompts for
modified compositions and a dirty saved session (same rules as quit / open
session), then replaces the window with a new untitled empty session and an
empty shared media pool.

**Tabs:** a newly opened document is a transient (unpinned) tab. Opening another
file while a transient tab exists can replace that tab. Pinning keeps the tab.
explorer “open tab” opens and pins.

**Dirty / save prompts:** a composition is modified when its edit cursor,
markers, named region collections, or in-memory display title differ from the
last save. Channel layout, monitor chain, and playback-channel subset alone do
not dirty a composition. An untitled session does not prompt to save even if
its membership changed; a session prompts only when it is dirty **and** already
has a path.

Save Session writes JSON (`kind: fasession`, `format_version: 2`) atomically
(temp file then rename). Document URLs are stored relative to the session file
when possible. The ordered `groups` list is persisted (including empty groups).
Save fails if any document has no file URL. Older `format_version: 1` files are
rejected (no path-only migration). Suggested file name is
`{workflow}.fasession` when a workflow is bound, otherwise `session.fasession`.

## Composition

A composition is a clip-tree timeline over media addressed by `media:<blake3>`
ids. The identity hash covers basename (no parent dir) plus sample rate,
channel count, frame count, bits per sample, size, container, and codec.
**`modified` is not hashed** so a filesystem touch does not mint a new id;
freshness (including mtime) is checked separately and may warn on open while
keeping the recorded media id. Source PCM is never stored in `.facomp`.
Timeline edits change the EDL and clip tree only.

`.facomp` JSON (`kind: facomp`, `format_version: 7`) includes a stable
composition `id` (`comp:<uuid>`), optional `parent` (`comp:<uuid>` when
broken out), sample rate, channel count, descriptors for media **used** by
this composition, the initial clip tree (`media:` ids), edit ops and cursor,
optional `undo_floor` (break-out founding Trim), markers, marker types, named
collections, optional `channel_layout`, `monitor_chain`, and
`playback_channels`. Older `format_version` values (≤6) are rejected. **Save
As** mints a new `id` while keeping `parent`.

Open compositions in a window share one `MediaStore` (pool + block pager).
Intern-by-hash deduplicates matching media across documents. Break-out
children share the parent’s store Arc.

Parent→child lineage lives on the composition, not the session. Any session
that has both files open rebuilds the tree by matching `parent` to an open
`id`. A child opened alone displays as a root until its parent is added.
Opening a child into an already-open session nests it under its parent in
the explorer when that parent is present; restoring a `.fasession` keeps
each document’s saved group and order (a detached child stays in its
group).

**Break Out to Composition** (Edit menu / `edit.break_out`) extracts each
selected span into a new child composition that shares the parent’s media
store. The child EDL is the parent’s reconstruction ops plus a founding
`Trim`; Undo cannot go past that Trim. Each child gets a default display
title of `N-` + parent name (or `N.M-` when breaking out from a child whose
name already matches that pattern), with `N` / `M` unique among open
siblings. Saving the child writes a standalone `.facomp` that still
references the media it uses.

On open, if probed media disagrees with the recorded descriptor (identity
stats or mtime-only), the app warns (Messages + dialog) and still opens,
keeping the recorded `media:` id. Decoded blocks may spill under the process
temp directory (`FieldAssist/blocks/…`); that cache is not the source file.

**Channel layout** is a named Lua layout (see [SPEC-workflows.md](SPEC-workflows.md)).
It labels waveform lanes and can default a monitor chain. `playback_channels` is
an optional 0-based subset (and order) of composition channels fed to the
monitor; `nil` means all channels in file order.

## User interface

```
Title bar (app mark on Windows/Linux, menus, header readout)
  Explorer | playhead / hover / selection | Script | Detail
Dock:
  Left   Explorer (session documents)
  Center Waveform + transport, or empty pane
  Right  Detail: Markers, Regions, Edits, Monitor
  Bottom Script: REPL, Messages (created when first shown)
Workflow bar (when a stateful workflow has toolbar items)
Status bar (file stats, layout picker, monitor, preview, progress)
Render sheet (modal overlay)
```

Drag-drop onto the center waveform or empty pane shows the workflow overlay
described in [SPEC-workflows.md](SPEC-workflows.md). File → Open does not use
that overlay.

### Explorer

Lists every session document. Ungrouped documents appear under **session**;
named entries from the session `groups` registry become sections (including
empty groups), in registry order. Group headers can be renamed, added, deleted,
or drag-reordered; order is saved in the `.fasession`. Within a section,
documents that sit in a contiguous block under an open parent are shown nested
(indent by attached depth). A child moved out of that block stays linked but
is not indented; a link icon is the child cue (click selects the parent;
Shift+click reattaches). Documents can be activated, opened as a pinned tab,
closed, renamed in place (Shift+Return; in-memory title that dirties until
save), regrouped, or reordered (including drag within or between sections,
with a horizontal insertion marker). Modified compositions are marked. Context
menu: Reveal Parent when a parent is open; group headers offer Rename / Add
Group / Delete Group. An anchored Info tool shows the selected composition’s
path, the open parent’s name when linked, and a scrolling Media table (path,
sample rate, channels, length, size) for media in that composition’s pool.

### Header and status

The header shows playhead, hover sample, and selection. The status bar shows
Preview; sample rate, bit depth, channel count, duration, and source size;
muted error and warning counts (one control that toggles Messages) to the
right of that file metadata; a layout picker and Monitor control on the far
right; and background job text (opening, building peaks, rendering).

### Script dock

The Script panel is a Lua REPL. `print` goes to the transcript. `app:info`,
`app:warn`, and `app:error` go to Messages (and colored stdout when attached).
The Messages tab has no badge; totals appear on the status bar. The Media tab
lists every entry in the window-shared media pool (descriptor fields plus
resolved path). Pool-only entries added via Lua are shown there but are not
persisted in `.fasession`. See [SCRIPT.md](../SCRIPT.md).

## Review editor

### Waveform

One lane per channel, labeled from the effective layout. Overview paint uses
peak bins (256 samples per bin) built on a background thread with progress.
That peak gather is the first shipping instance of analysis (pull-based when
the waveform needs overview bins). Analyze → Envelope → Peak and Detect
Transients, plus View → Show Envelope Peak, are also shipping; see
[SPEC-analysis.md](SPEC-analysis.md). Sample-accurate zoom reads PCM through a
block pager. Until peaks are ready, overview does not fold PCM on the UI
thread.

Zoom factor 1.25 (minimum 1/50 sample per pixel). Fit-all and Frame (selection
or caret) are view commands. Horizontal pan uses drag, a scrollbar, or
Shift+wheel. **Follow Playhead** (View menu, default on) keeps the playhead
centered in the waveform while playing or after transport seeks, whenever the
viewport does not already show the full buffer. Toggle it off to pan freely
during playback.

### Selection

Click-drag replaces the session **selection** collection. Secondary-modifier
drag (Cmd on macOS, Ctrl elsewhere) adds a disjoint region. Shift-drag or
shift-click extends the nearest endpoint. Alt scopes the gesture to the
lane’s channel. Click without drag sets the caret; secondary-modifier click
keeps the current selection.

The selection collection is session state and is not saved in `.facomp`. Named
collections persist on the composition; the Regions panel can adopt a named
collection into the selection. Regions may overlap. Region bounds in the UI
are inclusive sample indices.

Snap: optional zero-crossing snap (context menu) and optional snap-to-marker
(Selection menu, per type). Marker snap uses a fraction of the visible
timeline as latch radius.

### Markers

Built-in types: Blue (default), Yellow, Purple, Transient (pink; used by
Analyze → Mark → Transients). Custom types (name + color) live on the
composition. A marker is a type instance at a frame, with an optional note.
Color is on the type. At most one marker of a given type may occupy a frame;
different types may share a frame.

Add at Hover (Selection menu) uses the sample under the pointer when set;
otherwise the caret. Delete removes the marker at the target. Types can be
created from the menu. Removing a type deletes its markers.

### Timeline edits

Edits apply to every region in the current selection, matching the Edit menu.
They are recorded on the composition EDL with undo/redo.

| Command | Effect |
| --- | --- |
| Cut | Copy then remove (timeline shrinks) |
| Copy | Clipboard of clip-tree slices |
| Paste | At caret, or replace the first selected span; sample rate and channel count must match |
| Clear (Backspace) | Silence of the same length |
| Remove (Delete) | Close the gap |
| Duplicate | Insert a copy after the range |
| Trim | Keep selected spans concatenated; discard the rest |
| Break Out to Composition | Each selected span becomes a named child composition sharing media |

The Edits panel lists EDL steps and can jump the cursor. The clip tree may
also represent move and roll; those are not first-class menu commands.

## Playback

Playback uses cpal through a `PlaybackSession`. Transport states are playing,
paused, and stopped.

- Play/Pause (Space) toggles playback
- Stop returns to stopped
- Home / End go to timeline start / end
- Start plays from the in-point (selection start when a selection exists)
- Previous / Next jump among **anchors**: marker frames and selection/named
  region bounds. While playing, Previous skips an anchor that is only a short
  distance behind the playhead so repeated presses walk backward
- Loop uses the selection’s bounding span as in/out when a selection exists
- Preview (status bar): when enabled, activating a document starts playback
  from sample 0

The playhead is the document caret. Output device is the host default unless
`--output` or `app.output_device` selects another. Device choice is not stored
on `.facomp`; pin it from `init.lua` if it must survive relaunch.

### Monitor chains

Monitor DSP is **playback only**: it answers “how do I listen to this
recording on this device?” It is not applied on render. A future processing
chain ([SPEC-processing.md](SPEC-processing.md)) is a separate stage for
corrective or creative work and export; both can exist at once (processed
audio still passes through the monitor when you listen).

Chains are fixed Faust graphs compiled into the binary:

| Id | Label | Inputs → outputs |
| --- | --- | --- |
| `mono` | Mono | 1 → 2 |
| `stereo` | Stereo | 2 → 2 |
| `ms` | M/S | 2 → 2 |
| `foa` | B-Format (AmbiX) | 4 → 2 |
| `foa_fuma` | B-Format (FuMa) | 4 → 2 |

No chain means 1:1 mapping onto the output device. Parameters (`Output/Gain`,
crossfeed, M/S trims, FOA yaw/orientation, and so on) live in the Monitor
detail tab, grouped by Faust label prefix. Input and output RMS meters appear
as VU strips above the Output section. A composition may **pin** its parameter
snapshot; otherwise working parameters are session defaults. Switching chain
crossfades on the order of 20 ms.

Default chain for a layout is assigned from Lua `define_layout` / `detect_layout`.
Detecting a layout fills `monitor_chain` only when it is still unset. An
explicit Monitor-tab or script assignment is saved on the `.facomp` and wins
on reload. Changing layout does not clear a custom `playback_channels` subset.

Second-order Ambisonics (`2OA`) is a labeling layout only; it has no monitor
chain.

## Render

File → Render opens a sheet: encoder, PCM format (when the encoder stores
PCM), sample-rate preset, per-channel checkboxes, directory, and filename.
The job runs off the UI thread:

1. Read the composition as planar float (clip-tree / EDL; **no** monitor DSP)
2. Keep the selected channels
3. Resample with rubato FFT if the destination rate differs
4. Encode to the chosen path (`File::create`; that path is overwritten)

| Encoder | Extension | Notes |
| --- | --- | --- |
| `wav` | `.wav` | Integer and float PCM |
| `flac` | `.flac` | Integer PCM, 1–8 channels |
| `ogg` | `.ogg` | Vorbis; UI PCM format is n/a; mono is written dual-mono |

Defaults: WAV, source bit depth snapped toward S24, composition sample rate,
all channels, `{display_name}.{ext}` in the suggested save directory. Rate
presets include 22.05 kHz through 192 kHz.

Render is a one-shot export of the edited composition. It does not walk
session groups or named regions, and it does not run a processing chain.
Those belong to the future processing layer.

## Commands

Menus, keymap, and Lua `app:command` share these ids:

**File:** `file.open`, `file.save`, `file.save_as`, `file.save_session`,
`file.save_session_as`, `file.close_session`, `file.close`, `file.render`,
`file.quit`

**Help:** `help.about`

**View:** `view.fit_all`, `view.frame`, `view.zoom_in`, `view.zoom_out`, and
show / hide / toggle for `explorer`, `detail`, and `script`. Show and hide are
idempotent; menus and status-bar pane buttons use the toggles.

**Transport:** `transport.home`, `transport.previous`, `transport.start`,
`transport.play_pause`, `transport.stop`, `transport.next`, `transport.end`,
`transport.loop`, `transport.preview`

**Edit:** `edit.undo`, `edit.redo`, `edit.cut`, `edit.copy`, `edit.paste`,
`edit.clear`, `edit.remove`, `edit.duplicate`, `edit.trim`, `edit.break_out`

**Selection:** `selection.select_all`, `selection.select_none`,
`selection.invert`, `selection.marker_type_blue`,
`selection.marker_type_yellow`, `selection.marker_type_purple`,
`selection.snap_to_marker`, `selection.add_at_hover`,
`selection.add_marker`, `selection.delete_marker`

Letter keys are scoped to the waveform so they do not steal Script/REPL typing.

## Non-destructive rule

Source media files are never rewritten by edits, save, peak build, or
playback. Save writes `.facomp` / `.fasession` JSON. Render writes a new
file at the path the user chose. Temp spill is under the OS temp directory.

## Out of scope for the as-built application

The following appeared in earlier FieldAssist specs and are **not** in this
application. Do not treat them as missing bugs of the current build:

- SQLite (or any database) project store
- Staging copies of source files into a workspace directory
- Third-party plugin hosting (VST, AU, CLAP, …)
- Named processing-chain shelf, default chain, interactive chain preview
- Background batch over many files or regions with captured chain state
- Analyze menu, waveform spectrum representation, and analysis stream
  overlays (RMS, correlation, …) beyond overview peaks and Envelope Peak

Those processing items and remaining analysis catalog entries are **future
additions**, not missing bugs of the current build. Overview peaks, Envelope
Peak overlay, and Transient detection are shipping; see
[SPEC-analysis.md](SPEC-analysis.md). Incremental review of many files is in
[SPEC-workflows.md](SPEC-workflows.md) (built-in Review), not a collection
`review_status` enum.
