# `field.workflow`

<!-- markdownlint-disable MD013 MD060 -->
Declare and run named workflow prototypes. Prototypes and instances are
**Lua tables** meant to be extended with methods (`:start`, `:suspend`, …).

## field.workflow Properties
(none)

## field.workflow Functions
| Function  | Arguments                                 | Returns                 | Description                                                               |
| --------- | ----------------------------------------- | ----------------------- | ------------------------------------------------------------------------- |
| `create`  | `properties: table`                       | prototype table         | Build an unregistered prototype                                           |
| `declare` | prototype **or** `(properties, function)` | —                       | Register by `name` (one-shot when the second form supplies only `:start`) |
| `run`     | `name` [, `payload`]                      | instance table or `nil` | Start a registered workflow. Default payload `{ scope = "run" }`.         |
| `finish`  | —                                         | —                       | End the active run; calls instance `:finish(session)` if present          |
| `cancel`  | —                                         | —                       | Cancel the active run; calls instance `:cancel(session)` if present       |
| `load_workflows` | —                                    | —                       | Load `workflow_*.lua` from the scripting Search Path                      |

## `create` / `declare` property table
| Key            | Required | Type            | Description                                                                           |
| -------------- | -------- | --------------- | ------------------------------------------------------------------------------------- |
| `name`         | yes      | `string`        | Registry key                                                                          |
| `display_name` | no       | `string`        | UI label (defaults to `name`)                                                         |
| `description`  | no       | `string`        | Short description                                                                     |
| `scopes`       | yes      | `{ string, … }` | e.g. `"drag-drop"`, `"menu"`, `"run"`                                                 |
| `drop`         | no       | table           | Overlay cell: `{ row`, `priority`, `color }` — defaults `row=1`, `priority=1.0`, gray |

## Returned type: workflow prototype / instance (table)
Installed methods (and optional host-called methods) apply to both the
prototype returned by `create` and instances produced by `run` / declare.

### Properties (installed)
| Property            | Access   | Type     | Description                                   |
| ------------------- | -------- | -------- | --------------------------------------------- |
| `__base_properties` | **ro**   | userdata | See below                                     |
| `__fa_toolbar`      | internal | table    | Snapshot consumed by the host toolbar         |
| user fields         | **rw**   | any      | Free-form instance state (e.g. `self.output`) |

`__base_properties` getters: `name`, `display_name`, `description`, `scopes`,
`row`, `priority`, `color` (**ro**).

### Methods (installed)
| Method          | Arguments                 | Returns         | Description                                                                         |
| --------------- | ------------------------- | --------------- | ----------------------------------------------------------------------------------- |
| `:name`         | —                         | `string`        | Prototype name                                                                      |
| `:display_name` | —                         | `string`        | Display name                                                                        |
| `:description`  | —                         | `string`        | Description                                                                         |
| `:scopes`       | —                         | `{ string, … }` | Scope list                                                                          |
| `:on`           | `event`, `handler`        | —               | Instance hook; handler gets `(self, …)` then the same args as [field.on](field-workflow.md) |
| `:set_toolbar`  | `{ control, … }` or `nil` | —               | Replace the toolbar from [field.ui](field-workflow.md) controls                             |
| `:set_item`     | `id`, `props`             | —               | Merge properties into a control by `id`                                             |
| `:set_sheet`    | `{ title?, panes, current? }` | —           | Open / replace a modal sheet (see below)                                            |
| `:set_pane`     | `id`, `{ text?, controls?, buttons? }` | —  | Merge into one sheet pane                                                           |
| `:go`           | `pane_id`                 | —               | Select the current sheet pane                                                       |
| `:close_sheet`  | —                         | —               | Dismiss the sheet; does not call `:finish` / `:cancel`                              |
| `:defer`        | `function`                | —               | Run `fn(self)` on a later UI frame                                                  |

### Sheet panes
`panes` is a list of `{ id, name, text?, controls?, buttons? }`. `name` is
the trail label. `text` is the read-only guide. `controls` are body
[field.ui](field-workflow.md) controls (including sheet-only `progress` / `log`).
`buttons` must be `field.ui.button` only (`align = "left"` for the left
cluster; omit `align` or use `"right"` for the right-justified primary
cluster). Control ids are unique across all panes.

### Optional user methods (host calls)
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
