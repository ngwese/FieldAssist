# Design

## Context

See proposal.md for motivation. Today `OutputPeriod::SnapTwicePlatformDefault`
probes `BufferSize::Default`, snaps to ≥ 2× via `snap_period_frames`, and
walks only larger catalog sizes in `open_snapped_period`
(`field-audio-playback`). `SupportedStreamConfig.buffer_size`
(`SupportedBufferSize::Range` / `Unknown`) is available on the config already
passed into that path but unused. App settings persist the first successful
Fixed open via `AudioSettings::opened_period_frames_to_persist`. Prefetch ring
capacity stays fixed at 8192 and is out of scope.

CPAL cannot request a preferred range — only `Fixed(n)` or `Default`. Graceful
behavior is therefore: intersect our catalog with the reported range, order
candidates, try Fixed opens, then Default; read `stream.buffer_size()` after
success for logging / `OpenedPeriod`.

Negotiation runs only while building/reopening the stream (same thread as
today's open helpers). Prefetch and the CPAL callback are unchanged; realtime
quality gates are preserved because no open-path work moves onto
`fill_output`.

## Goals / Non-Goals

**Goals:**

- Replace snap-up-only with range-aware A+B candidate selection.
- Rename setting/API surface from period → preferred buffer size without
  dropping old JSON.
- Stop sticky auto-persist; always renegotiate when unset.
- Surface actual opened frames in logs and `OpenedPeriod`.

**Non-Goals:**

- Native PipeWire CPAL host; prefetch retune; xrun root-cause beyond open
  policy (see proposal Non-goals).
- New crate edges; keep logic in `field-audio-playback` + settings/UI adapters.

## Decisions

### 1. Candidate selection (A+B)

**Choice:** When auto-negotiating:

1. Probe Default → read negotiated frames `D` (existing probe).
2. Read `default_config.buffer_size()`; if `Range { min, max }`, filter the
   catalog to `[min, max]`; if `Unknown`, use the full catalog.
3. Preferred target `T` = smallest catalog entry ≥ `2×D` that is in range;
   if none, largest in-range entry ≤ `2×D`, else nearest in-range entry.
4. Candidate order: `T`, then larger in-range catalog, then smaller in-range
   catalog (descending), deduped; try each as `Fixed`; then `Default`.
5. User `Frames(n)`: try `Fixed(n)` first; on reject, run the same auto path.

**Alternatives:** Always Default when unset (abandons headroom preference on
healthy hosts). Walk smaller before larger (worse latency on hosts that can
do 2×). Ignore SupportedBufferSize and only react to open errors (more failed
opens / log noise on constrained hosts).

### 2. Rename `OutputPeriod` / settings field

**Choice:** Rename user-facing and settings to buffer size:

- JSON: `buffer_size_frames` with `#[serde(alias = "period_frames")]`.
- Rust settings: `AudioSettings.buffer_size_frames`; remove
  `opened_period_frames_to_persist` auto-write path from app open (delete or
  gut helper; keep tests for “never persist on open”).
- Playback API: rename `OutputPeriod` → `OutputBufferSize` (or keep enum name
  briefly with renamed variants) — prefer rename for clarity:
  `PreferPlatformDefault` (was `SnapTwicePlatformDefault`) and `Frames(u32)`.
- Module/file `period.rs` may become `buffer_size.rs` or keep filename with
  updated docs; either is fine if public names match.

**Alternatives:** UI-only rename (rejected — user asked for JSON key rename).

### 3. `OpenedPeriod` carries actual frames

**Choice:** Extend open result to include `actual_frames: u32` from
`stream.buffer_size()` after a successful open (Fixed or Default). Keep
`requested_frames: Option<u32>` for Fixed requests (`None` = Default
request). Log both when useful, always log actual.

**Alternatives:** Log only via `eprintln!` without API change (weaker for
field-play / UI). Persist actual into settings (rejected).

### 4. Stop persist-on-open

**Choice:** Remove `persist_opened_period_frames` behavior that writes when
`prior` is `None`. FieldAssist still applies settings → reopen when the user
changes the dropdown; reset to unset restores auto negotiation.

**Migration:** Users who already have a sticky snapped value in
`period_frames` keep it as an explicit preference via the alias until they
reset Buffer Size. Document that reset returns to auto.

### 5. Docs and hosts

Update `docs/ARCHITECTURE.md`, `docs/spec/SPEC-application.md`,
`docs/scripting/guide.md`, `field-audio-playback/AGENTS.md`, and field-play
messages (`Output period` → buffer size wording). field-batch unchanged in
code unless it references the old key in docs only.

## Risks / Trade-offs

- **[Risk]** SupportedBufferSize on pipewire-alsa may report a wide Range that
  still rejects some Fixed sizes at open → Mitigation: keep open-error walk
  and Default fallback; range is a filter, not a guarantee.
- **[Risk]** Dropping auto-persist + softening ×2 may restore xruns on the
  host that motivated ×2 → Mitigation: still prefer 2× when in range; out of
  scope to chase remaining xruns after successful open.
- **[Risk]** Legacy sticky `period_frames` values remain explicit preferences
  → Mitigation: alias load + UI reset; optional one-line note in release
  notes / SPEC.
- **[Trade-off]** More open attempts on constrained hosts (probe + several
  Fixed + Default) → Acceptable; already probes and retries today; stay under
  existing `OUTPUT_OPEN_TIMEOUT`.

## Migration Plan

1. Ship rename with serde alias; new saves emit `buffer_size_frames`.
2. No forced settings rewrite; first save after an unrelated settings edit
   may drop the old key when the struct serializes skip_if_none / new field.
3. Rollback: revert change; alias means mixed files remain readable if old
   code only knows `period_frames` only after users re-save under new key —
   note that rollback after a new-key-only save needs the alias on old builds
   or manual key rename (acceptable for a pre-1.0 / local settings file).

## Open Questions

None that block implementation. Exact public enum/module rename spelling can
follow crate style during apply.
