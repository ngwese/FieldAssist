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
current File → Render of the active composition. Lua workflows (especially
Review) cover incremental pass-through of a set of files.

## Terminology

| This document | Earlier spec | Meaning in the as-built app |
| --- | --- | --- |
| **Session** (`.fasession`) | Workspace | The open set of compositions, workflow binding, grouping, and optional UI layout |
| **Composition** (`.facomp`) | File + EDL | A non-destructive timeline over referenced media, plus markers and named region collections |
| **Document** | Collection item | A composition as it appears in the session (id, paths, tab, group, state, properties) |
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

## Session

There is always one UI-active session. It starts empty (untitled). Documents
are added by File → Open, the CLI, OS open-file events, Lua `session:open`, or
drag-drop **workflows** (not File → Open’s type rules). Opening a `.fasession`
from File → Open, the CLI, or `session:open` **replaces** the active session
after unsaved compositions (and, if needed, the saved session itself) are
handled.

A session holds:

- A stable UUID
- Optional path of the `.fasession` file
- Ordered named `groups` registry (may include empty sections; order is the
  explorer section order)
- Ordered documents, each with a UUID
- Which document is active
- Optional bound workflow name
- String→string `properties` (session-level)
- `capture_ui` (default true): when saving, persist window size and which docks
  are open
- Dirty flag

Each **document** records:

- `source_path` for audio, or `project_path` for `.facomp` (the file URL used
  on save is project path if set, otherwise source path)
- Whether its editor tab is open and pinned
- Optional `group` (explorer section; Review uses `todo` / `reviewed` / `drop`)
- Optional `state` (persisted; unused by built-in Review)
- String→string `properties` (not written into `.facomp`)

Closing a **tab** hides the editor and clears pin; the document stays in the
session. Closing a **document** (File → Close or explorer Close) removes it
from the session after a modified-composition prompt. Nested `.fasession`
documents are rejected.

**Tabs:** a newly opened document is a transient (unpinned) tab. Opening another
file while a transient tab exists can replace that tab. Pinning keeps the tab.
Explorer “open tab” opens and pins.

**Dirty / save prompts:** a composition is modified when its edit cursor,
markers, named region collections, or in-memory display title differ from the
last save. Channel layout, monitor chain, and playback-channel subset alone do
not dirty a composition. An untitled session does not prompt to save even if
its membership changed; a session prompts only when it is dirty **and** already
has a path.

Save Session writes JSON (`kind: fasession`, `format_version: 1`) atomically
(temp file then rename). Document URLs are stored relative to the session file
when possible. The ordered `groups` list is persisted (including empty groups).
Save fails if any document has no file URL. Suggested file name is
`{workflow}.fasession` when a workflow is bound, otherwise `session.fasession`.

## Composition

A composition is a clip-tree timeline over a media pool. Source audio is
referenced by URL and file stats; PCM is never stored in `.facomp`. Timeline
edits change the EDL and clip tree only.

`.facomp` JSON (`kind: facomp`, `format_version: 6`) includes a stable
composition `id` (UUID), optional `parent` (parent composition UUID when
broken out), sample rate, channel count, media metadata, the initial clip
tree, edit ops and cursor, optional `undo_floor` (break-out founding Trim),
markers, marker types, named collections, optional `channel_layout`,
`monitor_chain`, and `playback_channels`. Legacy v1–5 files mint an `id` on
load and are marked dirty so the next save persists it. **Save As** mints a
new `id` while keeping `parent`.

Parent→child lineage lives on the composition, not the session. Any session
that has both files open rebuilds the tree by matching `parent` to an open
`id`. A child opened alone displays as a root until its parent is added.

**Break Out to Composition** (Edit menu / `edit.break_out`) extracts each
selected span into a new child composition that shares the parent’s media
pool and decode cache. The child EDL is the parent’s reconstruction ops plus
a founding `Trim`; Undo cannot go past that Trim. Saving the child writes a
standalone `.facomp` that still references the original media.

On open, if a media file’s size or mtime disagrees with the stored stats, the
app warns that source media changed and re-probes. Decoded blocks may spill
under the process temp directory (`FieldAssist/blocks/…`); that cache is not
the source file.

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
documents that share an open parent composition are shown as a nested tree
(indent by depth). Documents can be activated, opened as a pinned tab, closed,
renamed in place (Shift+Return; in-memory title that dirties until save),
regrouped, or reordered (including drag within or between sections, with a
horizontal insertion marker). Modified compositions are marked. Context menu:
Reveal Parent when a parent is open; group headers offer Rename / Add Group /
Delete Group. An anchored Info tool shows the selected composition’s path and a
scrolling Media table (path, sample rate, channels, length, size) for media in
that composition’s pool.

### Header and status

The header shows playhead, hover sample, and selection. The status bar shows
sample rate, bit depth (from the first media file), channel count, duration,
and source size; a layout picker; a control that focuses the Monitor tab;
Preview; and background job text (opening, building peaks, rendering).

### Script dock

The Script panel is a Lua REPL. `print` goes to the transcript. `app:info`,
`app:warn`, and `app:error` go to Messages (and colored stdout when attached).
See [SCRIPT.md](../SCRIPT.md).

## Review editor

### Waveform

One lane per channel, labeled from the effective layout. Overview paint uses
peak bins (256 samples per bin) built on a background thread with progress.
Sample-accurate zoom reads PCM through a block pager. Until peaks are ready,
overview does not fold PCM on the UI thread.

Zoom factor 1.25 (minimum 1/50 sample per pixel). Fit-all and Frame (selection
or caret) are view commands. Horizontal pan uses drag, a scrollbar, or
Shift+wheel.

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

Built-in types: Blue (default), Yellow, Purple. Custom types (name + color)
live on the composition. A marker is a type instance at a frame, with an
optional note. Color is on the type. At most one marker of a given type may
occupy a frame; different types may share a frame.

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
| Break Out to Composition | Each selected span becomes a new child composition sharing media |

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
`file.save_session_as`, `file.close`, `file.render`, `file.quit`

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

Those last two items are **future additions**, not a redesign of monitor DSP.
See [SPEC-processing.md](SPEC-processing.md). Incremental review of many
files is in [SPEC-workflows.md](SPEC-workflows.md) (built-in Review), not a
collection `review_status` enum.
