# `field.session`

<!-- markdownlint-disable MD013 MD060 -->
Constructors and accessors for session objects (`.fasession` membership,
groups, properties, open compositions).

## field.session Properties
(none on the module table)

## field.session Functions
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

## Returned type: session
### session Properties
| Property        | Access | Type                                                            | Description                                    |
| --------------- | ------ | --------------------------------------------------------------- | ---------------------------------------------- |
| `id`            | **ro** | `string`                                                        | Session id                                     |
| `url`           | **ro** | url userdata or `nil`                                           | On-disk `.fasession` location                  |
| `workflow_name` | **rw** | `string` or `nil`                                               | Persisted stateful workflow name               |
| `capture_ui`    | **rw** | `boolean`                                                       | Whether UI chrome is captured with the session |
| `properties`    | **rw** | `{ [string] = string }`                                         | Workflow string map (not variables)            |
| `variables`     | **rw** | bindings userdata (see [field.variables](field-variables.md)); assign also accepts map / row-array tables | Session-scoped variables |
| `composition`   | **rw** | composition or `nil`                                            | Active document                                |
| `compositions`  | **ro** | `{ composition, … }`                                            | Session documents in order                     |
| `groups`        | **rw** | `{ string, … }`                                                 | Group name list                                |

### session Methods
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
| `:variable_resolver` | —                 | VariableResolver | Session site (`user.*` + session); see [field.variables](field-variables.md) |

---
