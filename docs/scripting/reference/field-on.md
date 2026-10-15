# `field.on`

<!-- markdownlint-disable MD013 MD060 -->
Registers process-wide model/host hooks. Unknown event names error.

## field.on Properties
(none)

## field.on Functions
| Function | Arguments                             | Returns | Description               |
| -------- | ------------------------------------- | ------- | ------------------------- |
| `on`     | `event: string`, `callback: function` | —       | Append a hook for `event` |

## field.on Events
| Event                  | Callback arguments                          | Notes                                                                         |
| ---------------------- | ------------------------------------------- | ----------------------------------------------------------------------------- |
| `loaded`               | `composition`, `elapsed`                    | `elapsed` is seconds since open started                                       |
| `saved`                | `composition`, `elapsed`                    |                                                                               |
| `enrich_composition`   | `composition`                               | Fired when a composition is **built from media**; mutate `c.variables` in place |
| `enrich_session`       | `session`                                   | Fired when an **empty session is created**; mutate `s.variables` / `s.properties` |
| `enrich_media`         | `media`                                     | Fired after probe on `field.media.open` / pool `:add(path)` and before composition enrich on media open; mutate `m.variables` / `m:bindings("source…")` |
| `detect_layout`        | `composition`, `chosen` → `string` or `nil` | `chosen` is the current layout name or `nil`; return a registered layout name |
| `session_loaded`       | `session`                                   |                                                                               |
| `session_saved`        | `session`                                   |                                                                               |
| `session_selected`     | `session`                                   |                                                                               |
| `composition_selected` | `composition` or `nil`                      |                                                                               |

`enrich_media` runs after probe fills `source` / `source.*` and before the
caller builds paths. Top-level `source` and script-created `source.*` scopes
are writable; probe tag scopes (`source.bwf`, `source.ixml`, …) stay
read-only. Embedded `init.lua` sets `source.event.name` to the parent folder
name. Order when opening media into a composition: `enrich_media`, then
`enrich_composition`, then `detect_layout`, then `loaded`.

```lua
field.on("enrich_media", function(m)
  local parent = m.url and m.url.parent
  local name = parent and parent.basename
  if name and name ~= "" then
    m:bindings("source.event").values.name = name
  end
end)
```

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
**instance** as the first argument (see [field.workflow](field-on.md)).

---
