# Proposal

## Why

On hosts where PipeWire (or similar) constrains the ALSA client period —
for example a graph locked to quantum 128 — FieldAssist's snap-up-only
period policy probes one negotiated size, doubles it, and tries only
larger catalog Fixed sizes. Those requests are rejected; fallback to
`BufferSize::Default` eventually opens, but the policy fights the
platform instead of negotiating within what CPAL reports as supported.
The prior "2× platform default" preference was a workaround for xruns on
one platform, not a root-cause fix, and auto-persisting the snapped Fixed
size turns that workaround into a sticky setting.

## What Changes

- Negotiate output buffer size on every stream open using CPAL's
  supported buffer-size range when available: prefer ~2× the probed
  platform default when that target fits the range; otherwise walk the
  in-range catalog (larger and smaller) and accept `BufferSize::Default`.
- Treat a user-selected catalog size as a **preferred** Fixed request,
  not a guarantee; fall back with the same negotiation path when rejected.
- **BREAKING** (settings key): rename `audio.period_frames` to
  `audio.buffer_size_frames` in `settings.json`, with a serde read alias
  so existing files keep loading.
- Stop auto-persisting the negotiated/opened size into settings; only an
  explicit user choice is stored. Unset always renegotiates on open.
- Rename Settings UI "Period" → "Buffer Size" and describe it as preferred
  frames, not guaranteed.
- After open, log the **actual** buffer size from the live stream
  (including Default opens).
- Update docs (`docs/ARCHITECTURE.md`, `docs/spec/SPEC-application.md`,
  scripting guide, crate `AGENTS.md`) to match.

## Non-goals

- Enabling CPAL's native PipeWire host or changing the Linux ALSA host
  choice.
- Resizing or retuning the prefetch ring relative to driver period.
- Diagnosing or fixing residual xruns that remain after a successful
  Default (or other accepted) open — scheduling / fill timing stays out
  of scope.
- Changing sample rate, channel layout, or device selection policy.
- Guaranteeing that a preferred Fixed size is honored by the OS.

## Capabilities

### New Capabilities

- `playback/output-buffer`: How FieldAssist chooses and reports the CPAL
  output buffer size (preferred setting, negotiation on open, logging).

### Modified Capabilities

- (none — no existing openspec capability covers output buffer policy)

## Impact

- **Crates**: `field-audio-playback` (negotiation), `field-settings`
  (key rename + persist helper), `field-assist` (settings UI + open
  wiring), `field-play` (CLI period flag / unset path).
- **Hosts**: FieldAssist desktop, `field-play`. `field-batch` is
  unaffected unless it reads `audio.period_frames` (scripting docs only
  if they document the key).
- **Realtime callback**: Negotiation and logging stay on the stream-open
  path only; no change to `fill_output` quality gates.
- **Settings compatibility**: Old `period_frames` keys load via alias;
  new writes use `buffer_size_frames` only.
