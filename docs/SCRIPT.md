# Scripting

FieldAssist embeds Lua 5.4. Scripts run in the **Script** panel (View → Show Script)
and from `init.lua`.

A default `init.lua` is baked into the executable. Dump it with
`FieldAssist --dump-init`. If a user file exists in the app config directory, that
file is loaded **instead** of the embedded default:

- macOS: `~/Library/Application Support/snd-review/init.lua`
- Windows: `%APPDATA%\snd-review\init.lua`
- Linux: `$XDG_CONFIG_HOME/snd-review/init.lua` (or `~/.config/snd-review/init.lua`)

`init.lua` is loaded once at startup, **after** the built-in workflow scripts
and **before** user `workflow_*.lua` files in the same config directory.
Register `loaded`, `detect_layout`, `saved`, `session_loaded`, and
`session_saved` hooks in `init.lua`. The status bar
(right side) shows the effective channel layout
and lets you pick one of the defined layouts; a pick is saved on the
composition. Output device selection is session-level: set `app.output_device` in
`init.lua` to pin a device across launches.

`print(...)` writes to the Script panel. `app:info`, `app:warn`, and `app:error`
write to the Messages tab (View → Show Script, then Messages). If stdout is a
terminal, those log lines are also printed in color. Standard Lua libraries are
available.

Times on the timeline are **sample indices** (frames), starting at `0`. Channel
indices are also 0-based.

## Globals

| Name | Description |
| --- | --- |
| `app` | The running application. Always present. |
| `print(...)` | Writes a line to the Script panel. |
| `app:info(topic, message)` | Info log to the Messages tab (and stdout if connected). |
| `app:warn(topic, message)` | Warning log; unseen warnings badge the Messages tab. |
| `app:error(topic, message)` | Error log; unseen errors badge the Messages tab. |
| `app:alert(subject, body)` | Modal dialog (same widget as a failed file open). |

There is no other host-provided global besides `app`. Open documents are reached
through `app.active`, `app.documents`, and `app.session`.

## `app`

```lua
local c = app.active          -- composition, or nil if none is open
local all = app.documents     -- array of open compositions
local s = app.session         -- the one active session (always present)
local opened = app:open(path) -- open a file; returns the composition
app.output_device = "Focusrite" -- substring match; nil = System Default
local names = app.output_devices -- live output device names
app:command("edit.trim")      -- run a menu/keymap command by id
app:dofile("extra.lua")       -- execute a Lua file
app:find_files(dir)           -- recursive files; paths relative to dir
app:find_files(dir, { "wav", "facomp" })
app:find_files(dir, function(dirname, basename) return true end)
app:info("layout", "stereo")  -- Messages tab (info / warn / error)
app:alert("Cannot replace", "Drop a single file.")
app:create_workflow({ name = "review", scopes = { "drag-drop" } })
app:declare_workflow({
  name = "add",
  display_name = "Add",       -- optional; defaults to name
  description = "Add to the session",
  scopes = { "drag-drop" },
  drop = {                    -- optional; used by drag-drop
    row = 1,                  -- default 1
    priority = 1,             -- default 1; sort key and width weight
    color = app.theme.semantic.success,
  },
}, function(payload)
  -- payload.scope ("drag-drop" or "menu"); payload.paths for drag-drop
end)
app:finish_workflow()
app:cancel_workflow()
app:on("loaded", function(c, elapsed)
  app:info("load", string.format("%s in %.0f ms", c.name, elapsed * 1000))
end)
app:on("saved", function(c, elapsed)
  app:info("save", string.format("%s in %.0f ms", c.name, elapsed * 1000))
end)
app:on("session_loaded", function(s)
  app:info("session", s.path)
end)
app:on("session_saved", function(s)
  app:info("session", "saved")
end)
app:define_layout({
  name = "stereo",
  description = "Left / Right",
  channels = { [0] = "L", [1] = "R" },
  monitor = { chain = "stereo" },  -- optional default monitor DSP
})
app:on("detect_layout", function(c, chosen)
  if chosen then
    return chosen
  end
  if c.channels == 1 then return "mono" end
end)
```

