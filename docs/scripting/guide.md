# Scripting guide

## Hosts and scope

| Host                | `app.name`       | Runtime                                                      |
| ------------------- | ---------------- | ------------------------------------------------------------ |
| FieldAssist desktop | `"field-assist"` | GPUI app; embedded Add / Replace / Review workflows          |
| field-batch CLI     | `"field-batch"`  | Headless REPL / script / Unix shebang over `field-scripting` |
| field-play CLI      | `"field-play"`   | Headless playback; `init.lua` + enrich + `detect_layout`     |

```bash
field-batch                     # interactive REPL
field-batch script.lua a b      # app.args = { "a", "b" }
#!/usr/bin/env field-batch      # script path is argv[1]
field-play take.wav             # init.lua → enrich → detect_layout → play
field-play --config-dir DIR …   # optional init override
```

**Host coverage.** The `field.`* surface is shared by `field-scripting` and is
fully available in FieldAssist. FieldAssist supplies a desktop
`ScriptBackend`, its host-only `app` facade, and GPUI toolbar paint glue.
field-play uses the headless backend only long enough to load init and fire
`enrich_composition` then `detect_layout`; it does not run workflows or a REPL.

| Module                                        | field-batch / field-scripting             | field-play     | FieldAssist today                        |
| --------------------------------------------- | ----------------------------------------- | -------------- | ---------------------------------------- |
| `field.log` / `field.on`                      | full                                      | init + enrich + detect | full                              |
| `field.include`                               | path or `field.url`; cache + search stack | via init       | full                                     |
| `field.scripting`                             | full                                      | via init       | full                                     |
| `field.url`                                   | full                                      | via init       | full                                     |
| `field.fs`                                    | full                                      | via init       | full                                     |
| `field.session`                               | `focused` / `new` / `open`                | open via host  | `focused` / `new` / `open`               |
| `field.composition`                           | baseline userdata                         | baseline       | full, plus desktop chrome fields/methods |
| `field.media` / `field.workflow` / `field.ui` | full                                      | unused at play | full                                     |
| `field.layouts`                               | `define` + `shared_registry`              | via init       | `define` + `shared_registry`             |
| `field.exports`                               | `define` + `shared_registry`              | unused at play | `define` + `shared_registry`             |
| `field.variables`                             | full                                      | via init       | full                                     |
| `field.audio_devices`                         | full                                      | unused at play | full                                     |

## Initialization

### FieldAssist

Before Lua runs, FieldAssist loads `settings.json` via `field-settings` and
applies scripting + device + experimental + application + waveform groups from
Rust. Then:

1. Embedded `resolver_default.lua` — declares the `"default"` last-wins
   variable resolver (see [field.variables](reference/field-variables.md))
2. `init.lua` — user config file if present, else embedded default
   (`field_scripting::EMBEDDED_INIT`)
3. `resolver_*.lua` along the scripting **Search Path** (config directory
   first, then extra folders from Settings → Scripting; sorted by name
   within each folder)
4. Embedded `workflow_add.lua`, `workflow_replace.lua`, `workflow_review.lua`
5. `workflow_*.lua` along the same Search Path (same order). A later
   `declare` / `declare_resolver` replaces an earlier registration with the
   same name.

**Search Path** (`scripting.search_path`) is both:

- Extra folders scanned for `resolver_*.lua` / `workflow_*.lua`
- Extra templates appended to Lua `package.path` / `package.cpath` before
  `init.lua` (after the config-dir + cwd baseline)

The config directory is always first for discovery and path templates and is
not stored. Path extras for `package.path` apply on the **next** launch when
changed mid-session (same as discovery). Settings → Scripting → **Modules**
toggles (`enable_system_package_paths`, `enable_native_modules`) apply before
`init.lua`; enabling them live from Settings also updates the running host.
Disabling mid-session leaves the runtime enabled until relaunch. Lua
`field.scripting.enable_*` remains available for late opt-in.

| OS      | Config directory                                            |
| ------- | ----------------------------------------------------------- |
| macOS   | `~/Library/Application Support/FieldAssist/`                |
| Windows | `%APPDATA%\FieldAssist\`                                    |
| Linux   | `$XDG_CONFIG_HOME/FieldAssist/` or `~/.config/FieldAssist/` |

Resolved by `field_scripting::user_config_dir()` (shared by FieldAssist,
field-batch, and field-play). Dump the embedded default with
`FieldAssist --dump-init`.

The same directory may also hold:

| File             | Role                                                                       |
| ---------------- | -------------------------------------------------------------------------- |
| `init.lua`       | Startup script (user override or embedded default)                         |
| `keymap.json`    | Optional keybinding overlay                                                |
| `settings.json`  | Shared preferences (`field-settings`); hosts choose which groups to apply |
| `variables.json` | User-scoped variables (independent of settings reset); read into the scripting host on `load_init` |
| `resolver_*.lua` | Optional user resolvers (auto-loaded after `init.lua`)                     |
| `workflow_*.lua` | Optional user workflows (auto-loaded after embedded workflows)             |

Schema and I/O live in the `field-settings` leaf crate (no GPUI). FieldAssist
keeps a thin Global store + Settings UI. Hosts apply groups **before**
`init.lua`; embedded init does **not** call `app:load_settings()`. That method
remains for manual reload from scripts or the Settings UI. Missing or invalid
`settings.json` keeps built-in defaults (no error). CLI `--output` still wins
over the saved device.

`require` search paths default to the config directory plus cwd, then Search
Path extras (see [field.scripting](reference/field-scripting.md)). Prefer Settings →
Scripting → Modules (or `settings.json`) over calling `field.scripting.enable_*`
at the top of every `init.lua`; the Lua helpers stay for late opt-in.

### field-batch

Optional `--config-dir` (default: same FieldAssist config directory). Loads
`settings.json`, applies scripting (+ keeps experimental flags in a local
registry; device ignored until needed), then loads `{config}/variables.json`
into host user variables (missing/invalid → empty), then loads user `init.lua`
if present, else the shared embedded default. Does **not** auto-load Add /
Replace / Review. After init, fires `enrich_session` for the empty focused
session. Opening a **new media** path via `field.composition.open` (or session
open of media) fires `enrich_composition` before the open returns. Re-focusing
an already-open path or opening a `.facomp` does not. `field.session.new()`
fires `enrich_session`; `field.session.open` (load `.fasession`) does not.

### field-play

Optional `--config-dir` (default: same FieldAssist config directory). Loads
`settings.json`, applies scripting, uses `audio.output_device` and
`audio.buffer_size_frames` when resolving
playback, then loads `{config}/variables.json` into host user variables
(missing/invalid → empty), then loads user `init.lua` if present, else the
shared embedded default (layouts + `detect_layout`). Does **not** load workflow
bundles. After init, fires `enrich_session` for the empty world session. After
opening the path, fires `enrich_composition` then `detect_layout` once so unset
`monitor_chain` values pick up the layout default before playback.

## Conventions

- **Access:** **ro** = read-only from Lua; **rw** = get and set.
- **Colors:** RGBA tables are 1-based `{ r, g, b, a }` with components in
`0.0`…`1.0`.
- **Channels:** where a channels value is accepted: `nil` / `"all"` / a
0-based index array.
- **Paths:** native path strings or `field.url` userdata. Session, composition,
and media expose `.url` (url userdata) rather than `.path`. Use `:as_path()`
when a native filesystem string is required.
- **Workflow UI:** `field.ui.`* constructors build control tables.
FieldAssist renders them; field-batch stores them and ignores paint.
