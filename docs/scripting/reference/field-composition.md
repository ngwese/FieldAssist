# `field.composition`

<!-- markdownlint-disable MD013 MD060 -->
Open compositions outside of (or in addition to) a session. Session membership
still uses `session:open`.

## field.composition Properties
(none on the module table)

## field.composition Functions
| Function | Arguments              | Returns     | Description             |
| -------- | ---------------------- | ----------- | ----------------------- |
| `open`   | `path` (string or url) | composition | Open audio or `.facomp` |

## Returned type: composition
### composition Properties
| Property       | Access | Type                                                            | Description                                                                      |
| -------------- | ------ | --------------------------------------------------------------- | -------------------------------------------------------------------------------- |
| `name`         | **rw** | `string`                                                        | Display name                                                                     |
| `url`          | **ro** | url userdata or `nil`                                           | Source / project location                                                        |
| `id`           | **ro** | `string`                                                        | Composition id                                                                   |
| `group`        | **rw** | `string` or `nil`                                               | Session group membership                                                         |
| `state`        | **rw** | `string` or `nil`                                               | Free-form document state                                                         |
| `properties`   | **rw** | `{ [string] = string }`                                         | Workflow string map on the session document                                      |
| `variables`    | **rw** | bindings userdata (see [field.variables](field-variables.md)); assign also accepts map / row-array tables | Composition-scoped variables (`.facomp`)                                         |
| `frames`       | **ro** | `integer`                                                       | Timeline length in samples                                                       |
| `sample_rate`  | **ro** | `integer`                                                       | Hz                                                                               |
| `channels`     | **ro** | `integer`                                                       | Channel count                                                                    |
| `selection`    | **rw** | [collection](nested-types.md) or selection table                    | Primary selection; set `nil`, a collection, or `{ kind, start, stop, channels }` |
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

### composition Methods
| Method                                                                      | Arguments                                                          | Returns                    | Description                                                                          |
| --------------------------------------------------------------------------- | ------------------------------------------------------------------ | -------------------------- | ------------------------------------------------------------------------------------ |
| `:select`                                                                   | `start`, `stop` [, `channels`]                                     | —                          | Select a half-open sample range                                                      |
| `:select_all`                                                               | —                                                                  | —                          | Select the whole timeline                                                            |
| `:clear_selection`                                                          | —                                                                  | —                          | Clear selection                                                                      |
| `:collection`                                                               | `name: string`                                                     | [collection](nested-types.md)  | Named region collection                                                              |
| `:add_region`                                                               | `{ start, stop, channels?, label?, collection? }`                  | [region](nested-types.md)          | Add a labeled region                                                                 |
| `:remove_region`                                                            | `id: integer`                                                      | `boolean`                  | Remove by id                                                                         |
| `:add_marker`                                                               | frame[, type] **or** `{ frame\|sample, type\|kind?, color?, note? }` | [marker](nested-types.md) or `nil` | Add a marker                                                                         |
| `:remove_marker`                                                            | marker or id                                                       | `boolean`                  | Remove one marker                                                                    |
| `:remove_marker_at`                                                         | `frame` [, `type`]                                                 | `boolean`                  | Remove at sample                                                                     |
| `:remove_marker_by_type`                                                    | `type: string`                                                     | `boolean`                  | Remove all of a type                                                                 |
| `:marker_at`                                                                | `frame` [, `type`]                                                 | marker or `nil`            | Lookup                                                                               |
| `:add_marker_type`                                                          | `name`, `color`                                                    | `boolean`                  | Register a type color                                                                |
| `:remove_marker_type`                                                       | `name`                                                             | `boolean`                  | Unregister a type                                                                    |
| `:save`                                                                     | —                                                                  | —                          | Persist `.facomp` (FieldAssist; headless stub)                                       |
| `:close`                                                                    | `[ { discard = bool } ]`                                           | —                          | Close the document; `discard = true` skips unsaved-change prompts (FieldAssist)      |
| `:replace`                                                                  | `path` (string or url)                                             | composition                | Replace media/project from path                                                      |
| `:undo` / `:redo`                                                           | —                                                                  | `boolean`                  | Edit history                                                                         |
| `:cut` / `:copy` / `:paste` / `:clear` / `:remove` / `:duplicate` / `:trim` | —                                                                  | `true`                     | Edit ops on the selection                                                            |
| `:export`                                                                   | profile name, profile userdata, or options table                   | `true`                     | Encode to disk (same pipeline as File → Export); see [field.exports](field-exports.md) |
| `:render`                                                                   | render-plan table (see below)                                      | `true`                     | Multi-output render: amortized source read, per-output optional offline DSP; blocks the host |
| `:begin_render`                                                             | render-plan table                                                  | [render job](field-composition.md) | Same render on a background thread; poll `job.finished` from a `:defer` loop |
| `:variable_resolver`                                                        | —                                                                  | VariableResolver           | Composition site lookup (Variables pane); see [field.variables](field-variables.md) |