`app:command` uses the same ids as the keymap (`file.open`, `transport.play_pause`,
`selection.add_marker`, …). Unknown ids are an error. See [Commands](#commands).

`app:find_files(dir)` walks `dir` recursively and returns a list of file paths
**relative** to `dir`, using `/` as the separator. Directories and symlinks are
skipped. A non-directory `dir` is a Lua error. The optional second argument
filters the walk:

- a list of extensions (`"wav"` or `".WAV"`) keeps matching files
- a function `function(dirname, basename)` is called for each file;
  `dirname` is the parent relative to `dir` (empty at the root of the walk),
  `basename` is the file name. A truthy return keeps the file.

`app:on` accepts `"loaded"`, `"detect_layout"`, `"saved"`, `"session_loaded"`,
and `"session_saved"`. `loaded` is `function(c, elapsed)` and `saved` is
`function(c, elapsed)`, where `elapsed` is wall-clock seconds for the load or
save. `session_loaded` and `session_saved` are `function(s)` with the session
object. `detect_layout` is `function(c, chosen)` where `chosen` is the
persisted user-explicit layout name or `nil`. Return a defined layout name, or
`nil` if it cannot be inferred. Hooks run in registration order; the last
non-nil valid name wins.

`app.theme.named` and `app.theme.semantic` are the current GPUI theme colors as
`{ r, g, b, a }` tables (`0..1`). They can be passed to `drop.color` (and
anywhere else a color table is accepted). Values are read from the live theme
when the field is accessed, so `declare_workflow` snapshots the color at
registration. Unknown names are a Lua runtime error.

Named palette: `red`, `red_light`, `green`, `green_light`, `blue`,
`blue_light`, `yellow`, `yellow_light`, `magenta`, `magenta_light`, `cyan`,
`cyan_light`.

Semantic: `accent`, `accent_foreground`, `background`, `border`, `danger`,
`danger_active`, `danger_foreground`, `danger_hover`, `drop_target`,
`foreground`, `info`, `info_active`, `info_foreground`, `info_hover`, `input`,
`link`, `link_active`, `link_hover`, `muted`, `muted_foreground`, `popover`,
`popover_foreground`, `primary`, `primary_active`, `primary_foreground`,
`primary_hover`, `ring`, `secondary`, `secondary_active`,
`secondary_foreground`, `secondary_hover`, `selection`, `success`,
`success_active`, `success_foreground`, `success_hover`, `warning`,
`warning_active`, `warning_foreground`, `warning_hover`, `chart_1` … `chart_5`,
`chart_bullish`, `chart_bearish`.

## Session

`app.session` is always the UI-active session. Audio and `.facomp` files add
documents to it; `s:open` of a `.fasession` replaces it. File → Open and the
CLI use that same type-based behavior. Drag-drop onto the center waveform
uses Lua workflows instead (see [Workflows](#workflows)).

`s.workflow` is the host-assigned name of the running stateful workflow. It is
persisted in `.fasession` so the workflow can resume when the session is opened
again. Scripts should not set it to bind a workflow; the host writes it after
`:start` on a stateful prototype. One-shot handlers never bind. Assign `nil`
only when clearing an unknown name from a Keep/Clear prompt (the host APIs
`app:finish_workflow()` / `app:cancel_workflow()` are the normal way to unbind).

```lua
local s = app.session
print(s.id, s.path, s.workflow)
s.capture_ui = true
s.properties = { batch = "2026-09" }   -- string→string; nil values rejected
local all = s.documents        -- same composition objects as app.documents
local c = s.active             -- or nil
c = s:open(path)               -- audio/facomp → add; .fasession → replace
print(s:group_count("todo"))   -- documents with c.group == "todo"
s:save()
s:save_as(path)

local incoming = app:load_session(path)  -- .fasession; does not replace s
for _, doc in ipairs(incoming.documents) do
  print(doc.path)
end
incoming:close()               -- drop a Lua-held session; not the active one
print(#app.sessions)           -- active session plus any loaded sessions
```

`s.id` is the session UUID (read-only). `s.path` is the `.fasession` file, or
`nil` until it is saved. `s:group_count(name)` is the number of documents whose
`group` equals `name`. `app.active`, `app.documents`, and `app:open` remain
aliases of `s.active`, `s.documents`, and `s:open`. `app:load_session` holds
another session in memory for scripts (document `path` / `id` / `name` work;
edits require the composition to be open in the UI). `open` / `save` /
`save_as` are only available on the active session.

## Workflows

Workflows are Lua files named `workflow_<name>.lua`. Built-in Add, Replace, and
Review are embedded. At startup FieldAssist loads:

1. Embedded `workflow_add.lua`, `workflow_replace.lua`, and
   `workflow_review.lua`
2. `init.lua` (user file if present, otherwise the embedded default)
3. User `workflow_*.lua` next to `init.lua` (sorted by filename)

`app:create_workflow(properties)` builds a prototype table (`name`,
`display_name`, `description`, `scopes`, `drop`) but does not register it.
`app:declare_workflow(prototype)` registers it (a later declaration with the
same `name` replaces the previous one). `app:declare_workflow(properties, func)`
is shorthand: create, set `:start` to `func` (payload only, no `self`), and
declare. Add and Replace use the shorthand; they are one-shot.

A prototype is **stateful** when it defines `:suspend` or `:resume`. After
`:start` returns, the host binds `s.workflow` to that name (one running stateful
workflow per active session). One-shot handlers never bind and never show a
toolbar.

```lua
local Review = app:create_workflow({
  name = "review",
  display_name = "Review",
  scopes = { "drag-drop", "menu" },
  drop = { row = 2, priority = 1, color = app.theme.semantic.info },
})

function Review:start(payload) ... end
function Review:suspend(session)   -- persist on `session.properties`; return false to abort save/quit
  return true
end
function Review:resume(session) ... end
function Review:cancel(session) ... end   -- optional
function Review:finish(session) ... end   -- optional

Review:on("command", function(command) ... end)
Review:set_toolbar({
  { command = "next", label = "Next" },
  { command = "skip", label = "Skip" },
  { command = "finish", label = "Finish" },
})

app:declare_workflow(Review)
```

`:start` receives `{ scope = "drag-drop", paths = { ... } }` from the drop
overlay, or `{ scope = "menu" }` (no `paths`) from the Workflow menu. The
one-shot shorthand uses the same payload. `:on("command", handler)` registers a
workflow-local command callback (last registration wins). These strings are not
global `app:command` / keymap ids. `:set_toolbar(items)` or `:set_toolbar(nil)`
may be called from `start`, `resume`, or a command handler. Missing optional
methods are no-ops.

`app:finish_workflow()` / `app:cancel_workflow()` end the run: the host calls
`:finish(session)` or `:cancel(session)` if defined, clears `s.workflow`, and
hides the toolbar.

Drop overlay still lists every declared workflow whose `scopes` include
`"drag-drop"`. One-shot vs stateful is not a layout distinction. Dropping the
**same** running workflow calls `:start` again. Dropping a **different**
stateful workflow while one is running shows `app:alert` and does not start.
One-shot Add/Replace may still run while Review is active.

When files are dragged over the center waveform (or the empty editor pane),
drag-drop workflows are collected, sorted, and laid out **once** for that
gesture. Rows use `drop.row` (missing row numbers are not padded). Within a
row, higher `drop.priority` is first (left); ties sort by `display_name`.
Box width is `priority / sum(priorities in the row)`. Declaring a workflow
during a drag does not reshape the overlay; the next drag rebuilds from the
live registry.

The **Workflow** menu sits after View. **Cancel** is first and is disabled when
no workflow is running on the session. A divider follows, then one item per
workflow whose `scopes` include `"menu"`, labeled with `display_name` and
sorted alphabetically. Choosing an item starts that workflow with
`scope == "menu"` and no `payload.paths`.

Use `app:alert(subject, body)` for a user-visible error. Uncaught Lua errors go
to the Script panel. Drop colors may be `{ r, g, b, a }` or a value from
`app.theme.named` / `app.theme.semantic`.

The host calls `:suspend(session)` before File → Save Session / Save Session As,
and during quit or open-session after compositions are clean (before the unsaved
session prompt). Return `false` to abort that save or quit (the host alerts).
A Lua error in `:suspend` is reported in the Script panel and also aborts.

`:resume(session)` runs after a `.fasession` is installed as the UI session when
`s.workflow` names a declared stateful prototype. An unknown or one-shot name
logs an error, shows no toolbar, and prompts Keep (leave `s.workflow`) or Clear
(unbind the name only; no `:cancel` / `:finish`; other `s.properties` stay).
Replacing the active session cancels a running outgoing workflow, then resumes
the incoming session’s workflow if any.

A workflow bar sits between the dock and the status bar while toolbar items are
set. It shows the workflow `display_name` and the script buttons.

Built-in **Add** opens audio/`.facomp` into the current session. A dropped
`.fasession` is loaded with `app:load_session` and each document whose path is
not already in the current session is opened (existing path wins). Built-in
**Replace** opens a `.fasession` as a session replace, or calls
`c:replace(path)` on the active document for audio/`.facomp`. Built-in
**Review** is a stateful mock: Workflow → Review (`scope == "menu"`) marks every
open session document `"todo"`. Dropped files are kept as-is, dropped folders
are expanded with `app:find_files` to readable audio/`.facomp` files, each
opened document is placed in the `"todo"` group, and the toolbar offers Next /
Skip / Finish. Finish warns if any session documents are still in `"todo"`.

`app.output_device` is the session output device name, or `nil` for System
Default (the host default device). Assignment uses the same name, index, and
substring matching as `--output`. Unknown names are a Lua runtime error. The
value is not saved on the `.facomp`; persist it from `init.lua`.
`app.output_devices` is a read-only array of current device names.

`define_layout` registers (or replaces) a named layout. Channel keys are
0-based. The default embedded script defines `mono`, `stereo`, `MS`,
`B-Format (AmbiX)`, `B-Format (FuMa)`, and `2OA`. Optional
`monitor = { chain = "stereo" }` (or `"mono"` / `"ms"` / `"foa"` /
`"foa_fuma"`) is the default Monitor-tab DSP when that layout is chosen.
Detecting a layout only fills `c.monitor_chain` when it is still unset. An
explicit Monitor-tab or script assignment is saved on the `.facomp` and wins
on reload. Changing layout does not clear a custom `playback_channels` subset.

After honoring a persisted `chosen` name (`"1OA"` is remapped to
`B-Format (AmbiX)`), detection uses the file `basename` (including
extension) and channel count:

1. `n >= 4` and basename contains `"fuma"` (case-insensitive) →
   `B-Format (FuMa)`
2. Else `n >= 4` and basename contains `"ambix"` → `B-Format (AmbiX)`
3. Else count fallbacks: `1 → mono`, `2 → stereo`, `4 → B-Format (AmbiX)`,
   `9 → 2OA`

If both keywords appear, FuMa wins. A file with more than four channels and
an Ambix/FuMa name still gets the four-channel B-format layout; extra
channels are unlabeled.

## Composition

`app.active` and each entry in `app.documents` is a **composition**. Edits,
markers, regions, and selection live on this object.

### Fields

| Field | Type | Notes |
| --- | --- | --- |
| `name` | string | Display title (file name when saved). |
| `id` | string | Document UUID in the session (read-only). |
| `path` | string or nil | Source or project path, if any. |
| `group` | string or nil | Session grouping label. Assign `nil` to clear. |
| `state` | string or nil | Session workflow state. Assign `nil` to clear. |
| `properties` | table | String→string map on the session (not saved in `.facomp`). |
| `frames` | integer | Timeline length in samples. |
| `sample_rate` | integer | Hz. |
| `channels` | integer | Channel count. |
| `codec` | string or nil | First media codec, if any. |
| `bit_depth` | integer or nil | First media bit depth, if known. |
| `basename` | string or nil | File name when the source is file-backed. |
| `dirname` | string or nil | Parent directory when the source is file-backed. |
| `channel_layout` | string or nil | Effective layout name. Assign a defined name to choose it (saved); assign `nil` to clear. |
| `monitor_chain` | string or nil | Monitor DSP id: `"mono"`, `"stereo"`, `"ms"`, `"foa"`, or `"foa_fuma"`. Saved on the composition. Assign `nil` for 1:1 device mapping. |
| `playback_channels` | array, `"all"`, or nil | 0-based composition channels fed to the monitor DSP, in file order. `nil` or `"all"` means every channel (the default). Extra channels are unused; missing DSP inputs are silence. |
| `duration` | number | Length in seconds. |
| `position` | integer or nil | Playhead / caret sample. Assignable. |
| `selection` | Collection | Session selection collection. Assign `nil` to clear. |
| `regions` | array of Region | Regions in the selection collection. |
| `collections` | array of strings | `"selection"` plus persisted collection names. |
| `markers` | array of Marker | User markers, ordered by frame then type. |
| `marker_types` | array of `{ name, color }` | Composition type registry. |

Reading `markers`, `regions`, `collections`, `marker_types`, or `properties`
returns a snapshot table. Later adds/removes do not update a table you already
hold; read the field again. Region and Marker objects themselves are live.

```lua
c:save()   -- File → Save for this composition
c:close()  -- File → Close (prompts if unsaved)
c:replace(path)  -- reload this document from audio/.facomp (prompts if unsaved)
```

### Selection methods

```lua
c:select(start, stop)              -- replace selection with one region
c:select(start, stop, {0, 1})      -- specific channels
c:select_all()
c:clear_selection()
c.position = 44100
c.selection = { kind = "region", start = 0, stop = 100 }
c.selection = nil                  -- same as clear_selection()
```

`c.selection` is a **collection**. Iterate `c.selection.regions` (or `c.regions`).

### Collections and regions

Named collections persist in the `.facomp`. `"selection"` is session state and
is not saved.

```lua
local r = c:add_region({
  start = 0,
  stop = 44100,
  label = "intro",      -- optional
  channels = "all",     -- optional; "all" or {0, 1, ...}
  collection = "cues",  -- optional; default "selection"
})
print(r.id, r.start, r.stop, r.label, r.collection)
c:remove_region(r.id)

local silent = c:collection("silent")  -- get or create
for _, region in ipairs(silent.regions) do
  print(region.start, region.stop)
end
```

### Markers

A **marker type** is a named definition (`name` + `color`). A **marker** is an
instance of a type (`frame`, `type`, `note`). Color lives on the type, not the
instance; `marker.color` reads the type registry.

One marker of a given type may exist at a given frame. A second insert of the
same type at that frame is ignored and returns `nil`. Different types may share
a frame.

Built-in types: `"Blue"`, `"Yellow"`, `"Purple"`. The default type is `"Blue"`.
Custom types are stored on the composition.

```lua
c:add_marker_type("Red", {1, 0, 0, 1})  -- color required for a new name
c:remove_marker_type("Red")             -- also deletes markers of that type

-- table form
local m = c:add_marker({
  frame = 1000,           -- or sample = 1000
  type = "Yellow",        -- or kind; default "Blue"
  note = "door slam",     -- optional
  color = {1, 0, 0, 1},   -- optional; registers a new type if the name is unknown
})

-- positional form
c:add_marker(1000)             -- Blue at sample 1000
c:add_marker(1000, "Purple")

-- iterate
for _, marker in ipairs(c.markers) do
  print(marker.id, marker.frame, marker.type, marker.note)
end

-- lookup
local yellow = c:marker_at(1000, "Yellow")
local any = c:marker_at(1000)  -- first marker at that frame

-- delete
c:remove_marker(m)             -- marker object or integer id
c:remove_marker(m.id)
m:remove()
c:remove_marker_at(1000)           -- every type at that frame
c:remove_marker_at(1000, "Blue")   -- one type
```

`add_marker` returns the new Marker, or `nil` if that type already occupies the
frame.

### Timeline edits

These apply to every region in the current **selection** collection, same as
the Edit menu:

| Method | Effect |
| --- | --- |
| `c:undo()` / `c:redo()` | History. `undo`/`redo` return whether a step ran. |
| `c:cut()` / `c:copy()` / `c:paste()` | Clipboard. |
| `c:clear()` | Silence each selected span (keep length). |
| `c:remove()` | Cut selected spans out (timeline shrinks). |
| `c:duplicate()` | Duplicate each selected span in place. |
| `c:trim()` | Keep selected spans concatenated; discard gaps. |

## Collection

| Field | Type |
| --- | --- |
| `name` | string (`"selection"` or a persisted name) |
| `regions` | array of Region |

`#collection` is the number of regions.

Assigning a named collection to `c.selection` copies its regions into the
session selection.

## Region

| Field | Type |
| --- | --- |
| `id` | integer |
| `start` | integer (sample) |
| `stop` | integer (sample, inclusive) |
| `channels` | `"all"` or array of integers |
| `label` | string or nil |
| `collection` | string |

Fields are live: they read the current document. A region that has been removed
errors if you access its fields.

## Marker

| Field | Type |
| --- | --- |
| `id` | integer |
| `frame` | integer (sample). Alias: `sample`. |
| `type` | string (e.g. `"Blue"`) |
| `color` | array of four numbers `r, g, b, a` in `0..1` (from the type registry) |
| `note` | string or nil |

| Method | Effect |
| --- | --- |
| `marker:remove()` | Delete this marker. Returns whether it still existed. |

Fields are live against the composition. After a successful `remove`, further
field access errors.

## Channels

Anywhere a channel scope is accepted (`select`, `add_region`):

- omit the argument, pass `nil`, or pass `"all"` — every channel
- pass `{0}` or `{0, 1}` — those channel indices

## Commands

`app:command(id)` runs the same actions as menus and key bindings:

**File:** `file.open`, `file.save`, `file.save_as`, `file.save_session`,
`file.save_session_as`, `file.close`, `file.render`, `file.quit`

**Help:** `help.about`

**View:** `view.fit_all`, `view.frame`, `view.zoom_in`, `view.zoom_out`,
`view.explorer`, `view.detail`, `view.script`

**Transport:** `transport.home`, `transport.previous`, `transport.start`,
`transport.play_pause`, `transport.stop`, `transport.next`, `transport.end`,
`transport.loop`

**Edit:** `edit.undo`, `edit.redo`, `edit.cut`, `edit.copy`, `edit.paste`,
`edit.clear`, `edit.remove`, `edit.duplicate`, `edit.trim`

**Selection / markers:** `selection.select_all`, `selection.select_none`,
`selection.invert`, `selection.marker_type_blue`, `selection.marker_type_yellow`,
`selection.marker_type_purple`, `selection.snap_to_marker`,
`selection.add_at_hover`, `selection.add_marker`, `selection.delete_marker`

Marker commands use the active marker type and the Add at Hover setting from
the Selection menu. Prefer `c:add_marker` / `c:remove_marker` when the script
should choose the frame and type itself.

## Example

```lua
local c = app.active
if not c then
  return
end

c:select(0, c.frames - 1)
local intro = c:add_marker({
  frame = 0,
  type = "Blue",
  note = "start",
})
print("marker", intro.id, "at", intro.frame)

for _, marker in ipairs(c.markers) do
  print(marker.type, marker.frame, marker.note)
end
```
