# Scripting

<!-- markdownlint-disable MD013 MD060 -->

As-built Lua 5.4 reference for FieldAssist, **field-batch**, and **field-play**.
Hosts share the `field-scripting` API shape under the global `field`
namespace. The global `app` object is a thin host facade; `app.name`
identifies the process.

This document describes **what is implemented today**. Intended-but-missing
APIs (confirm dialogs, SQLite, C2PA, handoff) live in
[spec/SPEC-field-recording.md](spec/SPEC-field-recording.md). Copyable pipeline
examples are under
[examples/workflows/field-recording/](examples/workflows/field-recording/).

## Table of contents

- [Hosts and scope](#hosts-and-scope)
- [Initialization](#initialization)
  - [FieldAssist](#fieldassist)
  - [field-batch](#field-batch)
  - [field-play](#field-play)
- [Conventions](#conventions)
- [app (host-only)](#app-host-only)
  - [app Properties](#app-properties)
  - [app Functions / methods](#app-functions--methods)
  - [app.theme (FieldAssist)](#apptheme-fieldassist)
- [field.log](#fieldlog)
  - [field.log Properties](#fieldlog-properties)
  - [field.log Functions](#fieldlog-functions)
- [field.on](#fieldon)
  - [field.on Properties](#fieldon-properties)
  - [field.on Functions](#fieldon-functions)
  - [field.on Events](#fieldon-events)
- [field.include](#fieldinclude)
  - [field.include Properties](#fieldinclude-properties)
  - [field.include Functions](#fieldinclude-functions)
- [field.scripting](#fieldscripting)
  - [field.scripting Properties](#fieldscripting-properties)
  - [Default package.path / package.cpath](#default-packagepath--packagecpath)
  - [field.scripting Functions](#fieldscripting-functions)
- [field.url](#fieldurl)
  - [field.url Properties](#fieldurl-properties)
  - [field.url Functions](#fieldurl-functions)
  - [Returned type: url userdata](#returned-type-url-userdata)
- [field.fs](#fieldfs)
  - [field.fs Properties](#fieldfs-properties)
  - [field.fs Functions](#fieldfs-functions)
- [field.session](#fieldsession)
  - [field.session Properties](#fieldsession-properties)
  - [field.session Functions](#fieldsession-functions)
  - [Returned type: session](#returned-type-session)
- [field.composition](#fieldcomposition)
  - [field.composition Properties](#fieldcomposition-properties)
  - [field.composition Functions](#fieldcomposition-functions)
  - [Returned type: composition](#returned-type-composition)
- [field.media](#fieldmedia)
  - [field.media Properties](#fieldmedia-properties)
  - [field.media Functions](#fieldmedia-functions)
  - [Returned type: media pool](#returned-type-media-pool)
  - [Returned type: media](#returned-type-media)
- [field.layouts](#fieldlayouts)
  - [field.layouts Properties](#fieldlayouts-properties)
  - [field.layouts Functions](#fieldlayouts-functions)
  - [field.layouts define(spec) keys](#fieldlayouts-definespec-keys)
  - [Returned type: layout registry](#returned-type-layout-registry)
  - [Returned type: layout](#returned-type-layout)
- [field.exports](#fieldexports)
  - [field.exports Properties](#fieldexports-properties)
  - [field.exports Functions](#fieldexports-functions)
  - [field.exports define(spec) keys](#fieldexports-definespec-keys)
  - [composition:export(arg)](#compositionexportarg)
  - [Setting resolution](#setting-resolution)
  - [Returned type: export registry](#returned-type-export-registry)
  - [Returned type: export profile](#returned-type-export-profile)
- [field.variables](#fieldvariables)
  - [field.variables Functions](#fieldvariables-functions)
  - [Returned type: VariableResolver](#returned-type-variableresolver)
  - [Returned type: bindings](#returned-type-bindings)
  - [Resolver prototype](#resolver-prototype)
- [field.workflow](#fieldworkflow)
  - [field.workflow Properties](#fieldworkflow-properties)
  - [field.workflow Functions](#fieldworkflow-functions)
  - [create / declare property table](#create--declare-property-table)
  - [Returned type: workflow prototype / instance (table)](#returned-type-workflow-prototype--instance-table)
- [field.ui](#fieldui)
  - [field.ui Properties](#fieldui-properties)
  - [field.ui Functions](#fieldui-functions)
  - [Returned type: control table](#returned-type-control-table)
  - [Palette: field.ui.named](#palette-fielduinamed)
  - [Palette: field.ui.semantic](#palette-fielduisemantic)
- [field.audio_devices](#fieldaudio_devices)
  - [field.audio_devices Properties](#fieldaudio_devices-properties)
  - [field.audio_devices Functions](#fieldaudio_devices-functions)
- [Nested types](#nested-types)
  - [collection](#collection)
  - [region](#region)
  - [marker](#marker)

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
   variable resolver (see [field.variables](#fieldvariables))
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
Path extras (see [field.scripting](#fieldscripting)). Prefer Settings →
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
`settings.json`, applies scripting, uses `audio.output_device` when resolving
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

---

## `app` (host-only)

Process facade unique to the enclosing binary. Model constructors and shared
APIs live under `field.*`, not here.

### app Properties

| Property         | Access | Hosts       | Type              | Description                                                             |
| ---------------- | ------ | ----------- | ----------------- | ----------------------------------------------------------------------- |
| `name`           | **ro** | all         | `string`          | Stable process id (`"field-assist"` / `"field-batch"` / `"field-play"`) |
| `args`           | **ro** | field-batch | `{ string, … }`   | Script argv after the file / `--`                                       |
| `session`        | **ro** | all         | session           | Sugar for `field.session.focused()`                                     |
| `composition`    | **rw** | all         | composition or `nil` | Sugar for `app.session.composition`                                  |
| `workflow`       | **ro** | all         | `table` or `nil`  | Active workflow instance                                                |
| `theme`          | **ro** | FieldAssist | theme userdata    | Live GPUI theme (see below)                                             |
| `themes`         | **ro** | FieldAssist | `{ string, … }`   | Registered theme names                                                  |
| `looping`        | **ro** | FieldAssist | `boolean`         | Transport loop flag                                                     |
| `preview`        | **ro** | FieldAssist | `boolean`         | Preview playback flag                                                   |
| `explorer`       | **ro** | FieldAssist | `boolean`         | Explorer dock open                                                      |
| `output_device`  | **rw** | FieldAssist | `string` or `nil` | Preferred output device (substring match)                               |
| `output_devices` | **ro** | FieldAssist | `{ string, … }`   | Known output device names                                               |

### app Functions / methods

| Function            | Arguments         | Returns | Hosts       | Description                                                                                                             |
| ------------------- | ----------------- | ------- | ----------- | ----------------------------------------------------------------------------------------------------------------------- |
| `app:alert`         | `subject`, `body` | —       | all         | Host alert (dialog / stderr). Args are stringified.                                                                     |
| `app:command`       | `id: string`      | —       | FieldAssist | Invoke a UI command (e.g. `"view.show-explorer"`)                                                                       |
| `app:load_settings` | —                 | —       | FieldAssist | Manual reload: read `settings.json` (or defaults), update the Global store, re-apply theme / docks / waveform / selection / output device / scripting policy |

field-scripting also keeps temporary migration shims
`app:info` / `app:warn` / `app:error` and
`app.finish_workflow` / `app.cancel_workflow` / `app.run_workflow`; prefer
`field.log` and `field.workflow`.

### `app.theme` (FieldAssist)

| Property   | Access | Type                 | Description                                 |
| ---------- | ------ | -------------------- | ------------------------------------------- |
| `name`     | **rw** | `string`             | Theme registry name                         |
| `mode`     | **rw** | `"light"` \| `"dark"` | Appearance mode                             |
| `named`    | **ro** | palette userdata     | Same keys as [field.ui.named](#palette-fielduinamed)    |
| `semantic` | **ro** | palette userdata     | Same keys as [field.ui.semantic](#palette-fielduisemantic) |

```lua
-- Manual reload (startup already applied settings from Rust):
if app.name == "field-assist" and app.load_settings then
  app:load_settings()
end

if app.name == "field-assist" and app.command then
  app:command("view.show-explorer")
end
```

---

## `field.log`

Structured logging into the host log sink (Script panel / stderr / Messages).

### field.log Properties

(none)

### field.log Functions

| Function | Arguments          | Returns | Description        |
| -------- | ------------------ | ------- | ------------------ |
| `info`   | `topic`, `message` | —       | Informational line |
| `warn`   | `topic`, `message` | —       | Warning line       |
| `error`  | `topic`, `message` | —       | Error line         |

On FieldAssist, `topic` / `message` may be any Lua values (stringified). On
field-batch they are strings.

---

## `field.on`

Registers process-wide model/host hooks. Unknown event names error.

### field.on Properties

(none)

### field.on Functions

| Function | Arguments                             | Returns | Description               |
| -------- | ------------------------------------- | ------- | ------------------------- |
| `on`     | `event: string`, `callback: function` | —       | Append a hook for `event` |

### field.on Events

| Event                  | Callback arguments                          | Notes                                                                         |
| ---------------------- | ------------------------------------------- | ----------------------------------------------------------------------------- |
| `loaded`               | `composition`, `elapsed`                    | `elapsed` is seconds since open started                                       |
| `saved`                | `composition`, `elapsed`                    |                                                                               |
| `enrich_composition`   | `composition`                               | Fired when a composition is **built from media**; mutate `c.variables` in place |
| `enrich_session`       | `session`                                   | Fired when an **empty session is created**; mutate `s.variables` / `s.properties` |
| `detect_layout`        | `composition`, `chosen` → `string` or `nil` | `chosen` is the current layout name or `nil`; return a registered layout name |
| `session_loaded`       | `session`                                   |                                                                               |
| `session_saved`        | `session`                                   |                                                                               |
| `session_selected`     | `session`                                   |                                                                               |
| `composition_selected` | `composition` or `nil`                      |                                                                               |

`enrich_composition` runs before `detect_layout` (and before `loaded` when
that event also fires). Use it to set composition variables from paths,
probe/`source.*` metadata (`c:variable_resolver():resolve`), or external lookups. It does
**not** run for `.facomp` opens or session restore. FieldAssist also fires it
after break-out regions/channels.

```lua
field.on("enrich_composition", function(c)
  local dir = c.dirname
  if dir then
    c.variables.values.project = field.url(dir).basename or dir
  end
  local flag = c:variable_resolver():resolve("TIMECODE_FLAG")
  if flag then
    c.variables.values.timecode_flag = flag
  end
end)
```

`enrich_session` runs when a session is **initially created** (cold start with
no `.fasession`, File → Close Session / new empty, `field.session.new`,
field-batch / field-play after `load_init`). When `session_loaded` also runs
(empty-session reset), order is `enrich_session` → `session_loaded` →
`session_selected`. It does **not** run for `.fasession` open / restore or
`field.session.open`.

```lua
field.on("enrich_session", function(s)
  s.variables.values.studio = "default"
  s.properties = { review_mode = "todo" }
end)
```

Workflow prototypes can also `:on` the same events; those handlers receive the
**instance** as the first argument (see [field.workflow](#fieldworkflow)).

---

## `field.include`

Load and run another Lua chunk relative to the caller (replaces
`debug.getinfo` + `dofile`).

### field.include Properties

(none)

### field.include Functions

| Function  | Arguments | Returns               | Description                 |
| --------- | --------- | --------------------- | --------------------------- |
| `include` | `spec`    | chunk return value(s) | Load and execute a Lua file |

`spec` **(field-scripting):** path `string` or [field.url](#fieldurl) userdata.
Search: absolute path → directory of the calling file → config dir → relative
to cwd. Results are cached by resolved path while the host lives.

`spec` **(FieldAssist today):** path `string` only; `lua.load` of that path
(no URL, no shared include cache).

---

## `field.scripting`

Host controls for Lua `require` search paths and native C modules. Paths are
locked down so workflows stay self-contained by default.

### field.scripting Properties

(none)

### Default `package.path` / `package.cpath`

| Kind | Templates                                                          |
| ---- | ------------------------------------------------------------------ |
| Lua  | `{config}/?.lua`, `{config}/?/init.lua`, `./?.lua`, `./?/init.lua` |
| C    | `{config}/?.so` (`.dll` on Windows), `./?.so` / `./?.dll`          |

`{config}` is the host config directory when known. System-wide bundled Lua
locations (`/usr/local/...`, Windows `!\\...`) and foreign `LUA_PATH*` /
`LUA_CPATH*` roots are **not** on the path until opted in. C loaders remain
stubbed until opted in even if a `.so` sits on `package.cpath`.

Prefer `[field.include](#fieldinclude)` for FieldAssist/field-batch Lua that
should resolve relative to the caller or config dir.

### field.scripting Functions

| Function                      | Arguments | Returns | Description                                                    |
| ----------------------------- | --------- | ------- | -------------------------------------------------------------- |
| `enable_system_package_paths` | —         | —       | Append the saved system-wide path/cpath templates (idempotent) |
| `enable_native_modules`       | —         | —       | Restore `package.loadlib` and C searchers (idempotent)         |

Either call is independent. Typical place is the top of user `init.lua`:

```lua
field.scripting.enable_system_package_paths()
field.scripting.enable_native_modules()
```

For a worked example that builds a Lua 5.4–compatible `lsqlite3.so` for
field-batch and indexes audio paths into SQLite, see
[examples/field-batch/lsqlite-module/](examples/field-batch/lsqlite-module/).

---

## `field.url`

Location userdata over `field_core::Location` (relative, `file://`, or
`memory://` URLs). Bound on all hosts that use `field-scripting` (including
FieldAssist).

### field.url Properties

(none on the module table)

### field.url Functions

| Function         | Arguments       | Returns      | Description                                              |
| ---------------- | --------------- | ------------ | -------------------------------------------------------- |
| `parse`          | `input: string` | url userdata | Parse a path, `file://`, relative, or `memory://` string |
| `from_path`      | `path: string`  | url userdata | Encode a filesystem path                                 |
| `from_file_path` | `path: string`  | url userdata | Alias of `from_path`                                     |

### Returned type: url userdata

#### url Properties

| Property     | Access | Type              | Description                            |
| ------------ | ------ | ----------------- | -------------------------------------- |
| `scheme`     | **ro** | `string`          | e.g. `file`, `memory`, or URL scheme   |
| `host`       | **ro** | `string` or `nil` | Host component when present            |
| `path`       | **ro** | `string`          | Path component of the URL              |
| `query`      | **ro** | `string` or `nil` | Query string                           |
| `native`     | **ro** | `string`          | Native filesystem path when applicable |
| `normalized` | **ro** | `string`          | Canonical serialized form              |
| `parent`     | **ro** | url userdata      | Parent directory (filesystem)          |
| `basename`   | **ro** | `string`          | Final path segment                     |
| `stem`       | **ro** | `string`          | Basename without extension             |
| `extension`  | **ro** | `string`          | Extension without leading `.`          |

#### url Methods

| Method            | Arguments                 | Returns      | Description                              |
| ----------------- | ------------------------- | ------------ | ---------------------------------------- |
| `:as_path()`      | —                         | `string`     | Platform-native filesystem path          |
| `:join(...)`      | one or more path segments | url userdata | Append path segments                     |
| `:with_extension` | `ext: string`             | url userdata | Replace extension (leading `.` optional) |
| `:relative_to`    | `base` (url or string)    | `string`     | Relative path from `base`                |
| `__tostring`      | —                         | `string`     | Stored raw form                          |

---

## `field.fs`

Filesystem helpers. Paths accept a `string` or (in field-scripting) url
userdata.

### field.fs Properties

(none)

### field.fs Functions

| Function     | Arguments                                | Returns                             | Hosts           | Description                                                                                                                                                  |
| ------------ | ---------------------------------------- | ----------------------------------- | --------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `find_files` | `dir [, filter]`                         | `{ string, … }`                     | all             | Recursive listing of paths **relative to** `dir`, `/`-separated. `filter` = `nil`, extension list `{ "wav", … }`, or `function(dirname, basename) → truthy`. |
| `mkdir`      | `path [, { recursive = bool }]`          | —                                   | field-scripting | Create directory. Default `recursive = true`.                                                                                                                |
| `remove`     | `path [, { recursive = bool }]`          | —                                   | field-scripting | Remove file or directory. Default `recursive = false`.                                                                                                       |
| `copy`       | `src`, `dest` [, `{ recursive = bool }`] | —                                   | field-scripting | Copy file, or directory when `recursive = true`.                                                                                                             |
| `exists`     | `path`                                   | `boolean`                           | field-scripting | Path exists                                                                                                                                                  |
| `stat`       | `path`                                   | `{ size, is_dir, is_file, mtime? }` | field-scripting | Metadata; `mtime` is unix seconds when available                                                                                                             |
| `checksum`   | `path [, algo]`                          | `string`                            | field-scripting | Hex digest; only `"blake3"` (default)                                                                                                                        |

FieldAssist currently binds `find_files` **only**.

---

## `field.session`

Constructors and accessors for session objects (`.fasession` membership,
groups, properties, open compositions).

### field.session Properties

(none on the module table)

### field.session Functions

| Function  | Arguments              | Returns | Hosts | Description                               |
| --------- | ---------------------- | ------- | ----- | ----------------------------------------- |
| `focused` | —                      | session | all   | Process / UI-active session               |
| `new`     | —                      | session | all   | Create an empty detached session          |
| `open`    | `path` (string or url) | session | all   | Load a `.fasession` as a detached session |

`field.session.focused()` is the host's current world/UI session. In contrast,
`field.session.new()` and `field.session.open(path)` create or load detached
sessions: they do not replace the active FieldAssist session, do not open
their documents in the desktop, and are intended for manifest inspection or
metadata work. To load a `.fasession` into the focused desktop session, call
`field.session.focused():open(path)`.

### Returned type: session

#### session Properties

| Property        | Access | Type                                                            | Description                                    |
| --------------- | ------ | --------------------------------------------------------------- | ---------------------------------------------- |
| `id`            | **ro** | `string`                                                        | Session id                                     |
| `url`           | **ro** | url userdata or `nil`                                           | On-disk `.fasession` location                  |
| `workflow_name` | **rw** | `string` or `nil`                                               | Persisted stateful workflow name               |
| `capture_ui`    | **rw** | `boolean`                                                       | Whether UI chrome is captured with the session |
| `properties`    | **rw** | `{ [string] = string }`                                         | Workflow string map (not variables)            |
| `variables`     | **rw** | bindings userdata (see [field.variables](#fieldvariables)); assign also accepts map / row-array tables | Session-scoped variables |
| `composition`   | **rw** | composition or `nil`                                            | Active document                                |
| `compositions`  | **ro** | `{ composition, … }`                                            | Session documents in order                     |
| `groups`        | **rw** | `{ string, … }`                                                 | Group name list                                |

#### session Methods

| Method          | Arguments              | Returns     | Description                                                |
| --------------- | ---------------------- | ----------- | ---------------------------------------------------------- |
| `:group_count`  | `group: string`        | `integer`   | Documents in `group`                                       |
| `:add_group`    | `name: string`         | —           | Append a group                                             |
| `:rename_group` | `old`, `new`           | —           | Rename a group                                             |
| `:delete_group` | `name: string`         | —           | Remove a group                                             |
| `:move_group`   | `name`, `index`        | —           | Reorder group (`index` clamped ≥ 0)                        |
| `:move`         | `composition`, `index` | —           | Reorder a document                                         |
| `:open`         | `path` (string or url) | composition | Open media / `.facomp` into the session (not `.fasession`) |
| `:save`         | —                      | —           | Save session (FieldAssist; headless stub errors)           |
| `:save_as`      | `path` (string or url) | —           | Save session to a new path                                 |
| `:close`        | —                      | —           | Close / detach (FieldAssist; headless stub errors)         |
| `:variable_resolver` | —                 | VariableResolver | Session site (`user.*` + session); see [field.variables](#fieldvariables) |

---

## `field.composition`

Open compositions outside of (or in addition to) a session. Session membership
still uses `session:open`.

### field.composition Properties

(none on the module table)

### field.composition Functions

| Function | Arguments              | Returns     | Description             |
| -------- | ---------------------- | ----------- | ----------------------- |
| `open`   | `path` (string or url) | composition | Open audio or `.facomp` |

### Returned type: composition

#### composition Properties

| Property       | Access | Type                                                            | Description                                                                      |
| -------------- | ------ | --------------------------------------------------------------- | -------------------------------------------------------------------------------- |
| `name`         | **rw** | `string`                                                        | Display name                                                                     |
| `url`          | **ro** | url userdata or `nil`                                           | Source / project location                                                        |
| `id`           | **ro** | `string`                                                        | Composition id                                                                   |
| `group`        | **rw** | `string` or `nil`                                               | Session group membership                                                         |
| `state`        | **rw** | `string` or `nil`                                               | Free-form document state                                                         |
| `properties`   | **rw** | `{ [string] = string }`                                         | Workflow string map on the session document                                      |
| `variables`    | **rw** | bindings userdata (see [field.variables](#fieldvariables)); assign also accepts map / row-array tables | Composition-scoped variables (`.facomp`)                                         |
| `frames`       | **ro** | `integer`                                                       | Timeline length in samples                                                       |
| `sample_rate`  | **ro** | `integer`                                                       | Hz                                                                               |
| `channels`     | **ro** | `integer`                                                       | Channel count                                                                    |
| `selection`    | **rw** | [collection](#collection) or selection table                    | Primary selection; set `nil`, a collection, or `{ kind, start, stop, channels }` |
| `position`     | **rw** | `integer` or `nil`                                              | Playhead sample                                                                  |
| `collections`  | **ro** | `{ string, … }`                                                 | Named region collection names                                                    |
| `markers`      | **ro** | `{ marker, … }`                                                 | Marker list                                                                      |
| `marker_types` | **ro** | `{ { name, color }, … }`                                        | Registered marker types                                                          |
| `media`        | **ro** | `{ media, … }`                                                  | Media referenced by this composition                                             |

**FieldAssist-only properties:** `parent`, `children`, `codec`, `bit_depth`,
`basename`, `dirname`, `channel_layout` (**rw**), `monitor_chain` (**rw**),
`playback_channels` (**rw**), `source_channels` (**ro**), `duration` (**ro**),
`regions` (**ro** — selection as region userdata array).

**Selection table setter:** `kind` = `"none"` \| `"position"` \| `"region"`;
`start` / `stop` samples; `channels` as above.

#### composition Methods

| Method                                                                      | Arguments                                                          | Returns                    | Description                                                                          |
| --------------------------------------------------------------------------- | ------------------------------------------------------------------ | -------------------------- | ------------------------------------------------------------------------------------ |
| `:select`                                                                   | `start`, `stop` [, `channels`]                                     | —                          | Select a half-open sample range                                                      |
| `:select_all`                                                               | —                                                                  | —                          | Select the whole timeline                                                            |
| `:clear_selection`                                                          | —                                                                  | —                          | Clear selection                                                                      |
| `:collection`                                                               | `name: string`                                                     | [collection](#collection)  | Named region collection                                                              |
| `:add_region`                                                               | `{ start, stop, channels?, label?, collection? }`                  | [region](#region)          | Add a labeled region                                                                 |
| `:remove_region`                                                            | `id: integer`                                                      | `boolean`                  | Remove by id                                                                         |
| `:add_marker`                                                               | frame[, type] **or** `{ frame\|sample, type\|kind?, color?, note? }` | [marker](#marker) or `nil` | Add a marker                                                                         |
| `:remove_marker`                                                            | marker or id                                                       | `boolean`                  | Remove one marker                                                                    |
| `:remove_marker_at`                                                         | `frame` [, `type`]                                                 | `boolean`                  | Remove at sample                                                                     |
| `:remove_marker_by_type`                                                    | `type: string`                                                     | `boolean`                  | Remove all of a type                                                                 |
| `:marker_at`                                                                | `frame` [, `type`]                                                 | marker or `nil`            | Lookup                                                                               |
| `:add_marker_type`                                                          | `name`, `color`                                                    | `boolean`                  | Register a type color                                                                |
| `:remove_marker_type`                                                       | `name`                                                             | `boolean`                  | Unregister a type                                                                    |
| `:save`                                                                     | —                                                                  | —                          | Persist `.facomp` (FieldAssist; headless stub)                                       |
| `:close`                                                                    | —                                                                  | —                          | Close the document                                                                   |
| `:replace`                                                                  | `path` (string or url)                                             | composition                | Replace media/project from path                                                      |
| `:undo` / `:redo`                                                           | —                                                                  | `boolean`                  | Edit history                                                                         |
| `:cut` / `:copy` / `:paste` / `:clear` / `:remove` / `:duplicate` / `:trim` | —                                                                  | `true`                     | Edit ops on the selection                                                            |
| `:export`                                                                   | profile name, profile userdata, or options table                   | `true`                     | Encode to disk (same pipeline as File → Export); see [field.exports](#fieldexports) |
| `:variable_resolver`                                                        | —                                                                  | VariableResolver           | Composition site lookup (Variables pane); see [field.variables](#fieldvariables) |

**FieldAssist-only methods:** `:break_out_regions()` → composition or array;
`:break_out_channels(channels?)` → composition.

---

## `field.media`

Shared media pool (interned media rows used by compositions).

### field.media Properties

(none)

### field.media Functions

| Function      | Arguments | Returns                   | Description                  |
| ------------- | --------- | ------------------------- | ---------------------------- |
| `shared_pool` | —         | [media pool](#returned-type-media-pool) | Process singleton media pool |
| `open`        | `path` (string or url) | [media](#returned-type-media) | Probe a file without adding it to the pool (detached media) |
| `transcode`   | `media`, `dest`, `profile` [, `on_progress`] | — | Stream-transcode `media` to `dest` (blocks the host) |
| `begin_transcode` | `media`, `dest`, `profile` | [transcode job](#returned-type-transcode-job) | Same encode on a background thread for UI polling |

`field.media.transcode(media, dest, profile [, on_progress])` writes `dest`
without loading the whole source into memory. `profile` may be a registered
profile name, export-profile userdata, or an options table (`encoder`,
`sample_rate`, `sample_format`, `channels`, …) resolved the same way as
`composition:export`. Sample-rate and PCM format from the profile are applied
(bandlimited SRC when rates differ). Optional `on_progress(done, total)`
receives **source** frame counts. Prefer `begin_transcode` from a sheet so
progress can update between `:defer` turns without freezing the UI.

### Returned type: transcode job

Returned by `field.media.begin_transcode`. Poll from a `:defer` loop.

#### transcode job Properties

| Property   | Access | Type      | Description                        |
| ---------- | ------ | --------- | ---------------------------------- |
| `done`     | **ro** | `integer` | Source frames completed            |
| `total`    | **ro** | `integer` | Source frame count (0 until known) |
| `finished` | **ro** | `boolean` | True when the worker has exited    |

#### transcode job Methods

| Method      | Arguments | Returns           | Description                                   |
| ----------- | --------- | ----------------- | --------------------------------------------- |
| `:progress` | —         | `done`, `total`   | Same counters as the fields                   |
| `:error`    | —         | `string` or `nil` | Failure message after `finished`              |
| `:join`     | —         | —                 | Block until done; errors if the worker failed |

### Returned type: media pool

#### media pool Properties

(none)

#### media pool Methods

| Method    | Arguments              | Returns              | Description                            |
| --------- | ---------------------- | -------------------- | -------------------------------------- |
| `:add`    | `path` (string or url) **or** [media](#returned-type-media) | [media](#returned-type-media) | Probe+intern a path, or intern detached media (no re-probe); already-pooled media is idempotent |
| `:remove` | media or id string     | `boolean` (headless) | Remove from the pool when unreferenced |
| `:items`  | —                      | `{ media, … }`       | Current pool rows                      |

### Returned type: media

#### media Properties

| Property           | Access | Type               | Description                                      |
| ------------------ | ------ | ------------------ | ------------------------------------------------ |
| `id`               | **ro** | `string`           | Media id                                         |
| `url`              | **ro** | url userdata       | Portable location (`:as_path()` for native path) |
| `basename`         | **ro** | `string`           | File basename                                    |
| `sample_rate`      | **ro** | `integer`          | Hz                                               |
| `channels`         | **ro** | `integer`          | Channel count                                    |
| `frames`           | **ro** | `integer`          | Frame count                                      |
| `bit_depth`        | **ro** | `integer` or `nil` | Bits per sample when known                       |
| `size_bytes`       | **ro** | `integer`          | File size                                        |
| `modified`         | **ro** | `string`           | Modification time (host formatting)              |
| `container_format` | **ro** | `string`           | Container                                        |
| `codec`            | **ro** | `string`           | Codec                                            |
| `duration`         | **ro** | `number`           | Seconds                                          |

#### media Methods

(none)

---

## `field.layouts`

Channel layout registry used by detect hooks and composition layout choice.

### field.layouts Properties

(none)

### field.layouts Functions

| Function          | Arguments     | Returns                             | Hosts           | Description                                |
| ----------------- | ------------- | ----------------------------------- | --------------- | ------------------------------------------ |
| `define`          | `spec: table` | —                                   | all             | Sugar for `shared_registry():define(spec)` |
| `shared_registry` | —             | [layout registry](#returned-type-layout-registry) | field-scripting | Process layout registry                    |

### field.layouts `define(spec)` keys

| Key           | Required | Type     | Description                                                                  |
| ------------- | -------- | -------- | ---------------------------------------------------------------------------- |
| `name`        | yes      | `string` | Non-empty layout id                                                          |
| `code`        | no       | `string` | Filename-safe token for `${channel_layout}` (default = `name`; empty → `name`) |
| `description` | no       | `string` | Human label (default `""`)                                                   |
| `channels`    | yes      | map      | **0-based** channel index → label string                                     |
| `monitor`     | no       | table    | JSON-compatible monitor table (often `{ chain = "…" }`)                      |

### Returned type: layout registry

#### layout registry Properties

(none)

#### layout registry Methods

| Method    | Arguments             | Returns         | Description                                 |
| --------- | --------------------- | --------------- | ------------------------------------------- |
| `:define` | `spec: table`         | —               | Register or replace a layout by `name`      |
| `:remove` | layout or name string | —               | Drop a registered layout (no-op if missing) |
| `:items`  | —                     | `{ layout, … }` | Registered layouts in definition order      |

### Returned type: layout

#### layout Properties

| Property      | Access | Type           | Description                                              |
| ------------- | ------ | -------------- | -------------------------------------------------------- |
| `name`        | **ro** | `string`       | Layout id                                                |
| `code`        | **ro** | `string`       | Filename-safe token (defaults to `name` when unset)      |
| `description` | **ro** | `string`       | Human label                                              |
| `channels`    | **ro** | map            | **0-based** channel index → label string                 |
| `monitor`     | **ro** | table or `nil` | Monitor table from `define`                              |

#### layout Methods

(none)

---

## `field.exports`

Export profile registry for scripted one-shot encodes (`composition:export`).
Profiles express the same options as the File → Export sheet.

### field.exports Properties

(none)

### field.exports Functions

| Function          | Arguments     | Returns                             | Hosts           | Description                                |
| ----------------- | ------------- | ----------------------------------- | --------------- | ------------------------------------------ |
| `define`          | `spec: table` | —                                   | all             | Sugar for `shared_registry():define(spec)` |
| `shared_registry` | —             | [export registry](#returned-type-export-registry) | field-scripting | Process export profile registry            |

### field.exports `define(spec)` keys

| Key             | Required | Type                      | Description                                                                            |
| --------------- | -------- | ------------------------- | -------------------------------------------------------------------------------------- |
| `name`          | yes      | `string`                  | Non-empty profile id                                                                   |
| `description`   | no       | `string`                  | Human label (default `""`)                                                             |
| `encoder`       | no       | `string`                  | Encoder id (`wav`, `flac`, `ogg`; default `wav` at export)                             |
| `extension`     | no       | `string`                  | Output file extension without a leading dot (default: same as `encoder`)               |
| `sample_format` | no       | `string`                  | PCM label (`S16`, `S24`, `F32`, …); snapped when the encoder stores PCM                |
| `sample_rate`   | no       | `integer`                 | Output Hz (default: composition rate)                                                  |
| `channels`      | no       | `"all"` or `{ indices… }` | **0-based** channel indices (default all)                                              |
| `directory`     | no       | string or url             | Output directory (`${…}` interpolated)                                                 |
| `filename`      | no       | `string`                  | Output filename (`${…}` interpolated)                                                  |
| `path`          | no       | string or url             | Full destination; wins over `directory` + `filename` (`${…}` interpolated)             |
| `variables`     | no       | table                     | Export-scoped variables (same shapes as `session.variables`)                           |
| `metadata`      | no       | `{ [string] = string }`   | Canonical tag templates (`title`, `artist`, …) interpolated then written into the file |

### `composition:export(arg)`

`arg` may be:

- a profile **name** string or profile **userdata**
- an options table: ad-hoc encode settings, and/or `{ profile = name, …overrides }`
- `{ name = "profile", path = "…", … }` when `name` matches a registered profile

Destination requires `path`, or `directory` (with optional `filename`). Returns
`true` on success; raises a Lua error on failure. Encode matches File → Export
(no monitor DSP). Processing chains and background batch jobs are future work.

### Setting resolution

Export settings are resolved in layers. A later layer supplies a value **only
when that field is explicitly set**; omitted fields keep the previous layer.

1. **Source defaults** from the open composition / primary media:

   - `sample_rate` ← composition sample rate
   - `sample_format` ← primary media bit depth (else preference `S24`)
   - `channels` ← all composition channels
   - `encoder` ← product default `"wav"`
   - destination ← unset (caller must supply `path` or `directory`)

2. **Profile** (`field.exports.define` / named profile): each set field
   overrides the source for that field only
3. **Call-time overrides** (inline table keys, or Export-sheet UI edits after
   picking a profile): same sparse overlay as a profile

After merging, the encoder's capabilities snap `sample_format` and validate
rate / channel count. File → Export applies the same source ← profile merge
when a Profile is chosen in the sheet; opening the sheet with **Custom** uses
source ← session `export.*` prefs instead.

The Export sheet also maintains an **export** variable layer: the selected
profile’s `variables` table, plus live Format/channel upserts so path
templates can reference encode settings:

| Leaf | Meaning |
| --- | --- |
| `encoder` | Encoder id |
| `sample_format` | PCM label, or `""` when the encoder does not store PCM |
| `sample_rate` | Output Hz as a string |
| `channels` | Selected channel count as a string |
| `extension` | Encoder file extension |

Export-site compose (source → user → session → composition → export) feeds the
sheet’s Variables disclosure, the soft **Resolved** path preview, and (on
Export) strict path interpolate plus tag map build — matching
`composition:export` / `build_job`.

### Returned type: export registry

#### export registry Properties

(none)

#### export registry Methods

| Method    | Arguments              | Returns          | Description                                  |
| --------- | ---------------------- | ---------------- | -------------------------------------------- |
| `:define` | `spec: table`          | —                | Register or replace a profile by `name`      |
| `:remove` | profile or name string | —                | Drop a registered profile (no-op if missing) |
| `:find`   | `name: string`         | profile or `nil` | Look up a registered profile by name         |
| `:names`  | —                      | `{ string, … }`  | Registered profile names in definition order |
| `:items`  | —                      | `{ profile, … }` | Registered profiles in definition order      |

### Returned type: export profile

#### export profile Properties

| Property        | Access | Type                      | Description       |
| --------------- | ------ | ------------------------- | ----------------- |
| `name`          | **ro** | `string`                  | Profile id        |
| `description`   | **ro** | `string`                  | Human label       |
| `encoder`       | **ro** | `string` or `nil`         | Encoder id        |
| `extension`     | **ro** | `string`                  | File extension (defaults to `encoder`, else `"wav"`) |
| `sample_format` | **ro** | `string` or `nil`         | PCM label         |
| `sample_rate`   | **ro** | `integer` or `nil`        | Hz                |
| `channels`      | **ro** | `"all"` or `{ indices… }` | Channel selection |
| `directory`     | **ro** | `string` or `nil`         | Directory         |
| `filename`      | **ro** | `string` or `nil`         | Filename          |
| `path`          | **ro** | `string` or `nil`         | Full path         |

#### export profile Methods

(none)

---

---

## `field.variables`

Scoped string variables, Bindings userdata, and user-extensible resolvers.
Product contract: [spec/SPEC-metadata.md](spec/SPEC-metadata.md).

### field.variables Functions

| Function            | Arguments                         | Returns   | Description |
| ------------------- | --------------------------------- | --------- | ----------- |
| `create_bindings`   | `scope: string`                   | bindings  | Detached **r/w** Bindings for an arbitrary scope |
| `user`              | —                                 | bindings  | Live **r/w** user-scoped Bindings (host-held / `variables.json`) |
| `create_resolver`   | `{ name = string, … }`            | prototype | Build a resolver prototype (`:name()`) |
| `declare_resolver`  | prototype                         | —         | Register by `:name()`; `"default"` replaces the built-in |
| `set_resolver`      | `name: string`                    | —         | Select the active resolver (process lifetime; startup script) |
| `flatten`           | `{ bindings, … }`                 | table     | Materialize a bindings list with the active resolver (row array + leaf map) |
| `getenv`            | `name: string`                    | string or `nil` | Process environment value (`std::env::var`); used by the default resolver’s `env` scope |
| `load_resolvers`    | —                                 | —         | Load `resolver_*.lua` from the scripting Search Path |

### Returned type: bindings

One scope's leaf name → string map.

| Member / method | Access | Description |
| --------------- | ------ | ----------- |
| `values`        | proxy  | Table-like: `values[name]` get/set string (r/o Bindings reject writes) |
| `:scope()`      | —      | Scope string for this instance |
| `:names()`      | —      | Array of leaf names |
| `:variable_resolver()` | — | User-site VariableResolver (`user.*` only). Only on `field.variables.user()` / `user_scope` Bindings |

`session.variables` and `composition.variables` return live **single-scope**
Bindings for that store only (writes hit the session / `.facomp` file). They do
**not** include probe/`source.*` rows shown in the Variables pane. Composition
site resolution also injects derived `channel_layout` (layout export `code`,
or `""` when unset) into the composition layer for `${channel_layout}` / pane
display; when `code` was omitted at `define` time it equals the layout `name`.
It is not stored in `composition.variables` itself.

### Returned type: VariableResolver

Site-scoped lookup / template expand (detached snapshots of the site bindings).
Obtain via:

| Receiver | Site bindings |
| -------- | ------------- |
| `session:variable_resolver()` | `user.*` + `session` |
| `composition:variable_resolver()` | `source.*` + `user.*` + `session` + `composition` |
| `field.variables.user():variable_resolver()` | all live `user` / `user.*` scopes |

| Method | Arguments | Returns | Description |
| ------ | --------- | ------- | ----------- |
| `:resolve` | `expr: string` [, `expand: boolean`] | `value, scope` or `nil, nil` | Bare leaf (`"title"`) or qualified (`"user.ingest.root_dir"`, `"env.HOME"`, `"source.basename"`). When `expand == true`, soft-expand `${…}` in the resolved value against this site before returning (same soft rules as `:expand`) |
| `:expand` | `template` [, `strict`] | `string` | Soft by default (unresolved `${…}` left as-is); `strict == true` errors like export |

```lua
local c = field.session.focused().composition
local r = c:variable_resolver()
local value, scope = r:resolve("TIMECODE_FLAG")
local value, scope = r:resolve("source.ixml.TIMECODE_FLAG")
local staging, scope = r:resolve("user.ingest.staging_dir", true)  -- expand value
local path = r:expand("${source.parent}/${title}.wav")
```

`flatten` is the low-level bindings-list materializer (rename of the former
`field.variables.resolve`). Prefer site `:variable_resolver()` for lookups and
templates. Migration: `composition:resolve_variable(…)` →
`composition:variable_resolver():resolve(expr)`; `field.variables.resolve` →
`flatten`.

Probe / source Bindings used internally are read-only. Assignment to
`session.variables` / `composition.variables` still accepts map / `{ name, value }`
row tables.

### Resolver prototype

| Method | Contract |
| ------ | -------- |
| `:init(bindings)` | Array of Bindings, host order. Short-lived instance per site |
| `:name()` | Registered name |
| `:resolve(scope, name)` | Returns `value, resolved_scope`. `scope == nil` is composed leaf lookup. Unresolved → `nil, nil` |
| `:names()` | Leaf names to materialize |

Sites pass different lists (same active prototype):

- **Session** — `user`, `session`
- **Composition** — each `source.*` (r/o), then `user`, `session`, `composition`
- **Export** — composition list, then profile `export` bindings

Embedded `resolver_default.lua` (loaded before `init.lua`) declares `"default"`
as last-wins across the `:init` list. It also resolves virtual scope `env` via
`field.variables.getenv` (Rust `std::env::var`, same view as `${env.NAME}`):
`:resolve("env", "HOME")` → value, `"env"` (unset → `nil, nil`). Env names are
not added to `:names()`. Template `${env.NAME}` is substituted by Rust
interpolate against the process environment (templates use the materialized
variable table; `env` is the virtual exception).

```lua
local R = field.variables.create_resolver({ name = "studio" })
function R:init(bindings) self._b = bindings end
function R:names()
  local n = {}
  for _, b in ipairs(self._b) do
    for _, name in ipairs(b:names()) do n[#n + 1] = name end
  end
  n[#n + 1] = "today"
  return n
end
function R:resolve(scope, name)
  if name == "today" then return os.date("%Y-%m-%d"), "computed" end
  -- fall through to last-wins …
end
field.variables.declare_resolver(R)
field.variables.set_resolver("studio")
```

---

## `field.workflow`

Declare and run named workflow prototypes. Prototypes and instances are
**Lua tables** meant to be extended with methods (`:start`, `:suspend`, …).

### field.workflow Properties

(none)

### field.workflow Functions

| Function  | Arguments                                 | Returns                 | Description                                                               |
| --------- | ----------------------------------------- | ----------------------- | ------------------------------------------------------------------------- |
| `create`  | `properties: table`                       | prototype table         | Build an unregistered prototype                                           |
| `declare` | prototype **or** `(properties, function)` | —                       | Register by `name` (one-shot when the second form supplies only `:start`) |
| `run`     | `name` [, `payload`]                      | instance table or `nil` | Start a registered workflow. Default payload `{ scope = "run" }`.         |
| `finish`  | —                                         | —                       | End the active run; calls instance `:finish(session)` if present          |
| `cancel`  | —                                         | —                       | Cancel the active run; calls instance `:cancel(session)` if present       |
| `load_workflows` | —                                    | —                       | Load `workflow_*.lua` from the scripting Search Path                      |

### `create` / `declare` property table

| Key            | Required | Type            | Description                                                                           |
| -------------- | -------- | --------------- | ------------------------------------------------------------------------------------- |
| `name`         | yes      | `string`        | Registry key                                                                          |
| `display_name` | no       | `string`        | UI label (defaults to `name`)                                                         |
| `description`  | no       | `string`        | Short description                                                                     |
| `scopes`       | yes      | `{ string, … }` | e.g. `"drag-drop"`, `"menu"`, `"run"`                                                 |
| `drop`         | no       | table           | Overlay cell: `{ row`, `priority`, `color }` — defaults `row=1`, `priority=1.0`, gray |

### Returned type: workflow prototype / instance (table)

Installed methods (and optional host-called methods) apply to both the
prototype returned by `create` and instances produced by `run` / declare.

#### Properties (installed)

| Property            | Access   | Type     | Description                                   |
| ------------------- | -------- | -------- | --------------------------------------------- |
| `__base_properties` | **ro**   | userdata | See below                                     |
| `__fa_toolbar`      | internal | table    | Snapshot consumed by the host toolbar         |
| user fields         | **rw**   | any      | Free-form instance state (e.g. `self.output`) |

`__base_properties` getters: `name`, `display_name`, `description`, `scopes`,
`row`, `priority`, `color` (**ro**).

#### Methods (installed)

| Method          | Arguments                 | Returns         | Description                                                                         |
| --------------- | ------------------------- | --------------- | ----------------------------------------------------------------------------------- |
| `:name`         | —                         | `string`        | Prototype name                                                                      |
| `:display_name` | —                         | `string`        | Display name                                                                        |
| `:description`  | —                         | `string`        | Description                                                                         |
| `:scopes`       | —                         | `{ string, … }` | Scope list                                                                          |
| `:on`           | `event`, `handler`        | —               | Instance hook; handler gets `(self, …)` then the same args as [field.on](#fieldon) |
| `:set_toolbar`  | `{ control, … }` or `nil` | —               | Replace the toolbar from [field.ui](#fieldui) controls                             |
| `:set_item`     | `id`, `props`             | —               | Merge properties into a control by `id`                                             |
| `:set_sheet`    | `{ title?, panes, current? }` | —           | Open / replace a modal sheet (see below)                                            |
| `:set_pane`     | `id`, `{ text?, controls?, buttons? }` | —  | Merge into one sheet pane                                                           |
| `:go`           | `pane_id`                 | —               | Select the current sheet pane                                                       |
| `:close_sheet`  | —                         | —               | Dismiss the sheet; does not call `:finish` / `:cancel`                              |
| `:defer`        | `function`                | —               | Run `fn(self)` on a later UI frame                                                  |

#### Sheet panes

`panes` is a list of `{ id, name, text?, controls?, buttons? }`. `name` is
the trail label. `text` is the read-only guide. `controls` are body
[field.ui](#fieldui) controls (including sheet-only `progress` / `log`).
`buttons` must be `field.ui.button` only (`align = "left"` for the left
cluster; omit `align` or use `"right"` for the right-justified primary
cluster). Control ids are unique across all panes.

#### Optional user methods (host calls)

| Method    | Signature                                  | When                                              |
| --------- | ------------------------------------------ | ------------------------------------------------- |
| `init`    | `:init()`                                  | New instance                                      |
| `start`   | `:start(payload)`                          | `field.workflow.run` / menu / drop                |
| `suspend` | `:suspend(session)` → `true` \| `false` \| `nil` | Before session save / quit (`nil`/`true` = allow) |
| `resume`  | `:resume(session)`                         | After load when `session.workflow_name` matches   |
| `finish`  | `:finish(session)`                         | `field.workflow.finish`                           |
| `cancel`  | `:cancel(session)`                         | `field.workflow.cancel`                           |

A workflow is **stateful** when the prototype defines `suspend` and/or
`resume`. Only one stateful workflow may bind the session at a time.

---

## `field.ui`

Toolbar control constructors (data tables, not GPUI widgets) plus RGBA
palettes. In FieldAssist, `field.ui.named` and `field.ui.semantic` are
overridden after the shared namespace is bound, so every named and semantic
color is read from the live GPUI theme. Headless hosts use the portable
defaults.

### field.ui Properties

| Property   | Access | Type             | Description                                                   |
| ---------- | ------ | ---------------- | ------------------------------------------------------------- |
| `named`    | **ro** | palette userdata | Live GPUI named colors in FieldAssist; defaults headlessly    |
| `semantic` | **ro** | palette userdata | Live GPUI semantic colors in FieldAssist; defaults headlessly |

### field.ui Functions

| Function     | Arguments        | Returns       | Description                     |
| ------------ | ---------------- | ------------- | ------------------------------- |
| `button`     | `props: table`   | control table | Push-button                     |
| `toggle`     | `props: table`   | control table | On/off toggle                   |
| `message`    | `props: table`   | control table | Static / updatable text         |
| `text_entry` | `props: table`   | control table | Single-line text field          |
| `path_entry` | `props: table`   | control table | Path field with optional browse |
| `select`     | `props: table`   | control table | Dropdown of fixed choices       |
| `divider`    | `props` or `nil` | control table | Visual separator                |
| `progress`   | `props: table`   | control table | Sheet-only progress bar/circle  |
| `log`        | `props: table`   | control table | Sheet-only scrolling log        |

Controls are tables with a control metatable and a `kind` field. Optional
`action = function(control, workflow)` runs on click / commit. Assigning
watched props (`label`, `value`, `text`, `color`, `on_color`, `off_color`,
`align`, `icon`, `on_icon`, `choices`, `enabled`, `loading`, `variant`)
refreshes the host toolbar or sheet. `progress` and `log` are rejected by
`:set_toolbar`.

### Returned type: control table

#### Common properties

| Property | Access | Type                 | Description                         |
| -------- | ------ | -------------------- | ----------------------------------- |
| `kind`   | **ro** | `string`             | Set by the constructor              |
| `id`     | **rw** | `string`             | Stable id (required for most kinds) |
| `align`  | **rw** | `"left"` \| `"right"` | Toolbar / button-bar alignment (default left) |
| `enabled`| **rw** | `boolean`            | Interactive controls (default true) |
| `action` | **rw** | `function` or `nil`  | `function(control, workflow)`       |

#### Per-kind properties

| Kind           | Properties                                                                                                                                                                   |
| -------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **button**     | `id?`, `label?` (one of id/label required), `icon?` (`check`, `circle-check` / `circle_check`, `circle-x`, `circle-alert`, `arrow-left`, `arrow-right`), `align?`, `enabled?`, `action?` |
| **toggle**     | `id` (required), `label?` (defaults to id), `value?` bool (default `false`), `on_color?`, `off_color?`, `on_icon?` (default `"check"`), `align?`, `enabled?`, `action?`                  |
| **message**    | `id` (required), `text` (required string; may be `""`), `color?`, `align?`                                                                                                   |
| **text_entry** | `id` (required), `label?`, `value` (string, default `""`), `align?`, `enabled?`, `action?`                                                                                               |
| **path_entry** | `id` (required), `label?`, `value` (string), `browse?` (`"file"` \| `"directory"` \| `true` (=directory) \| `false` \| `nil`), `align?`, `enabled?`, `action?`                               |
| **select**     | `id` (required), `label?`, `value` (selected choice id, default `""`), `choices` (array of strings or `{ value, label? }`), `align?`, `enabled?`, `action?`                                 |
| **divider**    | `id?`, `align?`                                                                                                                                                              |
| **progress**   | `id` (required), `label?`, `variant?` (`"bar"` default \| `"circle"`), `value?` (0–100), `loading?` bool, `text?` caption. Omit `value` or set `loading` for indeterminate |
| **log**        | `id` (required), `label?`, `text?`. Method `:append(line)` appends a line (cap 1000)                                                                                          |

### Palette: `field.ui.named`

| Property                                                                                                                                     | Access | Type             |
| -------------------------------------------------------------------------------------------------------------------------------------------- | ------ | ---------------- |
| `red`, `red_light`, `green`, `green_light`, `blue`, `blue_light`, `yellow`, `yellow_light`, `magenta`, `magenta_light`, `cyan`, `cyan_light` | **ro** | `{ r, g, b, a }` |

### Palette: `field.ui.semantic`

| Property                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   | Access | Type             |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------ | ---------------- |
| `accent`, `accent_foreground`, `background`, `border`, `danger`, `danger_active`, `danger_foreground`, `danger_hover`, `drop_target`, `foreground`, `info`, `info_active`, `info_foreground`, `info_hover`, `input`, `link`, `link_active`, `link_hover`, `muted`, `muted_foreground`, `popover`, `popover_foreground`, `primary`, `primary_active`, `primary_foreground`, `primary_hover`, `ring`, `secondary`, `secondary_active`, `secondary_foreground`, `secondary_hover`, `selection`, `success`, `success_active`, `success_foreground`, `success_hover`, `warning`, `warning_active`, `warning_foreground`, `warning_hover`, `chart_1`…`chart_5`, `chart_bullish`, `chart_bearish` | **ro** | `{ r, g, b, a }` |

---

## `field.audio_devices`

Enumerate playback output devices (via `field-audio-playback`).

### field.audio_devices Properties

(none)

### field.audio_devices Functions

| Function | Arguments | Returns         | Description                         |
| -------- | --------- | --------------- | ----------------------------------- |
| `list`   | —         | `{ device, … }` | Output devices (may be empty in CI) |

Each device table has:

| Property        | Type               | Description                  |
| --------------- | ------------------ | ---------------------------- |
| `index`         | `integer`          | Host device list index       |
| `name`          | `string`           | Display name                 |
| `is_default`    | `boolean`          | Host default output          |
| `sample_rate`   | `integer` or `nil` | Default config Hz            |
| `channels`      | `integer` or `nil` | Default config channel count |
| `sample_format` | `string` or `nil`  | e.g. `F32`, `I16`            |

**Index:** field-scripting uses the device’s native `index`. FieldAssist’s
`field.audio_devices.list` currently returns `{ index, name }` only
(1-based `index`).

---

## Nested types

### collection

Region list returned by `composition.selection` or `composition:collection(name)`.

#### collection Properties

| Property  | Access | Type            | Description                |
| --------- | ------ | --------------- | -------------------------- |
| `name`    | **ro** | `string`        | `"selection"` or user name |
| `regions` | **ro** | `{ region, … }` | Regions in the collection  |

`#collection` is the region count.

#### collection Methods

(none)

### region

#### region Properties

| Property     | Access | Type                    | Description                           |
| ------------ | ------ | ----------------------- | ------------------------------------- |
| `id`         | **ro** | `integer`               | Region id                             |
| `start`      | **ro** | `integer`               | Start sample                          |
| `stop`       | **ro** | `integer`               | End sample (inclusive span as stored) |
| `channels`   | **ro** | `"all"` or `{ int, … }` | Channel mask                          |
| `label`      | **ro** | `string` or `nil`       | Optional label                        |
| `collection` | **ro** | `string`                | Owning collection name                |

#### region Methods

(none)

### marker
