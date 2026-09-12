# FieldAssist crate architecture

FieldAssist is a Cargo workspace. Library crates form a DAG from leaf utilities
up to the desktop application. Sibling crates in the same layer do not depend
on each other; traits live in the crate that *consumes* data, and higher layers
supply adapters.

Product behavior is described in [docs/spec/](spec/). This file covers only
crate boundaries and composition patterns.

## Crate DAG

```text
field-core
    ├── field-audio-model
    │       ├── field-audio-process
    │       └── field-composition ← also field-audio-io, field-core
    ├── field-audio-io
    ├── field-session
    └── (used by most crates)

field-audio-monitor     (Faust listen DSP; lock-free ParamStore)
field-audio-playback    (cpal engine; PlaybackDataProvider + MonitorProcess)
field-ui-components     (gpui widgets + host traits / DTOs)

field-assist (package name FieldAssist)
    depends on all of the above
```

| Crate | Level | Responsibility |
| --- | --- | --- |
| `field-core` | leaf | File URLs, `ProgressHandle` |
| `field-audio-model` | leaf | `PcmBuffer`, regions, markers, media pool, `BlockPager` / `BlockSource` |
| `field-audio-io` | leaf | Probe/decode/encode above Symphonia and format encoders |
| `field-audio-process` | mid | Offline peaks, resampling; future analysis/ops |
| `field-audio-monitor` | mid | Monitor chain Faust DSP, UI schema, lock-free params |
| `field-audio-playback` | mid | Realtime device I/O, transport, playhead |
| `field-ui-components` | mid | Reusable GPUI chrome; host-owned tab titles; data traits |
| `field-composition` | high | `.facomp` I/O, EDL, clip tree |
| `field-session` | high | `.fasession` I/O and membership |
| `FieldAssist` | app | Document editor, Lua, docks, `PlaybackSession`, adapters |

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
- **Transport** and **FileStatusBar** take play state / `FileStatus` DTOs and
  host-supplied actions or callbacks.
- Widgets that remain app-shaped (explorer, workspace shell, render sheet,
  workflow bar, Faust→`ParamUiNode` and `EditOp`→card mappers) stay in
  FieldAssist.

## Sample layout (dasp)

Shared sample type is `f32` (`dasp::sample::Sample`).

- **Planar** storage in the model, pager, Faust `compute`, and waveform reads
- **Interleaved** frames at the device callback and `PlaybackDataProvider::read_interleaved`

Do not force pull-based `dasp::signal::Signal` through random-access DAW paths.

## Future binaries

A CLI or mobile tool can depend on a subset, for example:

- probe/peaks CLI: `field-audio-io` + `field-audio-process`
- session batch tool: `field-session` + `field-composition` + I/O (no GPUI, no Faust)

The desktop app remains `crates/field-assist` (Cargo package `FieldAssist`).

## Docs

Every library crate uses `#![deny(missing_docs)]`, a crate-level overview with a
short example, and docs on public items. Generate with:

```bash
cargo doc --workspace --no-deps --open
```
