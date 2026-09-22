# Scripting

As-built Lua 5.4 reference for FieldAssist, **field-batch**, and **field-play**.
Hosts share the `field-scripting` API shape under the global **`field`**
namespace. The global **`app`** object is a thin host facade; `app.name`
identifies the process.

This document describes **what is implemented today**. Intended-but-missing
APIs (confirm dialogs, SQLite, C2PA, handoff) live in
[spec/SPEC-field-recording.md](spec/SPEC-field-recording.md). Copyable pipeline
examples are under
[examples/workflows/field-recording/](examples/workflows/field-recording/).

## Table of contents

1. [Hosts and scope](#hosts-and-scope)
2. [Initialization](#initialization)
3. [Conventions](#conventions)
4. [app (host-only)](#app-host-only)
5. [field.log](#field-log)
6. [field.on](#field-on)
7. [field.include](#field-include)
8. [field.scripting](#field-scripting)
9. [field.url](#field-url)
10. [field.fs](#field-fs)
11. [field.session](#field-session)
12. [field.composition](#field-composition)
13. [field.media](#field-media)
14. [field.layouts](#field-layouts)
15. [field.workflow](#field-workflow)
16. [field.ui](#field-ui)
17. [field.audio_devices](#field-audio-devices)
18. [Nested types](#nested-types)
19. [Migration from app:*](#migration-from-app)

## Hosts and scope

| Host | `app.name` | Runtime |
| --- | --- | --- |
| FieldAssist desktop | `"field-assist"` | GPUI app; embedded Add / Replace / Review workflows |
| field-batch CLI | `"field-batch"` | Headless REPL / script / Unix shebang over `field-scripting` |
| field-play CLI | `"field-play"` | Headless playback; `init.lua` + one-shot `detect_layout` |

```bash
field-batch                     # interactive REPL
field-batch script.lua a b      # app.args = { "a", "b" }
#!/usr/bin/env field-batch      # script path is argv[1]
field-play take.wav             # init.lua → detect_layout → play
field-play --config-dir DIR …   # optional init override
```

**Host coverage.** The `field.*` surface is shared by `field-scripting` and is
fully available in FieldAssist. FieldAssist supplies a desktop
`ScriptBackend`, its host-only `app` facade, and GPUI toolbar paint glue.
field-play uses the headless backend only long enough to load init and fire
`detect_layout`; it does not run workflows or a REPL.

| Module | field-batch / field-scripting | field-play | FieldAssist today |
| --- | --- | --- | --- |
| `field.log` / `field.on` | full | init + detect | full |
| `field.include` | path or `field.url`; cache + search stack | via init | full |
| `field.scripting` | full | via init | full |
| `field.url` | full | via init | full |
| `field.fs` | full | via init | full |
| `field.session` | `focused` / `new` / `open` | open via host | `focused` / `new` / `open` |
| `field.composition` | baseline userdata | baseline | full, plus desktop chrome fields/methods |
| `field.media` / `field.workflow` / `field.ui` | full | unused at play | full |
| `field.layouts` | `define` + `shared_registry` | via init | `define` + `shared_registry` |
| `field.audio_devices` | full | unused at play | full |

## Initialization

### FieldAssist

1. `init.lua` — user config file if present, else embedded default
   (`field_scripting::EMBEDDED_INIT`)
2. Embedded `workflow_add.lua`, `workflow_replace.lua`, `workflow_review.lua`
3. User `workflow_*.lua` next to `init.lua` (sorted by name)

| OS | Config directory |
| --- | --- |
| macOS | `~/Library/Application Support/FieldAssist/` |
| Windows | `%APPDATA%\FieldAssist\` |
| Linux | `$XDG_CONFIG_HOME/FieldAssist/` or `~/.config/FieldAssist/` |

Resolved by `field_scripting::user_config_dir()` (shared by FieldAssist,
field-batch, and field-play). Dump the embedded default with
`FieldAssist --dump-init`.

`require` search paths default to this config directory plus the process
cwd (see [field.scripting](#field-scripting)). Call
`field.scripting.enable_*` at the top of `init.lua` when a script needs
system Lua paths or native C modules.

### field-batch

Optional `--config-dir` (default: same FieldAssist config directory). Loads
user `init.lua` if present, else the shared embedded default. Does **not**
auto-load Add / Replace / Review.

### field-play

Optional `--config-dir` (default: same FieldAssist config directory). Loads
user `init.lua` if present, else the shared embedded default (layouts +
`detect_layout`). Does **not** load workflow bundles. After opening the
path, fires `detect_layout` once so unset `monitor_chain` values pick up
the layout default before playback.

## Conventions

- **Access:** **ro** = read-only from Lua; **rw** = get and set.
- **Colors:** RGBA tables are 1-based `{ r, g, b, a }` with components in
  `0.0`…`1.0`.
- **Channels:** where a channels value is accepted: `nil` / `"all"` / a
  0-based index array.
- **Paths:** native strings, and (in field-scripting) `field.url` userdata
  where noted.
- **Workflow UI:** `field.ui.*` constructors build control tables.
  FieldAssist renders them; field-batch stores them and ignores paint.

---

<a id="app-host-only"></a>

## `app` (host-only)

Process facade unique to the enclosing binary. Model constructors and shared
APIs live under `field.*`, not here.

### Properties

| Property | Access | Hosts | Type | Description |
| --- | --- | --- | --- | --- |
| `name` | **ro** | all | `string` | Stable process id (`"field-assist"` / `"field-batch"` / `"field-play"`) |
| `args` | **ro** | field-batch | `{ string, … }` | Script argv after the file / `--` |
| `workflow` | **ro** | all | `table` or `nil` | Active workflow instance |
| `theme` | **ro** | FieldAssist | theme userdata | Live GPUI theme (see below) |
| `themes` | **ro** | FieldAssist | `{ string, … }` | Registered theme names |
| `looping` | **ro** | FieldAssist | `boolean` | Transport loop flag |
| `preview` | **ro** | FieldAssist | `boolean` | Preview playback flag |
| `explorer` | **ro** | FieldAssist | `boolean` | Explorer dock open |
| `output_device` | **rw** | FieldAssist | `string` or `nil` | Preferred output device (substring match) |
| `output_devices` | **ro** | FieldAssist | `{ string, … }` | Known output device names |

### Functions / methods

| Function | Arguments | Returns | Hosts | Description |
| --- | --- | --- | --- | --- |
| `app:alert` | `subject`, `body` | — | all | Host alert (dialog / stderr). Args are stringified. |
| `app:command` | `id: string` | — | FieldAssist | Invoke a UI command (e.g. `"view.show-explorer"`) |

field-scripting also keeps temporary migration shims
`app:info` / `app:warn` / `app:error` and
`app.finish_workflow` / `app.cancel_workflow` / `app.run_workflow`; prefer
`field.log` and `field.workflow`.

### `app.theme` (FieldAssist)

| Property | Access | Type | Description |
| --- | --- | --- | --- |
| `name` | **rw** | `string` | Theme registry name |
| `mode` | **rw** | `"light"` \| `"dark"` | Appearance mode |
| `named` | **ro** | palette userdata | Same keys as [field.ui.named](#field-ui) |
| `semantic` | **ro** | palette userdata | Same keys as [field.ui.semantic](#field-ui) |

```lua
if app.name == "field-assist" and app.command then
  app:command("view.show-explorer")
end
```

---

<a id="field-log"></a>

## `field.log`

Structured logging into the host log sink (Script panel / stderr / Messages).

### Properties

*(none)*

### Functions

| Function | Arguments | Returns | Description |
| --- | --- | --- | --- |
| `info` | `topic`, `message` | — | Informational line |
| `warn` | `topic`, `message` | — | Warning line |
| `error` | `topic`, `message` | — | Error line |

On FieldAssist, `topic` / `message` may be any Lua values (stringified). On
field-batch they are strings.

---

<a id="field-on"></a>

## `field.on`

Registers process-wide model/host hooks. Unknown event names error.

### Properties

*(none)*

### Functions

| Function | Arguments | Returns | Description |
| --- | --- | --- | --- |
| `on` | `event: string`, `callback: function` | — | Append a hook for `event` |

### Events

| Event | Callback arguments | Notes |
| --- | --- | --- |
| `loaded` | `composition`, `elapsed` | `elapsed` is seconds since open started |
| `saved` | `composition`, `elapsed` | |
| `detect_layout` | `composition`, `chosen` → `string` or `nil` | `chosen` is the current layout name or `nil`; return a registered layout name |
| `session_loaded` | `session` | |
| `session_saved` | `session` | |
| `session_selected` | `session` | |
| `composition_selected` | `composition` or `nil` | |

Workflow prototypes can also `:on` the same events; those handlers receive the
**instance** as the first argument (see [field.workflow](#field-workflow)).

---

<a id="field-include"></a>

## `field.include`

Load and run another Lua chunk relative to the caller (replaces
`debug.getinfo` + `dofile`).

### Properties

*(none)*

### Functions

| Function | Arguments | Returns | Description |
| --- | --- | --- | --- |
| `include` | `spec` | chunk return value(s) | Load and execute a Lua file |

**`spec` (field-scripting):** path `string` or [field.url](#field-url) userdata.
Search: absolute path → directory of the calling file → config dir → relative
to cwd. Results are cached by resolved path while the host lives.

**`spec` (FieldAssist today):** path `string` only; `lua.load` of that path
(no URL, no shared include cache).

---

<a id="field-scripting"></a>

## `field.scripting`

Host controls for Lua `require` search paths and native C modules. Paths are
locked down so workflows stay self-contained by default.

### Properties

*(none)*

### Default `package.path` / `package.cpath`

| Kind | Templates |
| --- | --- |
| Lua | `{config}/?.lua`, `{config}/?/init.lua`, `./?.lua`, `./?/init.lua` |
| C | `{config}/?.so` (`.dll` on Windows), `./?.so` / `./?.dll` |

`{config}` is the host config directory when known. System-wide bundled Lua
locations (`/usr/local/...`, Windows `!\\...`) and foreign `LUA_PATH*` /
`LUA_CPATH*` roots are **not** on the path until opted in. C loaders remain
stubbed until opted in even if a `.so` sits on `package.cpath`.

Prefer [`field.include`](#field-include) for FieldAssist/field-batch Lua that
should resolve relative to the caller or config dir.

### Functions

| Function | Arguments | Returns | Description |
| --- | --- | --- | --- |
| `enable_system_package_paths` | — | — | Append the saved system-wide path/cpath templates (idempotent) |
| `enable_native_modules` | — | — | Restore `package.loadlib` and C searchers (idempotent) |

Either call is independent. Typical place is the top of user `init.lua`:

```lua
field.scripting.enable_system_package_paths()
field.scripting.enable_native_modules()
```

---

<a id="field-url"></a>

## `field.url`

Location userdata over the Rust `url` crate plus `field-core` file /
`memory://` URLs. **Bound in field-scripting / field-batch only** (not yet on
FieldAssist).

### Properties

*(none on the module table)*

### Functions

| Function | Arguments | Returns | Description |
| --- | --- | --- | --- |
| `parse` | `input: string` | url userdata | Parse a path, `file://`, relative, or `memory://` string |
| `from_path` | `path: string` | url userdata | Encode a filesystem path |
| `from_file_path` | `path: string` | url userdata | Alias of `from_path` |

### Returned type: url userdata

#### Properties

| Property | Access | Type | Description |
| --- | --- | --- | --- |
| `scheme` | **ro** | `string` | e.g. `file`, `memory`, or URL scheme |
| `host` | **ro** | `string` or `nil` | Host component when present |
| `path` | **ro** | `string` | Path component |
| `query` | **ro** | `string` or `nil` | Query string |
| `native` | **ro** | `string` | Native filesystem path when applicable |
| `normalized` | **ro** | `string` | Canonical serialized form |
| `parent` | **ro** | url userdata | Parent directory (filesystem) |
| `basename` | **ro** | `string` | Final path segment |
| `stem` | **ro** | `string` | Basename without extension |
| `extension` | **ro** | `string` | Extension without leading `.` |

#### Methods

| Method | Arguments | Returns | Description |
| --- | --- | --- | --- |
| `:join(...)` | one or more path segments | url userdata | Append path segments |
| `:with_extension` | `ext: string` | url userdata | Replace extension (leading `.` optional) |
| `:relative_to` | `base` (url or string) | `string` | Relative path from `base` |
| `__tostring` | — | `string` | Stored raw form |

---

<a id="field-fs"></a>

## `field.fs`

Filesystem helpers. Paths accept a `string` or (in field-scripting) url
userdata.

### Properties

*(none)*

### Functions

| Function | Arguments | Returns | Hosts | Description |
| --- | --- | --- | --- | --- |
| `find_files` | `dir [, filter]` | `{ string, … }` | all | Recursive listing of paths **relative to `dir`**, `/`-separated. `filter` = `nil`, extension list `{ "wav", … }`, or `function(dirname, basename) → truthy`. |
| `mkdir` | `path [, { recursive = bool }]` | — | field-scripting | Create directory. Default `recursive = true`. |
| `remove` | `path [, { recursive = bool }]` | — | field-scripting | Remove file or directory. Default `recursive = false`. |
| `copy` | `src`, `dest` [, `{ recursive = bool }`] | — | field-scripting | Copy file, or directory when `recursive = true`. |
| `exists` | `path` | `boolean` | field-scripting | Path exists |
| `stat` | `path` | `{ size, is_dir, is_file, mtime? }` | field-scripting | Metadata; `mtime` is unix seconds when available |
| `checksum` | `path [, algo]` | `string` | field-scripting | Hex digest; only `"blake3"` (default) |

FieldAssist currently binds **`find_files` only**.

---

<a id="field-session"></a>

## `field.session`

Constructors and accessors for session objects (`.fasession` membership,
groups, properties, open compositions).

### Properties

*(none on the module table)*

### Functions

| Function | Arguments | Returns | Hosts | Description |
| --- | --- | --- | --- | --- |
| `focused` | — | session | all | Process / UI-active session |
| `new` | — | session | all | Create an empty detached session |
| `open` | `path: string` | session | all | Load a `.fasession` as a detached session |

`field.session.focused()` is the host's current world/UI session. In contrast,
`field.session.new()` and `field.session.open(path)` create or load detached
sessions: they do not replace the active FieldAssist session, do not open
their documents in the desktop, and are intended for manifest inspection or
metadata work. To load a `.fasession` into the focused desktop session, call
`field.session.focused():open(path)`.

### Returned type: session

#### Properties

| Property | Access | Type | Description |
| --- | --- | --- | --- |
| `id` | **ro** | `string` | Session id |
| `path` | **ro** | `string` or `nil` | On-disk `.fasession` path |
| `workflow_name` | **rw** | `string` or `nil` | Persisted stateful workflow name |
| `capture_ui` | **rw** | `boolean` | Whether UI chrome is captured with the session |
| `properties` | **rw** | `{ [string] = string }` | String map |
| `composition` | **rw** | composition or `nil` | Active document |
| `compositions` | **ro** | `{ composition, … }` | Session documents in order |
| `groups` | **rw** | `{ string, … }` | Group name list |

#### Methods

| Method | Arguments | Returns | Description |
| --- | --- | --- | --- |
| `:group_count` | `group: string` | `integer` | Documents in `group` |
| `:add_group` | `name: string` | — | Append a group |
| `:rename_group` | `old`, `new` | — | Rename a group |
| `:delete_group` | `name: string` | — | Remove a group |
| `:move_group` | `name`, `index` | — | Reorder group (`index` clamped ≥ 0) |
| `:move` | `composition`, `index` | — | Reorder a document |
| `:open` | `path: string` | composition | Open media / `.facomp` into the session (not `.fasession`) |
| `:save` | — | — | Save session (FieldAssist; headless stub errors) |
| `:save_as` | `path: string` | — | Save session to a new path |
| `:close` | — | — | Close / detach (FieldAssist; headless stub errors) |

---

<a id="field-composition"></a>

## `field.composition`

Open compositions outside of (or in addition to) a session. Session membership
still uses `session:open`.

### Properties

*(none on the module table)*

### Functions

| Function | Arguments | Returns | Description |
| --- | --- | --- | --- |
| `open` | `path: string` | composition | Open audio or `.facomp` |

### Returned type: composition

#### Properties

| Property | Access | Type | Description |
| --- | --- | --- | --- |
| `name` | **rw** | `string` | Display name |
| `path` | **ro** | `string` or `nil` | Source / project path |
| `id` | **ro** | `string` | Composition id |
| `group` | **rw** | `string` or `nil` | Session group membership |
| `state` | **rw** | `string` or `nil` | Free-form document state |
| `properties` | **rw** | `{ [string] = string }` | String map |
| `frames` | **ro** | `integer` | Timeline length in samples |
| `sample_rate` | **ro** | `integer` | Hz |
| `channels` | **ro** | `integer` | Channel count |
| `selection` | **rw** | [collection](#collection) or selection table | Primary selection; set `nil`, a collection, or `{ kind, start, stop, channels }` |
| `position` | **rw** | `integer` or `nil` | Playhead sample |
| `collections` | **ro** | `{ string, … }` | Named region collection names |
| `markers` | **ro** | `{ marker, … }` | Marker list |
| `marker_types` | **ro** | `{ { name, color }, … }` | Registered marker types |

**FieldAssist-only properties:** `parent`, `children`, `codec`, `bit_depth`,
`basename`, `dirname`, `channel_layout` (**rw**), `monitor_chain` (**rw**),
`playback_channels` (**rw**), `source_channels` (**ro**), `duration` (**ro**),
`regions` (**ro** — selection as region userdata array).

**Selection table setter:** `kind` = `"none"` \| `"position"` \| `"region"`;
`start` / `stop` samples; `channels` as above.

#### Methods

| Method | Arguments | Returns | Description |
| --- | --- | --- | --- |
| `:select` | `start`, `stop` [, `channels`] | — | Select a half-open sample range |
| `:select_all` | — | — | Select the whole timeline |
| `:clear_selection` | — | — | Clear selection |
| `:collection` | `name: string` | [collection](#collection) | Named region collection |
| `:add_region` | `{ start, stop, channels?, label?, collection? }` | [region](#region) | Add a labeled region |
| `:remove_region` | `id: integer` | `boolean` | Remove by id |
| `:add_marker` | frame[, type] **or** `{ frame\|sample, type\|kind?, color?, note? }` | [marker](#marker) or `nil` | Add a marker |
| `:remove_marker` | marker or id | `boolean` | Remove one marker |
| `:remove_marker_at` | `frame` [, `type`] | `boolean` | Remove at sample |
| `:remove_marker_by_type` | `type: string` | `boolean` | Remove all of a type |
| `:marker_at` | `frame` [, `type`] | marker or `nil` | Lookup |
| `:add_marker_type` | `name`, `color` | `boolean` | Register a type color |
| `:remove_marker_type` | `name` | `boolean` | Unregister a type |
| `:save` | — | — | Persist `.facomp` (FieldAssist; headless stub) |
| `:close` | — | — | Close the document |
| `:replace` | `path: string` | composition | Replace media/project from path |
| `:undo` / `:redo` | — | `boolean` | Edit history |
| `:cut` / `:copy` / `:paste` / `:clear` / `:remove` / `:duplicate` / `:trim` | — | `true` | Edit ops on the selection |

**FieldAssist-only methods:** `:break_out_regions()` → composition or array;
`:break_out_channels(channels?)` → composition.

---

<a id="field-media"></a>

## `field.media`

Shared media pool (interned media rows used by compositions).

### Properties

*(none)*

### Functions

| Function | Arguments | Returns | Description |
| --- | --- | --- | --- |
| `shared_pool` | — | [media pool](#media-pool) | Process singleton media pool |

<a id="media-pool"></a>

### Returned type: media pool
#### Properties

*(none)*

#### Methods

| Method | Arguments | Returns | Description |
| --- | --- | --- | --- |
| `:add` | `path: string` | [media](#media) | Probe and intern a file |
| `:remove` | media or id string | `boolean` (headless) | Remove from the pool when unreferenced |
| `:list` | — | `{ media, … }` | Current pool rows |

<a id="media"></a>

### Returned type: media
#### Properties

| Property | Access | Type | Description |
| --- | --- | --- | --- |
| `id` | **ro** | `string` | Media id |
| `url` | **ro** | `string` | Canonical URL |
| `path` | **ro** | `string` | Native path |
| `basename` | **ro** | `string` | File basename |
| `sample_rate` | **ro** | `integer` | Hz |
| `channels` | **ro** | `integer` | Channel count |
| `frames` | **ro** | `integer` | Frame count |
| `bit_depth` | **ro** | `integer` or `nil` | Bits per sample when known |
| `size_bytes` | **ro** | `integer` | File size |
| `modified` | **ro** | `string` | Modification time (host formatting) |
| `container_format` | **ro** | `string` | Container |
| `codec` | **ro** | `string` | Codec |
| `duration` | **ro** | `number` | Seconds |

#### Methods

*(none)*

---

<a id="field-layouts"></a>

## `field.layouts`

Channel layout registry used by detect hooks and composition layout choice.

### Properties

*(none)*

### Functions

| Function | Arguments | Returns | Hosts | Description |
| --- | --- | --- | --- | --- |
| `define` | `spec: table` | — | all | Register or replace a layout by `name` |
| `shared_registry` | — | `{ string, … }` | field-scripting | Registered layout names |

### `define(spec)` keys

| Key | Required | Type | Description |
| --- | --- | --- | --- |
| `name` | yes | `string` | Non-empty layout id |
| `description` | no | `string` | Human label (default `""`) |
| `channels` | yes | map | **0-based** channel index → label string |
| `monitor` | no | table | JSON-compatible monitor table (often `{ chain = "…" }`) |

---

<a id="field-workflow"></a>

## `field.workflow`

Declare and run named workflow prototypes. Prototypes and instances are
**Lua tables** meant to be extended with methods (`:start`, `:suspend`, …).

### Properties

*(none)*

### Functions

| Function | Arguments | Returns | Description |
| --- | --- | --- | --- |
| `create` | `properties: table` | prototype table | Build an unregistered prototype |
| `declare` | prototype **or** `(properties, function)` | — | Register by `name` (one-shot when the second form supplies only `:start`) |
| `run` | `name` [, `payload`] | instance table or `nil` | Start a registered workflow. Default payload `{ scope = "run" }`. |
| `finish` | — | — | End the active run; calls instance `:finish(session)` if present |
| `cancel` | — | — | Cancel the active run; calls instance `:cancel(session)` if present |

### `create` / `declare` property table

| Key | Required | Type | Description |
| --- | --- | --- | --- |
| `name` | yes | `string` | Registry key |
| `display_name` | no | `string` | UI label (defaults to `name`) |
| `description` | no | `string` | Short description |
| `scopes` | yes | `{ string, … }` | e.g. `"drag-drop"`, `"menu"`, `"run"` |
| `drop` | no | table | Overlay cell: `{ row`, `priority`, `color }` — defaults `row=1`, `priority=1.0`, gray |

### Returned type: workflow prototype / instance (table)

Installed methods (and optional host-called methods) apply to both the
prototype returned by `create` and instances produced by `run` / declare.

#### Properties (installed)

| Property | Access | Type | Description |
| --- | --- | --- | --- |
| `__base_properties` | **ro** | userdata | See below |
| `__fa_toolbar` | internal | table | Snapshot consumed by the host toolbar |
| user fields | **rw** | any | Free-form instance state (e.g. `self.output`) |

`__base_properties` getters: `name`, `display_name`, `description`, `scopes`,
`row`, `priority`, `color` (**ro**).

#### Methods (installed)

| Method | Arguments | Returns | Description |
| --- | --- | --- | --- |
| `:name` | — | `string` | Prototype name |
| `:display_name` | — | `string` | Display name |
| `:description` | — | `string` | Description |
| `:scopes` | — | `{ string, … }` | Scope list |
| `:on` | `event`, `handler` | — | Instance hook; handler gets `(self, …)` then the same args as [field.on](#field-on) |
| `:set_toolbar` | `{ control, … }` or `nil` | — | Replace the toolbar from [field.ui](#field-ui) controls |
| `:set_item` | `id`, `props` | — | Merge properties into a control by `id` |

#### Optional user methods (host calls)

| Method | Signature | When |
| --- | --- | --- |
| `init` | `:init()` | New instance |
| `start` | `:start(payload)` | `field.workflow.run` / menu / drop |
| `suspend` | `:suspend(session)` → `true`\|`false`\|`nil` | Before session save / quit (`nil`/`true` = allow) |
| `resume` | `:resume(session)` | After load when `session.workflow_name` matches |
| `finish` | `:finish(session)` | `field.workflow.finish` |
| `cancel` | `:cancel(session)` | `field.workflow.cancel` |

A workflow is **stateful** when the prototype defines `suspend` and/or
`resume`. Only one stateful workflow may bind the session at a time.

---

<a id="field-ui"></a>

## `field.ui`

Toolbar control constructors (data tables, not GPUI widgets) plus RGBA
palettes. In FieldAssist, `field.ui.named` and `field.ui.semantic` are
overridden after the shared namespace is bound, so every named and semantic
color is read from the live GPUI theme. Headless hosts use the portable
defaults.

### Properties

| Property | Access | Type | Description |
| --- | --- | --- | --- |
| `named` | **ro** | palette userdata | Live GPUI named colors in FieldAssist; defaults headlessly |
| `semantic` | **ro** | palette userdata | Live GPUI semantic colors in FieldAssist; defaults headlessly |

### Functions

| Function | Arguments | Returns | Description |
| --- | --- | --- | --- |
| `button` | `props: table` | control table | Push-button |
| `toggle` | `props: table` | control table | On/off toggle |
| `message` | `props: table` | control table | Static / updatable text |
| `text_entry` | `props: table` | control table | Single-line text field |
| `path_entry` | `props: table` | control table | Path field with optional browse |
| `divider` | `props` or `nil` | control table | Visual separator |

Controls are tables with a control metatable and a `kind` field. Optional
`action = function(control, workflow)` runs on click / commit. Assigning
watched props (`label`, `value`, `text`, `color`, `on_color`, `off_color`,
`align`, `icon`, `on_icon`) refreshes the host toolbar.

### Returned type: control table

#### Common properties

| Property | Access | Type | Description |
| --- | --- | --- | --- |
| `kind` | **ro** | `string` | Set by the constructor |
| `id` | **rw** | `string` | Stable id (required for most kinds) |
| `align` | **rw** | `"left"` \| `"right"` | Toolbar alignment (default left) |
| `action` | **rw** | `function` or `nil` | `function(control, workflow)` |

#### Per-kind properties

| Kind | Properties |
| --- | --- |
| **button** | `id?`, `label?` (one of id/label required), `icon?` (`check`, `circle-check` / `circle_check`, `circle-x`, `circle-alert`, `arrow-left`, `arrow-right`), `align?`, `action?` |
| **toggle** | `id` (required), `label?` (defaults to id), `value?` bool (default `false`), `on_color?`, `off_color?`, `on_icon?` (default `"check"`), `align?`, `action?` |
| **message** | `id` (required), `text` (required string; may be `""`), `color?`, `align?` |
| **text_entry** | `id` (required), `label?`, `value` (string, default `""`), `align?`, `action?` |
| **path_entry** | `id` (required), `label?`, `value` (string), `browse?` (`"file"` \| `"directory"` \| `true` (=directory) \| `false` \| `nil`), `align?`, `action?` |
| **divider** | `id?`, `align?` |

### Palette: `field.ui.named`

| Property | Access | Type |
| --- | --- | --- |
| `red`, `red_light`, `green`, `green_light`, `blue`, `blue_light`, `yellow`, `yellow_light`, `magenta`, `magenta_light`, `cyan`, `cyan_light` | **ro** | `{ r, g, b, a }` |

### Palette: `field.ui.semantic`

| Property | Access | Type |
| --- | --- | --- |
| `accent`, `accent_foreground`, `background`, `border`, `danger`, `danger_active`, `danger_foreground`, `danger_hover`, `drop_target`, `foreground`, `info`, `info_active`, `info_foreground`, `info_hover`, `input`, `link`, `link_active`, `link_hover`, `muted`, `muted_foreground`, `popover`, `popover_foreground`, `primary`, `primary_active`, `primary_foreground`, `primary_hover`, `ring`, `secondary`, `secondary_active`, `secondary_foreground`, `secondary_hover`, `selection`, `success`, `success_active`, `success_foreground`, `success_hover`, `warning`, `warning_active`, `warning_foreground`, `warning_hover`, `chart_1`…`chart_5`, `chart_bullish`, `chart_bearish` | **ro** | `{ r, g, b, a }` |

---

<a id="field-audio-devices"></a>

## `field.audio_devices`

Enumerate playback output devices (via `field-audio-playback`).

### Properties

*(none)*

### Functions

| Function | Arguments | Returns | Description |
| --- | --- | --- | --- |
| `list` | — | `{ device, … }` | Output devices (may be empty in CI) |

Each device table has:

| Property | Type | Description |
| --- | --- | --- |
| `index` | `integer` | Host device list index |
| `name` | `string` | Display name |
| `is_default` | `boolean` | Host default output |
| `sample_rate` | `integer` or `nil` | Default config Hz |
| `channels` | `integer` or `nil` | Default config channel count |
| `sample_format` | `string` or `nil` | e.g. `F32`, `I16` |

**Index:** field-scripting uses the device’s native `index`. FieldAssist’s
`field.audio_devices.list` currently returns `{ index, name }` only
(1-based `index`).

---

## Nested types

<a id="collection"></a>

### collection

Region list returned by `composition.selection` or `composition:collection(name)`.

#### Properties

| Property | Access | Type | Description |
| --- | --- | --- | --- |
| `name` | **ro** | `string` | `"selection"` or user name |
| `regions` | **ro** | `{ region, … }` | Regions in the collection |

`#collection` is the region count.

#### Methods

*(none)*

<a id="region"></a>

### region
#### Properties

| Property | Access | Type | Description |
| --- | --- | --- | --- |
| `id` | **ro** | `integer` | Region id |
| `start` | **ro** | `integer` | Start sample |
| `stop` | **ro** | `integer` | End sample (inclusive span as stored) |
| `channels` | **ro** | `"all"` or `{ int, … }` | Channel mask |
| `label` | **ro** | `string` or `nil` | Optional label |
| `collection` | **ro** | `string` | Owning collection name |

#### Methods

*(none)*

<a id="marker"></a>

### marker
#### Properties

| Property | Access | Type | Description |
| --- | --- | --- | --- |
| `id` | **ro** | `integer` | Marker id |
| `frame` | **ro** | `integer` | Sample position |
| `sample` | **ro** | `integer` | Alias of `frame` |
| `type` | **ro** | `string` | Marker type name |
| `color` | **ro** | `{ r, g, b, a }` | Type color |
| `note` | **ro** | `string` or `nil` | Optional note |

#### Methods

| Method | Arguments | Returns | Description |
| --- | --- | --- | --- |
| `:remove` | — | `boolean` | Delete this marker |

---

<a id="migration-from-app"></a>

## Migration from `app:*`
| Old | New |
| --- | --- |
| `app.session` | `field.session.focused()` |
| `app:find_files` | `field.fs.find_files` |
| `app:define_layout` | `field.layouts.define` |
| `app:on` / `app:info` | `field.on` / `field.log.info` |
| `app:create_workflow` / `declare_workflow` | `field.workflow.create` / `declare` |
| `app:finish_workflow` / `cancel_workflow` | `field.workflow.finish` / `cancel` |
| `app.ui.*` | `field.ui.*` |
| `app:add_media` | `field.media.shared_pool():add` |

There are **no** long-term `app.session` / `app:find_files` compatibility
aliases. Branch on `app.name` for host-specific chrome (`app.command`,
`app.theme`, …).