**FieldAssist-only methods:** `:break_out_regions()` → composition or array;
`:break_out_channels(channels?)` → composition.

### composition:render / begin_render — render plan
Both `:render` and `:begin_render` accept a table `{ outputs = { … } }` where
each output entry is:

| Key       | Required | Type                                          | Description                                                                         |
| --------- | -------- | --------------------------------------------- | ----------------------------------------------------------------------------------- |
| `path`    | yes      | string or url                                 | Destination file path                                                               |
| `profile` | yes      | profile name, profile userdata, or options table | Encode profile (same resolution as `:export`)                                       |
| `chain`   | no       | `"foa"` \| `"foa_fuma"` \| `"ms"`            | Offline Faust mixdown chain; omit for identity encode                               |
| `params`  | no       | `{ [address] = number, … }`                   | Per-output DSP parameter overrides (Faust addresses); omit to use chain defaults    |

Parent directories are created automatically.  Destination existence is
re-checked immediately before each write; if a file appears between plan
construction and write, that output is skipped and reported in the results.
The source composition is read once; per-output DSP/encode then runs in
parallel.  Async jobs report progress in **source frames** (`job.done` /
`job.total`).

```lua
-- Identity + stereo AmbiX sidecar
doc:render({
  outputs = {
    { path = "/lib/event/take-ambix.flac", profile = "FLAC (Source Equivalent)" },
    { path = "/lib/event/take-st.flac",    profile = "FLAC (Source Equivalent)", chain = "foa" },
  },
})

-- Async variant (catalog use case)
local job = doc:begin_render({
  outputs = {
    { path = dest, profile = "FLAC (Source Equivalent)" },
  },
})
workflow:defer(function(wf)
  if job.finished then
    local results = job:results()  -- { { path, status, detail? }, … }
    -- …
  end
end)
```

### Catalog variables
| Variable                    | Scope   | Description                                                         |
| --------------------------- | ------- | ------------------------------------------------------------------- |
| `catalog.library_root`      | session | Library root for this session (overrides `user.catalog.library_root`) |
| `user.catalog.library_root` | user    | Default library root across all sessions                            |
| `catalog.export_profile`    | session | Default export profile name for this session                        |

---

## Returned type: render job
Returned by `composition:begin_render`.  Poll from a `:defer` loop.

### render job Properties
| Property   | Access | Type      | Description                                                                 |
| ---------- | ------ | --------- | --------------------------------------------------------------------------- |
| `done`     | **ro** | `integer` | Source frames completed (`0…total`; advances during parallel per-output work) |
| `total`    | **ro** | `integer` | Composition source frame count                                              |
| `finished` | **ro** | `boolean` | `true` when the worker has exited                                           |

### render job Methods
| Method      | Arguments | Returns                          | Description                                             |
| ----------- | --------- | -------------------------------- | ------------------------------------------------------- |
| `:progress` | —         | `done`, `total`                  | Same counters as the fields                             |
| `:error`    | —         | `string` or `nil`                | Hard error message after `finished`; `nil` on success   |
| `:join`     | —         | —                                | Block until done; raises if a hard error occurred       |
| `:results`  | —         | `{ { path, status, detail? }, … }` | Per-output results; `status` = `"ok"` \| `"skipped"` \| `"failed"` |

---
