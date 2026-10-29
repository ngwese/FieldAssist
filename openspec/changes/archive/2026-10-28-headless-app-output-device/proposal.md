# Proposal

## Why

`init.lua` can set `app.output_device` on FieldAssist, and the shared
embedded init comments that pattern, but field-play and field-batch use the
base `field-scripting` `app` facade which rejects the field. Shared config
scripts that pin an output device therefore break headless hosts even though
field-play already reads `settings.json` `audio.output_device`.

## What Changes

- Expose `app.output_device` (rw) and `app.output_devices` (ro) on the shared
  `field-scripting` `app` facade used by field-play and field-batch.
- Seed the preference from `settings.json` `audio.output_device` before
  `init.lua`, then let `app.output_device = …` in init override it.
- field-play uses the post-init preference when resolving the CPAL device
  (same substring / index match as today).
- field-batch stores and exposes the preference for scripts; it does not open
  a playback stream in this change.
- Update scripting docs so Hosts for these fields include all three hosts.

## Non-goals

- Changing FieldAssist's desktop `app` facade or live stream switch behavior.
- Adding a field-play / field-batch CLI `--output` flag.
- Persisting `app.output_device` writes back into `settings.json`.
- Opening or switching a CPAL stream from field-batch.
- Changing `field.audio_devices`, monitor-chain selection, or buffer-size
  negotiation.

## Capabilities

### New Capabilities

- `scripting/app-output-device`: Preferred playback output device via the
  shared Lua `app` facade (get/set, device list, settings seed, field-play
  resolve).

### Modified Capabilities

- (none)

## Impact

- **Crates**: `field-scripting` (shared `app` fields + preference state),
  `field-play` (resolve from host preference after init), `field-batch`
  (seed preference; drop unused local). Docs under `docs/scripting/`.
- **Hosts**: field-play and field-batch gain the API. FieldAssist unchanged
  (keeps its own `app` userdata).
- **Realtime callback**: Preference storage and resolve stay on the
  stream-open / init path only; no change to the CPAL output callback.
