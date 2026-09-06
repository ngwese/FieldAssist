# Scripting

FieldAssist embeds Lua 5.4. Scripts run in the **Script** panel (View → Show Script)
and from `init.lua`.

A default `init.lua` is baked into the executable. Dump it with
`FieldAssist --dump-init`. If a user file exists in the app config directory, that
file is loaded **instead** of the embedded default:

- macOS: `~/Library/Application Support/snd-review/init.lua`
- Windows: `%APPDATA%\snd-review\init.lua`
- Linux: `$XDG_CONFIG_HOME/snd-review/init.lua` (or `~/.config/snd-review/init.lua`)

`init.lua` is loaded once at startup. Register `loaded`, `detect_layout`, and
`saved` hooks there. The status bar (right side) shows the effective channel layout
and lets you pick one of the defined layouts; a pick is saved on the
composition.

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

There is no other host-provided global besides `app`. Open documents are reached
through `app.active` and `app.documents`.

## `app`

```lua
local c = app.active          -- composition, or nil if none is open
local all = app.documents     -- array of open compositions
local opened = app:open(path) -- open a file; returns the composition
app:command("edit.trim")      -- run a menu/keymap command by id
app:dofile("extra.lua")       -- execute a Lua file
app:info("layout", "stereo")  -- Messages tab (info / warn / error)
app:on("loaded", function(c, elapsed)
  app:info("load", string.format("%s in %.0f ms", c.name, elapsed * 1000))
end)
app:on("saved", function(c, elapsed)
  app:info("save", string.format("%s in %.0f ms", c.name, elapsed * 1000))
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

`app:on` accepts `"loaded"`, `"detect_layout"`, and `"saved"`. `loaded` is
`function(c, elapsed)` and `saved` is `function(c, elapsed)`, where `elapsed`
is wall-clock seconds for the load or save. `detect_layout` is
`function(c, chosen)` where `chosen` is the persisted user-explicit layout name
or `nil`. Return a defined layout name, or `nil` if it cannot be inferred.
Hooks run in registration order; the last non-nil valid name wins.

`define_layout` registers (or replaces) a named layout. Channel keys are
0-based. The default embedded script defines `mono`, `stereo`, `MS`, `1OA`,
and `2OA`. Optional `monitor = { chain = "stereo" }` (or `"mono"` / `"ms"` /
`"foa"`) is the default Monitor-tab DSP when that layout is chosen. Detecting
a layout only fills `c.monitor_chain` when it is still unset. An explicit
Monitor-tab or script assignment is saved on the `.facomp` and wins on reload.
Changing layout does not clear a custom `playback_channels` subset.

## Composition

`app.active` and each entry in `app.documents` is a **composition**. Edits,
markers, regions, and selection live on this object.

### Fields

| Field | Type | Notes |
| --- | --- | --- |
| `name` | string | Display title (file name when saved). |
| `path` | string or nil | Source or project path, if any. |
| `frames` | integer | Timeline length in samples. |
| `sample_rate` | integer | Hz. |
| `channels` | integer | Channel count. |
| `codec` | string or nil | First media codec, if any. |
| `bit_depth` | integer or nil | First media bit depth, if known. |
| `basename` | string or nil | File name when the source is file-backed. |
| `dirname` | string or nil | Parent directory when the source is file-backed. |
| `channel_layout` | string or nil | Effective layout name. Assign a defined name to choose it (saved); assign `nil` to clear. |
| `monitor_chain` | string or nil | Monitor DSP id: `"mono"`, `"stereo"`, `"ms"`, or `"foa"`. Saved on the composition. Assign `nil` for 1:1 device mapping. |
| `playback_channels` | array, `"all"`, or nil | 0-based composition channels fed to the monitor DSP, in file order. `nil` or `"all"` means every channel (the default). Extra channels are unused; missing DSP inputs are silence. |
| `duration` | number | Length in seconds. |
| `position` | integer or nil | Playhead / caret sample. Assignable. |
| `selection` | Collection | Session selection collection. Assign `nil` to clear. |
| `regions` | array of Region | Regions in the selection collection. |
| `collections` | array of strings | `"selection"` plus persisted collection names. |
| `markers` | array of Marker | User markers, ordered by frame then type. |
| `marker_types` | array of `{ name, color }` | Composition type registry. |

Reading `markers`, `regions`, `collections`, or `marker_types` returns a
snapshot table. Later adds/removes do not update a table you already hold;
read the field again. Region and Marker objects themselves are live.

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

**File:** `file.open`, `file.save`, `file.save_as`, `file.close`, `file.render`,
`file.quit`

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
