# `field.include`

<!-- markdownlint-disable MD013 MD060 -->
Load and run another Lua chunk relative to the caller (replaces
`debug.getinfo` + `dofile`).

## field.include Properties
(none)

## field.include Functions
| Function  | Arguments | Returns               | Description                 |
| --------- | --------- | --------------------- | --------------------------- |
| `include` | `spec`    | chunk return value(s) | Load and execute a Lua file |

`spec` **(field-scripting):** path `string` or [field.url](field-include.md) userdata.
Search: absolute path → directory of the calling file → config dir → relative
to cwd. Results are cached by resolved path while the host lives.

`spec` **(FieldAssist today):** path `string` only; `lua.load` of that path
(no URL, no shared include cache).

---
