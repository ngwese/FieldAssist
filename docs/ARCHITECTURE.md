# FieldAssist crate architecture

FieldAssist is a Cargo workspace. Library crates form a DAG from leaf utilities
up to the desktop application. Sibling crates in the same layer do not depend
on each other; traits live in the crate that *consumes* data, and higher layers
supply adapters.

Product behavior is described in [docs/spec/](spec/) (application, workflows,
[analysis](spec/SPEC-analysis.md), [processing](spec/SPEC-processing.md)). This
file covers only crate boundaries and composition patterns.

## Crate DAG

```text
field-core              (file URLs, ProgressHandle, CompositionId `comp:`)
    ├── field-audio-model
    │       (MediaId `media:`, MediaDescriptor, MediaPool, MediaStore, pager)
    │       ├── field-audio-process
    │       └── field-composition ← also field-audio-io, field-core
    ├── field-audio-io
    ├── field-session   (depends on field-core + field-audio-model serde;
    │                    DocumentId `doc:`, SessionId `session:`; no
    │                    field-composition edge)
    └── (used by most crates)

field-audio-monitor     (Faust listen DSP; lock-free ParamStore)
field-audio-playback    (cpal engine; PlaybackDataProvider + MonitorProcess)
field-ui-components     (gpui widgets + host traits / DTOs)

field-play              (example CLI: .facomp → default device + monitor)
field-assist (package name FieldAssist)
    depends on all of the above (except field-play)
```

| Crate | Level | Responsibility |
| --- | --- | --- |
| `field-core` | leaf | File URLs, `ProgressHandle`, prefixed `CompositionId` |
| `field-audio-model` | leaf | `PcmBuffer`, regions, markers, `MediaId` / descriptors, `MediaPool` / `MediaStore`, `BlockPager` / `BlockSource` |
| `field-audio-io` | leaf | Probe/decode/encode above Symphonia and format encoders |
| `field-audio-process` | mid | Offline peaks, resampling; future analysis/ops ([SPEC-analysis.md](spec/SPEC-analysis.md)) |
| `field-audio-monitor` | mid | Monitor chain Faust DSP, UI schema, lock-free params |
| `field-audio-playback` | mid | Realtime device I/O, transport, playhead |
| `field-ui-components` | mid | Reusable GPUI chrome; host-owned tab titles; data traits |
| `field-composition` | high | `.facomp` I/O (v7), EDL, clip tree; re-exports `CompositionId` |
| `field-session` | high | `.fasession` I/O (v2) and membership; media\|composition targets |
| `field-play` | example | Headless `.facomp` playback on the default output |
| `FieldAssist` | app | Document editor, Lua, docks, shared session `MediaStore`, `PlaybackSession`, adapters |

## Trait-at-leaf composition

- **`BlockSource`** (`field-audio-model`): pager asks for file ranges; `field-audio-io::SymphoniaBlockSource` implements it; composition wires the two.
- **`PlaybackDataProvider`** (`field-audio-playback`): engine pulls interleaved PCM; the app implements it for `Composition` / buffers.
- **`MonitorProcess`** (`field-audio-playback`): engine runs listen DSP without depending on Faust; `field-assist` adapts `MonitorHost` → `MonitorProcess`.
- **`WaveformDataProvider`** + **`WaveformEditor`** (`field-ui-components`):
  paint/read and selection/drag overlays; `BufferDocument` implements both.
  UI types use `LaneScope` / `PaintRegion` / `u64` edit ids; the app maps
  `ChannelScope` / `Region` / `EditId` at the trait boundary.
- **List-panel data traits** (`RegionsData`, `MarkersData`, `EditsData`):
  snapshot + fingerprint for observe-and-skip; selection/navigation via
  callbacks so panels never name `WaveformDisplay` or model id newtypes.
- **Monitor host surface** (`MonitorSnapshot` / `ParamUiNode` + callbacks):
  Faust JSON and `MonitorChain` stay in the app (`monitor_schema` adapter);
  the UI panel is Faust-free.
- **`FormatEncoder`** (`field-audio-io`): encode planar PCM; render orchestration stays above I/O.

## field-ui-components host contract

FieldAssist imports `field_ui_components` at call sites. Do not re-export UI
crate types through the application.

- **Tab titles** are host-owned (`dock_titles` in FieldAssist). Pass title
  slices to `tool_dock_min_size` and into panel constructors.
- **Transport** and **SessionStatusBar** take play state / `FileStatus` DTOs and
  host-supplied actions or callbacks.
- Widgets that remain app-shaped (explorer, workspace shell, render sheet,
  workflow bar, Faust→`ParamUiNode` and `EditOp`→card mappers) stay in
  FieldAssist.

## Sample layout (dasp)

Shared sample type is `f32` (`dasp::sample::Sample`).

- **Planar** storage in the model, pager, Faust `compute`, and waveform reads
- **Interleaved** frames at the device callback and `PlaybackDataProvider::read_interleaved`

Do not force pull-based `dasp::signal::Signal` through random-access DAW paths.

`dasp::ring_buffer::{Fixed, Bounded}` are useful for **single-owner** DSP
(delays, etc.). They are **not** used for the playback prefetch↔callback
boundary: push/pop need `&mut self`, so sharing them across threads would
require a mutex. `field-audio-playback` uses an atomic SPSC
[`PrefetchRing`](../crates/field-audio-playback/src/prefetch.rs) instead.

## Realtime playback path

The CPAL output callback must stay realtime-safe. Quality gates (also in
[`crates/field-audio-playback/AGENTS.md`](../crates/field-audio-playback/AGENTS.md)):

1. **Zero heap allocation** on the callback
2. **Zero blocking lock contention** on the callback
3. **No decode / filesystem I/O** on the callback

`PlaybackEngine` runs a dedicated `fa-prefetch` thread that may allocate, lock
the composition pager, and decode FLAC/etc. It pushes **pre-monitor**
device-rate interleaved source frames into the SPSC ring, using **bandlimited
FFT sample-rate conversion** when the source rate differs from the device rate
(matched rates copy bit-exactly). The CPAL callback pops those frames, runs
monitor DSP (Faust) or Direct channel mapping, and writes the device buffer so
live parameters track the audible playhead (not ring depth). Composition
`read_interleaved` uses planar pager fills (one lock per request) on that
prefetch thread. Direct bypasses Faust only — rate conversion still applies
whenever source and device rates differ.

## Future binaries

A CLI or mobile tool can depend on a subset, for example:

- probe/peaks CLI: `field-audio-io` + `field-audio-process`
- session batch tool: `field-session` + `field-composition` + I/O (no GPUI, no Faust)
- **`field-play`**: `field-composition` + `field-audio-playback` +
  `field-audio-monitor` — plays a `.facomp` on the system default device,
  using the composition's monitoring chain when set, otherwise Direct

```bash
cargo run -p field-play -- path/to/project.facomp
```

The desktop app remains `crates/field-assist` (Cargo package `FieldAssist`).

## Docs

Every library crate uses `#![deny(missing_docs)]`, a crate-level overview with a
short example, and docs on public items. Generate with:

```bash
cargo doc --workspace --no-deps --open
```
