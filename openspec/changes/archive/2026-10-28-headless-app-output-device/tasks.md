# Tasks

## 1. Shared host preference API

- [x] 1.1 Add `output_device: Option<String>` to `HostInner` and
      `ScriptHost::{output_device, set_output_device}` accessors in
      `field-scripting`; verify with a unit test that set/get/nil round-trip
      on a headless host
- [x] 1.2 Expose Lua `app.output_device` (rw) and `app.output_devices` (ro)
      on shared `LuaApp`; reject non-string/non-nil sets; verify with
      `cargo test -p field-scripting` covering set/get, nil clear, type
      error, and that `output_devices` is a table of strings (empty OK when
      enumeration fails)

## 2. Headless host wiring

- [x] 2.1 In field-batch, seed `host.set_output_device` from
      `settings.audio.output_device` before `load_init` and drop the unused
      `_preferred_output` local; verify with `cargo test -p field-batch`
      and/or a `--eval` smoke that reads `app.output_device` after a temp
      config with settings + init override
- [x] 2.2 In field-play `open_with_detect`, seed the same way, then return
      `host.output_device()` (not raw settings) for
      `resolve_output_device`; verify with `cargo test -p field-play` that
      a temp config whose init sets `app.output_device` yields that
      preference (device open may skip when no audio hardware)

## 3. Docs and gates

- [x] 3.1 Update `docs/scripting/reference/app.md` Hosts for
      `output_device` / `output_devices` to all hosts, and note in
      `docs/scripting/guide.md` that headless hosts seed from settings and
      do not live-switch; verify the documented `app.output_device = "…"`
      example matches embedded init comments
- [x] 3.2 Run `cargo fmt`, then `cargo build` and `cargo test` with no
      errors or warnings (focused `-p field-scripting -p field-batch -p
      field-play` first, then full workspace)
