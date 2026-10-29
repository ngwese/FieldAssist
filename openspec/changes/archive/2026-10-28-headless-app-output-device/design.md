# Design

## Context

See proposal.md for motivation. Today FieldAssist replaces the global `app`
userdata (`crates/field-assist/src/script/app.rs`) and wires
`output_device` / `output_devices` through `DesktopBackend` into live
playback. field-play and field-batch use `field_scripting::app::LuaApp`,
which has no such fields, so shared `init.lua` lines like
`app.output_device = "Focusrite"` fail at load.

field-play already loads `settings.json` and passes
`settings.audio.output_device` into `resolve_output_device` after init
(`open_with_detect`), ignoring any Lua preference. field-batch clones the
same setting into `_preferred_output` and never uses it.

No new crate edges: `field-scripting` already depends on
`field-audio-playback` for `field.audio_devices`. Preference storage and
resolve stay on the init / stream-open path; the CPAL callback is
untouched.

## Goals / Non-Goals

**Goals:**

- Shared headless `app.output_device` / `app.output_devices` matching the
  FieldAssist Lua surface for get/set and listing.
- Seed from settings, allow init override, have field-play honor the
  post-init value.
- Keep FieldAssist's desktop facade and live-switch semantics unchanged.

**Non-Goals:**

- Extending `ScriptBackend` or changing DesktopBackend.
- CLI `--output`, settings persistence from Lua, or batch playback open
  (see proposal Non-goals).

## Decisions

### 1. Store preference on `HostInner`, not on `ScriptBackend`

**Choice:** Add `output_device: Option<String>` to `HostInner` with
`ScriptHost::{output_device, set_output_device}` accessors. Lua get/set
read and write that field. `output_devices` lists via
`field_audio_playback::list_output_devices` (names only), same source
family as `field.audio_devices`.

**Why:** Preference is host process state, not session/document state.
Avoiding a trait method keeps FieldAssist's `DesktopBackend` free of a
no-op stub and avoids a sibling-crate edge.

**Alternatives:** Extend `ScriptBackend` (forces DesktopBackend + every
test backend to implement unused methods). Duplicate host-local `LuaApp`
in field-play and field-batch (splits the shared facade again).

### 2. Set stores the string; do not open CPAL on assign

**Choice:** `app.output_device = spec` stores `spec` (or clears on nil)
without calling `resolve_output_device` / opening a stream. Invalid types
error. field-play resolves once when opening playback.

**Why:** Headless hosts have no live engine during init. Matching
`settings.json` semantics (store preferred substring; fail at open) keeps
init scripts portable across machines that lack the named device until
play time. FieldAssist continues to resolve+open on set via its own
facade.

**Alternatives:** Validate-on-set via `resolve_output_device` (fails early
but breaks pinning a device that is temporarily absent). Resolve and store
canonical name (diverges from settings seed round-trip).

### 3. Seed + consume at the host mains

**Choice:**

1. After `ScriptHost` construction and scripting settings apply, both
   field-play and field-batch call `host.set_output_device` with
   `settings.audio.output_device` (or a dedicated seed helper).
2. Then `load_init()` (Lua may override).
3. field-play reads `host.output_device()` for `resolve_output_device`
   instead of `settings.audio.output_device` directly.
4. field-batch keeps the preference on the host for scripts; no stream
   open.

**Alternatives:** Only teach field-play to eval `return app.output_device`
after init (skips batch API parity and skips a typed host accessor).
Inject preference into `HeadlessWorld` (wrong layer).

### 4. Docs only for FieldAssist Hosts column

**Choice:** Update `docs/scripting/reference/app.md` Hosts for
`output_device` / `output_devices` to all hosts; note in the guide that
headless hosts seed from settings and do not live-switch. Leave
FieldAssist code alone.

**Alternatives:** Unify FieldAssist onto the shared `LuaApp` (large
refactor; out of scope).

## Risks / Trade-offs

- **[Risk] Two `app` implementations diverge further** → Mitigation: keep
  field semantics identical (string/nil, substring preference, list of
  names); document Hosts as all; do not invent headless-only fields.
- **[Risk] Invalid device name only fails at field-play open** → Mitigation:
  accept as intentional (same as settings-only path today); open error
  remains clear via existing `resolve_output_device` context.
- **[Trade-off] field-batch cannot audition the device** → Acceptable;
  non-goal for this change.

## Migration Plan

- Existing `settings.json` `audio.output_device` keeps working; init that
  sets `app.output_device` starts working on headless hosts without further
  migration.
- Users who wrapped the assignment in `if app.name == "field-assist"` can
  drop the guard once shipped; no forced rewrite.
- Rollback: revert the three crates; headless init that sets the field
  fails again as today.

## Open Questions

None.
