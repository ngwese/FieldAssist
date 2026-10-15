# `field.variables`

<!-- markdownlint-disable MD013 MD060 -->
Scoped string variables, Bindings userdata, and user-extensible resolvers.
Product contract: [spec/SPEC-metadata.md](../../spec/SPEC-metadata.md).

## field.variables Functions
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

## Returned type: bindings
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

## Returned type: VariableResolver
Site-scoped lookup / template expand (detached snapshots of the site bindings).
Obtain via:

| Receiver | Site bindings |
| -------- | ------------- |
| `session:variable_resolver()` | `user.*` + `session` |
| `composition:variable_resolver()` | `source.*` + `user.*` + `session` + `composition` |
| `field.variables.user():variable_resolver()` | all live `user` / `user.*` scopes |

| Method | Arguments | Returns | Description |
| ------ | --------- | ------- | ----------- |
| `:bindings` | — | `{ bindings, … }` | Site Bindings array (same order as resolve / the Variables pane). Pass to `field.variables.flatten` or iterate `:scope()` / `:names()` / `values` |
| `:resolve` | `expr: string` [, `expand: boolean`] | `value, scope` or `nil, nil` | Bare leaf (`"title"`) or qualified (`"user.ingest.root_dir"`, `"env.HOME"`, `"source.basename"`). When `expand == true`, soft-expand `${…}` in the resolved value against this site before returning (same soft rules as `:expand`) |
| `:expand` | `template` [, `strict`] | `string` | Soft by default (unresolved `${…}` left as-is); `strict == true` errors like export |

```lua
local c = field.session.focused().composition
local r = c:variable_resolver()
local value, scope = r:resolve("TIMECODE_FLAG")
local value, scope = r:resolve("source.ixml.TIMECODE_FLAG")
local staging, scope = r:resolve("user.ingest.staging_dir", true)  -- expand value
local path = r:expand("${source.parent}/${title}.wav")
for _, b in ipairs(r:bindings()) do
  if b:scope() == "source" or b:scope():match("^source%.") then
    for _, name in ipairs(b:names()) do
      print(b:scope(), name, b.values[name])
    end
  end
end
```

`flatten` is the low-level bindings-list materializer (rename of the former
`field.variables.resolve`). Prefer site `:variable_resolver()` for lookups and
templates. Migration: `composition:resolve_variable(…)` →
`composition:variable_resolver():resolve(expr)`; `field.variables.resolve` →
`flatten`.

Probe / source Bindings used internally are read-only. Assignment to
`session.variables` / `composition.variables` still accepts map / `{ name, value }`
row tables.

## Resolver prototype
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
